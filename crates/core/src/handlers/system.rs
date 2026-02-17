use std::sync::Arc;
use tracing::{info, error};
use async_trait::async_trait;

use exiv_shared::{
    Plugin, PluginCast, PluginManifest,
    ExivEvent, ExivMessage
};
use crate::managers::{AgentManager, PluginRegistry};

pub struct SystemHandler {
    registry: Arc<PluginRegistry>,
    agent_manager: AgentManager,
    default_agent_id: String,
    sender: tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
    memory_context_limit: usize,
    metrics: Arc<crate::managers::SystemMetrics>,
    consensus_engines: Vec<String>,
}

impl SystemHandler {
    pub fn new(
        registry: Arc<PluginRegistry>,
        agent_manager: AgentManager,
        default_agent_id: String,
        sender: tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
        memory_context_limit: usize,
        metrics: Arc<crate::managers::SystemMetrics>,
        consensus_engines: Vec<String>,
    ) -> Self {
        Self { registry, agent_manager, default_agent_id, sender, memory_context_limit, metrics, consensus_engines }
    }

    pub async fn handle_message(&self, msg: ExivMessage) -> anyhow::Result<()> {
        let target_agent_id = msg.metadata.get("target_agent_id")
            .cloned()
            .unwrap_or_else(|| self.default_agent_id.clone());

        // 1. エージェント情報の取得
        let (agent, _engine_id) = self.agent_manager.get_agent_config(&target_agent_id).await?;

        // Block disabled agents from processing messages
        if !agent.enabled {
            info!(agent_id = %target_agent_id, "🔌 Agent is powered off. Message dropped.");
            return Ok(());
        }

        // Passive heartbeat: update last_seen on message routing
        self.agent_manager.touch_last_seen(&target_agent_id).await.ok();

        // 2. メモリからのコンテキスト取得
        let memory_plugin = if let Some(preferred_id) = agent.metadata.get("preferred_memory") {
            self.registry.get_engine(preferred_id).await
        } else {
            self.registry.find_memory().await
        };

        let context = if let Some(ref plugin) = memory_plugin {
            if let Some(mem) = plugin.as_memory() {
                // 🛑 停滞対策: メモリの呼び出しにタイムアウトを設定
                match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    mem.recall(agent.id.clone(), &msg.content, self.memory_context_limit)
                ).await {
                    Ok(Ok(ctx)) => ctx,
                    Ok(Err(e)) => {
                        error!(agent_id = %agent.id, error = %e, "❌ Memory recall failed");
                        vec![]
                    }
                    Err(_) => {
                        error!(agent_id = %agent.id, "⏱️ Memory recall timed out");
                        vec![]
                    }
                }
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        // 3. 【核心】思考要求イベントを発行
        info!(
            target_agent_id = %target_agent_id,
            agent_name = %agent.name,
            engine_id = %_engine_id,
            "📢 Dispatching Thought/Consensus Request"
        );

        let trace_id = exiv_shared::ExivId::new_trace_id();

        if msg.content.to_lowercase().starts_with("consensus:") {
            // 合意形成モード
            let thought_event_data = exiv_shared::ExivEventData::ConsensusRequested {
                task: msg.content.clone(),
                engine_ids: self.consensus_engines.clone(),
            };
            
            let envelope = crate::EnvelopedEvent {
                event: Arc::new(exiv_shared::ExivEvent::with_trace(trace_id, thought_event_data)),
                issuer: None,
                correlation_id: None,
                depth: 0,
            };
            if let Err(e) = self.sender.send(envelope).await {
                error!("Failed to dispatch ConsensusRequested: {}", e);
            }

            // 各エンジンにも個別にThoughtRequestedを投げる (Moderatorが拾うため)
            for engine in &self.consensus_engines {
                let inner_thought = exiv_shared::ExivEventData::ThoughtRequested {
                    agent: agent.clone(),
                    engine_id: engine.to_string(),
                    message: msg.clone(),
                    context: context.clone(),
                };
                let env = crate::EnvelopedEvent {
                    event: Arc::new(exiv_shared::ExivEvent::with_trace(trace_id, inner_thought)),
                    issuer: None,
                    correlation_id: Some(trace_id),
                    depth: 1,
                };
                if let Err(e) = self.sender.send(env).await {
                    error!("Failed to dispatch ThoughtRequested for engine {}: {}", engine, e);
                }
            }
        } else {
            // 通常モード
            let thought_event_data = exiv_shared::ExivEventData::ThoughtRequested {
                agent: agent.clone(),
                engine_id: _engine_id,
                message: msg.clone(),
                context,
            };
            
            let envelope = crate::EnvelopedEvent {
                event: Arc::new(exiv_shared::ExivEvent::with_trace(trace_id, thought_event_data)),
                issuer: None,
                correlation_id: None,
                depth: 0,
            };

            if let Err(e) = self.sender.send(envelope).await {
                error!(
                    target_agent_id = %target_agent_id,
                    error = %e,
                    "❌ Failed to dispatch ThoughtRequested"
                );
            }
        }

        // メモリへの保存
        if let Some(plugin) = memory_plugin {
            if let Some(_mem) = plugin.as_memory() {
                let agent_id = agent.id.clone();
                let plugin_clone = plugin.clone();
                let metrics = self.metrics.clone();
                // 🛑 停滞対策: 保存処理はバックグラウンドで行い、メインループをブロックしない
                tokio::spawn(async move {
                    if let Some(mem) = plugin_clone.as_memory() {
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            mem.store(agent_id.clone(), msg)
                        ).await {
                            Ok(Ok(())) => {
                                metrics.total_memories.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                            Ok(Err(e)) => {
                                error!(agent_id = %agent_id, error = %e, "❌ Memory store failed");
                            }
                            Err(_) => {
                                error!(agent_id = %agent_id, "❌ Memory store timed out (5s)");
                            }
                        }
                    }
                });
            }
        }

        Ok(())
    }
}

impl PluginCast for SystemHandler {
    fn as_any(&self) -> &dyn std::any::Any { self }
}

#[async_trait]
impl Plugin for SystemHandler {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "core.system".to_string(),
            name: "Kernel System Handler".to_string(),
            description: "Internal core logic handler".to_string(),
            version: "1.0.0".to_string(),
            category: exiv_shared::PluginCategory::System,
            service_type: exiv_shared::ServiceType::Reasoning,
            tags: vec![],
            is_active: true,
            is_configured: true,
            required_config_keys: vec![],
            action_icon: None,
            action_target: None,
            icon_data: None,
            magic_seal: 0x56455253,
            sdk_version: "internal".to_string(),
            required_permissions: vec![],
            provided_capabilities: vec![],
            provided_tools: vec![],
        }
    }

    async fn on_event(&self, event: &ExivEvent) -> anyhow::Result<Option<exiv_shared::ExivEventData>> {
        if let exiv_shared::ExivEventData::MessageReceived(msg) = &event.data {
            // Only trigger thinking for messages from users to prevent agent-agent loops
            if matches!(msg.source, exiv_shared::MessageSource::User { .. }) {
                let msg = msg.clone();
                self.handle_message(msg).await?;
            }
        }
        Ok(None)
    }
}
