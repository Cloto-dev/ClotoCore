//! Self-Evolution Benchmark Engine
//!
//! Implements the evolution tracking system defined in SELF_EVOLUTION_PROTOCOL.md.
//! The Kernel evaluates evolution but does NOT dictate evolution methods (Principle 1.1).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, error, warn};

use exiv_shared::{ExivEventData, PluginDataStore};

/// The plugin_id used for evolution data in PluginDataStore.
/// Evolution data lives in the Kernel namespace, not any plugin.
pub const EVOLUTION_STORE_ID: &str = "core.evolution";

/// Maximum number of rollbacks to the same target generation before skipping it.
const MAX_ROLLBACKS_PER_TARGET: u32 = 3;

/// Maximum entries kept in the fitness log to prevent unbounded growth.
const MAX_FITNESS_LOG_ENTRIES: usize = 10000;

/// Maximum entries kept in the rollback history.
const MAX_ROLLBACK_HISTORY_ENTRIES: usize = 100;

// ══════════════════════════════════════════════════════════════
// Data Structures
// ══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AutonomyLevel {
    L0 = 0,
    L1 = 1,
    L2 = 2,
    L3 = 3,
    L4 = 4,
    L5 = 5,
}

// Custom serialization: emit as normalized f64 (0.0-1.0) for frontend compatibility
impl Serialize for AutonomyLevel {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f64(self.normalized())
    }
}

impl<'de> Deserialize<'de> for AutonomyLevel {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de;

        struct AutonomyVisitor;
        impl<'de> de::Visitor<'de> for AutonomyVisitor {
            type Value = AutonomyLevel;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a float 0.0-1.0 or a string like \"L3\"")
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<AutonomyLevel, E> {
                Ok(AutonomyLevel::from_normalized(v))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<AutonomyLevel, E> {
                Ok(AutonomyLevel::from_normalized(v as f64 / 5.0))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<AutonomyLevel, E> {
                match v {
                    "L0" => Ok(AutonomyLevel::L0), "L1" => Ok(AutonomyLevel::L1),
                    "L2" => Ok(AutonomyLevel::L2), "L3" => Ok(AutonomyLevel::L3),
                    "L4" => Ok(AutonomyLevel::L4), "L5" => Ok(AutonomyLevel::L5),
                    _ => Err(de::Error::unknown_variant(v, &["L0","L1","L2","L3","L4","L5"])),
                }
            }
        }
        deserializer.deserialize_any(AutonomyVisitor)
    }
}

impl AutonomyLevel {
    /// Normalized value in [0.0, 1.0] for fitness calculation
    pub fn normalized(&self) -> f64 {
        (*self as u8) as f64 / 5.0
    }

    /// Create from a normalized f64 value (0.0-1.0), rounding to nearest level
    pub fn from_normalized(v: f64) -> Self {
        match (v * 5.0).round() as u8 {
            0 => Self::L0, 1 => Self::L1, 2 => Self::L2,
            3 => Self::L3, 4 => Self::L4, _ => Self::L5,
        }
    }
}

impl std::fmt::Display for AutonomyLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::L0 => write!(f, "L0"),
            Self::L1 => write!(f, "L1"),
            Self::L2 => write!(f, "L2"),
            Self::L3 => write!(f, "L3"),
            Self::L4 => write!(f, "L4"),
            Self::L5 => write!(f, "L5"),
        }
    }
}

/// 5-axis fitness scores
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitnessScores {
    pub cognitive: f64,
    pub behavioral: f64,
    pub safety: f64, // Binary: 0.0 or 1.0
    pub autonomy: AutonomyLevel,
    pub meta_learning: f64,
}

impl FitnessScores {
    /// Validates that all score values are finite and within [0.0, 1.0].
    pub fn validate(&self) -> anyhow::Result<()> {
        let fields = [
            ("cognitive", self.cognitive),
            ("behavioral", self.behavioral),
            ("safety", self.safety),
            ("meta_learning", self.meta_learning),
        ];
        for (name, val) in fields {
            if !val.is_finite() || val < 0.0 || val > 1.0 {
                anyhow::bail!("{} score must be in [0.0, 1.0], got {}", name, val);
            }
        }
        Ok(())
    }

    /// Returns the axis ranking (sorted by normalized score, descending).
    /// Safety is excluded because it's a binary gate (0.0 or 1.0), not a gradient score.
    pub fn axis_ranking(&self) -> Vec<(&str, f64)> {
        let mut axes = vec![
            ("cognitive", self.cognitive),
            ("behavioral", self.behavioral),
            ("autonomy", self.autonomy.normalized()),
            ("meta_learning", self.meta_learning),
        ];
        axes.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0)) // alphabetical tiebreaker for determinism
        });
        axes
    }
}

/// Per-axis weights for fitness calculation (user-customizable)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitnessWeights {
    pub cognitive: f64,
    pub behavioral: f64,
    pub safety: f64,
    pub autonomy: f64,
    pub meta_learning: f64,
}

impl Default for FitnessWeights {
    fn default() -> Self {
        Self {
            cognitive: 0.25,
            behavioral: 0.25,
            safety: 0.20,
            autonomy: 0.15,
            meta_learning: 0.15,
        }
    }
}

/// Evolution parameters (adjustable via dashboard)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionParams {
    pub alpha: f64,
    pub beta: f64,
    pub theta_min: f64,
    pub gamma: f64,
    pub min_interactions: u64,
    pub weights: FitnessWeights,
}

impl Default for EvolutionParams {
    fn default() -> Self {
        Self {
            alpha: 0.10,
            beta: 0.05,
            theta_min: 0.02,
            gamma: 0.25,
            min_interactions: 10,
            weights: FitnessWeights::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum GenerationTrigger {
    Evolution,
    Regression,
    Rebalance,
    SafetyBreach,
    CapabilityGain, // TODO: E7で実装予定
    AutonomyUpgrade,
}

impl std::fmt::Display for GenerationTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Evolution => write!(f, "evolution"),
            Self::Regression => write!(f, "regression"),
            Self::Rebalance => write!(f, "rebalance"),
            Self::SafetyBreach => write!(f, "safety_breach"),
            Self::CapabilityGain => write!(f, "capability_gain"),
            Self::AutonomyUpgrade => write!(f, "autonomy_upgrade"),
        }
    }
}

/// Agent state snapshot for rollback
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub active_plugins: Vec<String>,
    pub personality_hash: String,
    pub strategy_params: HashMap<String, serde_json::Value>,
}

/// A single generation record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationRecord {
    pub generation: u64,
    pub trigger: GenerationTrigger,
    pub timestamp: DateTime<Utc>,
    pub interactions_since_last: u64,
    pub scores: FitnessScores,
    pub delta: HashMap<String, f64>,
    pub fitness: f64,
    pub fitness_delta: f64,
    pub snapshot: AgentSnapshot,
}

/// Fitness log entry (time series)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitnessLogEntry {
    pub timestamp: DateTime<Utc>,
    pub interaction_count: u64,
    pub scores: FitnessScores,
    pub fitness: f64,
}

/// Rollback history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackRecord {
    pub timestamp: DateTime<Utc>,
    pub from_generation: u64,
    pub to_generation: u64,
    pub reason: String,
    pub rollback_count_to_target: u32,
}

/// Active grace period state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GracePeriodState {
    pub active: bool,
    pub started_at: DateTime<Utc>,
    pub interactions_at_start: u64,
    pub grace_interactions: u64,
    pub fitness_at_start: f64,
    pub affected_axis: String,
}

/// Current evolution status (for API responses)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionStatus {
    pub agent_id: String,
    pub current_generation: u64,
    pub fitness: f64,
    pub scores: FitnessScores,
    pub interaction_count: u64,
    pub interactions_since_last_gen: u64,
    pub trend: String,
    pub grace_period: Option<GracePeriodState>,
    pub autonomy_level: String,
    pub top_axes: Vec<(String, f64)>,
}

// ══════════════════════════════════════════════════════════════
// Pure Functions
// ══════════════════════════════════════════════════════════════

/// Calculate total fitness: Σ(w_i × Score_i) × SafetyGate
/// SafetyGate is multiplicative: safety violation = fitness 0
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

/// Compute delta between two FitnessScores (only changed axes)
pub fn compute_delta(current: &FitnessScores, previous: &FitnessScores) -> HashMap<String, f64> {
    let mut delta = HashMap::new();
    let d_cog = current.cognitive - previous.cognitive;
    let d_beh = current.behavioral - previous.behavioral;
    let d_aut = current.autonomy.normalized() - previous.autonomy.normalized();
    let d_met = current.meta_learning - previous.meta_learning;

    if d_cog.abs() > f64::EPSILON {
        delta.insert("cognitive".to_string(), d_cog);
    }
    if d_beh.abs() > f64::EPSILON {
        delta.insert("behavioral".to_string(), d_beh);
    }
    if d_aut.abs() > f64::EPSILON {
        delta.insert("autonomy".to_string(), d_aut);
    }
    if d_met.abs() > f64::EPSILON {
        delta.insert("meta_learning".to_string(), d_met);
    }
    delta
}

/// Detect axis ranking shift between two score sets
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

/// Check if a generation transition should occur.
/// Returns the trigger type, or None if no transition.
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

    // Negative jump (regression)
    if delta_f <= -theta_regression {
        return Some(GenerationTrigger::Regression);
    }

    // Positive jump (evolution)
    if delta_f >= theta_growth {
        return Some(GenerationTrigger::Evolution);
    }

    // Autonomy upgrade
    if current_scores.autonomy > previous_scores.autonomy {
        return Some(GenerationTrigger::AutonomyUpgrade);
    }

    // Axis rebalance
    let shifted = detect_rebalance(current_scores, previous_scores);
    if !shifted.is_empty() {
        return Some(GenerationTrigger::Rebalance);
    }

    None
}

/// Determine regression severity.
/// Returns (is_severe, threshold).
/// Mild: beta <= |ΔF| < 2*beta
/// Severe: |ΔF| >= 2*beta
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RegressionSeverity {
    None,
    Mild,
    Severe,
}

/// Calculate grace period length
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

// ══════════════════════════════════════════════════════════════
// Evolution Engine
// ══════════════════════════════════════════════════════════════

pub struct EvolutionEngine {
    store: Arc<dyn PluginDataStore>,
    pool: SqlitePool,
}

impl EvolutionEngine {
    pub fn new(store: Arc<dyn PluginDataStore>, pool: SqlitePool) -> Self {
        Self { store, pool }
    }

    // ── Storage Key Helpers ──

    fn key_generation(agent_id: &str, n: u64) -> String {
        format!("evolution:{}:generation:{}", agent_id, n)
    }

    fn key_latest(agent_id: &str) -> String {
        format!("evolution:{}:generation:latest", agent_id)
    }

    fn key_fitness_log(agent_id: &str) -> String {
        format!("evolution:{}:fitness_log", agent_id)
    }

    fn key_rollback_history(agent_id: &str) -> String {
        format!("evolution:{}:rollback_history", agent_id)
    }

    fn key_params(agent_id: &str) -> String {
        format!("evolution:{}:params", agent_id)
    }

    fn key_grace_period(agent_id: &str) -> String {
        format!("evolution:{}:grace_period", agent_id)
    }

    fn key_interaction_count(agent_id: &str) -> String {
        format!("evolution:{}:interaction_count", agent_id)
    }

    fn key_latest_fitness(agent_id: &str) -> String {
        format!("evolution:{}:latest_fitness", agent_id)
    }


    // ── Parameter Management ──

    pub async fn get_params(&self, agent_id: &str) -> anyhow::Result<EvolutionParams> {
        let key = Self::key_params(agent_id);
        match self.store.get_json(EVOLUTION_STORE_ID, &key).await? {
            Some(val) => Ok(serde_json::from_value(val)?),
            None => Ok(EvolutionParams::default()),
        }
    }

    pub async fn set_params(&self, agent_id: &str, params: &EvolutionParams) -> anyhow::Result<()> {
        let key = Self::key_params(agent_id);
        self.store.set_json(EVOLUTION_STORE_ID, &key, serde_json::to_value(params)?).await
    }

    // ── Interaction Tracking ──

    pub async fn get_interaction_count(&self, agent_id: &str) -> anyhow::Result<u64> {
        let key = Self::key_interaction_count(agent_id);
        match self.store.get_json(EVOLUTION_STORE_ID, &key).await? {
            Some(val) => Ok(serde_json::from_value(val)?),
            None => Ok(0),
        }
    }

    pub async fn increment_interaction(&self, agent_id: &str) -> anyhow::Result<u64> {
        let key = Self::key_interaction_count(agent_id);
        let count = self.store.increment_counter(EVOLUTION_STORE_ID, &key).await?;
        Ok(count as u64)
    }

    // ── Generation Management ──

    pub async fn get_latest_generation(&self, agent_id: &str) -> anyhow::Result<u64> {
        let key = Self::key_latest(agent_id);
        match self.store.get_json(EVOLUTION_STORE_ID, &key).await? {
            Some(val) => Ok(serde_json::from_value(val)?),
            None => Ok(0),
        }
    }

    pub async fn get_generation(&self, agent_id: &str, n: u64) -> anyhow::Result<Option<GenerationRecord>> {
        let key = Self::key_generation(agent_id, n);
        match self.store.get_json(EVOLUTION_STORE_ID, &key).await? {
            Some(val) => Ok(Some(serde_json::from_value(val)?)),
            None => Ok(None),
        }
    }

    pub async fn get_generation_history(&self, agent_id: &str, limit: usize) -> anyhow::Result<Vec<GenerationRecord>> {
        let latest = self.get_latest_generation(agent_id).await?;
        if latest == 0 {
            return Ok(vec![]);
        }

        let mut records = Vec::new();
        let start = if latest > limit as u64 { latest - limit as u64 + 1 } else { 1 };
        for n in (start..=latest).rev() {
            if let Some(record) = self.get_generation(agent_id, n).await? {
                records.push(record);
            }
        }
        Ok(records)
    }

    pub async fn create_generation(
        &self,
        agent_id: &str,
        trigger: GenerationTrigger,
        scores: FitnessScores,
        fitness: f64,
        fitness_delta: f64,
        delta: HashMap<String, f64>,
        interactions_since_last: u64,
        snapshot: AgentSnapshot,
    ) -> anyhow::Result<GenerationRecord> {
        let latest = self.get_latest_generation(agent_id).await?;
        let new_gen = latest + 1;

        let record = GenerationRecord {
            generation: new_gen,
            trigger,
            timestamp: Utc::now(),
            interactions_since_last,
            scores,
            delta,
            fitness,
            fitness_delta,
            snapshot,
        };

        // Store generation record
        let key = Self::key_generation(agent_id, new_gen);
        self.store.set_json(EVOLUTION_STORE_ID, &key, serde_json::to_value(&record)?).await?;

        // Update latest generation pointer
        let key = Self::key_latest(agent_id);
        self.store.set_json(EVOLUTION_STORE_ID, &key, serde_json::to_value(new_gen)?).await?;

        info!(
            agent_id = %agent_id,
            generation = new_gen,
            trigger = %record.trigger,
            fitness = fitness,
            "📈 New evolution generation"
        );

        Ok(record)
    }

    // ── Fitness Log ──

    pub async fn get_fitness_log(&self, agent_id: &str) -> anyhow::Result<Vec<FitnessLogEntry>> {
        let key = Self::key_fitness_log(agent_id);
        match self.store.get_json(EVOLUTION_STORE_ID, &key).await? {
            Some(val) => Ok(serde_json::from_value(val)?),
            None => Ok(vec![]),
        }
    }

    /// Appends an entry to the fitness log and returns the full log (including the new entry).
    /// Also caches the latest entry under a separate key for O(1) retrieval.
    pub async fn append_fitness_log(&self, agent_id: &str, entry: FitnessLogEntry) -> anyhow::Result<Vec<FitnessLogEntry>> {
        let mut log = self.get_fitness_log(agent_id).await?;
        log.push(entry);

        if log.len() > MAX_FITNESS_LOG_ENTRIES {
            log = log.split_off(log.len() - MAX_FITNESS_LOG_ENTRIES);
        }

        let key = Self::key_fitness_log(agent_id);
        self.store.set_json(EVOLUTION_STORE_ID, &key, serde_json::to_value(&log)?).await?;

        // Cache latest entry for O(1) access by get_latest_fitness()
        if let Some(latest) = log.last() {
            let cache_key = Self::key_latest_fitness(agent_id);
            self.store.set_json(EVOLUTION_STORE_ID, &cache_key, serde_json::to_value(latest)?).await?;
        }

        Ok(log)
    }

    pub async fn get_fitness_timeline(&self, agent_id: &str, limit: usize) -> anyhow::Result<Vec<FitnessLogEntry>> {
        let log = self.get_fitness_log(agent_id).await?;
        let start = if log.len() > limit { log.len() - limit } else { 0 };
        Ok(log[start..].to_vec())
    }

    // ── Grace Period ──

    pub async fn get_grace_period(&self, agent_id: &str) -> anyhow::Result<Option<GracePeriodState>> {
        let key = Self::key_grace_period(agent_id);
        match self.store.get_json(EVOLUTION_STORE_ID, &key).await? {
            Some(val) if !val.is_null() => {
                match serde_json::from_value::<GracePeriodState>(val) {
                    Ok(state) if state.active => Ok(Some(state)),
                    _ => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }

    pub async fn start_grace_period(
        &self,
        agent_id: &str,
        grace_interactions: u64,
        current_fitness: f64,
        affected_axis: &str,
    ) -> anyhow::Result<()> {
        let interaction_count = self.get_interaction_count(agent_id).await?;
        let state = GracePeriodState {
            active: true,
            started_at: Utc::now(),
            interactions_at_start: interaction_count,
            grace_interactions,
            fitness_at_start: current_fitness,
            affected_axis: affected_axis.to_string(),
        };
        let key = Self::key_grace_period(agent_id);
        self.store.set_json(EVOLUTION_STORE_ID, &key, serde_json::to_value(&state)?).await
    }

    pub async fn cancel_grace_period(&self, agent_id: &str) -> anyhow::Result<()> {
        let key = Self::key_grace_period(agent_id);
        // Set to null; get_grace_period will return None on deserialization failure
        self.store.set_json(EVOLUTION_STORE_ID, &key, serde_json::Value::Null).await
    }

    // ── Rollback History ──

    pub async fn get_rollback_history(&self, agent_id: &str) -> anyhow::Result<Vec<RollbackRecord>> {
        let key = Self::key_rollback_history(agent_id);
        match self.store.get_json(EVOLUTION_STORE_ID, &key).await? {
            Some(val) => Ok(serde_json::from_value(val)?),
            None => Ok(vec![]),
        }
    }

    async fn append_rollback_record(&self, agent_id: &str, record: RollbackRecord) -> anyhow::Result<()> {
        let mut history = self.get_rollback_history(agent_id).await?;
        history.push(record);

        if history.len() > MAX_ROLLBACK_HISTORY_ENTRIES {
            history = history.split_off(history.len() - MAX_ROLLBACK_HISTORY_ENTRIES);
        }

        let key = Self::key_rollback_history(agent_id);
        self.store.set_json(EVOLUTION_STORE_ID, &key, serde_json::to_value(&history)?).await
    }

    /// Count how many times we've rolled back to the given generation
    async fn rollback_count_to_gen(&self, agent_id: &str, target_gen: u64) -> anyhow::Result<u32> {
        let history = self.get_rollback_history(agent_id).await?;
        Ok(history.iter().filter(|r| r.to_generation == target_gen).count() as u32)
    }

    // ── Rollback Execution ──

    pub async fn execute_rollback(
        &self,
        agent_id: &str,
        to_generation: u64,
        reason: &str,
    ) -> anyhow::Result<Vec<ExivEventData>> {
        let from_gen = self.get_latest_generation(agent_id).await?;
        let mut events = Vec::new();
        let mut target_gen = to_generation;

        // Iterative cascade: find a valid rollback target
        let (target_record, rollback_count) = loop {
            let count = self.rollback_count_to_gen(agent_id, target_gen).await?;
            if count >= MAX_ROLLBACKS_PER_TARGET {
                warn!(
                    agent_id = %agent_id,
                    target_gen = target_gen,
                    max = MAX_ROLLBACKS_PER_TARGET,
                    "Max rollbacks reached for target generation, cascading to previous"
                );
                if target_gen > 1 {
                    target_gen -= 1;
                    continue;
                }
                // All generations exhausted
                error!(agent_id = %agent_id, "All generations exhausted, agent must be stopped");
                events.push(ExivEventData::EvolutionBreach {
                    agent_id: agent_id.to_string(),
                    violation_type: "rollback_exhausted".to_string(),
                    detail: "All generations exhausted after repeated rollbacks".to_string(),
                });
                return Ok(events);
            }

            match self.get_generation(agent_id, target_gen).await? {
                Some(record) => break (record, count),
                None => {
                    error!(agent_id = %agent_id, target_gen = target_gen, "Target generation not found, cascading to earlier");
                    if target_gen > 1 {
                        target_gen -= 1;
                        continue;
                    }
                    events.push(ExivEventData::EvolutionBreach {
                        agent_id: agent_id.to_string(),
                        violation_type: "rollback_target_missing".to_string(),
                        detail: format!("No valid generation found for rollback (tried down to gen {})", target_gen),
                    });
                    return Ok(events);
                }
            }
        };

        info!(
            agent_id = %agent_id,
            from_gen = from_gen,
            to_gen = target_gen,
            "🔄 Executing evolution rollback"
        );

        // Record rollback
        self.append_rollback_record(agent_id, RollbackRecord {
            timestamp: Utc::now(),
            from_generation: from_gen,
            to_generation: target_gen,
            reason: reason.to_string(),
            rollback_count_to_target: rollback_count + 1,
        }).await?;

        // Emit rollback event
        events.push(ExivEventData::EvolutionRollback {
            agent_id: agent_id.to_string(),
            from_generation: from_gen,
            to_generation: target_gen,
            reason: reason.to_string(),
        });

        // Create new generation with restored scores
        let delta = HashMap::new();
        let restored_fitness = calculate_fitness(&target_record.scores, &self.get_params(agent_id).await?.weights);
        let fitness_delta = restored_fitness - self.get_latest_fitness(agent_id).await?;

        self.create_generation(
            agent_id,
            GenerationTrigger::Regression,
            target_record.scores.clone(),
            restored_fitness,
            fitness_delta,
            delta,
            0,
            target_record.snapshot.clone(),
        ).await?;

        // Cancel grace period
        self.cancel_grace_period(agent_id).await?;

        // Audit log
        crate::db::spawn_audit_log(self.pool.clone(), crate::db::AuditLogEntry {
            timestamp: Utc::now(),
            event_type: "EVOLUTION_ROLLBACK".to_string(),
            actor_id: Some("kernel".to_string()),
            target_id: Some(agent_id.to_string()),
            permission: None,
            result: "SUCCESS".to_string(),
            reason: format!("Rollback gen {} → gen {}: {}", from_gen, target_gen, reason),
            metadata: None,
            trace_id: None,
        });

        Ok(events)
    }

    // ── Status ──

    async fn get_latest_fitness(&self, agent_id: &str) -> anyhow::Result<f64> {
        // Try cached latest entry first (O(1)), fall back to full log scan
        let cache_key = Self::key_latest_fitness(agent_id);
        if let Some(val) = self.store.get_json(EVOLUTION_STORE_ID, &cache_key).await? {
            if let Ok(entry) = serde_json::from_value::<FitnessLogEntry>(val) {
                return Ok(entry.fitness);
            }
        }
        let log = self.get_fitness_log(agent_id).await?;
        Ok(log.last().map(|e| e.fitness).unwrap_or(0.0))
    }

    pub async fn get_status(&self, agent_id: &str) -> anyhow::Result<EvolutionStatus> {
        // H-6: Parallel I/O for independent reads
        let (current_gen, fitness, total_interactions, grace) = tokio::join!(
            self.get_latest_generation(agent_id),
            self.get_latest_fitness(agent_id),
            self.get_interaction_count(agent_id),
            self.get_grace_period(agent_id),
        );
        let current_gen = current_gen?;
        let fitness = fitness?;
        let total_interactions = total_interactions?;
        let grace = grace?;

        // Get generation record for scores and interaction count
        let gen_record = if current_gen > 0 {
            self.get_generation(agent_id, current_gen).await?
        } else {
            None
        };

        // Interactions since last generation
        let interactions_since_last_gen = if let Some(ref record) = gen_record {
            let log = self.get_fitness_log(agent_id).await?;
            log.iter().filter(|e| e.timestamp >= record.timestamp).count() as u64
        } else {
            total_interactions
        };

        // Calculate trend from last few fitness entries
        let log = self.get_fitness_timeline(agent_id, 10).await?;
        let trend_val = if log.len() >= 2 {
            let recent = log.last().map(|e| e.fitness).unwrap_or(0.0);
            let earlier = log.first().map(|e| e.fitness).unwrap_or(0.0);
            recent - earlier
        } else {
            0.0
        };
        let trend = if trend_val > 0.01 { "improving".to_string() }
                    else if trend_val < -0.01 { "declining".to_string() }
                    else { "stable".to_string() };

        // Default scores when no generation exists
        let scores = gen_record.map(|r| r.scores).unwrap_or(FitnessScores {
            cognitive: 0.0,
            behavioral: 0.0,
            safety: 1.0,
            autonomy: AutonomyLevel::L0,
            meta_learning: 0.0,
        });

        let autonomy_level = scores.autonomy.to_string();
        let top_axes = scores.axis_ranking().into_iter()
            .map(|(name, val)| (name.to_string(), val))
            .collect();

        Ok(EvolutionStatus {
            agent_id: agent_id.to_string(),
            current_generation: current_gen,
            fitness,
            scores,
            interaction_count: total_interactions,
            interactions_since_last_gen,
            trend,
            grace_period: grace,
            autonomy_level,
            top_axes,
        })
    }

    // ── Main Evaluation Entry Point ──

    /// Called after each interaction. Evaluates fitness and checks for generation transitions.
    pub async fn evaluate(
        &self,
        agent_id: &str,
        scores: FitnessScores,
        snapshot: AgentSnapshot,
    ) -> anyhow::Result<Vec<ExivEventData>> {
        scores.validate()?;
        let params = self.get_params(agent_id).await?;
        let interaction_count = self.increment_interaction(agent_id).await?;
        let current_fitness = calculate_fitness(&scores, &params.weights);
        let mut events = Vec::new();

        // Append to fitness log and get the full log back (eliminates double-read)
        let log = self.append_fitness_log(agent_id, FitnessLogEntry {
            timestamp: Utc::now(),
            interaction_count,
            scores: scores.clone(),
            fitness: current_fitness,
        }).await?;
        if log.len() < 2 {
            // Not enough data to compare — if this is the first evaluation, create gen 0
            if self.get_latest_generation(agent_id).await? == 0 {
                let record = self.create_generation(
                    agent_id,
                    GenerationTrigger::Evolution,
                    scores,
                    current_fitness,
                    0.0,
                    HashMap::new(),
                    0,
                    snapshot,
                ).await?;
                events.push(ExivEventData::EvolutionGeneration {
                    agent_id: agent_id.to_string(),
                    generation: record.generation,
                    trigger: record.trigger.to_string(),
                });
            }
            return Ok(events);
        }

        let previous_entry = &log[log.len() - 2];
        let previous_fitness = previous_entry.fitness;
        let previous_scores = &previous_entry.scores;

        // Check grace period
        if let Some(grace) = self.get_grace_period(agent_id).await? {
            let elapsed = interaction_count - grace.interactions_at_start;
            if current_fitness >= grace.fitness_at_start {
                // Recovered → cancel grace
                info!(agent_id = %agent_id, "Grace period: fitness recovered, cancelling");
                self.cancel_grace_period(agent_id).await?;
            } else if elapsed >= grace.grace_interactions {
                // Grace expired → rollback
                warn!(agent_id = %agent_id, "Grace period expired, triggering rollback");
                let latest_gen = self.get_latest_generation(agent_id).await?;
                let target_gen = if latest_gen > 1 { latest_gen - 1 } else { 1 };
                let rollback_events = self.execute_rollback(
                    agent_id,
                    target_gen,
                    &format!("Grace period expired for {} axis", grace.affected_axis),
                ).await?;
                events.extend(rollback_events);
                return Ok(events);
            } else {
                // Still in grace period — emit warning
                let remaining = grace.grace_interactions - elapsed;
                events.push(ExivEventData::EvolutionWarning {
                    agent_id: agent_id.to_string(),
                    severity: "mild".to_string(),
                    affected_area: grace.affected_axis.clone(),
                    direction: "regression".to_string(),
                    grace_remaining: remaining,
                    suggestion: format!("{} patterns may need adjustment", grace.affected_axis),
                });
            }
        }

        // Calculate interactions since last generation
        let latest_gen = self.get_latest_generation(agent_id).await?;
        let last_gen_record = self.get_generation(agent_id, latest_gen).await?;
        let interactions_since_last_gen = if let Some(ref rec) = last_gen_record {
            log.iter().filter(|e| e.timestamp > rec.timestamp).count() as u64
        } else {
            interaction_count
        };

        // Check triggers
        let trigger = check_triggers(
            current_fitness,
            previous_fitness,
            &scores,
            previous_scores,
            &params,
            interactions_since_last_gen,
        );

        if let Some(trigger) = trigger {
            match trigger {
                GenerationTrigger::SafetyBreach => {
                    self.handle_safety_breach(agent_id, latest_gen, &mut events).await?;
                }
                GenerationTrigger::Regression => {
                    self.handle_regression(
                        agent_id, scores, previous_scores, current_fitness, previous_fitness,
                        &params, interactions_since_last_gen, snapshot, latest_gen, &mut events,
                    ).await?;
                }
                GenerationTrigger::Evolution
                | GenerationTrigger::AutonomyUpgrade
                | GenerationTrigger::CapabilityGain => {
                    self.handle_positive_trigger(
                        agent_id, trigger, scores, previous_scores, current_fitness, previous_fitness,
                        interactions_since_last_gen, snapshot, &mut events,
                    ).await?;
                }
                GenerationTrigger::Rebalance => {
                    self.handle_rebalance(
                        agent_id, scores, previous_scores, current_fitness, previous_fitness,
                        interactions_since_last_gen, snapshot, &mut events,
                    ).await?;
                }
            }
        }

        Ok(events)
    }

    // ── Trigger Handlers (extracted from evaluate) ──

    async fn handle_safety_breach(
        &self,
        agent_id: &str,
        latest_gen: u64,
        events: &mut Vec<ExivEventData>,
    ) -> anyhow::Result<()> {
        events.push(ExivEventData::EvolutionBreach {
            agent_id: agent_id.to_string(),
            violation_type: "safety_gate_zero".to_string(),
            detail: "SafetyGate triggered: safety score dropped below 1.0".to_string(),
        });
        if latest_gen > 1 {
            let rollback_events = self.execute_rollback(agent_id, latest_gen - 1, "Safety breach detected").await?;
            events.extend(rollback_events);
        } else if latest_gen == 1 {
            warn!(agent_id = %agent_id, "Safety breach on generation 1, no earlier generation available");
        }
        Ok(())
    }

    async fn handle_regression(
        &self,
        agent_id: &str,
        scores: FitnessScores,
        previous_scores: &FitnessScores,
        current_fitness: f64,
        previous_fitness: f64,
        params: &EvolutionParams,
        interactions_since_last_gen: u64,
        snapshot: AgentSnapshot,
        latest_gen: u64,
        events: &mut Vec<ExivEventData>,
    ) -> anyhow::Result<()> {
        let delta_f = current_fitness - previous_fitness;
        let severity = regression_severity(delta_f, previous_fitness, params);

        match severity {
            RegressionSeverity::Severe => {
                warn!(agent_id = %agent_id, delta = delta_f, "Severe regression, immediate rollback");
                if latest_gen > 1 {
                    let rollback_events = self.execute_rollback(agent_id, latest_gen - 1, "Severe regression detected").await?;
                    events.extend(rollback_events);
                } else if latest_gen == 1 {
                    warn!(agent_id = %agent_id, "Severe regression on generation 1, no earlier generation available");
                }
            }
            RegressionSeverity::Mild => {
                let grace_len = grace_period_length(interactions_since_last_gen, params.gamma, params.min_interactions);
                let affected_axis = compute_delta(&scores, previous_scores)
                    .into_iter()
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(k, _)| k)
                    .unwrap_or_else(|| "unknown".to_string());

                self.start_grace_period(agent_id, grace_len, current_fitness, &affected_axis).await?;
                events.push(ExivEventData::EvolutionWarning {
                    agent_id: agent_id.to_string(),
                    severity: "mild".to_string(),
                    affected_area: affected_axis.clone(),
                    direction: "regression".to_string(),
                    grace_remaining: grace_len,
                    suggestion: format!("{} patterns may need adjustment", affected_axis),
                });

                let delta = compute_delta(&scores, previous_scores);
                let record = self.create_generation(
                    agent_id, GenerationTrigger::Regression, scores,
                    current_fitness, current_fitness - previous_fitness,
                    delta, interactions_since_last_gen, snapshot,
                ).await?;
                events.push(ExivEventData::EvolutionGeneration {
                    agent_id: agent_id.to_string(),
                    generation: record.generation,
                    trigger: record.trigger.to_string(),
                });
            }
            RegressionSeverity::None => {}
        }
        Ok(())
    }

    async fn handle_positive_trigger(
        &self,
        agent_id: &str,
        trigger: GenerationTrigger,
        scores: FitnessScores,
        previous_scores: &FitnessScores,
        current_fitness: f64,
        previous_fitness: f64,
        interactions_since_last_gen: u64,
        snapshot: AgentSnapshot,
        events: &mut Vec<ExivEventData>,
    ) -> anyhow::Result<()> {
        self.cancel_grace_period(agent_id).await?;
        let delta = compute_delta(&scores, previous_scores);
        let record = self.create_generation(
            agent_id, trigger, scores,
            current_fitness, current_fitness - previous_fitness,
            delta, interactions_since_last_gen, snapshot,
        ).await?;
        events.push(ExivEventData::EvolutionGeneration {
            agent_id: agent_id.to_string(),
            generation: record.generation,
            trigger: record.trigger.to_string(),
        });
        Ok(())
    }

    async fn handle_rebalance(
        &self,
        agent_id: &str,
        scores: FitnessScores,
        previous_scores: &FitnessScores,
        current_fitness: f64,
        previous_fitness: f64,
        interactions_since_last_gen: u64,
        snapshot: AgentSnapshot,
        events: &mut Vec<ExivEventData>,
    ) -> anyhow::Result<()> {
        let shifted = detect_rebalance(&scores, previous_scores);
        let delta = compute_delta(&scores, previous_scores);
        let record = self.create_generation(
            agent_id, GenerationTrigger::Rebalance, scores,
            current_fitness, current_fitness - previous_fitness,
            delta, interactions_since_last_gen, snapshot,
        ).await?;
        events.push(ExivEventData::EvolutionRebalance {
            agent_id: agent_id.to_string(),
            shifted_axes: shifted,
            generation: record.generation,
        });
        events.push(ExivEventData::EvolutionGeneration {
            agent_id: agent_id.to_string(),
            generation: record.generation,
            trigger: record.trigger.to_string(),
        });
        Ok(())
    }

    /// Simplified interaction hook — checks grace period only.
    /// Does NOT increment the interaction counter (that's done by `evaluate()`)
    /// to prevent double-counting when both hooks fire.
    pub async fn on_interaction(&self, agent_id: &str) -> anyhow::Result<Vec<ExivEventData>> {
        let interaction_count = self.get_interaction_count(agent_id).await?;
        let mut events = Vec::new();

        // Check grace period expiry
        if let Some(grace) = self.get_grace_period(agent_id).await? {
            let elapsed = interaction_count - grace.interactions_at_start;
            if elapsed >= grace.grace_interactions {
                warn!(agent_id = %agent_id, "Grace period expired during interaction, triggering rollback");
                let latest_gen = self.get_latest_generation(agent_id).await?;
                let target_gen = if latest_gen > 1 { latest_gen - 1 } else { 1 };
                let rollback_events = self.execute_rollback(
                    agent_id,
                    target_gen,
                    &format!("Grace period expired for {} axis", grace.affected_axis),
                ).await?;
                events.extend(rollback_events);
            } else {
                let remaining = grace.grace_interactions - elapsed;
                events.push(ExivEventData::EvolutionWarning {
                    agent_id: agent_id.to_string(),
                    severity: "mild".to_string(),
                    affected_area: grace.affected_axis.clone(),
                    direction: "regression".to_string(),
                    grace_remaining: remaining,
                    suggestion: format!("{} patterns may need adjustment", grace.affected_axis),
                });
            }
        }

        Ok(events)
    }
}

// ══════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn default_weights() -> FitnessWeights {
        FitnessWeights::default()
    }

    fn default_params() -> EvolutionParams {
        EvolutionParams::default()
    }

    fn sample_scores(cognitive: f64, behavioral: f64, safety: f64, autonomy: AutonomyLevel, meta: f64) -> FitnessScores {
        FitnessScores {
            cognitive,
            behavioral,
            safety,
            autonomy,
            meta_learning: meta,
        }
    }

    // ── AutonomyLevel tests ──

    #[test]
    fn test_autonomy_level_normalized() {
        assert!((AutonomyLevel::L0.normalized() - 0.0).abs() < f64::EPSILON);
        assert!((AutonomyLevel::L1.normalized() - 0.2).abs() < f64::EPSILON);
        assert!((AutonomyLevel::L2.normalized() - 0.4).abs() < f64::EPSILON);
        assert!((AutonomyLevel::L3.normalized() - 0.6).abs() < f64::EPSILON);
        assert!((AutonomyLevel::L4.normalized() - 0.8).abs() < f64::EPSILON);
        assert!((AutonomyLevel::L5.normalized() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_autonomy_level_ordering() {
        assert!(AutonomyLevel::L0 < AutonomyLevel::L1);
        assert!(AutonomyLevel::L4 < AutonomyLevel::L5);
    }

    // ── calculate_fitness tests ──

    #[test]
    fn test_fitness_normal_calculation() {
        let scores = sample_scores(0.8, 0.7, 1.0, AutonomyLevel::L3, 0.5);
        let weights = default_weights();
        let fitness = calculate_fitness(&scores, &weights);
        // 0.25*0.8 + 0.25*0.7 + 0.20*1.0 + 0.15*0.6 + 0.15*0.5
        // = 0.20 + 0.175 + 0.20 + 0.09 + 0.075 = 0.74
        assert!((fitness - 0.74).abs() < 0.001);
    }

    #[test]
    fn test_safety_gate_zeroes_fitness() {
        let scores = sample_scores(1.0, 1.0, 0.0, AutonomyLevel::L5, 1.0);
        let weights = default_weights();
        let fitness = calculate_fitness(&scores, &weights);
        assert!((fitness - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_perfect_scores_fitness() {
        let scores = sample_scores(1.0, 1.0, 1.0, AutonomyLevel::L5, 1.0);
        let weights = default_weights();
        let fitness = calculate_fitness(&scores, &weights);
        assert!((fitness - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_zero_scores_with_safety() {
        let scores = sample_scores(0.0, 0.0, 1.0, AutonomyLevel::L0, 0.0);
        let weights = default_weights();
        let fitness = calculate_fitness(&scores, &weights);
        // Only safety weight contributes: 0.20 * 1.0 = 0.20
        assert!((fitness - 0.20).abs() < 0.001);
    }

    // ── check_triggers tests ──

    #[test]
    fn test_trigger_positive_jump() {
        let params = default_params();
        let prev_scores = sample_scores(0.5, 0.5, 1.0, AutonomyLevel::L2, 0.3);
        let prev_fitness = calculate_fitness(&prev_scores, &params.weights);

        // Significant improvement
        let curr_scores = sample_scores(0.8, 0.8, 1.0, AutonomyLevel::L2, 0.5);
        let curr_fitness = calculate_fitness(&curr_scores, &params.weights);

        let trigger = check_triggers(curr_fitness, prev_fitness, &curr_scores, &prev_scores, &params, 15);
        assert_eq!(trigger, Some(GenerationTrigger::Evolution));
    }

    #[test]
    fn test_trigger_negative_jump() {
        let params = default_params();
        let prev_scores = sample_scores(0.8, 0.8, 1.0, AutonomyLevel::L3, 0.6);
        let prev_fitness = calculate_fitness(&prev_scores, &params.weights);

        // Significant regression
        let curr_scores = sample_scores(0.3, 0.3, 1.0, AutonomyLevel::L3, 0.2);
        let curr_fitness = calculate_fitness(&curr_scores, &params.weights);

        let trigger = check_triggers(curr_fitness, prev_fitness, &curr_scores, &prev_scores, &params, 15);
        assert_eq!(trigger, Some(GenerationTrigger::Regression));
    }

    #[test]
    fn test_safety_breach_bypasses_debounce() {
        let params = default_params();
        let prev_scores = sample_scores(0.8, 0.8, 1.0, AutonomyLevel::L3, 0.6);
        let prev_fitness = calculate_fitness(&prev_scores, &params.weights);

        let curr_scores = sample_scores(0.8, 0.8, 0.0, AutonomyLevel::L3, 0.6);
        let curr_fitness = calculate_fitness(&curr_scores, &params.weights);

        // interactions=0 should normally be debounced, but safety breach bypasses
        let trigger = check_triggers(curr_fitness, prev_fitness, &curr_scores, &prev_scores, &params, 0);
        assert_eq!(trigger, Some(GenerationTrigger::SafetyBreach));
    }

    #[test]
    fn test_debounce_prevents_generation() {
        let params = default_params();
        let prev_scores = sample_scores(0.5, 0.5, 1.0, AutonomyLevel::L2, 0.3);
        let prev_fitness = calculate_fitness(&prev_scores, &params.weights);

        let curr_scores = sample_scores(0.8, 0.8, 1.0, AutonomyLevel::L2, 0.5);
        let curr_fitness = calculate_fitness(&curr_scores, &params.weights);

        // Only 5 interactions (below min_interactions=10)
        let trigger = check_triggers(curr_fitness, prev_fitness, &curr_scores, &prev_scores, &params, 5);
        assert_eq!(trigger, None);
    }

    #[test]
    fn test_trigger_autonomy_upgrade() {
        let params = default_params();
        let prev_scores = sample_scores(0.6, 0.6, 1.0, AutonomyLevel::L2, 0.4);
        let prev_fitness = calculate_fitness(&prev_scores, &params.weights);

        // Same scores but higher autonomy (small fitness change, under growth threshold)
        let curr_scores = sample_scores(0.6, 0.6, 1.0, AutonomyLevel::L3, 0.4);
        let curr_fitness = calculate_fitness(&curr_scores, &params.weights);

        let trigger = check_triggers(curr_fitness, prev_fitness, &curr_scores, &prev_scores, &params, 15);
        assert_eq!(trigger, Some(GenerationTrigger::AutonomyUpgrade));
    }

    #[test]
    fn test_trigger_rebalance() {
        let params = default_params();
        // cognitive > behavioral initially
        let prev_scores = sample_scores(0.8, 0.3, 1.0, AutonomyLevel::L2, 0.4);
        let prev_fitness = calculate_fitness(&prev_scores, &params.weights);

        // behavioral > cognitive now (swap, but similar total fitness)
        let curr_scores = sample_scores(0.3, 0.8, 1.0, AutonomyLevel::L2, 0.4);
        let curr_fitness = calculate_fitness(&curr_scores, &params.weights);

        let trigger = check_triggers(curr_fitness, prev_fitness, &curr_scores, &prev_scores, &params, 15);
        // Since fitness is similar but axes swapped, should detect rebalance
        assert_eq!(trigger, Some(GenerationTrigger::Rebalance));
    }

    #[test]
    fn test_no_trigger_on_small_change() {
        let params = default_params();
        // Use non-tied scores to avoid rebalance from ranking instability
        let prev_scores = sample_scores(0.6, 0.5, 1.0, AutonomyLevel::L2, 0.3);
        let prev_fitness = calculate_fitness(&prev_scores, &params.weights);

        // Very small change (cognitive 0.6 → 0.61)
        let curr_scores = sample_scores(0.61, 0.5, 1.0, AutonomyLevel::L2, 0.3);
        let curr_fitness = calculate_fitness(&curr_scores, &params.weights);

        let trigger = check_triggers(curr_fitness, prev_fitness, &curr_scores, &prev_scores, &params, 15);
        assert_eq!(trigger, None);
    }

    // ── Relative threshold tests ──

    #[test]
    fn test_relative_threshold_scaling() {
        let params = default_params();
        // High fitness → higher threshold
        let theta_high = f64::max(params.theta_min, params.alpha * 0.8);
        assert!((theta_high - 0.08).abs() < 0.001); // 0.10 * 0.8

        // Low fitness → theta_min floor
        let theta_low = f64::max(params.theta_min, params.alpha * 0.1);
        assert!((theta_low - 0.02).abs() < 0.001); // max(0.02, 0.01) = 0.02
    }

    // ── Regression severity tests ──

    #[test]
    fn test_regression_severity_mild() {
        let params = default_params();
        // beta = 0.05, threshold = max(0.02, 0.05 * 0.6) = 0.03
        // mild: 0.03 <= |ΔF| < 0.06
        let severity = regression_severity(-0.04, 0.6, &params);
        assert_eq!(severity, RegressionSeverity::Mild);
    }

    #[test]
    fn test_regression_severity_severe() {
        let params = default_params();
        // threshold = 0.03, severe: |ΔF| >= 0.06
        let severity = regression_severity(-0.10, 0.6, &params);
        assert_eq!(severity, RegressionSeverity::Severe);
    }

    #[test]
    fn test_regression_severity_none() {
        let params = default_params();
        let severity = regression_severity(-0.01, 0.6, &params);
        assert_eq!(severity, RegressionSeverity::None);
    }

    // ── Grace period length tests ──

    #[test]
    fn test_grace_period_length_minimum() {
        // gamma=0.25, interactions=20 → grace=5, but min=10
        let grace = grace_period_length(20, 0.25, 10);
        assert_eq!(grace, 10);
    }

    #[test]
    fn test_grace_period_length_calculated() {
        // gamma=0.25, interactions=100 → grace=25, above min=10
        let grace = grace_period_length(100, 0.25, 10);
        assert_eq!(grace, 25);
    }

    // ── Axis ranking / rebalance tests ──

    #[test]
    fn test_detect_rebalance_no_change() {
        // Rankings: cognitive > behavioral > meta_learning > autonomy (both)
        let a = sample_scores(0.8, 0.6, 1.0, AutonomyLevel::L1, 0.3);
        let b = sample_scores(0.85, 0.65, 1.0, AutonomyLevel::L1, 0.35);
        let shifted = detect_rebalance(&b, &a);
        assert!(shifted.is_empty());
    }

    #[test]
    fn test_detect_rebalance_with_swap() {
        let a = sample_scores(0.8, 0.3, 1.0, AutonomyLevel::L2, 0.4);
        let b = sample_scores(0.3, 0.8, 1.0, AutonomyLevel::L2, 0.4);
        let shifted = detect_rebalance(&b, &a);
        assert!(!shifted.is_empty());
    }

    // ── compute_delta tests ──

    #[test]
    fn test_compute_delta_changed_axes_only() {
        let a = sample_scores(0.5, 0.5, 1.0, AutonomyLevel::L2, 0.3);
        let b = sample_scores(0.7, 0.5, 1.0, AutonomyLevel::L2, 0.3);
        let delta = compute_delta(&b, &a);
        assert!(delta.contains_key("cognitive"));
        assert!(!delta.contains_key("behavioral"));
        assert!(!delta.contains_key("meta_learning"));
        assert!((delta["cognitive"] - 0.2).abs() < 0.001);
    }

    // ── EvolutionParams serialization test ──

    #[test]
    fn test_evolution_params_serialization() {
        let params = EvolutionParams::default();
        let json = serde_json::to_value(&params).unwrap();
        let deserialized: EvolutionParams = serde_json::from_value(json).unwrap();
        assert!((deserialized.alpha - 0.10).abs() < f64::EPSILON);
        assert!((deserialized.beta - 0.05).abs() < f64::EPSILON);
    }
}
