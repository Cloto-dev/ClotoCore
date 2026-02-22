use chrono::Utc;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use exiv_shared::{ExivEventData, PluginDataStore};

use super::calc::{
    calculate_fitness, check_triggers, compute_autonomy_level, compute_behavioral_score,
    compute_delta, compute_safety_score, detect_capability_gain, detect_rebalance,
    grace_period_length, regression_severity,
};
use super::types::{
    AgentSnapshot, AutonomyLevel, EvolutionParams, EvolutionStatus, FitnessLogEntry, FitnessScores,
    GenerationRecord, GenerationTrigger, GracePeriodState, InteractionMetrics, PluginContributions,
    RegressionSeverity, RollbackRecord, EVOLUTION_STORE_ID,
};

/// Maximum number of rollbacks to the same target generation before skipping it.
const MAX_ROLLBACKS_PER_TARGET: u32 = 3;

/// Maximum entries kept in the fitness log to prevent unbounded growth.
const MAX_FITNESS_LOG_ENTRIES: usize = 10000;

/// Maximum entries kept in the rollback history.
const MAX_ROLLBACK_HISTORY_ENTRIES: usize = 100;

// ══════════════════════════════════════════════════════════════
// Evolution Engine
// ══════════════════════════════════════════════════════════════

/// The evolution engine tracks agent fitness across generations.
///
/// Architecture note: The `pool` field is a direct dependency on `SqlitePool`,
/// used exclusively for audit logging (`spawn_audit_log`). All evolution data
/// is stored through the `PluginDataStore` abstraction. If audit logging is
/// refactored to use an event-based approach, the `pool` dependency can be removed.
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
        params.validate()?;
        let key = Self::key_params(agent_id);
        self.store
            .set_json(EVOLUTION_STORE_ID, &key, serde_json::to_value(params)?)
            .await
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
        let count = self
            .store
            .increment_counter(EVOLUTION_STORE_ID, &key)
            .await?;
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

    pub async fn get_generation(
        &self,
        agent_id: &str,
        n: u64,
    ) -> anyhow::Result<Option<GenerationRecord>> {
        let key = Self::key_generation(agent_id, n);
        match self.store.get_json(EVOLUTION_STORE_ID, &key).await? {
            Some(val) => Ok(Some(serde_json::from_value(val)?)),
            None => Ok(None),
        }
    }

    /// Retrieves recent generation records in reverse chronological order.
    /// Note: This performs sequential key lookups (one per generation).
    /// For large histories, a batch retrieval (key prefix scan) would be more efficient,
    /// but the current approach is sufficient given typical generation counts (<500).
    pub async fn get_generation_history(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<GenerationRecord>> {
        let latest = self.get_latest_generation(agent_id).await?;
        if latest == 0 {
            return Ok(vec![]);
        }

        let mut records = Vec::new();
        let start = if latest > limit as u64 {
            latest - limit as u64 + 1
        } else {
            1
        };
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
        // Atomically increment the generation counter (prevents duplicate generation numbers
        // under concurrent calls). increment_counter uses UPSERT with RETURNING.
        let key_latest = Self::key_latest(agent_id);
        let new_gen = self
            .store
            .increment_counter(EVOLUTION_STORE_ID, &key_latest)
            .await? as u64;

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
        self.store
            .set_json(EVOLUTION_STORE_ID, &key, serde_json::to_value(&record)?)
            .await?;

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
    pub async fn append_fitness_log(
        &self,
        agent_id: &str,
        entry: FitnessLogEntry,
    ) -> anyhow::Result<Vec<FitnessLogEntry>> {
        let mut log = self.get_fitness_log(agent_id).await?;
        log.push(entry);

        if log.len() > MAX_FITNESS_LOG_ENTRIES {
            log = log.split_off(log.len() - MAX_FITNESS_LOG_ENTRIES);
        }

        let key = Self::key_fitness_log(agent_id);
        self.store
            .set_json(EVOLUTION_STORE_ID, &key, serde_json::to_value(&log)?)
            .await?;

        // Cache latest entry for O(1) access by get_latest_fitness()
        if let Some(latest) = log.last() {
            let cache_key = Self::key_latest_fitness(agent_id);
            self.store
                .set_json(
                    EVOLUTION_STORE_ID,
                    &cache_key,
                    serde_json::to_value(latest)?,
                )
                .await?;
        }

        Ok(log)
    }

    pub async fn get_fitness_timeline(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<FitnessLogEntry>> {
        let log = self.get_fitness_log(agent_id).await?;
        let start = if log.len() > limit {
            log.len() - limit
        } else {
            0
        };
        Ok(log[start..].to_vec())
    }

    // ── Grace Period ──

    pub async fn get_grace_period(
        &self,
        agent_id: &str,
    ) -> anyhow::Result<Option<GracePeriodState>> {
        let key = Self::key_grace_period(agent_id);
        match self.store.get_json(EVOLUTION_STORE_ID, &key).await? {
            Some(val) if !val.is_null() => match serde_json::from_value::<GracePeriodState>(val) {
                Ok(state) if state.active => Ok(Some(state)),
                _ => Ok(None),
            },
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
        self.store
            .set_json(EVOLUTION_STORE_ID, &key, serde_json::to_value(&state)?)
            .await
    }

    pub async fn cancel_grace_period(&self, agent_id: &str) -> anyhow::Result<()> {
        let key = Self::key_grace_period(agent_id);
        // Set to null; get_grace_period will return None on deserialization failure
        self.store
            .set_json(EVOLUTION_STORE_ID, &key, serde_json::Value::Null)
            .await
    }

    // ── Rollback History ──

    pub async fn get_rollback_history(
        &self,
        agent_id: &str,
    ) -> anyhow::Result<Vec<RollbackRecord>> {
        let key = Self::key_rollback_history(agent_id);
        match self.store.get_json(EVOLUTION_STORE_ID, &key).await? {
            Some(val) => Ok(serde_json::from_value(val)?),
            None => Ok(vec![]),
        }
    }

    async fn append_rollback_record(
        &self,
        agent_id: &str,
        record: RollbackRecord,
    ) -> anyhow::Result<()> {
        let mut history = self.get_rollback_history(agent_id).await?;
        history.push(record);

        if history.len() > MAX_ROLLBACK_HISTORY_ENTRIES {
            history = history.split_off(history.len() - MAX_ROLLBACK_HISTORY_ENTRIES);
        }

        let key = Self::key_rollback_history(agent_id);
        self.store
            .set_json(EVOLUTION_STORE_ID, &key, serde_json::to_value(&history)?)
            .await
    }

    /// Count how many times we've rolled back to the given generation
    async fn rollback_count_to_gen(&self, agent_id: &str, target_gen: u64) -> anyhow::Result<u32> {
        let history = self.get_rollback_history(agent_id).await?;
        Ok(history
            .iter()
            .filter(|r| r.to_generation == target_gen)
            .count() as u32)
    }

    // ── Rollback Execution ──

    /// Rolls back to a previous generation and records the rollback.
    /// The new generation created after rollback uses `Regression` as its trigger,
    /// regardless of the original cause (SafetyBreach, grace period expiry, etc.).
    /// The `reason` parameter captures the actual cause in the rollback record.
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

            if let Some(record) = self.get_generation(agent_id, target_gen).await? {
                break (record, count);
            }
            error!(agent_id = %agent_id, target_gen = target_gen, "Target generation not found, cascading to earlier");
            if target_gen > 1 {
                target_gen -= 1;
                continue;
            }
            events.push(ExivEventData::EvolutionBreach {
                agent_id: agent_id.to_string(),
                violation_type: "rollback_target_missing".to_string(),
                detail: format!(
                    "No valid generation found for rollback (tried down to gen {})",
                    target_gen
                ),
            });
            return Ok(events);
        };

        info!(
            agent_id = %agent_id,
            from_gen = from_gen,
            to_gen = target_gen,
            "🔄 Executing evolution rollback"
        );

        // Record rollback
        self.append_rollback_record(
            agent_id,
            RollbackRecord {
                timestamp: Utc::now(),
                from_generation: from_gen,
                to_generation: target_gen,
                reason: reason.to_string(),
                rollback_count_to_target: rollback_count + 1,
            },
        )
        .await?;

        // Emit rollback event
        events.push(ExivEventData::EvolutionRollback {
            agent_id: agent_id.to_string(),
            from_generation: from_gen,
            to_generation: target_gen,
            reason: reason.to_string(),
        });

        // Create new generation with restored scores
        let delta = HashMap::new();
        let restored_fitness = calculate_fitness(
            &target_record.scores,
            &self.get_params(agent_id).await?.weights,
        );
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
        )
        .await?;

        // Cancel grace period
        self.cancel_grace_period(agent_id).await?;

        // Audit log
        crate::db::spawn_audit_log(
            self.pool.clone(),
            crate::db::AuditLogEntry {
                timestamp: Utc::now(),
                event_type: "EVOLUTION_ROLLBACK".to_string(),
                actor_id: Some("kernel".to_string()),
                target_id: Some(agent_id.to_string()),
                permission: None,
                result: "SUCCESS".to_string(),
                reason: format!("Rollback gen {} → gen {}: {}", from_gen, target_gen, reason),
                metadata: None,
                trace_id: None,
            },
        );

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
        Ok(log.last().map_or(0.0, |e| e.fitness))
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

        // Interactions since last generation (consistent with evaluate(): strictly after)
        // Read log once and reuse for both interactions count and trend calculation
        let log = self.get_fitness_log(agent_id).await?;
        let interactions_since_last_gen = if let Some(ref record) = gen_record {
            log.iter()
                .filter(|e| e.timestamp > record.timestamp)
                .count() as u64
        } else {
            total_interactions
        };

        // Calculate trend from last few fitness entries (reuse log from above)
        let trend_start = if log.len() > 10 { log.len() - 10 } else { 0 };
        let log = &log[trend_start..];
        let trend_val = if log.len() >= 2 {
            let recent = log.last().map_or(0.0, |e| e.fitness);
            let earlier = log.first().map_or(0.0, |e| e.fitness);
            recent - earlier
        } else {
            0.0
        };
        let trend = if trend_val > 0.01 {
            "improving".to_string()
        } else if trend_val < -0.01 {
            "declining".to_string()
        } else {
            "stable".to_string()
        };

        // Default scores when no generation exists
        let scores = gen_record.map_or(
            FitnessScores {
                cognitive: 0.0,
                behavioral: 0.0,
                safety: 1.0,
                autonomy: AutonomyLevel::L0,
                meta_learning: 0.0,
            },
            |r| r.scores,
        );

        let autonomy_level = scores.autonomy.to_string();
        let top_axes = scores
            .axis_ranking()
            .into_iter()
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
        let log = self
            .append_fitness_log(
                agent_id,
                FitnessLogEntry {
                    timestamp: Utc::now(),
                    interaction_count,
                    scores: scores.clone(),
                    fitness: current_fitness,
                },
            )
            .await?;
        if log.len() < 2 {
            // Not enough data to compare — if this is the first evaluation, create gen 0
            if self.get_latest_generation(agent_id).await? == 0 {
                let record = self
                    .create_generation(
                        agent_id,
                        GenerationTrigger::Evolution,
                        scores,
                        current_fitness,
                        0.0,
                        HashMap::new(),
                        0,
                        snapshot,
                    )
                    .await?;
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
                let rollback_events = self
                    .execute_rollback(
                        agent_id,
                        target_gen,
                        &format!("Grace period expired for {} axis", grace.affected_axis),
                    )
                    .await?;
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
        // Note: This read is intentionally separate from the grace-period path (early return above)
        // and the post-handler read (H-10 fix below) — each operates on a different execution phase.
        let latest_gen = self.get_latest_generation(agent_id).await?;
        let last_gen_record = self.get_generation(agent_id, latest_gen).await?;
        let interactions_since_last_gen = if let Some(ref rec) = last_gen_record {
            log.iter().filter(|e| e.timestamp > rec.timestamp).count() as u64
        } else {
            interaction_count
        };

        // Phase 1: Metric-based trigger detection (pure function).
        let metric_trigger = check_triggers(
            current_fitness,
            previous_fitness,
            &scores,
            previous_scores,
            &params,
            interactions_since_last_gen,
        );

        // Phase 2: Structure-based capability detection.
        // Compare previous generation's snapshot with current to find new plugins/capabilities.
        let capability_changes = match last_gen_record.as_ref() {
            Some(prev_gen) => detect_capability_gain(&prev_gen.snapshot, &snapshot),
            None => vec![],
        };

        // Phase 3: Resolve final trigger using priority rules.
        // SafetyBreach > Regression > CapabilityGain > AutonomyUpgrade > Rebalance > Evolution
        // See docs/e7-capability-gain.md for rationale.
        let trigger = match (metric_trigger, capability_changes.is_empty()) {
            // Defensive triggers always win (safety-first principle)
            (Some(t @ (GenerationTrigger::SafetyBreach | GenerationTrigger::Regression)), _) => {
                Some(t)
            }
            // Growth trigger + capability changes → CapabilityGain (higher explanatory value)
            (Some(_), false) => Some(GenerationTrigger::CapabilityGain),
            // No metric trigger + capability changes → CapabilityGain (if debounce satisfied)
            (None, false) if interactions_since_last_gen >= params.min_interactions => {
                Some(GenerationTrigger::CapabilityGain)
            }
            // Otherwise keep metric trigger as-is
            (t, _) => t,
        };

        if let Some(trigger) = trigger {
            match trigger {
                GenerationTrigger::SafetyBreach => {
                    self.handle_safety_breach(agent_id, latest_gen, &mut events)
                        .await?;
                }
                GenerationTrigger::Regression => {
                    self.handle_regression(
                        agent_id,
                        scores,
                        previous_scores,
                        current_fitness,
                        previous_fitness,
                        &params,
                        interactions_since_last_gen,
                        snapshot,
                        latest_gen,
                        &mut events,
                    )
                    .await?;
                }
                GenerationTrigger::Evolution
                | GenerationTrigger::AutonomyUpgrade
                | GenerationTrigger::CapabilityGain => {
                    self.handle_positive_trigger(
                        agent_id,
                        trigger,
                        scores,
                        previous_scores,
                        current_fitness,
                        previous_fitness,
                        interactions_since_last_gen,
                        snapshot,
                        &mut events,
                    )
                    .await?;
                }
                GenerationTrigger::Rebalance => {
                    self.handle_rebalance(
                        agent_id,
                        scores,
                        previous_scores,
                        current_fitness,
                        previous_fitness,
                        interactions_since_last_gen,
                        snapshot,
                        &mut events,
                    )
                    .await?;
                }
            }
        }

        // Emit EvolutionCapability events AFTER trigger handlers execute,
        // so the generation number reflects the actual created generation.
        if !capability_changes.is_empty() {
            let actual_gen = self.get_latest_generation(agent_id).await?;
            for change in &capability_changes {
                for cap in &change.capabilities {
                    events.push(ExivEventData::EvolutionCapability {
                        agent_id: agent_id.to_string(),
                        capability: format!(
                            "{}:{}",
                            if change.is_major { "major" } else { "minor" },
                            cap,
                        ),
                        generation: actual_gen,
                    });
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
            let rollback_events = self
                .execute_rollback(agent_id, latest_gen - 1, "Safety breach detected")
                .await?;
            events.extend(rollback_events);
        } else if latest_gen == 1 {
            warn!(agent_id = %agent_id, "Safety breach on generation 1, no earlier generation available");
        } else {
            warn!(agent_id = %agent_id, "Safety breach on generation 0, no rollback target exists");
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
                    let rollback_events = self
                        .execute_rollback(agent_id, latest_gen - 1, "Severe regression detected")
                        .await?;
                    events.extend(rollback_events);
                } else if latest_gen == 1 {
                    warn!(agent_id = %agent_id, "Severe regression on generation 1, no earlier generation available");
                }
            }
            RegressionSeverity::Mild => {
                let grace_len = grace_period_length(
                    interactions_since_last_gen,
                    params.gamma,
                    params.min_interactions,
                );
                let affected_axis = compute_delta(&scores, previous_scores)
                    .into_iter()
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map_or_else(|| "unknown".to_string(), |(k, _)| k);

                self.start_grace_period(agent_id, grace_len, current_fitness, &affected_axis)
                    .await?;
                events.push(ExivEventData::EvolutionWarning {
                    agent_id: agent_id.to_string(),
                    severity: "mild".to_string(),
                    affected_area: affected_axis.clone(),
                    direction: "regression".to_string(),
                    grace_remaining: grace_len,
                    suggestion: format!("{} patterns may need adjustment", affected_axis),
                });

                let delta = compute_delta(&scores, previous_scores);
                let record = self
                    .create_generation(
                        agent_id,
                        GenerationTrigger::Regression,
                        scores,
                        current_fitness,
                        current_fitness - previous_fitness,
                        delta,
                        interactions_since_last_gen,
                        snapshot,
                    )
                    .await?;
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
        let record = self
            .create_generation(
                agent_id,
                trigger,
                scores,
                current_fitness,
                current_fitness - previous_fitness,
                delta,
                interactions_since_last_gen,
                snapshot,
            )
            .await?;
        events.push(ExivEventData::EvolutionGeneration {
            agent_id: agent_id.to_string(),
            generation: record.generation,
            trigger: record.trigger.to_string(),
        });
        Ok(())
    }

    /// Handles axis rebalance trigger.
    /// Note: `detect_rebalance` is called again here because `check_triggers()`
    /// does not return the shifted axes (it only returns the trigger type).
    /// This is a minor inefficiency accepted to keep `check_triggers` as a pure function.
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
        let record = self
            .create_generation(
                agent_id,
                GenerationTrigger::Rebalance,
                scores,
                current_fitness,
                current_fitness - previous_fitness,
                delta,
                interactions_since_last_gen,
                snapshot,
            )
            .await?;
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
    ///
    /// Note: Fitness recovery cannot be checked here because we don't have
    /// the current scores. The grace period fitness recovery check happens
    /// in `evaluate()` where scores are available.
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
                let rollback_events = self
                    .execute_rollback(
                        agent_id,
                        target_gen,
                        &format!("Grace period expired for {} axis", grace.affected_axis),
                    )
                    .await?;
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
// Automatic Fitness Scoring (Principle 1.1: event counting only)
// ══════════════════════════════════════════════════════════════

/// Observes events, accumulates per-agent metrics, and computes fitness scores.
///
/// Principle 1.1: Only counts events, measures rates, tracks success/failure.
/// Does NOT interpret LLM output content.
/// Principle 1.3: Plugin contributions arrive via FitnessContribution events.
pub struct FitnessCollector {
    metrics: RwLock<HashMap<String, InteractionMetrics>>,
    contributions: RwLock<HashMap<String, PluginContributions>>,
    enabled: bool,
}

impl FitnessCollector {
    #[must_use]
    pub fn new(enabled: bool) -> Self {
        Self {
            metrics: RwLock::new(HashMap::new()),
            contributions: RwLock::new(HashMap::new()),
            enabled,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Observe an event and update the appropriate agent's metrics.
    /// Returns the agent_id if this event should trigger auto-evaluation
    /// (i.e., it was a ThoughtResponse).
    pub async fn observe(&self, event_data: &ExivEventData) -> Option<String> {
        if !self.enabled {
            return None;
        }

        match event_data {
            ExivEventData::ThoughtRequested { agent, .. } => {
                let mut metrics = self.metrics.write().await;
                let m = metrics.entry(agent.id.clone()).or_default();
                m.thought_requests += 1;
                m.total_interactions += 1;
                None
            }
            ExivEventData::ThoughtResponse { agent_id, .. } => {
                let mut metrics = self.metrics.write().await;
                let m = metrics.entry(agent_id.clone()).or_default();
                m.thought_responses += 1;
                m.total_interactions += 1;
                Some(agent_id.clone())
            }
            ExivEventData::PermissionRequested { .. } => {
                // PermissionRequested lacks agent_id; tracked as system-level metric.
                // Future enhancement: correlate with recent ThoughtResponse agent_id.
                None
            }
            ExivEventData::PermissionGranted { .. } => None,
            ExivEventData::ActionRequested { .. } => None,
            ExivEventData::EvolutionBreach { agent_id, .. } => {
                let mut metrics = self.metrics.write().await;
                let m = metrics.entry(agent_id.clone()).or_default();
                m.safety_violation = true;
                m.errors += 1;
                None
            }
            ExivEventData::ToolInvoked {
                agent_id, success, ..
            } => {
                let mut metrics = self.metrics.write().await;
                let m = metrics.entry(agent_id.clone()).or_default();
                m.autonomous_actions += 1;
                m.total_interactions += 1;
                if !*success {
                    m.errors += 1;
                }
                None
            }
            ExivEventData::AgenticLoopCompleted { .. } => {
                // L-03: Don't increment total_interactions here; ToolInvoked already counts each call
                None
            }
            _ => None,
        }
    }

    /// Record a plugin fitness contribution for an agent.
    pub async fn record_contribution(&self, agent_id: &str, axis: &str, score: f64) {
        let score = score.clamp(0.0, 1.0);
        let mut contributions = self.contributions.write().await;
        let c = contributions.entry(agent_id.to_string()).or_default();
        match axis {
            "cognitive" => c.cognitive = Some(score),
            "meta_learning" => c.meta_learning = Some(score),
            _ => {
                tracing::warn!(agent_id = %agent_id, axis = %axis, "Unknown fitness contribution axis");
            }
        }
    }

    /// Compute FitnessScores from accumulated metrics for the given agent.
    pub async fn compute_scores(&self, agent_id: &str) -> FitnessScores {
        let metrics = self.metrics.read().await;
        let m = metrics.get(agent_id).cloned().unwrap_or_default();
        let contributions = self.contributions.read().await;
        let c = contributions.get(agent_id).cloned().unwrap_or_default();

        let behavioral = compute_behavioral_score(&m);
        let safety = compute_safety_score(&m);
        let autonomy = compute_autonomy_level(&m);
        let cognitive = c.cognitive.unwrap_or(0.5);
        let meta_learning = c.meta_learning.unwrap_or(0.5);

        FitnessScores {
            cognitive: cognitive.clamp(0.0, 1.0),
            behavioral: behavioral.clamp(0.0, 1.0),
            safety,
            autonomy,
            meta_learning: meta_learning.clamp(0.0, 1.0),
        }
    }

    /// Reset metrics for an agent (called on generation transition).
    pub async fn reset(&self, agent_id: &str) {
        let mut metrics = self.metrics.write().await;
        metrics.remove(agent_id);
    }
}

// ══════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::super::calc::*;
    use super::super::types::*;

    fn default_weights() -> FitnessWeights {
        FitnessWeights::default()
    }

    fn default_params() -> EvolutionParams {
        EvolutionParams::default()
    }

    fn sample_scores(
        cognitive: f64,
        behavioral: f64,
        safety: f64,
        autonomy: AutonomyLevel,
        meta: f64,
    ) -> FitnessScores {
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

        // Significant improvement (meta_learning stays below autonomy to avoid Rebalance)
        let curr_scores = sample_scores(0.8, 0.8, 1.0, AutonomyLevel::L2, 0.35);
        let curr_fitness = calculate_fitness(&curr_scores, &params.weights);

        let trigger = check_triggers(
            curr_fitness,
            prev_fitness,
            &curr_scores,
            &prev_scores,
            &params,
            15,
        );
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

        let trigger = check_triggers(
            curr_fitness,
            prev_fitness,
            &curr_scores,
            &prev_scores,
            &params,
            15,
        );
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
        let trigger = check_triggers(
            curr_fitness,
            prev_fitness,
            &curr_scores,
            &prev_scores,
            &params,
            0,
        );
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
        let trigger = check_triggers(
            curr_fitness,
            prev_fitness,
            &curr_scores,
            &prev_scores,
            &params,
            5,
        );
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

        let trigger = check_triggers(
            curr_fitness,
            prev_fitness,
            &curr_scores,
            &prev_scores,
            &params,
            15,
        );
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

        let trigger = check_triggers(
            curr_fitness,
            prev_fitness,
            &curr_scores,
            &prev_scores,
            &params,
            15,
        );
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

        let trigger = check_triggers(
            curr_fitness,
            prev_fitness,
            &curr_scores,
            &prev_scores,
            &params,
            15,
        );
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
