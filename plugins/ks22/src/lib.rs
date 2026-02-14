use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use exiv_shared::{
    MemoryProvider, Plugin, PluginConfig, ReasoningEngine, ExivMessage,
    exiv_plugin, PluginRuntimeContext, PluginDataStore, SALExt
};

#[exiv_plugin(
    name = "core.ks22", 
    kind = "Reasoning",
    description = "Persistent key-value memory with chronological recall.",
    version = "0.1.0",
    category = "Memory",
    permissions = ["MemoryRead", "MemoryWrite"],
    capabilities = ["Reasoning", "Memory"]
)]
pub struct Ks22Plugin {
    state: Arc<RwLock<Ks22State>>,
}

struct Ks22State {
    store: Option<Arc<dyn PluginDataStore>>,
}

impl Ks22Plugin {
    pub async fn new_plugin(_config: PluginConfig) -> anyhow::Result<Self> {
        Ok(Self { 
            state: Arc::new(RwLock::new(Ks22State { store: None })) 
        })
    }
}

#[async_trait]
impl Plugin for Ks22Plugin {
    fn manifest(&self) -> exiv_shared::PluginManifest {
        self.auto_manifest()
    }

    async fn on_plugin_init(
        &self,
        context: PluginRuntimeContext,
        _network: Option<Arc<dyn exiv_shared::NetworkCapability>>,
    ) -> anyhow::Result<()> {
        let mut state = self.state.write().await;
        state.store = Some(context.store);
        Ok(())
    }

    async fn on_event(&self, event: &exiv_shared::ExivEvent) -> anyhow::Result<Option<exiv_shared::ExivEventData>> {
        if let exiv_shared::ExivEventData::ThoughtRequested { agent, engine_id, message, context } = &event.data {
            if engine_id == "core.ks22" {
                let content = self.think(agent, message, context.clone()).await?;
                return Ok(Some(exiv_shared::ExivEventData::ThoughtResponse {
                    agent_id: agent.id.clone(),
                    engine_id: "core.ks22".to_string(),
                    content,
                    source_message_id: message.id.clone(),
                }));
            }
        }
        Ok(None)
    }
}

#[async_trait]
impl ReasoningEngine for Ks22Plugin {
    fn name(&self) -> &str { "KS2.2-Mind" }

    async fn think(
        &self,
        _agent: &exiv_shared::AgentMetadata,
        message: &ExivMessage,
        _context: Vec<ExivMessage>,
    ) -> anyhow::Result<String> {
        Ok(format!("KS2.2 received: '{}'.", message.content))
    }
}

#[async_trait]
impl MemoryProvider for Ks22Plugin {
    fn name(&self) -> &str { "KS2.2-Storage" }

    async fn store(&self, agent_id: String, message: ExivMessage) -> anyhow::Result<()> {
        let state = self.state.read().await;
        let store = state.store.as_ref().ok_or_else(|| anyhow::anyhow!("Store not initialized"))?;
        
        let key = store.generate_mem_key(&agent_id, &message);
        store.save("core.ks22", &key, &message).await?;
        Ok(())
    }

    async fn recall(
        &self,
        agent_id: String,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<ExivMessage>> {
        let state = self.state.read().await;
        let store = state.store.as_ref().ok_or_else(|| anyhow::anyhow!("Store not initialized"))?;
        
        let prefix = format!("mem:{}:", agent_id);
        // Kernel 側で DESC ソートされているため、最新のものが最初に来る
        let mut items = store.get_all_json("core.ks22", &prefix).await?;
        // H-03: Cap loaded items to prevent unbounded memory consumption
        items.truncate(500);

        let mut messages = Vec::new();
        let query_lower = query.to_lowercase();

        for (key, value) in items {
            match serde_json::from_value::<ExivMessage>(value) {
                Ok(msg) => {
                    // 🔍 シンプルなキーワードマッチング (もしクエリがあれば)
                    if query.is_empty() || msg.content.to_lowercase().contains(&query_lower) {
                        messages.push(msg);
                    }
                }
                Err(e) => {
                    // M-15: Log deserialization failures instead of silently ignoring
                    tracing::warn!(key = %key, error = %e, "Failed to deserialize memory entry");
                }
            }
            if messages.len() >= limit {
                break;
            }
        }
        
        // 🔄 LLM に渡すために時系列順（昇順）に戻す
        messages.reverse();
        Ok(messages)
    }
}