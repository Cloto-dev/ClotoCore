//! Consensus Orchestrator — kernel-level collective intelligence.
//!
//! Ported from `plugins/moderator/src/lib.rs` (~150 lines of state machine).
//! Manages multi-engine consensus sessions: collecting proposals from engines,
//! then synthesizing a unified response via a designated synthesizer engine.

use exiv_shared::{AgentMetadata, ExivEvent, ExivEventData, ExivId, ExivMessage, MessageSource};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Named constant for the synthetic consensus agent (prevents type confusion).
const SYSTEM_CONSENSUS_AGENT: &str = "system.consensus";

// ============================================================
// Configuration
// ============================================================

#[derive(Clone)]
pub struct ConsensusConfig {
    /// Engine ID used for synthesis. Empty = use first engine from ConsensusRequested.
    pub synthesizer_engine: String,
    /// Minimum proposals required before synthesis starts.
    pub min_proposals: usize,
    /// Session timeout in seconds.
    pub session_timeout_secs: u64,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        Self {
            synthesizer_engine: String::new(),
            min_proposals: 2,
            session_timeout_secs: 60,
        }
    }
}

// ============================================================
// Session State Machine
// ============================================================

struct Proposal {
    #[allow(dead_code)]
    engine_id: String,
    content: String,
}

enum SessionState {
    /// Collecting proposals from engines.
    Collecting {
        proposals: Vec<Proposal>,
        fallback_engine: String,
        created_at: std::time::Instant,
    },
    /// Waiting for the synthesizer to produce a final response.
    Synthesizing {
        created_at: std::time::Instant,
    },
}

// ============================================================
// ConsensusOrchestrator
// ============================================================

pub struct ConsensusOrchestrator {
    sessions: RwLock<HashMap<ExivId, SessionState>>,
    config: RwLock<ConsensusConfig>,
}

impl ConsensusOrchestrator {
    pub fn new(config: ConsensusConfig) -> Arc<Self> {
        let orchestrator = Arc::new(Self {
            sessions: RwLock::new(HashMap::new()),
            config: RwLock::new(config),
        });
        orchestrator.spawn_cleanup_task();
        orchestrator
    }

    /// Update configuration at runtime (e.g., from ConfigUpdated event).
    pub async fn update_config(&self, config: ConsensusConfig) {
        *self.config.write().await = config;
    }

    /// Handle a consensus-related event. Returns an optional response event.
    pub async fn handle_event(&self, event: &ExivEvent) -> Option<ExivEventData> {
        match &event.data {
            ExivEventData::ConsensusRequested {
                task: _,
                engine_ids,
            } => {
                self.on_consensus_requested(event.trace_id, engine_ids)
                    .await
            }

            ExivEventData::ThoughtResponse {
                agent_id,
                content,
                ..
            } => {
                self.on_thought_response(event.trace_id, agent_id, content)
                    .await
            }

            _ => None,
        }
    }

    // ── Event Handlers ──

    async fn on_consensus_requested(
        &self,
        trace_id: ExivId,
        engine_ids: &[String],
    ) -> Option<ExivEventData> {
        info!(
            trace_id = %trace_id,
            "🤝 Consensus process started for {} engines",
            engine_ids.len()
        );

        let fallback_engine = engine_ids.first().cloned().unwrap_or_default();

        let mut sessions = self.sessions.write().await;
        sessions.insert(
            trace_id,
            SessionState::Collecting {
                proposals: Vec::new(),
                fallback_engine,
                created_at: std::time::Instant::now(),
            },
        );

        None
    }

    async fn on_thought_response(
        &self,
        trace_id: ExivId,
        agent_id: &str,
        content: &str,
    ) -> Option<ExivEventData> {
        // Ignore responses from the consensus system itself
        if agent_id == SYSTEM_CONSENSUS_AGENT {
            return None;
        }

        let min_proposals = self.config.read().await.min_proposals;
        let mut sessions = self.sessions.write().await;

        let state = sessions.get_mut(&trace_id)?;

        match state {
            SessionState::Collecting {
                proposals,
                fallback_engine,
                created_at,
            } => {
                // 1. Collect proposal
                proposals.push(Proposal {
                    engine_id: agent_id.to_string(),
                    content: content.to_string(),
                });

                info!(
                    trace_id = %trace_id,
                    "📥 Collected proposal from {} ({}/{})",
                    agent_id,
                    proposals.len(),
                    min_proposals,
                );

                if proposals.len() >= min_proposals {
                    // Build synthesis prompt
                    let combined_views = proposals
                        .iter()
                        .enumerate()
                        .map(|(i, p)| format!("## Opinion {}:\n{}", i + 1, p.content))
                        .collect::<Vec<_>>()
                        .join("\n\n");

                    let fallback = fallback_engine.clone();
                    let created = *created_at;

                    // 2. Transition to Synthesizing
                    *state = SessionState::Synthesizing {
                        created_at: created,
                    };

                    // Resolve synthesizer engine (must drop sessions lock first)
                    drop(sessions);
                    let synthesizer = self.resolve_synthesizer(&fallback).await;

                    info!(
                        trace_id = %trace_id,
                        synthesizer = %synthesizer,
                        "⚗️ Starting synthesis phase...",
                    );

                    let synthesis_prompt = format!(
                        "You are a wise moderator. Synthesize the following opinions into a single, coherent conclusion.\n\n{}",
                        combined_views
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

                    return Some(
                        ExivEvent::with_trace(
                            trace_id,
                            ExivEventData::ThoughtRequested {
                                agent: synthesizer_agent,
                                engine_id: synthesizer,
                                message: ExivMessage::new(
                                    MessageSource::System,
                                    synthesis_prompt,
                                ),
                                context: vec![],
                            },
                        )
                        .data,
                    );
                }

                None
            }

            SessionState::Synthesizing { .. } => {
                // 3. Synthesis complete — final response
                info!(
                    trace_id = %trace_id,
                    "🏁 Synthesis complete via {}",
                    agent_id
                );

                sessions.remove(&trace_id);

                Some(ExivEventData::ThoughtResponse {
                    agent_id: SYSTEM_CONSENSUS_AGENT.to_string(),
                    engine_id: "consensus".to_string(),
                    content: content.to_string(),
                    source_message_id: "consensus".to_string(),
                })
            }
        }
    }

    // ── Helpers ──

    async fn resolve_synthesizer(&self, fallback: &str) -> String {
        let cfg = self.config.read().await;
        if cfg.synthesizer_engine.is_empty() {
            fallback.to_string()
        } else {
            cfg.synthesizer_engine.clone()
        }
    }

    fn spawn_cleanup_task(self: &Arc<Self>) {
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                let Some(orchestrator) = this.upgrade() else {
                    break; // Orchestrator dropped, stop cleanup
                };
                let timeout_secs = orchestrator.config.read().await.session_timeout_secs;
                let mut map = orchestrator.sessions.write().await;
                let before = map.len();
                map.retain(|trace_id, state| {
                    let created_at = match state {
                        SessionState::Collecting { created_at, .. } => *created_at,
                        SessionState::Synthesizing { created_at } => *created_at,
                    };
                    if created_at.elapsed().as_secs() > timeout_secs {
                        warn!(trace_id = %trace_id, "🕐 Consensus session timed out, removing");
                        false
                    } else {
                        true
                    }
                });
                let removed = before - map.len();
                if removed > 0 {
                    info!("🧹 Cleaned up {} stale consensus sessions", removed);
                }
            }
        });
    }
}
