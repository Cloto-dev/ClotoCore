use std::collections::{HashMap, HashSet};

use super::types::{
    AgentSnapshot, AutonomyLevel, CapabilityChange, EvolutionParams, FitnessScores, FitnessWeights,
    GenerationTrigger, InteractionMetrics, RegressionSeverity,
};

// ══════════════════════════════════════════════════════════════
// Pure Functions
// ══════════════════════════════════════════════════════════════

/// Calculate total fitness: Σ(w_i × Score_i) × SafetyGate
/// SafetyGate is multiplicative: safety violation = fitness 0
#[must_use]
pub fn calculate_fitness(scores: &FitnessScores, weights: &FitnessWeights) -> f64 {
    // SafetyGate: if safety != 1.0, everything is 0
    if scores.safety < 1.0 {
        return 0.0;
    }

    let weighted_sum = weights.cognitive * scores.cognitive
        + weights.behavioral * scores.behavioral
        + weights.safety * scores.safety
        + weights.autonomy * scores.autonomy.normalized()
        + weights.meta_learning * scores.meta_learning;

    // Clamp to [0.0, 1.0]
    weighted_sum.clamp(0.0, 1.0)
}

/// Threshold for detecting meaningful axis changes in delta computation.
/// Using 1e-6 instead of f64::EPSILON (~2.2e-16) because score changes below
/// 1e-6 are not practically significant for evolution tracking.
const DELTA_THRESHOLD: f64 = 1e-6;

/// Compute delta between two FitnessScores (only changed axes)
#[must_use]
pub fn compute_delta(current: &FitnessScores, previous: &FitnessScores) -> HashMap<String, f64> {
    let mut delta = HashMap::new();
    let d_cog = current.cognitive - previous.cognitive;
    let d_beh = current.behavioral - previous.behavioral;
    let d_aut = current.autonomy.normalized() - previous.autonomy.normalized();
    let d_met = current.meta_learning - previous.meta_learning;

    if d_cog.abs() > DELTA_THRESHOLD {
        delta.insert("cognitive".to_string(), d_cog);
    }
    if d_beh.abs() > DELTA_THRESHOLD {
        delta.insert("behavioral".to_string(), d_beh);
    }
    if d_aut.abs() > DELTA_THRESHOLD {
        delta.insert("autonomy".to_string(), d_aut);
    }
    if d_met.abs() > DELTA_THRESHOLD {
        delta.insert("meta_learning".to_string(), d_met);
    }
    delta
}

/// Detect axis ranking shift between two score sets
#[must_use]
pub fn detect_rebalance(current: &FitnessScores, previous: &FitnessScores) -> Vec<String> {
    let curr_rank = current.axis_ranking();
    let prev_rank = previous.axis_ranking();

    let mut shifted = Vec::new();
    for (i, (curr_axis, _)) in curr_rank.iter().enumerate() {
        if let Some((prev_axis, _)) = prev_rank.get(i) {
            if curr_axis != prev_axis {
                shifted.push(curr_axis.to_string());
            }
        }
    }
    shifted
}

/// Check if a generation transition should occur (metric-based).
///
/// This is a **pure function** operating on numerical inputs only.
/// It does NOT detect structural changes (plugin/capability additions);
/// that is handled separately by `detect_capability_gain()` in `evaluate()`.
///
/// Returns the metric-based trigger type, or None if no transition threshold is met.
/// The caller (`evaluate()`) integrates this with capability detection results
/// using the priority rules defined in `docs/e7-capability-gain.md`.
#[must_use]
pub fn check_triggers(
    current_fitness: f64,
    previous_fitness: f64,
    current_scores: &FitnessScores,
    previous_scores: &FitnessScores,
    params: &EvolutionParams,
    interactions_since_last_gen: u64,
) -> Option<GenerationTrigger> {
    // Safety breach always triggers, bypasses debounce
    if current_scores.safety < 1.0 {
        return Some(GenerationTrigger::SafetyBreach);
    }

    // Debounce: require minimum interactions (except safety breach)
    if interactions_since_last_gen < params.min_interactions {
        return None;
    }

    let delta_f = current_fitness - previous_fitness;

    // Relative thresholds
    let theta_growth = f64::max(params.theta_min, params.alpha * previous_fitness);
    let theta_regression = f64::max(params.theta_min, params.beta * previous_fitness);

    // Negative jump (regression) — priority 2
    if delta_f <= -theta_regression {
        return Some(GenerationTrigger::Regression);
    }

    // Autonomy upgrade — priority 4 (above Rebalance and Evolution)
    if current_scores.autonomy > previous_scores.autonomy {
        return Some(GenerationTrigger::AutonomyUpgrade);
    }

    // Axis rebalance — priority 5 (above Evolution)
    let shifted = detect_rebalance(current_scores, previous_scores);
    if !shifted.is_empty() {
        return Some(GenerationTrigger::Rebalance);
    }

    // Positive jump (evolution) — priority 6 (lowest growth trigger)
    if delta_f >= theta_growth {
        return Some(GenerationTrigger::Evolution);
    }

    None
}

/// Detect structural capability changes between two generation snapshots.
///
/// Compares `active_plugins` and `plugin_capabilities` between the previous and
/// current snapshots. Returns a list of changes for newly added plugins.
///
/// **Asymmetric**: Only detects capability *gains* (new plugins), not *losses*
/// (removed plugins). Capability loss is handled indirectly by Regression
/// (fitness drops after removal) or a future `CapabilityLoss` trigger.
///
/// This is a **pure function** operating on set-valued inputs (Vec<String>),
/// independent of the metric-based `check_triggers()`.
/// See `docs/e7-capability-gain.md` for design rationale.
#[must_use]
pub fn detect_capability_gain(
    prev_snapshot: &AgentSnapshot,
    curr_snapshot: &AgentSnapshot,
) -> Vec<CapabilityChange> {
    // If previous snapshot has no capability data (pre-E7), skip detection
    if prev_snapshot.plugin_capabilities.is_empty() && curr_snapshot.plugin_capabilities.is_empty()
    {
        return vec![];
    }

    // Step 1: new_plugins = curr.active_plugins ∖ prev.active_plugins
    let prev_plugins: HashSet<&str> = prev_snapshot
        .active_plugins
        .iter()
        .map(std::string::String::as_str)
        .collect();
    let new_plugins: Vec<&str> = curr_snapshot
        .active_plugins
        .iter()
        .filter(|p| !prev_plugins.contains(p.as_str()))
        .map(std::string::String::as_str)
        .collect();

    if new_plugins.is_empty() {
        return vec![];
    }

    // Step 2: prev_caps = ∪ { capabilities(p) | p ∈ prev.active_plugins }
    let prev_caps: HashSet<&str> = prev_snapshot
        .plugin_capabilities
        .values()
        .flat_map(|caps| caps.iter().map(std::string::String::as_str))
        .collect();

    // Step 3: For each new plugin, classify as major or minor
    let mut changes = Vec::new();
    for plugin_id in new_plugins {
        let caps = curr_snapshot
            .plugin_capabilities
            .get(plugin_id)
            .cloned()
            .unwrap_or_default();

        let is_major = caps.iter().any(|c| !prev_caps.contains(c.as_str()));

        changes.push(CapabilityChange {
            plugin_id: plugin_id.to_string(),
            capabilities: caps,
            is_major,
        });
    }

    changes
}

/// Determine regression severity.
/// Returns (is_severe, threshold).
/// Mild: beta <= |ΔF| < 2*beta
/// Severe: |ΔF| >= 2*beta
#[must_use]
pub fn regression_severity(
    delta_f: f64,
    previous_fitness: f64,
    params: &EvolutionParams,
) -> RegressionSeverity {
    let theta_regression = f64::max(params.theta_min, params.beta * previous_fitness);
    let abs_delta = delta_f.abs();

    if abs_delta >= 2.0 * theta_regression {
        RegressionSeverity::Severe
    } else if abs_delta >= theta_regression {
        RegressionSeverity::Mild
    } else {
        RegressionSeverity::None
    }
}

/// Calculate grace period length
#[must_use]
pub fn grace_period_length(
    interactions_in_last_gen: u64,
    gamma: f64,
    min_interactions: u64,
) -> u64 {
    let raw = gamma * interactions_in_last_gen as f64;
    if !raw.is_finite() || raw < 0.0 {
        return min_interactions;
    }
    let grace = raw.round() as u64;
    std::cmp::max(min_interactions, grace)
}

/// Compute behavioral score from interaction metrics.
/// Formula: 0.4 × response_rate + 0.3 × permission_precision + 0.3 × error_avoidance
#[must_use]
pub fn compute_behavioral_score(m: &InteractionMetrics) -> f64 {
    let total = m.total_interactions.max(1) as f64;
    let response_rate = m.thought_responses as f64 / total;
    let permission_precision = if m.permissions_requested > 0 {
        m.permissions_approved as f64 / m.permissions_requested as f64
    } else {
        1.0 // No permissions requested = no errors in permission handling
    };
    let error_avoidance = 1.0 - (m.errors as f64 / total);

    0.4 * response_rate + 0.3 * permission_precision + 0.3 * error_avoidance
}

/// Compute safety score from interaction metrics. Binary: 1.0 or 0.0.
#[must_use]
pub fn compute_safety_score(m: &InteractionMetrics) -> f64 {
    if m.safety_violation {
        0.0
    } else {
        1.0
    }
}

/// Compute autonomy level from intervention ratio.
#[must_use]
pub fn compute_autonomy_level(m: &InteractionMetrics) -> AutonomyLevel {
    let total = m.total_interactions.max(1) as f64;
    let intervention_ratio = m.human_interventions as f64 / total;

    if intervention_ratio >= 0.8 {
        AutonomyLevel::L0
    } else if intervention_ratio >= 0.6 {
        AutonomyLevel::L1
    } else if intervention_ratio >= 0.4 {
        AutonomyLevel::L2
    } else if intervention_ratio >= 0.2 {
        AutonomyLevel::L3
    } else if intervention_ratio >= 0.05 {
        AutonomyLevel::L4
    } else {
        AutonomyLevel::L5
    }
}
