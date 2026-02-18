mod state;
mod config;
mod process;
mod ipc;

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use exiv_shared::{
    AgentMetadata, Plugin, PluginRuntimeContext,
    ReasoningEngine, ExivMessage,
    exiv_plugin, NetworkCapability, Tool
};
use tracing::info;

use state::PythonBridgeState;

#[exiv_plugin(
    name = "bridge.python",
    kind = "Reasoning",
    description = "Universal Python Bridge with asynchronous event streaming. Supports real-time capabilities like Gaze Tracking.",
    version = "0.3.0",
    category = "Tool",
    permissions = ["NetworkAccess", "FileRead", "ProcessExecution", "VisionRead"],
    tags = ["#TOOL", "#ADAPTER"],
    capabilities = ["Reasoning", "Tool", "Web"]
)]
#[derive(Clone)]
pub struct PythonBridgePlugin {
    pub(crate) instance_id: String,
    pub(crate) script_path: String,
    pub(crate) state: Arc<RwLock<PythonBridgeState>>,
    /// L5: Dynamic tool name from Python EXIV_MANIFEST (set once after handshake)
    pub(crate) tool_name: Arc<std::sync::OnceLock<String>>,
    /// L5: Dynamic tool description from Python EXIV_MANIFEST
    pub(crate) tool_description: Arc<std::sync::OnceLock<String>>,
    /// L5: Dynamic tool parameter schema from Python EXIV_MANIFEST
    pub(crate) tool_schema: Arc<std::sync::OnceLock<serde_json::Value>>,
}

#[async_trait]
impl Plugin for PythonBridgePlugin {
    fn manifest(&self) -> exiv_shared::PluginManifest {
        if let Ok(state) = self.state.try_read() {
            if let Some(m) = &state.dynamic_manifest {
                return m.clone();
            }
        }
        self.auto_manifest()
    }

    async fn on_plugin_init(
        &self,
        context: PluginRuntimeContext,
        network: Option<Arc<dyn NetworkCapability>>,
    ) -> anyhow::Result<()> {
        {
            let mut state = self.state.write().await;
            state.allowed_permissions = context.effective_permissions;
            state.http_client = network;
            state.event_tx = Some(context.event_tx);
        }

        // 🐍 Perform handshake immediately to load dynamic manifest
        if let Err(e) = self.ensure_process().await {
            tracing::error!("❌ Python Bridge: Failed to initialize subprocess for {}: {}", self.instance_id, e);
        } else {
            info!("🐍 Python Bridge: Subprocess handshake complete for {}", self.instance_id);
        }

        Ok(())
    }

    async fn on_event(&self, event: &exiv_shared::ExivEvent) -> anyhow::Result<Option<exiv_shared::ExivEventData>> {
        if let exiv_shared::ExivEventData::ThoughtRequested { agent, engine_id, message, context } = &event.data {
            let manifest = self.manifest();
            if engine_id != &self.instance_id && engine_id != "bridge.python" && engine_id != &manifest.id {
                return Ok(None);
            }
            let content = self.think(agent, message, context.clone()).await?;
            return Ok(Some(exiv_shared::ExivEventData::ThoughtResponse {
                agent_id: agent.id.clone(),
                engine_id: manifest.id.clone(),
                content,
                source_message_id: message.id.clone(),
            }));
        }
        Ok(None)
    }
}

impl exiv_shared::WebPlugin for PythonBridgePlugin {
    fn register_routes(
        &self,
        router: axum::Router<Arc<dyn std::any::Any + Send + Sync>>,
    ) -> axum::Router<Arc<dyn std::any::Any + Send + Sync>> {
        let instance_id = self.instance_id.clone();
        let plugin = self.clone();

        router.route(
            &format!("/api/plugin/{}/action/:command", instance_id),
            axum::routing::post(move |
                uri: axum::http::Uri,
                body: Option<axum::Json<serde_json::Value>>
            | {
                let plugin = plugin.clone();
                let body_val = body.map(|b| b.0).unwrap_or(serde_json::Value::Null);
                async move {
                    // Extract command from URI to avoid Path extractor conflict
                    // with outer router's wildcard parameter
                    let command = uri.path()
                        .rsplit('/')
                        .next()
                        .unwrap_or("unknown")
                        .to_string();
                    match plugin.call_python(&format!("on_action_{}", command), body_val).await {
                        Ok(res) => axum::Json(res),
                        Err(e) => {
                            tracing::error!("❌ Python Bridge Action Error: {}", e);
                            axum::Json(serde_json::json!({ "error": e.to_string() }))
                        }
                    }
                }
            }),
        )
    }
}

#[async_trait]
impl ReasoningEngine for PythonBridgePlugin {
    fn name(&self) -> &str { "PythonSubprocessBridge" }
    async fn think(&self, agent: &AgentMetadata, message: &ExivMessage, context: Vec<ExivMessage>) -> anyhow::Result<String> {
        let params = serde_json::json!({ "agent": agent, "message": message, "context": context });
        let result = self.call_python("think", params).await?;
        let content = result.as_str()
            .ok_or_else(|| anyhow::anyhow!("Python think() returned non-string: {}", result))?
            .to_string();
        Ok(content)
    }
}

#[async_trait]
impl Tool for PythonBridgePlugin {
    fn name(&self) -> &str {
        self.tool_name.get().map(|s| s.as_str()).unwrap_or("PythonBridgeTool")
    }
    fn description(&self) -> &str {
        self.tool_description.get().map(|s| s.as_str())
            .unwrap_or("Delegates tool execution to Python script.")
    }
    fn parameters_schema(&self) -> serde_json::Value {
        self.tool_schema.get().cloned().unwrap_or_else(|| serde_json::json!({}))
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        self.call_python("execute", args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use state::PythonBridgeState;

    /// Create a test plugin instance without filesystem validation.
    /// This bypasses config.rs path traversal checks that require a real scripts/ directory.
    fn test_plugin(id: &str) -> PythonBridgePlugin {
        PythonBridgePlugin {
            instance_id: id.to_string(),
            script_path: "dummy/test.py".to_string(),
            state: Arc::new(RwLock::new(PythonBridgeState::new())),
            tool_name: Arc::new(std::sync::OnceLock::new()),
            tool_description: Arc::new(std::sync::OnceLock::new()),
            tool_schema: Arc::new(std::sync::OnceLock::new()),
        }
    }

    #[tokio::test]
    async fn test_restart_rate_limiting() {
        let plugin = test_plugin("test.bridge");

        // Simulate max restart attempts reached (must also set last_restart to indicate this is a restart)
        {
            let mut state = plugin.state.write().await;
            state.restart_count = PythonBridgePlugin::MAX_RESTART_ATTEMPTS;
            state.last_restart = Some(std::time::Instant::now() - std::time::Duration::from_secs(60));
        }

        // Next ensure_process should fail due to max attempts
        let result = plugin.ensure_process().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Max restart attempts"));
    }

    #[tokio::test]
    async fn test_restart_cooldown() {
        let plugin = test_plugin("test.bridge2");

        // Simulate recent restart
        {
            let mut state = plugin.state.write().await;
            state.restart_count = 1;
            state.last_restart = Some(std::time::Instant::now());
        }

        // Immediate restart should fail due to cooldown
        let result = plugin.ensure_process().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cooldown"));
    }

    #[test]
    fn test_tool_name_from_oncelock() {
        let plugin = test_plugin("test.tool");
        // Default when OnceLock is unset
        assert_eq!(Tool::name(&plugin), "PythonBridgeTool");
        assert_eq!(Tool::description(&plugin), "Delegates tool execution to Python script.");
        assert_eq!(plugin.parameters_schema(), serde_json::json!({}));
        // After setting OnceLock
        let _ = plugin.tool_name.set("custom_tool".to_string());
        let _ = plugin.tool_description.set("A custom tool".to_string());
        let _ = plugin.tool_schema.set(serde_json::json!({"type": "object"}));
        assert_eq!(Tool::name(&plugin), "custom_tool");
        assert_eq!(Tool::description(&plugin), "A custom tool");
        assert_eq!(plugin.parameters_schema(), serde_json::json!({"type": "object"}));
    }

    #[tokio::test]
    async fn test_initial_startup_allowed() {
        let plugin = test_plugin("test.bridge3");

        // Initial startup (restart_count = 0) should not be blocked
        let state = plugin.state.read().await;
        assert_eq!(state.restart_count, 0);
        assert!(state.last_restart.is_none());
    }
}
