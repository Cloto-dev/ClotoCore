use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;

use cloto_shared::{ClotoId, Permission, Plugin, PluginManifest};

// Kernel-native tools are identified solely by the `mgp.` / `gui.` prefix.
// Closes bug-287: the previous hardcoded allowlist that named individual
// tool symbols (e.g. `create_mcp_server`) leaked the kernel's tool surface
// into the registry layer. `create_mcp_server` has been renamed to
// `mgp.kernel.create_mcp_server` so it is dispatched by prefix like every
// other kernel-native tool.

#[derive(sqlx::FromRow, Debug)]
pub struct PluginSetting {
    pub plugin_id: String,
    pub is_active: bool,
    pub allowed_permissions: sqlx::types::Json<Vec<Permission>>,
}

/// G1.3: Unified registry state — single RwLock avoids fragmented locking.
pub struct RegistryState {
    pub plugins: HashMap<String, Arc<dyn Plugin>>,
    pub effective_permissions: HashMap<ClotoId, Vec<Permission>>,
}

pub struct PluginRegistry {
    pub state: tokio::sync::RwLock<RegistryState>,
    pub event_timeout_secs: u64,
    pub max_event_depth: u8,
    pub event_semaphore: Arc<tokio::sync::Semaphore>,
    /// MCP Client Manager for dual dispatch (Rust plugins + MCP servers)
    pub mcp_manager: Option<Arc<super::McpClientManager>>,
}

pub struct SystemMetrics {
    pub total_requests: std::sync::atomic::AtomicU64,
    pub total_memories: std::sync::atomic::AtomicU64,
    pub total_episodes: std::sync::atomic::AtomicU64,
}

impl Default for SystemMetrics {
    fn default() -> Self {
        Self {
            total_requests: std::sync::atomic::AtomicU64::new(0),
            total_memories: std::sync::atomic::AtomicU64::new(0),
            total_episodes: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl SystemMetrics {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

fn rust_tool_schema(tool: &dyn cloto_shared::Tool) -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": tool.name(),
            "description": tool.description(),
            "parameters": tool.parameters_schema(),
        }
    })
}

impl PluginRegistry {
    #[must_use]
    pub fn new(
        event_timeout_secs: u64,
        max_event_depth: u8,
        event_concurrency_limit: usize,
    ) -> Self {
        Self {
            state: tokio::sync::RwLock::new(RegistryState {
                plugins: HashMap::new(),
                effective_permissions: HashMap::new(),
            }),
            event_timeout_secs,
            max_event_depth,
            event_semaphore: Arc::new(tokio::sync::Semaphore::new(event_concurrency_limit)),
            mcp_manager: None,
        }
    }

    /// Set the MCP Client Manager for dual dispatch.
    pub fn set_mcp_manager(&mut self, mcp_manager: Arc<super::McpClientManager>) {
        self.mcp_manager = Some(mcp_manager);
    }

    pub async fn update_effective_permissions(&self, plugin_id: ClotoId, permission: Permission) {
        let mut state = self.state.write().await;
        let perms = state.effective_permissions.entry(plugin_id).or_default();
        if !perms.contains(&permission) {
            perms.push(permission);
        }
    }

    pub async fn list_plugins(&self) -> Vec<PluginManifest> {
        let state = self.state.read().await;
        state.plugins.values().map(|p| p.manifest()).collect()
    }

    pub async fn get_engine(&self, id: &str) -> Option<Arc<dyn Plugin>> {
        let state = self.state.read().await;
        state.plugins.get(id).cloned()
    }

    pub async fn find_memory(&self) -> Option<Arc<dyn Plugin>> {
        let state = self.state.read().await;
        for plugin in state.plugins.values() {
            if plugin.as_memory().is_some() {
                return Some(plugin.clone());
            }
        }
        None
    }

    /// Collect tool schemas from all active Tool plugins + MCP servers (OpenAI function calling format).
    pub async fn collect_tool_schemas(&self) -> Vec<serde_json::Value> {
        let mut schemas: Vec<serde_json::Value> = {
            let state = self.state.read().await;
            state
                .plugins
                .values()
                .filter_map(|p| Some(rust_tool_schema(p.as_tool()?)))
                .collect()
        };

        // Dual Dispatch: also collect from MCP servers
        if let Some(ref mcp) = self.mcp_manager {
            schemas.extend(mcp.collect_tool_schemas().await);
        }

        schemas
    }

    /// Collect tool schemas filtered to a specific agent's allowed plugin set.
    pub async fn collect_tool_schemas_for(
        &self,
        allowed_plugin_ids: &[String],
    ) -> Vec<serde_json::Value> {
        let mut schemas: Vec<serde_json::Value> = {
            let state = self.state.read().await;
            state
                .plugins
                .iter()
                .filter_map(|(id, p)| {
                    if !allowed_plugin_ids.contains(id) {
                        return None;
                    }
                    Some(rust_tool_schema(p.as_tool()?))
                })
                .collect()
        };

        // Dual Dispatch: also collect from MCP servers matching allowed IDs
        if let Some(ref mcp) = self.mcp_manager {
            schemas.extend(mcp.collect_tool_schemas_for(allowed_plugin_ids).await);
        }

        schemas
    }

    /// Execute a tool by name with the given arguments.
    /// H-01: Drops the read lock before calling tool.execute() to avoid blocking
    /// plugin registration during long-running tool execution.
    /// Dual Dispatch: tries Rust plugins first, then falls back to MCP servers.
    pub async fn execute_tool(
        &self,
        caller: &crate::managers::Caller,
        tool_name: &str,
        args: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, cloto_shared::ToolFailure> {
        // 1. Try Rust plugins first
        let tool_plugin = {
            let state = self.state.read().await;
            state.plugins.values().find_map(|p| {
                let tool = p.as_tool()?;
                if tool.name() == tool_name {
                    Some(p.clone())
                } else {
                    None
                }
            })
        }; // read lock dropped here
        if let Some(plugin) = tool_plugin {
            if let Some(tool) = plugin.as_tool() {
                return tool.execute(args).await.map_err(Into::into);
            }
        }

        // 2. Fall back to MCP servers (gated by the central capability gate).
        if let Some(ref mcp) = self.mcp_manager {
            return mcp.execute_tool_internal(caller, tool_name, args).await;
        }

        Err(anyhow::anyhow!("Tool '{}' not found", tool_name).into())
    }

    /// Execute a tool by name, only if it belongs to the agent's allowed plugin set.
    /// Dual Dispatch: tries Rust plugins first, then falls back to MCP servers.
    pub async fn execute_tool_for(
        &self,
        caller: &crate::managers::Caller,
        allowed_plugin_ids: &[String],
        tool_name: &str,
        args: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, cloto_shared::ToolFailure> {
        // 1. Try Rust plugins first
        let tool_plugin = {
            let state = self.state.read().await;
            state.plugins.iter().find_map(|(id, p)| {
                if !allowed_plugin_ids.contains(id) {
                    return None;
                }
                let tool = p.as_tool()?;
                if tool.name() == tool_name {
                    Some(p.clone())
                } else {
                    None
                }
            })
        }; // read lock dropped here
        if let Some(plugin) = tool_plugin {
            if let Some(tool) = plugin.as_tool() {
                return tool.execute(args).await.map_err(Into::into);
            }
        }

        // 2. Fall back to MCP servers (if allowed)
        if let Some(ref mcp) = self.mcp_manager {
            // Check if any allowed ID matches an MCP server that provides this tool
            let mcp_schemas = mcp.collect_tool_schemas_for(allowed_plugin_ids).await;
            let has_tool = mcp_schemas.iter().any(|s| {
                s.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    == Some(tool_name)
            });
            if has_tool {
                return mcp.execute_tool_internal(caller, tool_name, args).await;
            }
        }

        Err(anyhow::anyhow!(
            "Tool '{}' not found or not available for this agent",
            tool_name
        )
        .into())
    }

    /// Collect tool schemas for a specific agent.
    /// Rust plugins: filtered by `allowed_plugin_ids` (same as `collect_tool_schemas_for`).
    /// MCP tools: filtered by `resolve_tool_access()` (3-level priority resolution).
    pub async fn collect_tool_schemas_for_agent(
        &self,
        allowed_plugin_ids: &[String],
        agent_id: &str,
    ) -> Vec<serde_json::Value> {
        let mut schemas: Vec<serde_json::Value> = {
            let state = self.state.read().await;
            state
                .plugins
                .iter()
                .filter_map(|(id, p)| {
                    if !allowed_plugin_ids.contains(id) {
                        return None;
                    }
                    Some(rust_tool_schema(p.as_tool()?))
                })
                .collect()
        };

        // MCP tools: resolve_tool_access per-tool
        if let Some(ref mcp) = self.mcp_manager {
            schemas.extend(mcp.collect_tool_schemas_for_agent(agent_id).await);
        }

        schemas
    }

    /// Execute a tool for a specific agent with access control.
    /// Rust plugins: checked against `allowed_plugin_ids`.
    /// MCP tools: checked via `resolve_tool_access()`.
    pub async fn execute_tool_for_agent(
        &self,
        allowed_plugin_ids: &[String],
        agent_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, cloto_shared::ToolFailure> {
        // 1. Try Rust plugins first (same gate as execute_tool_for)
        let tool_plugin = {
            let state = self.state.read().await;
            state.plugins.iter().find_map(|(id, p)| {
                if !allowed_plugin_ids.contains(id) {
                    return None;
                }
                let tool = p.as_tool()?;
                if tool.name() == tool_name {
                    Some(p.clone())
                } else {
                    None
                }
            })
        }; // read lock dropped here
        if let Some(plugin) = tool_plugin {
            if let Some(tool) = plugin.as_tool() {
                return tool.execute(args).await.map_err(Into::into);
            }
        }

        // 2. MCP servers: the per-agent capability gate — including the
        //    kernel-native Deny-only RBAC — is now enforced centrally inside
        //    `execute_tool_internal` (bug-421, PATH 2). No inline
        //    `check_tool_access` here: "shown" (presentation-layer filtering)
        //    and "allowed" (enforcement) are decoupled, single-sourced at the
        //    chokepoint.
        if let Some(ref mcp) = self.mcp_manager {
            return mcp
                .execute_tool_internal(
                    &crate::managers::Caller::Agent(agent_id.to_string()),
                    tool_name,
                    args,
                )
                .await;
        }

        Err(anyhow::anyhow!(
            "Tool '{}' not found or not available for this agent",
            tool_name
        )
        .into())
    }

    /// Deliver an event to every active plugin.
    pub async fn dispatch_event(
        &self,
        envelope: crate::EnvelopedEvent,
        event_tx: &tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
    ) {
        let event = envelope.event.clone();
        let current_depth = envelope.depth;

        // Warn on deep cascades before they hit the hard limit
        if current_depth > 5 {
            tracing::warn!(
                event_type = ?event,
                depth = current_depth,
                max = self.max_event_depth,
                "Event cascade depth exceeds warning threshold (>5)"
            );
        }

        // Guard against cascade explosion (Guardrail #2).
        if current_depth >= self.max_event_depth {
            error!(
                event_type = ?event,
                depth = current_depth,
                "🛑 Event cascading limit reached ({}). Dropping event to prevent infinite loop.",
                self.max_event_depth
            );
            return;
        }

        let state = self.state.read().await;

        use futures::stream::{FuturesUnordered, StreamExt};
        use futures::FutureExt;
        let mut futures = FuturesUnordered::new();

        for (id, plugin) in &state.plugins {
            let plugin = plugin.clone();
            let event = event.clone();
            let id = id.clone();
            let timeout_duration = std::time::Duration::from_secs(self.event_timeout_secs);
            let semaphore = self.event_semaphore.clone();

            futures.push(tokio::spawn(async move {
                let Ok(_permit) = semaphore.acquire().await else {
                    tracing::warn!("Semaphore closed during shutdown, skipping plugin {}", id);
                    return (id, Ok(Ok(None)));
                };
                // Catch panics to prevent semaphore permit leaks
                let result = tokio::time::timeout(timeout_duration, async {
                    match std::panic::AssertUnwindSafe(plugin.on_event(&event))
                        .catch_unwind()
                        .await
                    {
                        Ok(r) => r,
                        Err(_) => Err(anyhow::anyhow!("Plugin panicked during on_event")),
                    }
                })
                .await;
                // _permit dropped here automatically (even on panic path above)
                (id, result)
            }));
        }

        // Release the lock early.
        drop(state);

        // Process results in completion order.
        while let Some(join_result) = futures.next().await {
            let (id, timeout_result) = match join_result {
                Ok(pair) => pair,
                Err(e) => {
                    error!("🔥 Plugin task PANICKED or was cancelled: {}", e);
                    continue;
                }
            };

            match timeout_result {
                Ok(Ok(Some(new_event_data))) => {
                    let tx = event_tx.clone();
                    let id_clone = id.clone();
                    let trace_id = event.trace_id;
                    let semaphore = self.event_semaphore.clone();
                    tokio::spawn(redispatch_plugin_event(
                        tx,
                        id_clone,
                        trace_id,
                        new_event_data,
                        current_depth,
                        semaphore,
                    ));
                }
                Ok(Ok(None)) => {}
                Ok(Err(e)) => {
                    error!("🔌 Plugin {} on_event error: {}", id, e);
                }
                Err(_) => {
                    error!("⏱️ Plugin {} timed out during event processing", id);
                }
            }
        }
    }
}

/// Helper function to re-dispatch plugin events asynchronously
async fn redispatch_plugin_event(
    tx: tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
    plugin_id: String,
    trace_id: ClotoId,
    new_event_data: cloto_shared::ClotoEventData,
    current_depth: u8,
    semaphore: Arc<tokio::sync::Semaphore>,
) {
    let Ok(_permit) = semaphore.acquire().await else {
        tracing::warn!(
            "Semaphore closed during shutdown, skipping redispatch for {}",
            plugin_id
        );
        return;
    };
    let issuer_id = ClotoId::from_name(&plugin_id);
    let envelope = crate::EnvelopedEvent {
        event: Arc::new(cloto_shared::ClotoEvent::with_trace(
            trace_id,
            new_event_data,
        )),
        issuer: Some(issuer_id),
        correlation_id: Some(trace_id),
        depth: current_depth + 1,
    };
    if let Err(e) = tx.send(envelope).await {
        error!("🔌 Failed to re-dispatch plugin event: {}", e);
    }
}

#[cfg(test)]
mod tests {
    //! Characterization tests for the plugin registry.
    //!
    //! Registration is not an API here — callers insert directly into
    //! `state.plugins`. These tests pin what that surface actually does today,
    //! plus what the three constructor arguments (`event_timeout_secs`,
    //! `max_event_depth`, `event_concurrency_limit`) enforce.

    use super::*;
    use cloto_shared::{ClotoEvent, ClotoEventData, PluginCast, ServiceType, Tool};
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    fn manifest_for(id: &str) -> PluginManifest {
        PluginManifest {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            version: "1.0".to_string(),
            category: cloto_shared::PluginCategory::Tool,
            service_type: ServiceType::Skill,
            tags: vec![],
            is_active: true,
            is_configured: true,
            required_config_keys: vec![],
            action_icon: None,
            action_target: None,
            icon_data: None,
            magic_seal: 0x5645_5253,
            sdk_version: "1.0".to_string(),
            required_permissions: vec![],
            provided_capabilities: vec![],
            provided_tools: vec![],
        }
    }

    /// How a test plugin reacts to `on_event`.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Reaction {
        /// Record the call and return nothing.
        Record,
        /// Sleep `secs` before returning (to trip `event_timeout_secs`).
        Sleep(u64),
        /// Panic inside `on_event`.
        Panic,
        /// Emit a follow-up event exactly once (to exercise re-dispatch).
        EmitOnce,
    }

    struct TestPlugin {
        id: String,
        tool_name: Option<String>,
        reaction: Reaction,
        seen: Arc<AtomicUsize>,
        /// Highest number of simultaneous `on_event` bodies observed.
        in_flight: Arc<AtomicUsize>,
        peak_in_flight: Arc<AtomicUsize>,
    }

    impl TestPlugin {
        fn new(id: &str) -> Self {
            Self {
                id: id.to_string(),
                tool_name: None,
                reaction: Reaction::Record,
                seen: Arc::new(AtomicUsize::new(0)),
                in_flight: Arc::new(AtomicUsize::new(0)),
                peak_in_flight: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn with_tool(mut self, tool_name: &str) -> Self {
            self.tool_name = Some(tool_name.to_string());
            self
        }

        fn reacting(mut self, reaction: Reaction) -> Self {
            self.reaction = reaction;
            self
        }

        fn sharing_counters(
            mut self,
            in_flight: &Arc<AtomicUsize>,
            peak: &Arc<AtomicUsize>,
        ) -> Self {
            self.in_flight = in_flight.clone();
            self.peak_in_flight = peak.clone();
            self
        }
    }

    impl PluginCast for TestPlugin {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_tool(&self) -> Option<&dyn Tool> {
            self.tool_name.as_ref().map(|_| self as &dyn Tool)
        }
    }

    #[async_trait::async_trait]
    impl Plugin for TestPlugin {
        fn manifest(&self) -> PluginManifest {
            manifest_for(&self.id)
        }

        async fn on_event(&self, _event: &ClotoEvent) -> anyhow::Result<Option<ClotoEventData>> {
            let now = self.in_flight.fetch_add(1, AtomicOrdering::SeqCst) + 1;
            self.peak_in_flight.fetch_max(now, AtomicOrdering::SeqCst);
            let seen = self.seen.fetch_add(1, AtomicOrdering::SeqCst);

            let result = match self.reaction {
                Reaction::Record => Ok(None),
                Reaction::Sleep(secs) => {
                    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                    Ok(None)
                }
                Reaction::Panic => panic!("test plugin {} panicked on purpose", self.id),
                Reaction::EmitOnce => {
                    if seen == 0 {
                        Ok(Some(ClotoEventData::SystemNotification("echo".into())))
                    } else {
                        Ok(None)
                    }
                }
            };
            self.in_flight.fetch_sub(1, AtomicOrdering::SeqCst);
            result
        }
    }

    #[async_trait::async_trait]
    impl Tool for TestPlugin {
        fn name(&self) -> &str {
            self.tool_name.as_deref().unwrap_or("")
        }
        fn description(&self) -> &'static str {
            "test tool"
        }
        async fn execute(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!({ "ran": self.id, "args": args }))
        }
    }

    /// A plugin that also satisfies the memory capability.
    struct MemoryPlugin {
        id: String,
    }

    impl PluginCast for MemoryPlugin {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_memory(&self) -> Option<&dyn cloto_shared::MemoryProvider> {
            Some(self)
        }
    }

    #[async_trait::async_trait]
    impl Plugin for MemoryPlugin {
        fn manifest(&self) -> PluginManifest {
            manifest_for(&self.id)
        }
    }

    #[async_trait::async_trait]
    impl cloto_shared::MemoryProvider for MemoryPlugin {
        fn name(&self) -> &str {
            &self.id
        }
        async fn store(
            &self,
            _agent_id: String,
            _message: cloto_shared::ClotoMessage,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn recall(
            &self,
            _agent_id: String,
            _query: &str,
            _limit: usize,
        ) -> anyhow::Result<Vec<cloto_shared::ClotoMessage>> {
            Ok(vec![])
        }
    }

    async fn register(registry: &PluginRegistry, id: &str, plugin: Arc<dyn Plugin>) {
        registry
            .state
            .write()
            .await
            .plugins
            .insert(id.into(), plugin);
    }

    fn note_envelope(depth: u8) -> crate::EnvelopedEvent {
        crate::EnvelopedEvent {
            event: Arc::new(ClotoEvent::new(ClotoEventData::SystemNotification(
                "ping".into(),
            ))),
            issuer: None,
            correlation_id: None,
            depth,
        }
    }

    fn tool_names(schemas: &[serde_json::Value]) -> Vec<String> {
        let mut names: Vec<String> = schemas
            .iter()
            .filter_map(|s| s.get("function")?.get("name")?.as_str().map(str::to_string))
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn a_plugin_inserted_into_the_registry_is_found_by_id_and_listed() {
        let registry = PluginRegistry::new(5, 10, 50);
        assert!(registry.get_engine("plugin.a").await.is_none());

        register(
            &registry,
            "plugin.a",
            Arc::new(TestPlugin::new("plugin.a").with_tool("do_a")),
        )
        .await;

        assert!(registry.get_engine("plugin.a").await.is_some());
        let listed: Vec<String> = registry
            .list_plugins()
            .await
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(listed, vec!["plugin.a".to_string()]);
        assert_eq!(
            tool_names(&registry.collect_tool_schemas().await),
            vec!["do_a".to_string()]
        );

        // Unregistration is a plain map removal, and the tool surface follows.
        registry.state.write().await.plugins.remove("plugin.a");
        assert!(registry.get_engine("plugin.a").await.is_none());
        assert!(registry.collect_tool_schemas().await.is_empty());
    }

    #[tokio::test]
    async fn registering_a_second_plugin_under_a_used_id_silently_replaces_the_first() {
        // Quirk: there is no duplicate check anywhere on this path — the second
        // insert wins and the first plugin is dropped without a warning.
        let registry = PluginRegistry::new(5, 10, 50);
        register(
            &registry,
            "plugin.a",
            Arc::new(TestPlugin::new("first").with_tool("tool_v1")),
        )
        .await;
        register(
            &registry,
            "plugin.a",
            Arc::new(TestPlugin::new("second").with_tool("tool_v2")),
        )
        .await;

        assert_eq!(registry.state.read().await.plugins.len(), 1);
        assert_eq!(
            tool_names(&registry.collect_tool_schemas().await),
            vec!["tool_v2".to_string()],
            "the later registration is the one that answers"
        );
    }

    #[tokio::test]
    async fn two_plugins_exposing_the_same_tool_name_both_appear_in_the_schema_list() {
        // Quirk: nothing de-duplicates tool names across plugins, so the model
        // is offered the same function twice and `execute_tool` picks whichever
        // the HashMap iteration order reaches first.
        let registry = PluginRegistry::new(5, 10, 50);
        register(
            &registry,
            "plugin.a",
            Arc::new(TestPlugin::new("plugin.a").with_tool("shared")),
        )
        .await;
        register(
            &registry,
            "plugin.b",
            Arc::new(TestPlugin::new("plugin.b").with_tool("shared")),
        )
        .await;

        assert_eq!(
            tool_names(&registry.collect_tool_schemas().await),
            vec!["shared".to_string(), "shared".to_string()]
        );
    }

    #[tokio::test]
    async fn the_agent_scoped_tool_surface_only_shows_and_runs_allowed_plugins() {
        let registry = PluginRegistry::new(5, 10, 50);
        register(
            &registry,
            "plugin.allowed",
            Arc::new(TestPlugin::new("plugin.allowed").with_tool("allowed_tool")),
        )
        .await;
        register(
            &registry,
            "plugin.denied",
            Arc::new(TestPlugin::new("plugin.denied").with_tool("denied_tool")),
        )
        .await;

        let allowed = vec!["plugin.allowed".to_string()];
        assert_eq!(
            tool_names(&registry.collect_tool_schemas_for(&allowed).await),
            vec!["allowed_tool".to_string()]
        );

        let caller = crate::managers::Caller::System;
        assert!(registry
            .execute_tool_for(&caller, &allowed, "allowed_tool", serde_json::json!({}))
            .await
            .is_ok());

        let err = registry
            .execute_tool_for(&caller, &allowed, "denied_tool", serde_json::json!({}))
            .await
            .expect_err("a tool outside the allowed set must not run");
        assert!(
            err.to_string().contains("denied_tool"),
            "the failure names the tool: {err}"
        );
    }

    #[tokio::test]
    async fn find_memory_returns_a_memory_capable_plugin_and_ignores_the_others() {
        let registry = PluginRegistry::new(5, 10, 50);
        register(
            &registry,
            "plugin.tool",
            Arc::new(TestPlugin::new("plugin.tool").with_tool("t")),
        )
        .await;
        assert!(
            registry.find_memory().await.is_none(),
            "a tool-only registry has no memory provider"
        );

        register(
            &registry,
            "plugin.memory",
            Arc::new(MemoryPlugin {
                id: "plugin.memory".into(),
            }),
        )
        .await;
        let found = registry.find_memory().await.expect("memory plugin");
        assert_eq!(found.manifest().id, "plugin.memory");
    }

    #[tokio::test]
    async fn granting_the_same_permission_twice_does_not_duplicate_it() {
        let registry = PluginRegistry::new(5, 10, 50);
        let id = ClotoId::from_name("plugin.a");

        registry
            .update_effective_permissions(id, Permission::MemoryRead)
            .await;
        registry
            .update_effective_permissions(id, Permission::MemoryRead)
            .await;
        registry
            .update_effective_permissions(id, Permission::InputControl)
            .await;

        let state = registry.state.read().await;
        let perms = state.effective_permissions.get(&id).expect("permissions");
        assert_eq!(perms.len(), 2, "duplicates are collapsed: {perms:?}");
        assert!(perms.contains(&Permission::MemoryRead));
        assert!(perms.contains(&Permission::InputControl));
    }

    #[tokio::test]
    async fn dispatch_stops_delivering_once_the_depth_reaches_max_event_depth() {
        // max_event_depth = 2: depth 0 and 1 are delivered, depth 2 is dropped.
        let registry = PluginRegistry::new(5, 2, 50);
        let plugin = Arc::new(TestPlugin::new("plugin.a"));
        let seen = plugin.seen.clone();
        register(&registry, "plugin.a", plugin).await;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        registry.dispatch_event(note_envelope(0), &tx).await;
        registry.dispatch_event(note_envelope(1), &tx).await;
        assert_eq!(seen.load(AtomicOrdering::SeqCst), 2);

        registry.dispatch_event(note_envelope(2), &tx).await;
        registry.dispatch_event(note_envelope(9), &tx).await;
        assert_eq!(
            seen.load(AtomicOrdering::SeqCst),
            2,
            "events at or beyond max_event_depth are dropped, not delivered"
        );
    }

    #[tokio::test]
    async fn the_concurrency_limit_caps_how_many_plugins_run_at_once() {
        // event_concurrency_limit = 1 serialises the two plugins; the shared
        // counters record the highest overlap actually observed.
        let registry = PluginRegistry::new(5, 10, 1);
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        for id in ["plugin.a", "plugin.b", "plugin.c"] {
            register(
                &registry,
                id,
                Arc::new(
                    TestPlugin::new(id)
                        .reacting(Reaction::Sleep(0))
                        .sharing_counters(&in_flight, &peak),
                ),
            )
            .await;
        }
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        registry.dispatch_event(note_envelope(0), &tx).await;

        assert_eq!(
            peak.load(AtomicOrdering::SeqCst),
            1,
            "a semaphore of 1 must never let two plugin bodies overlap"
        );
    }

    #[tokio::test]
    async fn a_plugin_that_hangs_past_the_timeout_does_not_hold_up_the_others() {
        let registry = PluginRegistry::new(1, 10, 50); // 1s per-plugin timeout
        let fast = Arc::new(TestPlugin::new("plugin.fast"));
        let fast_seen = fast.seen.clone();
        register(&registry, "plugin.fast", fast).await;
        register(
            &registry,
            "plugin.slow",
            Arc::new(TestPlugin::new("plugin.slow").reacting(Reaction::Sleep(30))),
        )
        .await;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let started = std::time::Instant::now();
        registry.dispatch_event(note_envelope(0), &tx).await;
        let elapsed = started.elapsed();

        assert_eq!(fast_seen.load(AtomicOrdering::SeqCst), 1);
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "dispatch must return after the 1s timeout, not the 30s sleep (took {elapsed:?})"
        );
    }

    #[tokio::test]
    async fn a_panicking_plugin_is_contained_and_the_others_still_receive_the_event() {
        let registry = PluginRegistry::new(5, 10, 50);
        register(
            &registry,
            "plugin.boom",
            Arc::new(TestPlugin::new("plugin.boom").reacting(Reaction::Panic)),
        )
        .await;
        let survivor = Arc::new(TestPlugin::new("plugin.ok"));
        let survivor_seen = survivor.seen.clone();
        register(&registry, "plugin.ok", survivor).await;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        registry.dispatch_event(note_envelope(0), &tx).await;

        assert_eq!(survivor_seen.load(AtomicOrdering::SeqCst), 1);
        // The semaphore permit was returned, so a second dispatch still works.
        registry.dispatch_event(note_envelope(0), &tx).await;
        assert_eq!(survivor_seen.load(AtomicOrdering::SeqCst), 2);
    }

    #[tokio::test]
    async fn an_event_returned_by_a_plugin_is_requeued_as_that_plugin_one_level_deeper() {
        let registry = PluginRegistry::new(5, 10, 50);
        register(
            &registry,
            "plugin.echo",
            Arc::new(TestPlugin::new("plugin.echo").reacting(Reaction::EmitOnce)),
        )
        .await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);

        registry.dispatch_event(note_envelope(3), &tx).await;

        let requeued = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("re-dispatch must arrive")
            .expect("channel open");
        assert_eq!(
            requeued.issuer,
            Some(ClotoId::from_name("plugin.echo")),
            "the emitting plugin is stamped as the issuer"
        );
        assert_eq!(requeued.depth, 4, "the requeued event is one level deeper");
        assert!(matches!(
            requeued.event.data,
            ClotoEventData::SystemNotification(ref m) if m == "echo"
        ));
    }
}
