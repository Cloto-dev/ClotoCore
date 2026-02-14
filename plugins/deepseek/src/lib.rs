use async_trait::async_trait;
use std::any::Any;
use std::sync::Arc;
use tokio::sync::RwLock;
use exiv_shared::{
    AgentMetadata, Plugin, PluginConfig,
    ReasoningEngine, ExivMessage, Permission, PluginRuntimeContext,
    NetworkCapability, HttpRequest, exiv_plugin
};

#[exiv_plugin(
    name = "mind.deepseek",
    kind = "Reasoning",
    description = "Advanced reasoning using DeepSeek API.",
    version = "0.1.0",
    category = "Agent",
    icon = "assets/icon.svg",
    action_icon = "Settings",
    config_keys = ["api_key", "model_id", "api_url"],
    permissions = ["NetworkAccess"],
    capabilities = ["Reasoning", "Web"]
)]
pub struct DeepSeekPlugin {
    state: Arc<RwLock<DeepSeekState>>,
}

struct DeepSeekState {
    api_key: String,
    model_id: String,
    api_url: String,
    allowed_permissions: Vec<Permission>,
    http_client: Option<Arc<dyn NetworkCapability>>,
}

impl DeepSeekPlugin {
    pub async fn new_plugin(config: PluginConfig) -> anyhow::Result<Self> {
        let api_key = config.config_values.get("api_key").cloned().unwrap_or_default();
        if api_key.is_empty() {
            tracing::warn!("⚠️  DeepSeek plugin: No API key configured. Set 'api_key' in plugin config.");
        }
        let model_id = config.config_values.get("model_id").cloned().unwrap_or_else(|| "deepseek-chat".to_string());
        let api_url = config.config_values.get("api_url").cloned().unwrap_or_else(|| "https://api.deepseek.com/chat/completions".to_string());
        
        Ok(Self {
            state: Arc::new(RwLock::new(DeepSeekState {
                api_key,
                model_id,
                api_url,
                allowed_permissions: vec![],
                http_client: None,
            })),
        })
    }
}

#[async_trait]
impl Plugin for DeepSeekPlugin {
    fn manifest(&self) -> exiv_shared::PluginManifest {
        self.auto_manifest()
    }

    async fn on_plugin_init(
        &self,
        context: PluginRuntimeContext,
        network: Option<Arc<dyn NetworkCapability>>,
    ) -> anyhow::Result<()> {
        if network.is_none() {
            tracing::warn!("🔌 DeepSeek Plugin: NetworkCapability NOT provided. API calls will fail until permission is granted.");
        }
        
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
                    if let Some(url) = config.get("api_url") {
                        state.api_url = url.clone();
                    }
                    tracing::info!("🔌 DeepSeek Plugin configuration hot-reloaded.");
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
                tracing::info!("💉 DeepSeek Plugin: NetworkCapability injected live.");
            }
        }
        Ok(())
    }
}

impl exiv_shared::WebPlugin for DeepSeekPlugin {
    fn register_routes(
        &self,
        router: axum::Router<Arc<dyn Any + Send + Sync>>,
    ) -> axum::Router<Arc<dyn Any + Send + Sync>> {
        router.route(
            "/plugin/deepseek/status",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "status": "active",
                    "engine": "deepseek-reasoner"
                }))
            }),
        )
    }
}

#[async_trait]
impl ReasoningEngine for DeepSeekPlugin {
    fn name(&self) -> &str {
        "DeepSeek"
    }

    async fn think(
        &self,
        agent: &AgentMetadata,
        message: &ExivMessage,
        context: Vec<ExivMessage>,
    ) -> anyhow::Result<String> {
        // H-01: Clone needed data inside lock, release lock before network I/O
        let (api_key, model_id, api_url, client) = {
            let state = self.state.read().await;
            if state.api_key.is_empty() {
                return Err(anyhow::anyhow!("DeepSeek API Key not configured."));
            }
            let client = state.http_client.clone()
                .ok_or_else(|| anyhow::anyhow!("NetworkCapability not injected. Ensure 'NetworkAccess' permission is granted."))?;
            (state.api_key.clone(), state.model_id.clone(), state.api_url.clone(), client)
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
            url: api_url,
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
        let content = json.get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid DeepSeek API response: missing choices[0].message.content"))?
            .to_string();

        Ok(content)
    }
}