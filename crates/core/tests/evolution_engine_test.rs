//! Integration tests for the Self-Evolution Benchmark Engine.
//! Tests the EvolutionEngine through SqliteDataStore with an in-memory DB.

use std::sync::Arc;
use sqlx::SqlitePool;
use exiv_core::db::SqliteDataStore;
use exiv_core::evolution::{
    EvolutionEngine, FitnessScores, AutonomyLevel, AgentSnapshot,
};

const TEST_AGENT: &str = "agent.test";

async fn setup() -> EvolutionEngine {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE plugin_data (plugin_id TEXT, key TEXT, value TEXT, PRIMARY KEY(plugin_id, key))"
    ).execute(&pool).await.unwrap();
    sqlx::query(
        "CREATE TABLE audit_log (id INTEGER PRIMARY KEY AUTOINCREMENT, timestamp TEXT, event_type TEXT, actor_id TEXT, target_id TEXT, permission TEXT, result TEXT, reason TEXT, metadata TEXT, trace_id TEXT)"
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
