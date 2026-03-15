//! Consensus Orchestrator — kernel-level collective intelligence.
//!
//! Ported from `plugins/moderator/src/lib.rs` (~150 lines of state machine).
//! Manages multi-engine consensus sessions: collecting proposals from engines,
//! then synthesizing a unified response via a designated synthesizer engine.

use cloto_shared::{
    AgentMetadata, ClotoEvent, ClotoEventData, ClotoId, ClotoMessage, MessageSource,
};
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
    Synthesizing { created_at: std::time::Instant },
}

// ============================================================
// ConsensusOrchestrator
// ============================================================

/// G1.3: Unified state — single RwLock avoids fragmented locking.
struct ConsensusState {
    sessions: HashMap<ClotoId, SessionState>,
    config: ConsensusConfig,
}

pub struct ConsensusOrchestrator {
    state: RwLock<ConsensusState>,
}

impl ConsensusOrchestrator {
    #[must_use]
    pub fn new(config: ConsensusConfig) -> Arc<Self> {
        let orchestrator = Arc::new(Self {
            state: RwLock::new(ConsensusState {
                sessions: HashMap::new(),
                config,
            }),
        });
        orchestrator.spawn_cleanup_task();
        orchestrator
    }

    /// Update configuration at runtime (e.g., from ConfigUpdated event).
    pub async fn update_config(&self, config: ConsensusConfig) {
        self.state.write().await.config = config;
    }

    /// Handle a consensus-related event. Returns an optional response event.
    pub async fn handle_event(&self, event: &ClotoEvent) -> Option<ClotoEventData> {
        match &event.data {
            ClotoEventData::ConsensusRequested {
                task: _,
                engine_ids,
            } => {
                self.on_consensus_requested(event.trace_id, engine_ids)
                    .await
            }

            ClotoEventData::ThoughtResponse {
                agent_id, content, ..
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
        trace_id: ClotoId,
        engine_ids: &[String],
    ) -> Option<ClotoEventData> {
        info!(
            trace_id = %trace_id,
            "🤝 Consensus process started for {} engines",
            engine_ids.len()
        );

        let fallback_engine = engine_ids.first().cloned().unwrap_or_default();

        let mut state = self.state.write().await;
        state.sessions.insert(
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
        trace_id: ClotoId,
        agent_id: &str,
        content: &str,
    ) -> Option<ClotoEventData> {
        // Ignore responses from the consensus system itself
        if agent_id == SYSTEM_CONSENSUS_AGENT {
            return None;
        }

        // Determine action under a single write lock, then release before synthesis
        enum Action {
            None,
            NeedsSynthesis {
                combined_views: String,
                synthesizer: String,
            },
            Complete {
                content: String,
            },
        }

        let action = {
            let mut state = self.state.write().await;
            let min_proposals = state.config.min_proposals;
            let synthesizer_engine = state.config.synthesizer_engine.clone();

            let session = state.sessions.get_mut(&trace_id)?;

            match session {
                SessionState::Collecting {
                    proposals,
                    fallback_engine,
                    created_at,
                } => {
                    proposals.push(Proposal {
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
                        let combined_views = proposals
                            .iter()
                            .enumerate()
                            .map(|(i, p)| format!("## Opinion {}:\n{}", i + 1, p.content))
                            .collect::<Vec<_>>()
                            .join("\n\n");

                        let fallback = fallback_engine.clone();
                        let created = *created_at;

                        let synthesizer = if synthesizer_engine.is_empty() {
                            fallback
                        } else {
                            synthesizer_engine
                        };

                        *session = SessionState::Synthesizing {
                            created_at: created,
                        };

                        Action::NeedsSynthesis {
                            combined_views,
                            synthesizer,
                        }
                    } else {
                        Action::None
                    }
                }

                SessionState::Synthesizing { .. } => {
                    info!(
                        trace_id = %trace_id,
                        "🏁 Synthesis complete via {}",
                        agent_id
                    );
                    state.sessions.remove(&trace_id);
                    Action::Complete {
                        content: content.to_string(),
                    }
                }
            }
        }; // state lock dropped here

        match action {
            Action::None => None,
            Action::NeedsSynthesis {
                combined_views,
                synthesizer,
            } => {
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
                    metadata: HashMap::new(),
                    agent_type: "system".to_string(),
                };

                Some(
                    ClotoEvent::with_trace(
                        trace_id,
                        ClotoEventData::ThoughtRequested {
                            agent: synthesizer_agent,
                            engine_id: synthesizer,
                            message: ClotoMessage::new(MessageSource::System, synthesis_prompt),
                            context: vec![],
                        },
                    )
                    .data,
                )
            }
            Action::Complete { content } => Some(ClotoEventData::ThoughtResponse {
                agent_id: SYSTEM_CONSENSUS_AGENT.to_string(),
                engine_id: "consensus".to_string(),
                content,
                source_message_id: "consensus".to_string(),
                auto_spoken: false,
            }),
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
                let mut state = orchestrator.state.write().await;
                let timeout_secs = state.config.session_timeout_secs;
                let before = state.sessions.len();
                state.sessions.retain(|trace_id, session| {
                    let created_at = match session {
                        SessionState::Collecting { created_at, .. }
                        | SessionState::Synthesizing { created_at } => *created_at,
                    };
                    if created_at.elapsed().as_secs() > timeout_secs {
                        warn!(trace_id = %trace_id, "🕐 Consensus session timed out, removing");
                        false
                    } else {
                        true
                    }
                });
                let removed = before - state.sessions.len();
                if removed > 0 {
                    info!("🧹 Cleaned up {} stale consensus sessions", removed);
                }
            }
        });
    }
}
