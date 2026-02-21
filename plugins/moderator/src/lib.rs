use async_trait::async_trait;
use exiv_shared::{
    exiv_plugin, AgentMetadata, ExivEvent, ExivEventData, ExivId, ExivMessage, MessageSource,
    Plugin, PluginConfig,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[exiv_plugin(
    name = "core.moderator",
    kind = "Reasoning",
    description = "Consensus Moderator for collective intelligence.",
    version = "0.2.0",
    category = "Tool",
    tags = ["#TOOL"],
    capabilities = ["Reasoning"]
)]
pub struct ModeratorPlugin {
    /// trace_id -> SessionState
    sessions: Arc<RwLock<HashMap<ExivId, SessionState>>>,
}

enum SessionState {
    Collecting {
        proposals: Vec<Proposal>,
        created_at: std::time::Instant,
    },
    Synthesizing {
        created_at: std::time::Instant,
    },
}

const SESSION_TIMEOUT_SECS: u64 = 60;
// M-16: Named constant to prevent type confusion with string matching
const SYSTEM_CONSENSUS_AGENT: &str = "system.consensus";

struct Proposal {
    #[allow(dead_code)] // Reserved for future consensus attribution UI
    engine_id: String,
    content: String,
}

impl ModeratorPlugin {
    pub async fn new_plugin(_config: PluginConfig) -> anyhow::Result<Self> {
        let plugin = Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        };
        // C-05: Spawn cleanup task for stale sessions
        plugin.spawn_cleanup_task();
        Ok(plugin)
    }

    fn spawn_cleanup_task(&self) {
        let sessions = self.sessions.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                let mut map = sessions.write().await;
                let before = map.len();
                map.retain(|trace_id, state| {
                    let created_at = match state {
                        SessionState::Collecting { created_at, .. } => *created_at,
                        SessionState::Synthesizing { created_at } => *created_at,
                    };
                    if created_at.elapsed().as_secs() > SESSION_TIMEOUT_SECS {
                        tracing::warn!(trace_id = %trace_id, "🕐 Consensus session timed out, removing");
                        false
                    } else {
                        true
                    }
                });
                let removed = before - map.len();
                if removed > 0 {
                    tracing::info!("🧹 Cleaned up {} stale consensus sessions", removed);
                }
            }
        });
    }
}

#[async_trait]
impl exiv_shared::ReasoningEngine for ModeratorPlugin {
    fn name(&self) -> &str {
        "ConsensusModerator"
    }
    async fn think(
        &self,
        _agent: &AgentMetadata,
        _msg: &ExivMessage,
        _ctx: Vec<ExivMessage>,
    ) -> anyhow::Result<String> {
        Ok("I am observing the consensus process.".to_string())
    }
}

#[async_trait]
impl Plugin for ModeratorPlugin {
    fn manifest(&self) -> exiv_shared::PluginManifest {
        self.auto_manifest()
    }

    async fn on_event(&self, event: &ExivEvent) -> anyhow::Result<Option<ExivEventData>> {
        match &event.data {
            ExivEventData::ConsensusRequested {
                task: _,
                engine_ids,
            } => {
                tracing::info!(trace_id = %event.trace_id, "🤝 Consensus process started for {} engines", engine_ids.len());

                // セッションの初期化 (Collecting Phase)
                {
                    let mut sessions = self.sessions.write().await;
                    sessions.insert(
                        event.trace_id,
                        SessionState::Collecting {
                            proposals: Vec::new(),
                            created_at: std::time::Instant::now(),
                        },
                    );
                }

                return Ok(None);
            }

            ExivEventData::ThoughtResponse {
                agent_id,
                engine_id: _,
                content,
                source_message_id: _,
            } => {
                // 自分自身(Moderator)や、統合エンジン(Synthesizer)からの回答を区別する必要がある
                if agent_id == SYSTEM_CONSENSUS_AGENT {
                    return Ok(None);
                }

                let mut sessions = self.sessions.write().await;

                // 状態遷移のためのフラグ
                let mut start_synthesis = false;
                let mut synthesis_prompt = String::new();

                if let Some(state) = sessions.get_mut(&event.trace_id) {
                    match state {
                        SessionState::Collecting {
                            proposals,
                            created_at,
                        } => {
                            // 1. 提案の収集
                            proposals.push(Proposal {
                                engine_id: agent_id.clone(),
                                content: content.clone(),
                            });

                            tracing::info!(trace_id = %event.trace_id, "📥 Collected proposal from {} ({}/{})", agent_id, proposals.len(), 2);

                            if proposals.len() >= 2 {
                                // プロンプト作成用データを先に退避
                                let combined_views = proposals
                                    .iter()
                                    .enumerate()
                                    .map(|(i, p)| format!("## Opinion {}:\n{}", i + 1, p.content))
                                    .collect::<Vec<_>>()
                                    .join("\n\n");

                                // 2. 統合フェーズへ移行
                                *state = SessionState::Synthesizing {
                                    created_at: *created_at,
                                };
                                start_synthesis = true;

                                synthesis_prompt = format!(
                                    "You are a wise moderator. Synthesize the following opinions into a single, coherent conclusion.\n\n{}",
                                    combined_views
                                );
                            }
                        }
                        SessionState::Synthesizing { .. } => {
                            // 3. 統合回答の受信 (Final Phase)
                            tracing::info!(trace_id = %event.trace_id, "🏁 Synthesis complete via {}", agent_id);

                            // セッション終了 — ロックを解放してから返却
                            sessions.remove(&event.trace_id);
                            drop(sessions);

                            return Ok(Some(
                                ExivEvent::with_trace(
                                    event.trace_id,
                                    ExivEventData::ThoughtResponse {
                                        agent_id: SYSTEM_CONSENSUS_AGENT.to_string(),
                                        engine_id: "core.moderator".to_string(),
                                        content: content.clone(),
                                        source_message_id: "consensus".to_string(),
                                    },
                                )
                                .data,
                            ));
                        }
                    }
                }

                // ロックを解放してからイベント発行
                if start_synthesis {
                    tracing::info!(trace_id = %event.trace_id, "⚗️ Starting synthesis phase...");

                    // ダミーのエージェントメタデータ (統合用)
                    let synthesizer_agent = AgentMetadata {
                        id: "agent.synthesizer".to_string(),
                        name: "Synthesizer".to_string(),
                        description: "AI Moderator".to_string(),
                        enabled: true,
                        last_seen: 0,
                        status: "online".to_string(),
                        default_engine_id: Some("mind.deepseek".to_string()),
                        required_capabilities: vec![],
                        plugin_bindings: vec![],
                        metadata: HashMap::new(),
                    };

                    return Ok(Some(
                        ExivEvent::with_trace(
                            event.trace_id,
                            ExivEventData::ThoughtRequested {
                                agent: synthesizer_agent,
                                engine_id: "mind.deepseek".to_string(), // DeepSeekに統合を依頼
                                message: ExivMessage::new(MessageSource::System, synthesis_prompt),
                                context: vec![],
                            },
                        )
                        .data,
                    ));
                }
            }
            _ => {}
        }
        Ok(None)
    }
}
