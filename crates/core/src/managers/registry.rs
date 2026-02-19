use std::sync::Arc;
use std::collections::HashMap;
use tracing::error;

use exiv_shared::{Plugin, PluginManifest, ExivId, Permission};

#[derive(sqlx::FromRow, Debug)]
pub struct PluginSetting {
    pub plugin_id: String,
    pub is_active: bool,
    pub allowed_permissions: sqlx::types::Json<Vec<Permission>>,
}

pub struct PluginRegistry {
    pub plugins: tokio::sync::RwLock<HashMap<String, Arc<dyn Plugin>>>,
    pub effective_permissions: tokio::sync::RwLock<HashMap<ExivId, Vec<Permission>>>,
    pub event_timeout_secs: u64,
    pub max_event_depth: u8,
    pub event_semaphore: Arc<tokio::sync::Semaphore>,
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
        }
    }

    pub async fn update_effective_permissions(&self, plugin_id: ExivId, permission: Permission) {
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

    /// Collect tool schemas from all active Tool plugins (OpenAI function calling format).
    pub async fn collect_tool_schemas(&self) -> Vec<serde_json::Value> {
        let plugins = self.plugins.read().await;
        plugins.values().filter_map(|p| {
            let tool = p.as_tool()?;
            Some(serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name(),
                    "description": tool.description(),
                    "parameters": tool.parameters_schema(),
                }
            }))
        }).collect()
    }

    /// Execute a tool by name with the given arguments.
    /// H-01: Drops the read lock before calling tool.execute() to avoid blocking
    /// plugin registration during long-running tool execution.
    pub async fn execute_tool(&self, tool_name: &str, args: serde_json::Value)
        -> anyhow::Result<serde_json::Value>
    {
        let tool_plugin = {
            let plugins = self.plugins.read().await;
            plugins.values().find_map(|p| {
                let tool = p.as_tool()?;
                if tool.name() == tool_name { Some(p.clone()) } else { None }
            })
        }; // read lock dropped here
        if let Some(plugin) = tool_plugin {
            if let Some(tool) = plugin.as_tool() {
                return tool.execute(args).await;
            }
        }
        Err(anyhow::anyhow!("Tool '{}' not found", tool_name))
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
                let _permit = if let Ok(p) = semaphore.acquire().await { p } else {
                    tracing::warn!("Semaphore closed during shutdown, skipping plugin {}", id);
                    return (id, Ok(Ok(None)));
                };
                // Catch panics to prevent semaphore permit leaks
                let result = tokio::time::timeout(
                    timeout_duration,
                    async {
                        match std::panic::AssertUnwindSafe(plugin.on_event(&event))
                            .catch_unwind()
                            .await
                        {
                            Ok(r) => r,
                            Err(_) => Err(anyhow::anyhow!("Plugin panicked during on_event")),
                        }
                    }
                ).await;
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
    trace_id: ExivId,
    new_event_data: exiv_shared::ExivEventData,
    current_depth: u8,
    semaphore: Arc<tokio::sync::Semaphore>,
) {
    let _permit = if let Ok(p) = semaphore.acquire().await { p } else {
        tracing::warn!("Semaphore closed during shutdown, skipping redispatch for {}", plugin_id);
        return;
    };
    let issuer_id = ExivId::from_name(&plugin_id);
    let envelope = crate::EnvelopedEvent {
        event: Arc::new(exiv_shared::ExivEvent::with_trace(trace_id, new_event_data)),
        issuer: Some(issuer_id),
        correlation_id: Some(trace_id),
        depth: current_depth + 1,
    };
    if let Err(e) = tx.send(envelope).await {
        error!("🔌 Failed to re-dispatch plugin event: {}", e);
    }
}
