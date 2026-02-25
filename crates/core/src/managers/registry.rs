use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;

use cloto_shared::{ClotoId, Permission, Plugin, PluginManifest};

#[derive(sqlx::FromRow, Debug)]
pub struct PluginSetting {
    pub plugin_id: String,
    pub is_active: bool,
    pub allowed_permissions: sqlx::types::Json<Vec<Permission>>,
}

pub struct PluginRegistry {
    pub plugins: tokio::sync::RwLock<HashMap<String, Arc<dyn Plugin>>>,
    pub effective_permissions: tokio::sync::RwLock<HashMap<ClotoId, Vec<Permission>>>,
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

impl PluginRegistry {
    #[must_use]
    pub fn new(event_timeout_secs: u64, max_event_depth: u8) -> Self {
        Self {
            plugins: tokio::sync::RwLock::new(HashMap::new()),
            effective_permissions: tokio::sync::RwLock::new(HashMap::new()),
            event_timeout_secs,
            max_event_depth,
            event_semaphore: Arc::new(tokio::sync::Semaphore::new(50)),
            mcp_manager: None,
        }
    }

    /// Set the MCP Client Manager for dual dispatch.
    pub fn set_mcp_manager(&mut self, mcp_manager: Arc<super::McpClientManager>) {
        self.mcp_manager = Some(mcp_manager);
    }

    pub async fn update_effective_permissions(&self, plugin_id: ClotoId, permission: Permission) {
        let mut perms_lock = self.effective_permissions.write().await;
        let perms = perms_lock.entry(plugin_id).or_default();
        if !perms.contains(&permission) {
            perms.push(permission);
        }
    }

    pub async fn list_plugins(&self) -> Vec<PluginManifest> {
        let plugins = self.plugins.read().await;
        plugins.values().map(|p| p.manifest()).collect()
    }

    pub async fn get_engine(&self, id: &str) -> Option<Arc<dyn Plugin>> {
        let plugins = self.plugins.read().await;
        plugins.get(id).cloned()
    }

    pub async fn find_memory(&self) -> Option<Arc<dyn Plugin>> {
        let plugins = self.plugins.read().await;
        for plugin in plugins.values() {
            if plugin.as_memory().is_some() {
                return Some(plugin.clone());
            }
        }
        None
    }

    /// Collect tool schemas from all active Tool plugins + MCP servers (OpenAI function calling format).
    pub async fn collect_tool_schemas(&self) -> Vec<serde_json::Value> {
        let mut schemas: Vec<serde_json::Value> = {
            let plugins = self.plugins.read().await;
            plugins
                .values()
                .filter_map(|p| {
                    let tool = p.as_tool()?;
                    Some(serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name(),
                            "description": tool.description(),
                            "parameters": tool.parameters_schema(),
                        }
                    }))
                })
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
            let plugins = self.plugins.read().await;
            plugins
                .iter()
                .filter_map(|(id, p)| {
                    if !allowed_plugin_ids.contains(id) {
                        return None;
                    }
                    let tool = p.as_tool()?;
                    Some(serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name(),
                            "description": tool.description(),
                            "parameters": tool.parameters_schema(),
                        }
                    }))
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
        tool_name: &str,
        args: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        // 1. Try Rust plugins first
        let tool_plugin = {
            let plugins = self.plugins.read().await;
            plugins.values().find_map(|p| {
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
                return tool.execute(args).await;
            }
        }

        // 2. Fall back to MCP servers
        if let Some(ref mcp) = self.mcp_manager {
            return mcp.execute_tool(tool_name, args).await;
        }

        Err(anyhow::anyhow!("Tool '{}' not found", tool_name))
    }

    /// Execute a tool by name, only if it belongs to the agent's allowed plugin set.
    /// Dual Dispatch: tries Rust plugins first, then falls back to MCP servers.
    pub async fn execute_tool_for(
        &self,
        allowed_plugin_ids: &[String],
        tool_name: &str,
        args: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        // 1. Try Rust plugins first
        let tool_plugin = {
            let plugins = self.plugins.read().await;
            plugins.iter().find_map(|(id, p)| {
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
                return tool.execute(args).await;
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
                return mcp.execute_tool(tool_name, args).await;
            }
        }

        Err(anyhow::anyhow!(
            "Tool '{}' not found or not available for this agent",
            tool_name
        ))
    }

    /// 全てのアクティブなプラグインにイベントを配信する
    pub async fn dispatch_event(
        &self,
        envelope: crate::EnvelopedEvent,
        event_tx: &tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
    ) {
        let event = envelope.event.clone();
        let current_depth = envelope.depth;

        // 🚨 連鎖爆発の防止 (Guardrail #2)
        if current_depth >= self.max_event_depth {
            error!(
                event_type = ?event,
                depth = current_depth,
                "🛑 Event cascading limit reached ({}). Dropping event to prevent infinite loop.",
                self.max_event_depth
            );
            return;
        }

        let plugins = self.plugins.read().await;

        use futures::stream::{FuturesUnordered, StreamExt};
        use futures::FutureExt;
        let mut futures = FuturesUnordered::new();

        for (id, plugin) in plugins.iter() {
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

        // ロックを早めに解放
        drop(plugins);

        // 完了した順に結果を処理
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
        event: Arc::new(cloto_shared::ClotoEvent::with_trace(trace_id, new_event_data)),
        issuer: Some(issuer_id),
        correlation_id: Some(trace_id),
        depth: current_depth + 1,
    };
    if let Err(e) = tx.send(envelope).await {
        error!("🔌 Failed to re-dispatch plugin event: {}", e);
    }
}
