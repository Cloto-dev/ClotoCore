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
    description = "Configurable consensus moderator for collective intelligence.",
    version = "0.3.0",
    category = "Tool",
    tags = ["#TOOL"],
    config_keys = ["synthesizer_engine", "min_proposals", "session_timeout_secs"],
    capabilities = ["Reasoning"]
)]
pub struct ModeratorPlugin {
    /// trace_id -> SessionState
    sessions: Arc<RwLock<HashMap<ExivId, SessionState>>>,
    config: Arc<RwLock<ModeratorConfig>>,
}

struct ModeratorConfig {
    /// Engine ID used for synthesis. Empty = use first engine from ConsensusRequested.
    synthesizer_engine: String,
    /// Minimum proposals required before synthesis starts.
    min_proposals: usize,
    /// Session timeout in seconds.
    session_timeout_secs: u64,
}

impl ModeratorConfig {
    fn from_plugin_config(config: &PluginConfig) -> Self {
        let synthesizer_engine = config
            .config_values
            .get("synthesizer_engine")
            .cloned()
            .unwrap_or_default();

        let min_proposals = config
            .config_values
            .get("min_proposals")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(2)
            .max(2); // 最低2件は必須

        let session_timeout_secs = config
            .config_values
            .get("session_timeout_secs")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(60)
            .max(10); // 最低10秒

        Self {
            synthesizer_engine,
            min_proposals,
            session_timeout_secs,
        }
    }
}

enum SessionState {
    Collecting {
        proposals: Vec<Proposal>,
        /// First engine ID from ConsensusRequested (used as fallback synthesizer).
        fallback_engine: String,
        created_at: std::time::Instant,
    },
    Synthesizing {
        created_at: std::time::Instant,
    },
}

// M-16: Named constant to prevent type confusion with string matching
const SYSTEM_CONSENSUS_AGENT: &str = "system.consensus";

struct Proposal {
    #[allow(dead_code)] // Reserved for future consensus attribution UI
    engine_id: String,
    content: String,
}

impl ModeratorPlugin {
    pub async fn new_plugin(config: PluginConfig) -> anyhow::Result<Self> {
        let moderator_config = ModeratorConfig::from_plugin_config(&config);
        let plugin = Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(moderator_config)),
        };
        // C-05: Spawn cleanup task for stale sessions
        plugin.spawn_cleanup_task();
        Ok(plugin)
    }

    fn spawn_cleanup_task(&self) {
        let sessions = self.sessions.clone();
        let config = self.config.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                let timeout_secs = config.read().await.session_timeout_secs;
                let mut map = sessions.write().await;
                let before = map.len();
                map.retain(|trace_id, state| {
                    let created_at = match state {
                        SessionState::Collecting { created_at, .. } => *created_at,
                        SessionState::Synthesizing { created_at } => *created_at,
                    };
                    if created_at.elapsed().as_secs() > timeout_secs {
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

    /// Resolve which engine to use for synthesis.
    async fn resolve_synthesizer(&self, fallback: &str) -> String {
        let cfg = self.config.read().await;
        if cfg.synthesizer_engine.is_empty() {
            fallback.to_string()
        } else {
            cfg.synthesizer_engine.clone()
        }
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
        let sessions = self.sessions.read().await;
        let cfg = self.config.read().await;
        let synthesizer = if cfg.synthesizer_engine.is_empty() {
            "auto (first engine)"
        } else {
            &cfg.synthesizer_engine
        };
        Ok(format!(
            "Consensus moderator: {} active session(s), min_proposals={}, synthesizer={}",
            sessions.len(),
            cfg.min_proposals,
            synthesizer,
        ))
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

                let fallback_engine = engine_ids.first().cloned().unwrap_or_default();

                // セッションの初期化 (Collecting Phase)
                {
                    let mut sessions = self.sessions.write().await;
                    sessions.insert(
                        event.trace_id,
                        SessionState::Collecting {
                            proposals: Vec::new(),
                            fallback_engine,
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

                let min_proposals = self.config.read().await.min_proposals;
                let mut sessions = self.sessions.write().await;

                // 状態遷移のためのフラグ
                let mut start_synthesis = false;
                let mut synthesis_prompt = String::new();
                let mut fallback = String::new();

                if let Some(state) = sessions.get_mut(&event.trace_id) {
                    match state {
                        SessionState::Collecting {
                            proposals,
                            fallback_engine,
                            created_at,
                        } => {
                            // 1. 提案の収集
                            proposals.push(Proposal {
                                engine_id: agent_id.clone(),
                                content: content.clone(),
                            });

                            tracing::info!(
                                trace_id = %event.trace_id,
                                "📥 Collected proposal from {} ({}/{})",
                                agent_id, proposals.len(), min_proposals,
                            );

                            if proposals.len() >= min_proposals {
                                // プロンプト作成用データを先に退避
                                let combined_views = proposals
                                    .iter()
                                    .enumerate()
                                    .map(|(i, p)| format!("## Opinion {}:\n{}", i + 1, p.content))
                                    .collect::<Vec<_>>()
                                    .join("\n\n");

                                fallback = fallback_engine.clone();

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
                    let synthesizer = self.resolve_synthesizer(&fallback).await;
                    tracing::info!(
                        trace_id = %event.trace_id,
                        synthesizer = %synthesizer,
                        "⚗️ Starting synthesis phase...",
                    );

                    let synthesizer_agent = AgentMetadata {
                        id: "agent.synthesizer".to_string(),
                        name: "Synthesizer".to_string(),
                        description: "AI Moderator".to_string(),
                        enabled: true,
                        last_seen: 0,
                        status: "online".to_string(),
                        default_engine_id: Some(synthesizer.clone()),
                        required_capabilities: vec![],
                        plugin_bindings: vec![],
                        metadata: HashMap::new(),
                    };

                    return Ok(Some(
                        ExivEvent::with_trace(
                            event.trace_id,
                            ExivEventData::ThoughtRequested {
                                agent: synthesizer_agent,
                                engine_id: synthesizer,
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
