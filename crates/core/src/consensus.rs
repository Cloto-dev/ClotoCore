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

/// Default synthetic consensus agent id (bug-282: now a configurable
/// `ConsensusConfig` field; this is only the back-compat default).
pub(crate) const DEFAULT_CONSENSUS_AGENT_ID: &str = "system.consensus";

/// Agent id under which the synthesizer request is dispatched (and therefore the
/// agent id its `ThoughtResponse` carries back). The `Synthesizing` state matches
/// incoming responses against this so a late raw proposal from one of the
/// proposing engines cannot be mistaken for the synthesis result (bug-408).
pub(crate) const SYNTHESIZER_AGENT_ID: &str = "agent.synthesizer";

/// How often the background cleanup task sweeps stale consensus sessions.
/// Independent of `session_timeout_secs` (which controls per-session TTL).
const SESSION_CLEANUP_INTERVAL_SECS: u64 = 30;

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
    /// Synthetic agent id for consensus-generated responses (bug-282).
    /// Configurable so a deployment can rename it instead of the kernel
    /// hard-coding the literal. Defaults to `DEFAULT_CONSENSUS_AGENT_ID`.
    pub synthetic_agent_id: String,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        Self {
            synthesizer_engine: String::new(),
            min_proposals: 2,
            session_timeout_secs: 60,
            synthetic_agent_id: DEFAULT_CONSENSUS_AGENT_ID.to_string(),
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
    /// Waiting for the synthesizer to produce a final response. Holds the agent
    /// id we expect that response from (bug-408) so that a late proposal from
    /// one of the proposing engines is not mistaken for the synthesis result.
    Synthesizing {
        synthesizer_agent_id: String,
        created_at: std::time::Instant,
    },
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
    /// bug-282: cached from `config.synthetic_agent_id` for lock-free reads on
    /// the thought-response hot path.
    synthetic_agent_id: String,
}

impl ConsensusOrchestrator {
    #[must_use]
    pub fn new(config: ConsensusConfig) -> Arc<Self> {
        let synthetic_agent_id = config.synthetic_agent_id.clone();
        let orchestrator = Arc::new(Self {
            state: RwLock::new(ConsensusState {
                sessions: HashMap::new(),
                config,
            }),
            synthetic_agent_id,
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

    #[allow(clippy::too_many_lines)]
    async fn on_thought_response(
        &self,
        trace_id: ClotoId,
        agent_id: &str,
        content: &str,
    ) -> Option<ClotoEventData> {
        // Ignore responses from the consensus system itself
        if agent_id == self.synthetic_agent_id {
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
                            synthesizer_agent_id: SYNTHESIZER_AGENT_ID.to_string(),
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

                SessionState::Synthesizing {
                    synthesizer_agent_id,
                    ..
                } => {
                    // bug-408: only the synthesizer's OWN response completes the
                    // session. A proposal from one of the proposing engines that
                    // arrives after synthesis was dispatched (e.g. a slow third
                    // engine when min_proposals=2) must be ignored — otherwise it
                    // would be returned as the "consensus" and the real synthesis
                    // response silently dropped.
                    let is_synthesis = agent_id == synthesizer_agent_id;
                    if is_synthesis {
                        info!(
                            trace_id = %trace_id,
                            "🏁 Synthesis complete via {}",
                            agent_id
                        );
                        state.sessions.remove(&trace_id);
                        Action::Complete {
                            content: content.to_string(),
                        }
                    } else {
                        info!(
                            trace_id = %trace_id,
                            "⏭️ Ignoring late proposal from {} during synthesis",
                            agent_id
                        );
                        Action::None
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
                    id: SYNTHESIZER_AGENT_ID.to_string(),
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
                agent_id: self.synthetic_agent_id.clone(),
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
                tokio::time::sleep(tokio::time::Duration::from_secs(
                    SESSION_CLEANUP_INTERVAL_SECS,
                ))
                .await;
                let Some(orchestrator) = this.upgrade() else {
                    break; // Orchestrator dropped, stop cleanup
                };
                let mut state = orchestrator.state.write().await;
                let timeout_secs = state.config.session_timeout_secs;
                let before = state.sessions.len();
                state.sessions.retain(|trace_id, session| {
                    let created_at = match session {
                        SessionState::Collecting { created_at, .. }
                        | SessionState::Synthesizing { created_at, .. } => *created_at,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn thought_response(trace_id: ClotoId, agent_id: &str, content: &str) -> ClotoEvent {
        ClotoEvent::with_trace(
            trace_id,
            ClotoEventData::ThoughtResponse {
                agent_id: agent_id.to_string(),
                engine_id: "engine".to_string(),
                content: content.to_string(),
                source_message_id: "msg".to_string(),
                auto_spoken: false,
            },
        )
    }

    fn consensus_requested(trace_id: ClotoId, engines: &[&str]) -> ClotoEvent {
        ClotoEvent::with_trace(
            trace_id,
            ClotoEventData::ConsensusRequested {
                task: "decide".to_string(),
                engine_ids: engines.iter().map(|s| (*s).to_string()).collect(),
            },
        )
    }

    fn test_config() -> ConsensusConfig {
        ConsensusConfig {
            synthesizer_engine: "synth-engine".to_string(),
            min_proposals: 2,
            session_timeout_secs: 60,
            synthetic_agent_id: DEFAULT_CONSENSUS_AGENT_ID.to_string(),
        }
    }

    /// bug-408: a late proposal arriving during the Synthesizing phase must NOT
    /// be returned as the consensus, and must NOT drop the session — only the
    /// synthesizer's own response completes it.
    #[tokio::test]
    async fn late_proposal_during_synthesis_is_ignored_not_completed() {
        let orch = ConsensusOrchestrator::new(test_config());
        let trace = ClotoId::new();

        // 3 engines, but min_proposals = 2 → synthesis starts after the 2nd.
        assert!(orch
            .handle_event(&consensus_requested(trace, &["e1", "e2", "e3"]))
            .await
            .is_none());

        // First proposal: still collecting.
        assert!(orch
            .handle_event(&thought_response(trace, "agent.e1", "view 1"))
            .await
            .is_none());

        // Second proposal: reaches the threshold → dispatch synthesizer.
        let synth_dispatch = orch
            .handle_event(&thought_response(trace, "agent.e2", "view 2"))
            .await;
        assert!(
            matches!(
                synth_dispatch,
                Some(ClotoEventData::ThoughtRequested { .. })
            ),
            "second proposal should dispatch the synthesizer"
        );

        // The slow third engine's raw proposal arrives BEFORE the synthesizer
        // responds. It must be ignored (no event) and must not complete/remove
        // the session.
        let late = orch
            .handle_event(&thought_response(trace, "agent.e3", "late raw view 3"))
            .await;
        assert!(
            late.is_none(),
            "a late raw proposal must not be treated as the consensus result"
        );

        // Now the real synthesizer response arrives → completes with the
        // synthetic agent id and the synthesizer's content (not the late view).
        let completed = orch
            .handle_event(&thought_response(
                trace,
                SYNTHESIZER_AGENT_ID,
                "synthesized conclusion",
            ))
            .await;
        match completed {
            Some(ClotoEventData::ThoughtResponse {
                agent_id, content, ..
            }) => {
                assert_eq!(agent_id, DEFAULT_CONSENSUS_AGENT_ID);
                assert_eq!(content, "synthesized conclusion");
            }
            other => panic!("expected synthesis completion, got {other:?}"),
        }
    }

    /// The synthesizer's own response (not a late proposal) completes normally.
    #[tokio::test]
    async fn synthesizer_response_completes_session() {
        let orch = ConsensusOrchestrator::new(test_config());
        let trace = ClotoId::new();

        orch.handle_event(&consensus_requested(trace, &["e1", "e2"]))
            .await;
        orch.handle_event(&thought_response(trace, "agent.e1", "a"))
            .await;
        orch.handle_event(&thought_response(trace, "agent.e2", "b"))
            .await;

        let completed = orch
            .handle_event(&thought_response(trace, SYNTHESIZER_AGENT_ID, "final"))
            .await;
        assert!(matches!(
            completed,
            Some(ClotoEventData::ThoughtResponse { .. })
        ));

        // Session is gone — a further response for the same trace is a no-op.
        let after = orch
            .handle_event(&thought_response(trace, SYNTHESIZER_AGENT_ID, "again"))
            .await;
        assert!(
            after.is_none(),
            "session should have been removed on completion"
        );
    }
}
