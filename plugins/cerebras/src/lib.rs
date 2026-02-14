use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use exiv_shared::{
    AgentMetadata, Plugin, PluginConfig,
    ReasoningEngine, ExivMessage, Permission, PluginRuntimeContext,
    NetworkCapability, HttpRequest, exiv_plugin
};

#[exiv_plugin(
    name = "mind.cerebras",
    kind = "Reasoning",
    description = "Ultra-high-speed reasoning via Cerebras API.",
    version = "0.2.0",
    category = "Agent",
    action_icon = "Settings",
    config_keys = ["api_key", "model_id"],
    permissions = ["NetworkAccess"],
    capabilities = ["Reasoning"]
)]
pub struct CerebrasPlugin {
    state: Arc<RwLock<CerebrasState>>,
}

struct CerebrasState {
    api_key: String,
    model_id: String,
    allowed_permissions: Vec<Permission>,
    http_client: Option<Arc<dyn NetworkCapability>>,
}

impl CerebrasPlugin {
    pub async fn new_plugin(config: PluginConfig) -> anyhow::Result<Self> {
        let api_key = config.config_values.get("api_key").cloned().unwrap_or_default();
        if api_key.is_empty() {
            tracing::warn!("⚠️  Cerebras plugin: No API key configured. Set 'api_key' in plugin config.");
        }
        let model_id = config.config_values.get("model_id").cloned().unwrap_or_else(|| "llama3.1-70b".to_string());
        
        Ok(Self {
            state: Arc::new(RwLock::new(CerebrasState {
                api_key,
                model_id,
                allowed_permissions: vec![],
                http_client: None,
            })),
        })
    }
}

#[async_trait]
impl Plugin for CerebrasPlugin {
    fn manifest(&self) -> exiv_shared::PluginManifest {
        self.auto_manifest()
    }

    async fn on_plugin_init(
        &self,
        context: PluginRuntimeContext,
        network: Option<Arc<dyn NetworkCapability>>,
    ) -> anyhow::Result<()> {
        let mut state = self.state.write().await;
        state.allowed_permissions = context.effective_permissions;
        state.http_client = network;
        Ok(())
    }

    async fn on_event(
        &self,
        event: &exiv_shared::ExivEvent,
    ) -> anyhow::Result<Option<exiv_shared::ExivEventData>> {
        match &event.data {
            exiv_shared::ExivEventData::ThoughtRequested {
                agent,
                engine_id,
                message,
                context,
            } => {
                if engine_id != Self::PLUGIN_ID {
                    return Ok(None);
                }
                let content = self.think(agent, message, context.clone()).await?;
                return Ok(Some(exiv_shared::ExivEventData::ThoughtResponse {
                    agent_id: agent.id.clone(),
                    engine_id: Self::PLUGIN_ID.to_string(),
                    content,
                    source_message_id: message.id.clone(),
                }));
            }
            exiv_shared::ExivEventData::ConfigUpdated { plugin_id, config } => {
                if plugin_id == Self::PLUGIN_ID {
                    let mut state = self.state.write().await;
                    if let Some(key) = config.get("api_key") {
                        state.api_key = key.clone();
                    }
                    if let Some(model) = config.get("model_id") {
                        state.model_id = model.clone();
                    }
                    tracing::info!("🔌 Cerebras Plugin configuration hot-reloaded.");
                }
            }
            _ => {}
        }
        Ok(None)
    }

    async fn on_capability_injected(
        &self,
        capability: exiv_shared::PluginCapability,
    ) -> anyhow::Result<()> {
        match capability {
            exiv_shared::PluginCapability::Network(net) => {
                let mut state = self.state.write().await;
                state.http_client = Some(net);
                tracing::info!("💉 Cerebras Plugin: NetworkCapability injected live.");
            }
        }
        Ok(())
    }
}

#[async_trait]
impl ReasoningEngine for CerebrasPlugin {
    fn name(&self) -> &str {
        "Cerebras"
    }

    async fn think(
        &self,
        agent: &AgentMetadata,
        message: &ExivMessage,
        context: Vec<ExivMessage>,
    ) -> anyhow::Result<String> {
        // H-01: Clone needed data inside lock, release lock before network I/O
        let (api_key, model_id, client) = {
            let state = self.state.read().await;
            if state.api_key.is_empty() {
                return Err(anyhow::anyhow!("Cerebras API Key not configured."));
            }
            let client = state.http_client.clone()
                .ok_or_else(|| anyhow::anyhow!("NetworkCapability not injected."))?;
            (state.api_key.clone(), state.model_id.clone(), client)
        };

        let mut messages = Vec::new();
        messages.push(serde_json::json!({
            "role": "system",
            "content": format!("You are {}. {}.", agent.name, agent.description)
        }));

        for msg in context {
            let role = match msg.source {
                exiv_shared::MessageSource::User { .. } => "user",
                exiv_shared::MessageSource::Agent { .. } => "assistant",
                exiv_shared::MessageSource::System => "system",
            };
            messages.push(serde_json::json!({ "role": role, "content": msg.content }));
        }

        messages.push(serde_json::json!({ "role": "user", "content": message.content }));

        let req = HttpRequest {
            method: "POST".to_string(),
            url: "https://api.cerebras.ai/v1/chat/completions".to_string(),
            headers: [
                ("Authorization".to_string(), format!("Bearer {}", api_key)),
                ("Content-Type".to_string(), "application/json".to_string())
            ].into_iter().collect(),
            body: Some(serde_json::json!({
                "model": model_id,
                "messages": messages,
                "stream": false
            }).to_string()),
        };

        let resp = client.send_http_request(req).await?;
        let json: serde_json::Value = serde_json::from_str(&resp.body)?;

        // H-02: Safe JSON path access with descriptive error
        if let Some(error) = json.get("error") {
            let msg = error.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
            return Err(anyhow::anyhow!("Cerebras API Error: {}", msg));
        }

        let content = json.get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid Cerebras API response: missing choices[0].message.content"))?
            .to_string();

        Ok(content)
    }
}
