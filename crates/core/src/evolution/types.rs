use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The plugin_id used for evolution data in PluginDataStore.
/// Evolution data lives in the Kernel namespace, not any plugin.
pub const EVOLUTION_STORE_ID: &str = "core.evolution";

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
        impl de::Visitor<'_> for AutonomyVisitor {
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
    #[must_use] 
    pub fn normalized(&self) -> f64 {
        f64::from(*self as u8) / 5.0
    }

    /// Create from a normalized f64 value (0.0-1.0), rounding to nearest level.
    /// Non-finite, negative, or out-of-range values fall back to L0.
    #[must_use] 
    pub fn from_normalized(v: f64) -> Self {
        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
            return Self::L0;
        }
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
    /// Safety is expected to be binary (0.0 or 1.0) but intermediate values
    /// are accepted to allow gradual degradation reporting.
    pub fn validate(&self) -> anyhow::Result<()> {
        let fields = [
            ("cognitive", self.cognitive),
            ("behavioral", self.behavioral),
            ("safety", self.safety),
            ("meta_learning", self.meta_learning),
        ];
        for (name, val) in fields {
            if !val.is_finite() || !(0.0..=1.0).contains(&val) {
                anyhow::bail!("{} score must be in [0.0, 1.0], got {}", name, val);
            }
        }
        if self.safety != 0.0 && self.safety != 1.0 {
            tracing::debug!(safety = self.safety,
                "Safety score is non-binary; SafetyGate treats any value < 1.0 as a breach");
        }
        Ok(())
    }

    /// Returns the axis ranking (sorted by normalized score, descending).
    /// Safety is excluded because it's a binary gate (0.0 or 1.0), not a gradient score.
    #[must_use] 
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
                .then_with(|| a.0.cmp(b.0)) // alphabetical tiebreaker for determinism
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

impl FitnessWeights {
    pub fn validate(&self) -> anyhow::Result<()> {
        let fields = [("cognitive", self.cognitive), ("behavioral", self.behavioral),
                      ("safety", self.safety), ("autonomy", self.autonomy),
                      ("meta_learning", self.meta_learning)];
        for (name, val) in fields {
            if !val.is_finite() || val < 0.0 {
                anyhow::bail!("{} weight must be >= 0 and finite, got {}", name, val);
            }
        }
        let sum: f64 = fields.iter().map(|(_, v)| v).sum();
        if (sum - 1.0).abs() > 0.01 {
            anyhow::bail!("weights must sum to ~1.0, got {:.4}", sum);
        }
        Ok(())
    }
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

impl EvolutionParams {
    pub fn validate(&self) -> anyhow::Result<()> {
        for (name, val) in [("alpha", self.alpha), ("beta", self.beta),
                            ("theta_min", self.theta_min), ("gamma", self.gamma)] {
            if !val.is_finite() || !(0.0..=1.0).contains(&val) {
                anyhow::bail!("{} must be in [0.0, 1.0] and finite, got {}", name, val);
            }
        }
        if self.min_interactions == 0 {
            anyhow::bail!("min_interactions must be > 0");
        }
        self.weights.validate()?;
        Ok(())
    }
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

/// Trigger type for generation transitions.
///
/// Priority order (highest first):
///   1. SafetyBreach    — safety invariant violated (defensive, unconditional)
///   2. Regression       — fitness decline (defensive)
///   3. CapabilityGain   — structural change: new plugin/capability (growth)
///   4. AutonomyUpgrade  — autonomy level increase (growth)
///   5. Rebalance        — axis balance shift (growth)
///   6. Evolution        — default positive growth (growth)
///
/// Defensive triggers always override growth triggers.
/// Among growth triggers, structural change (CapabilityGain) overrides
/// metric-based triggers because it carries higher explanatory value.
///
/// See `docs/e7-capability-gain.md` for full design rationale.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum GenerationTrigger {
    Evolution,
    Regression,
    Rebalance,
    SafetyBreach,
    /// Structural change: new plugin activated or new CapabilityType acquired.
    /// Detected by comparing AgentSnapshot.active_plugins across generations,
    /// independently of metric-based check_triggers().
    /// See `docs/e7-capability-gain.md` for detection algorithm and priority rules.
    CapabilityGain,
    AutonomyUpgrade,
}

impl std::fmt::Display for GenerationTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Evolution => write!(f, "Evolution"),
            Self::Regression => write!(f, "Regression"),
            Self::Rebalance => write!(f, "Rebalance"),
            Self::SafetyBreach => write!(f, "SafetyBreach"),
            Self::CapabilityGain => write!(f, "CapabilityGain"),
            Self::AutonomyUpgrade => write!(f, "AutonomyUpgrade"),
        }
    }
}

/// Agent state snapshot for rollback.
/// Captures the full agent configuration at each generation boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub active_plugins: Vec<String>,
    /// Mapping of plugin_id → capability names (e.g. "Reasoning", "Vision").
    /// Used by `detect_capability_gain()` to identify structural changes.
    /// Defaults to empty for backward compatibility with pre-E7 snapshots.
    #[serde(default)]
    pub plugin_capabilities: HashMap<String, Vec<String>>,
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

/// Result of structural capability change detection.
#[derive(Debug, Clone)]
pub struct CapabilityChange {
    /// The plugin that was newly activated.
    pub plugin_id: String,
    /// Capability names provided by this plugin.
    pub capabilities: Vec<String>,
    /// True if this plugin brings a CapabilityType category not present in the previous generation.
    pub is_major: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RegressionSeverity {
    None,
    Mild,
    Severe,
}

// ══════════════════════════════════════════════════════════════
// Automatic Fitness Scoring (Principle 1.1: event counting only)
// ══════════════════════════════════════════════════════════════

/// Per-agent event counters for the current evaluation window.
/// All fields are kernel-observable (no content analysis).
/// Reset on generation transition (EvolutionGeneration event).
#[derive(Debug, Clone, Default)]
pub struct InteractionMetrics {
    pub thought_requests: u64,
    pub thought_responses: u64,
    pub permissions_requested: u64,
    pub permissions_approved: u64,
    pub errors: u64,
    pub total_interactions: u64,
    pub safety_violation: bool,
    pub human_interventions: u64,
    pub autonomous_actions: u64,
}

/// Scores contributed by plugins for axes that require content analysis.
#[derive(Debug, Clone, Default)]
pub struct PluginContributions {
    pub cognitive: Option<f64>,
    pub meta_learning: Option<f64>,
}
