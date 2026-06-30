//! Consensus configuration + prompt helpers.
//!
//! Consensus is a multi-engine deliberation mode: a `consensus:`-prefixed
//! message is sent to every configured engine, each returns an independent
//! proposal, and a designated synthesizer engine merges them into one answer.
//!
//! Since an earlier decision the orchestration runs **in-kernel**, synchronously, inside
//! `handlers/system.rs::run_consensus` — it fans the engines out through the
//! same `run_agentic_loop` the normal message path uses, collects the
//! proposals, then runs the synthesizer in-kernel. The kernel stamps the
//! `agent_id` on the synthesized response directly, so identity is structural
//! rather than inferred from event arrival order (this is what subsumes
//! bug-408; the reverted standalone guard `481279c` is no longer needed).
//!
//! This module is therefore reduced to the pieces the in-kernel path reuses:
//! the [`ConsensusConfig`] knobs and the two prompt builders. The former
//! event-driven `ConsensusOrchestrator` (sessions map + cleanup task +
//! `ThoughtRequested`/`ThoughtResponse` matching) was retired with the
//! redesign; see `docs/CONSENSUS_REVIVAL_DESIGN.md`.

/// Default synthetic consensus agent id (bug-282: now a configurable
/// `ConsensusConfig` field; this is only the back-compat default).
pub(crate) const DEFAULT_CONSENSUS_AGENT_ID: &str = "system.consensus";

// ============================================================
// Configuration
// ============================================================

#[derive(Clone)]
pub struct ConsensusConfig {
    /// Engine ID used for synthesis. Empty = use the first configured engine.
    pub synthesizer_engine: String,
    /// Minimum proposals required before synthesis starts.
    pub min_proposals: usize,
    /// Per-phase timeout in seconds (proposal collection and synthesis each).
    pub session_timeout_secs: u64,
    /// Synthetic agent id for consensus-generated responses (bug-282).
    /// Configurable so a deployment can rename it instead of the kernel
    /// hard-coding the literal. Defaults to `DEFAULT_CONSENSUS_AGENT_ID`.
    pub synthetic_agent_id: String,
    /// Fallback for when fewer distinct engines succeed than `min_proposals`
    /// (e.g. only one of two configured engines is registered): re-sample a
    /// working engine in an isolated session, through a varied framing, to reach
    /// quorum instead of hard-failing. Default on. (`CONSENSUS_ENGINE_REUSE`)
    pub engine_reuse: bool,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        Self {
            synthesizer_engine: String::new(),
            min_proposals: 2,
            session_timeout_secs: 60,
            synthetic_agent_id: DEFAULT_CONSENSUS_AGENT_ID.to_string(),
            engine_reuse: true,
        }
    }
}

/// Answer-neutral reasoning lenses used to re-frame a re-sampled proposal so the
/// same engine explores a different reasoning path instead of collapsing to an
/// identical answer (which low-temperature sampling would otherwise produce).
const REUSE_FRAMINGS: &[&str] = &[
    "Reason from first principles, then answer:",
    "Carefully weigh edge cases and caveats, then answer:",
    "Consider alternative viewpoints, then answer:",
    "Be precise and direct, then answer:",
];

/// Build the prompt for the `sample_index`-th sample of an engine. Sample 1 is
/// the original, un-framed proposal. Samples >= 2 are reuse fallbacks: the same
/// question re-asked through a rotating reasoning lens so re-sampled proposals
/// diverge instead of duplicating. The lenses are answer-neutral — they change
/// the reasoning path, not the requested answer.
pub(crate) fn framed_prompt(content: &str, sample_index: u32) -> String {
    if sample_index <= 1 {
        return content.to_string();
    }
    let lens = REUSE_FRAMINGS[(sample_index as usize - 2) % REUSE_FRAMINGS.len()];
    format!("{lens}\n\n{content}")
}

// ============================================================
// Prompt builders (reused by the in-kernel orchestrator)
// ============================================================

/// Format the collected proposals into a single combined-views block for the
/// synthesizer prompt.
pub(crate) fn combine_views(proposals: &[String]) -> String {
    proposals
        .iter()
        .enumerate()
        .map(|(i, p)| format!("## Opinion {}:\n{}", i + 1, p))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Build the synthesizer prompt from a combined-views block.
pub(crate) fn synthesis_prompt(combined_views: &str) -> String {
    format!(
        "You are a wise moderator. Synthesize the following opinions into a single, coherent conclusion.\n\n{combined_views}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_preserves_back_compat() {
        let c = ConsensusConfig::default();
        assert_eq!(c.synthetic_agent_id, DEFAULT_CONSENSUS_AGENT_ID);
        assert_eq!(c.min_proposals, 2);
        assert!(c.synthesizer_engine.is_empty());
        assert_eq!(c.session_timeout_secs, 60);
    }

    #[test]
    fn combine_views_numbers_each_opinion() {
        let combined = combine_views(&["alpha".to_string(), "beta".to_string()]);
        assert_eq!(combined, "## Opinion 1:\nalpha\n\n## Opinion 2:\nbeta");
    }

    #[test]
    fn combine_views_empty_is_empty() {
        assert_eq!(combine_views(&[]), "");
    }

    #[test]
    fn synthesis_prompt_embeds_views() {
        let p = synthesis_prompt("## Opinion 1:\nx");
        assert!(p.contains("wise moderator"));
        assert!(p.ends_with("## Opinion 1:\nx"));
    }

    #[test]
    fn framed_prompt_first_sample_is_unframed() {
        assert_eq!(framed_prompt("what is X?", 1), "what is X?");
        assert_eq!(framed_prompt("what is X?", 0), "what is X?");
    }

    #[test]
    fn framed_prompt_reuse_samples_diverge_but_keep_question() {
        let s2 = framed_prompt("what is X?", 2);
        let s3 = framed_prompt("what is X?", 3);
        // Both still carry the original question…
        assert!(s2.ends_with("what is X?"));
        assert!(s3.ends_with("what is X?"));
        // …but through different lenses, so re-samples don't duplicate.
        assert_ne!(s2, s3);
        assert_ne!(s2, "what is X?");
    }

    #[test]
    fn framed_prompt_lenses_rotate() {
        // 4 lenses → sample 2 and sample 6 reuse the same lens (wrap-around).
        assert_eq!(
            framed_prompt("q", 2),
            framed_prompt("q", 2 + REUSE_FRAMINGS.len() as u32)
        );
    }
}
