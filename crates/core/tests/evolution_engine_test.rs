//! Integration tests for the Self-Evolution Benchmark Engine.
//! Tests the EvolutionEngine through SqliteDataStore with an in-memory DB.

use std::collections::HashMap;
use std::sync::Arc;
use sqlx::SqlitePool;
use exiv_core::db::SqliteDataStore;
use exiv_core::evolution::{
    EvolutionEngine, FitnessScores, AutonomyLevel, AgentSnapshot,
    detect_capability_gain,
};

const TEST_AGENT: &str = "agent.test";

async fn setup() -> EvolutionEngine {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE plugin_data (plugin_id TEXT, key TEXT, value TEXT, PRIMARY KEY(plugin_id, key))"
    ).execute(&pool).await.unwrap();
    sqlx::query(
        "CREATE TABLE audit_logs (id INTEGER PRIMARY KEY AUTOINCREMENT, timestamp TEXT, event_type TEXT, actor_id TEXT, target_id TEXT, permission TEXT, result TEXT, reason TEXT, metadata TEXT, trace_id TEXT)"
    ).execute(&pool).await.unwrap();
    let store = Arc::new(SqliteDataStore::new(pool.clone()));
    EvolutionEngine::new(store, pool)
}

fn test_scores(c: f64, b: f64, s: f64, a: AutonomyLevel, m: f64) -> FitnessScores {
    FitnessScores {
        cognitive: c,
        behavioral: b,
        safety: s,
        autonomy: a,
        meta_learning: m,
    }
}

fn test_snapshot() -> AgentSnapshot {
    AgentSnapshot {
        active_plugins: vec!["test_plugin".to_string()],
        plugin_capabilities: HashMap::from([
            ("test_plugin".to_string(), vec!["Reasoning".to_string()]),
        ]),
        runtime_plugins: vec![],
        personality_hash: "abc123".to_string(),
        strategy_params: Default::default(),
    }
}

fn snapshot_with_plugins(plugins: Vec<(&str, Vec<&str>)>) -> AgentSnapshot {
    let active_plugins = plugins.iter().map(|(id, _)| id.to_string()).collect();
    let plugin_capabilities = plugins.iter()
        .map(|(id, caps)| (id.to_string(), caps.iter().map(|c| c.to_string()).collect()))
        .collect();
    AgentSnapshot {
        active_plugins,
        plugin_capabilities,
        runtime_plugins: vec![],
        personality_hash: "abc123".to_string(),
        strategy_params: Default::default(),
    }
}

// ── C-1 verification: Atomic increment ──

#[tokio::test]
async fn test_increment_interaction_sequential() {
    let engine = setup().await;
    let c1 = engine.increment_interaction(TEST_AGENT).await.unwrap();
    let c2 = engine.increment_interaction(TEST_AGENT).await.unwrap();
    let c3 = engine.increment_interaction(TEST_AGENT).await.unwrap();
    assert_eq!(c1, 1);
    assert_eq!(c2, 2);
    assert_eq!(c3, 3);
    let count = engine.get_interaction_count(TEST_AGENT).await.unwrap();
    assert_eq!(count, 3);
}

// ── Basic evaluate flow ──

#[tokio::test]
async fn test_evaluate_creates_first_generation() {
    let engine = setup().await;
    let scores = test_scores(0.5, 0.5, 1.0, AutonomyLevel::L1, 0.3);
    let events = engine.evaluate(TEST_AGENT, scores, test_snapshot()).await.unwrap();

    // First evaluation should create generation 1
    assert!(!events.is_empty(), "Should emit at least one event");
    let gen = engine.get_latest_generation(TEST_AGENT).await.unwrap();
    assert_eq!(gen, 1);
}

#[tokio::test]
async fn test_evaluate_second_call_no_generation_if_below_threshold() {
    let engine = setup().await;
    let scores = test_scores(0.5, 0.5, 1.0, AutonomyLevel::L1, 0.3);

    // First call: creates gen 1
    engine.evaluate(TEST_AGENT, scores.clone(), test_snapshot()).await.unwrap();

    // Second call with identical scores: no new generation (below alpha threshold)
    let events = engine.evaluate(TEST_AGENT, scores, test_snapshot()).await.unwrap();
    let gen = engine.get_latest_generation(TEST_AGENT).await.unwrap();
    // Still gen 1 (no trigger fired because delta = 0 and min_interactions not met)
    assert_eq!(gen, 1);
    // No EvolutionGeneration event
    let gen_events: Vec<_> = events.iter().filter(|e| {
        matches!(e, exiv_shared::ExivEventData::EvolutionGeneration { .. })
    }).collect();
    assert!(gen_events.is_empty());
}

// ── Fitness log ──

#[tokio::test]
async fn test_fitness_log_append_and_retrieve() {
    let engine = setup().await;
    let scores = test_scores(0.5, 0.5, 1.0, AutonomyLevel::L1, 0.3);

    // Evaluate multiple times to build up log
    for _ in 0..5 {
        engine.evaluate(TEST_AGENT, scores.clone(), test_snapshot()).await.unwrap();
    }

    let log = engine.get_fitness_log(TEST_AGENT).await.unwrap();
    assert_eq!(log.len(), 5);

    let timeline = engine.get_fitness_timeline(TEST_AGENT, 3).await.unwrap();
    assert_eq!(timeline.len(), 3);
}

// ── Grace period flow ──

#[tokio::test]
async fn test_grace_period_start_and_cancel() {
    let engine = setup().await;

    // No grace period initially
    let gp = engine.get_grace_period(TEST_AGENT).await.unwrap();
    assert!(gp.is_none());

    // Start grace period
    engine.start_grace_period(TEST_AGENT, 10, 0.5, "cognitive").await.unwrap();

    let gp = engine.get_grace_period(TEST_AGENT).await.unwrap();
    assert!(gp.is_some());
    let gp = gp.unwrap();
    assert!(gp.active);
    assert_eq!(gp.grace_interactions, 10);
    assert_eq!(gp.affected_axis, "cognitive");

    // Cancel
    engine.cancel_grace_period(TEST_AGENT).await.unwrap();
    let gp = engine.get_grace_period(TEST_AGENT).await.unwrap();
    assert!(gp.is_none() || !gp.unwrap().active);
}

// ── Rollback (iterative) ──

#[tokio::test]
async fn test_rollback_iterative() {
    let engine = setup().await;
    let scores = test_scores(0.8, 0.8, 1.0, AutonomyLevel::L2, 0.6);

    // Create gen 1 via evaluate
    engine.evaluate(TEST_AGENT, scores.clone(), test_snapshot()).await.unwrap();
    assert_eq!(engine.get_latest_generation(TEST_AGENT).await.unwrap(), 1);

    // Manually create gen 2 (simulate positive evolution)
    let scores2 = test_scores(0.9, 0.9, 1.0, AutonomyLevel::L3, 0.7);
    engine.create_generation(
        TEST_AGENT,
        exiv_core::evolution::GenerationTrigger::Evolution,
        scores2,
        0.88,
        0.05,
        Default::default(),
        15,
        test_snapshot(),
    ).await.unwrap();
    assert_eq!(engine.get_latest_generation(TEST_AGENT).await.unwrap(), 2);

    // Rollback from gen 2 to gen 1
    let events = engine.execute_rollback(TEST_AGENT, 1, "test rollback").await.unwrap();
    assert!(!events.is_empty());

    // After rollback, a new generation (gen 3) should be created with gen 1's scores
    let latest = engine.get_latest_generation(TEST_AGENT).await.unwrap();
    assert_eq!(latest, 3);

    // Verify rollback history
    let history = engine.get_rollback_history(TEST_AGENT).await.unwrap();
    assert!(!history.is_empty());
    assert_eq!(history[0].to_generation, 1);
}

// ── Status API ──

#[tokio::test]
async fn test_get_status_returns_valid_data() {
    let engine = setup().await;
    let scores = test_scores(0.6, 0.7, 1.0, AutonomyLevel::L2, 0.4);
    engine.evaluate(TEST_AGENT, scores, test_snapshot()).await.unwrap();

    let status = engine.get_status(TEST_AGENT).await.unwrap();
    assert_eq!(status.agent_id, TEST_AGENT);
    assert_eq!(status.current_generation, 1);
    assert!(status.fitness > 0.0);
    assert!(["improving", "declining", "stable"].contains(&status.trend.as_str()));
    assert!(!status.autonomy_level.is_empty());
}

// ── Params ──

#[tokio::test]
async fn test_params_get_set() {
    let engine = setup().await;

    // Get defaults
    let params = engine.get_params(TEST_AGENT).await.unwrap();
    assert_eq!(params.min_interactions, 10);

    // Update
    let mut new_params = params.clone();
    new_params.min_interactions = 20;
    engine.set_params(TEST_AGENT, &new_params).await.unwrap();

    let updated = engine.get_params(TEST_AGENT).await.unwrap();
    assert_eq!(updated.min_interactions, 20);
}

// ── E7: detect_capability_gain unit tests ──

#[test]
fn test_detect_capability_gain_no_change() {
    let snap_a = snapshot_with_plugins(vec![("plugA", vec!["Reasoning"])]);
    let snap_b = snapshot_with_plugins(vec![("plugA", vec!["Reasoning"])]);
    let changes = detect_capability_gain(&snap_a, &snap_b);
    assert!(changes.is_empty());
}

#[test]
fn test_detect_capability_gain_minor() {
    // New plugin provides Reasoning (already present) → minor
    let prev = snapshot_with_plugins(vec![("plugA", vec!["Reasoning"])]);
    let curr = snapshot_with_plugins(vec![
        ("plugA", vec!["Reasoning"]),
        ("plugB", vec!["Reasoning"]),
    ]);
    let changes = detect_capability_gain(&prev, &curr);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].plugin_id, "plugB");
    assert!(!changes[0].is_major);
}

#[test]
fn test_detect_capability_gain_major() {
    // New plugin provides Vision (not present before) → major
    let prev = snapshot_with_plugins(vec![("plugA", vec!["Reasoning"])]);
    let curr = snapshot_with_plugins(vec![
        ("plugA", vec!["Reasoning"]),
        ("plugC", vec!["Vision"]),
    ]);
    let changes = detect_capability_gain(&prev, &curr);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].plugin_id, "plugC");
    assert!(changes[0].is_major);
    assert_eq!(changes[0].capabilities, vec!["Vision"]);
}

#[test]
fn test_detect_capability_gain_mixed() {
    // Two new plugins: one minor, one major
    let prev = snapshot_with_plugins(vec![("plugA", vec!["Reasoning", "Memory"])]);
    let curr = snapshot_with_plugins(vec![
        ("plugA", vec!["Reasoning", "Memory"]),
        ("plugB", vec!["Reasoning"]),   // minor: Reasoning already exists
        ("plugC", vec!["Vision", "HAL"]), // major: Vision and HAL are new
    ]);
    let changes = detect_capability_gain(&prev, &curr);
    assert_eq!(changes.len(), 2);

    let minor = changes.iter().find(|c| c.plugin_id == "plugB").unwrap();
    assert!(!minor.is_major);

    let major = changes.iter().find(|c| c.plugin_id == "plugC").unwrap();
    assert!(major.is_major);
}

#[test]
fn test_detect_capability_gain_empty_prev_capabilities() {
    // Pre-E7 snapshot: no plugin_capabilities data → no detection
    let prev = AgentSnapshot {
        active_plugins: vec!["plugA".to_string()],
        plugin_capabilities: HashMap::new(),
        runtime_plugins: vec![],
        personality_hash: "abc".to_string(),
        strategy_params: Default::default(),
    };
    let curr = AgentSnapshot {
        active_plugins: vec!["plugA".to_string(), "plugB".to_string()],
        plugin_capabilities: HashMap::new(),
        runtime_plugins: vec![],
        personality_hash: "abc".to_string(),
        strategy_params: Default::default(),
    };
    let changes = detect_capability_gain(&prev, &curr);
    assert!(changes.is_empty(), "Should skip detection when both snapshots lack capability data");
}

// ── E7: evaluate() integration with CapabilityGain ──

#[tokio::test]
async fn test_capability_gain_overrides_evolution() {
    let engine = setup().await;

    // Lower min_interactions for easier testing
    let mut params = engine.get_params(TEST_AGENT).await.unwrap();
    params.min_interactions = 1;
    engine.set_params(TEST_AGENT, &params).await.unwrap();

    // Gen 1: plugin A with Reasoning
    let scores1 = test_scores(0.3, 0.3, 1.0, AutonomyLevel::L1, 0.2);
    let snap1 = snapshot_with_plugins(vec![("plugA", vec!["Reasoning"])]);
    engine.evaluate(TEST_AGENT, scores1, snap1).await.unwrap();
    assert_eq!(engine.get_latest_generation(TEST_AGENT).await.unwrap(), 1);

    // Gen 2: add plugin B (Vision) + big fitness jump → should be CapabilityGain, not Evolution
    let scores2 = test_scores(0.9, 0.9, 1.0, AutonomyLevel::L1, 0.8);
    let snap2 = snapshot_with_plugins(vec![
        ("plugA", vec!["Reasoning"]),
        ("plugB", vec!["Vision"]),
    ]);
    let events = engine.evaluate(TEST_AGENT, scores2, snap2).await.unwrap();

    let gen = engine.get_latest_generation(TEST_AGENT).await.unwrap();
    assert_eq!(gen, 2, "Should create gen 2");

    // Verify the generation was recorded with CapabilityGain trigger
    let record = engine.get_generation(TEST_AGENT, 2).await.unwrap().unwrap();
    assert_eq!(record.trigger, exiv_core::evolution::GenerationTrigger::CapabilityGain);

    // Verify EvolutionCapability event was emitted
    let cap_events: Vec<_> = events.iter().filter(|e| {
        matches!(e, exiv_shared::ExivEventData::EvolutionCapability { .. })
    }).collect();
    assert!(!cap_events.is_empty(), "Should emit EvolutionCapability events");
}

#[tokio::test]
async fn test_safety_breach_overrides_capability_gain() {
    let engine = setup().await;

    // Gen 1: plugin A
    let scores1 = test_scores(0.5, 0.5, 1.0, AutonomyLevel::L1, 0.3);
    let snap1 = snapshot_with_plugins(vec![("plugA", vec!["Reasoning"])]);
    engine.evaluate(TEST_AGENT, scores1, snap1).await.unwrap();

    // Gen 2: add plugin B + safety breach → SafetyBreach should win
    let scores2 = test_scores(0.5, 0.5, 0.0, AutonomyLevel::L1, 0.3);
    let snap2 = snapshot_with_plugins(vec![
        ("plugA", vec!["Reasoning"]),
        ("plugB", vec!["Vision"]),
    ]);
    let events = engine.evaluate(TEST_AGENT, scores2, snap2).await.unwrap();

    // SafetyBreach should trigger rollback, not CapabilityGain
    let breach_events: Vec<_> = events.iter().filter(|e| {
        matches!(e, exiv_shared::ExivEventData::EvolutionBreach { .. })
    }).collect();
    assert!(!breach_events.is_empty(), "SafetyBreach should override CapabilityGain");

    // But EvolutionCapability events should STILL be emitted (independent of trigger)
    let cap_events: Vec<_> = events.iter().filter(|e| {
        matches!(e, exiv_shared::ExivEventData::EvolutionCapability { .. })
    }).collect();
    assert!(!cap_events.is_empty(), "EvolutionCapability events should be emitted even with SafetyBreach");
}

#[tokio::test]
async fn test_capability_gain_standalone_no_metric_trigger() {
    let engine = setup().await;

    // Lower min_interactions
    let mut params = engine.get_params(TEST_AGENT).await.unwrap();
    params.min_interactions = 1;
    engine.set_params(TEST_AGENT, &params).await.unwrap();

    // Gen 1: plugin A
    let scores1 = test_scores(0.5, 0.5, 1.0, AutonomyLevel::L1, 0.3);
    let snap1 = snapshot_with_plugins(vec![("plugA", vec!["Reasoning"])]);
    engine.evaluate(TEST_AGENT, scores1, snap1).await.unwrap();

    // Gen 2: add plugin B but identical scores → no metric trigger, only capability change
    let scores2 = test_scores(0.5, 0.5, 1.0, AutonomyLevel::L1, 0.3);
    let snap2 = snapshot_with_plugins(vec![
        ("plugA", vec!["Reasoning"]),
        ("plugB", vec!["Memory"]),
    ]);
    engine.evaluate(TEST_AGENT, scores2, snap2).await.unwrap();

    // Should still create a new generation with CapabilityGain
    let gen = engine.get_latest_generation(TEST_AGENT).await.unwrap();
    assert_eq!(gen, 2, "CapabilityGain should trigger generation even without metric change");

    let record = engine.get_generation(TEST_AGENT, 2).await.unwrap().unwrap();
    assert_eq!(record.trigger, exiv_core::evolution::GenerationTrigger::CapabilityGain);
}

// ── Validation tests ──

#[tokio::test]
async fn test_set_params_rejects_nan() {
    let engine = setup().await;
    let mut params = engine.get_params(TEST_AGENT).await.unwrap();
    params.alpha = f64::NAN;
    let result = engine.set_params(TEST_AGENT, &params).await;
    assert!(result.is_err(), "set_params should reject NaN alpha");
}

#[tokio::test]
async fn test_set_params_rejects_invalid_weights_sum() {
    let engine = setup().await;
    let mut params = engine.get_params(TEST_AGENT).await.unwrap();
    params.weights.cognitive = 0.9; // sum would be ~1.65
    let result = engine.set_params(TEST_AGENT, &params).await;
    assert!(result.is_err(), "set_params should reject weights that don't sum to ~1.0");
}

#[test]
fn test_from_normalized_nan_returns_l0() {
    assert_eq!(AutonomyLevel::from_normalized(f64::NAN), AutonomyLevel::L0);
    assert_eq!(AutonomyLevel::from_normalized(f64::INFINITY), AutonomyLevel::L0);
    assert_eq!(AutonomyLevel::from_normalized(-0.1), AutonomyLevel::L0);
    assert_eq!(AutonomyLevel::from_normalized(1.1), AutonomyLevel::L0);
}

// ── Concurrent increment_counter test ──

#[tokio::test]
async fn test_increment_counter_concurrent() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE plugin_data (plugin_id TEXT, key TEXT, value TEXT, PRIMARY KEY(plugin_id, key))"
    ).execute(&pool).await.unwrap();
    let store = Arc::new(SqliteDataStore::new(pool.clone()));

    // Run 10 concurrent increments
    let mut handles = vec![];
    for _ in 0..10 {
        let store = store.clone();
        handles.push(tokio::spawn(async move {
            use exiv_shared::PluginDataStore;
            store.increment_counter("test", "counter").await.unwrap()
        }));
    }

    let mut results = vec![];
    for h in handles {
        results.push(h.await.unwrap());
    }

    // All values should be unique (1..=10)
    results.sort();
    let expected: Vec<i64> = (1..=10).collect();
    assert_eq!(results, expected, "Concurrent increments should produce unique sequential values");
}
