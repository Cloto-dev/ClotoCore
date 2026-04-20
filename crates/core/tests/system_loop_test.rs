use cloto_core::handlers::system::SystemHandler;
use cloto_core::managers::{AgentManager, PluginRegistry};
use cloto_shared::{ClotoMessage, MessageSource};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_system_handler_loop_prevention() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    cloto_core::db::init_db(&pool, "sqlite::memory:", "memory.cpersona")
        .await
        .unwrap();

    let agent_id = "agent.test";
    sqlx::query("INSERT INTO agents (id, name, description, status, default_engine_id, required_capabilities, metadata, enabled) VALUES (?, 'Test Agent', 'Desc', 'online', 'engine.test', '[\"Reasoning\", \"Memory\"]', '{}', 1)")
        .bind(agent_id)
        .execute(&pool).await.unwrap();

    let registry = Arc::new(PluginRegistry::new(5, 10, 50));
    let agent_manager = AgentManager::new(pool.clone(), 90_000);
    let (event_tx, mut event_rx) = mpsc::channel(10);

    let metrics = Arc::new(cloto_core::managers::SystemMetrics::new());
    let handler = SystemHandler::new(
        registry.clone(),
        agent_manager,
        agent_id.to_string(),
        event_tx,
        10, // memory_context_limit
        metrics,
        vec!["mind.deepseek".to_string(), "mind.cerebras".to_string()],
        16, // max_agentic_iterations
        30, // tool_execution_timeout_secs
        Arc::new(dashmap::DashMap::new()),
        Arc::new(dashmap::DashMap::new()),
        pool,
        Arc::new(dashmap::DashMap::new()),
        5,     // memory_timeout_secs
        false, // mcp_streaming_enabled
    );

    // Test User Message → triggers handle_message (agentic loop).
    // Without a registered engine, the loop errors gracefully.
    // The key assertion: handle_message does NOT panic.
    let user_msg = ClotoMessage::new(
        MessageSource::User {
            id: "user1".into(),
            name: "User".into(),
        },
        "Hello".into(),
    );

    let result = handler.handle_message(user_msg).await;
    assert!(
        result.is_ok(),
        "User message should be handled without panic"
    );

    // Drain any events produced by user message (e.g. error ThoughtResponse)
    while event_rx.try_recv().is_ok() {}
}

/// Build a `SystemHandler` and matching cron job row for the cron-related
/// regression tests below. Returns the handler together with shared handles
/// the tests assert against.
async fn build_cron_test_handler(
    agent_id: &str,
    agent_enabled: bool,
) -> (
    SystemHandler,
    SqlitePool,
    Arc<dashmap::DashMap<String, cloto_core::CronExecContext>>,
    mpsc::Receiver<cloto_core::EnvelopedEvent>,
) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    cloto_core::db::init_db(&pool, "sqlite::memory:", "memory.cpersona")
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO agents (id, name, description, status, default_engine_id, required_capabilities, metadata, enabled) VALUES (?, 'Cron Test', 'Desc', 'online', 'engine.test', '[\"Reasoning\"]', '{}', ?)",
    )
    .bind(agent_id)
    .bind(agent_enabled)
    .execute(&pool)
    .await
    .unwrap();

    let registry = Arc::new(PluginRegistry::new(5, 10, 50));
    let agent_manager = AgentManager::new(pool.clone(), 90_000);
    let (event_tx, event_rx) = mpsc::channel(10);
    let metrics = Arc::new(cloto_core::managers::SystemMetrics::new());
    let active_cron_contexts: Arc<dashmap::DashMap<String, cloto_core::CronExecContext>> =
        Arc::new(dashmap::DashMap::new());

    let handler = SystemHandler::new(
        registry,
        agent_manager,
        agent_id.to_string(),
        event_tx,
        10,
        metrics,
        vec![],
        16,
        30,
        Arc::new(dashmap::DashMap::new()),
        Arc::new(dashmap::DashMap::new()),
        pool.clone(),
        active_cron_contexts.clone(),
        5,
        false, // mcp_streaming_enabled
    );

    (handler, pool, active_cron_contexts, event_rx)
}

/// Insert a cron_jobs row that starts in the `"dispatched"` state — the same
/// state the scheduler writes right after `event_tx.send()` succeeds.
async fn insert_dispatched_cron_job(pool: &SqlitePool, job_id: &str, agent_id: &str) {
    sqlx::query(
        "INSERT INTO cron_jobs (id, agent_id, name, enabled, schedule_type, schedule_value, message, next_run_at, last_run_at, last_status, hide_prompt, cron_generation, source_type) \
         VALUES (?, ?, 'test', 0, 'once', '2099-01-01T00:00:00Z', 'hello', 9223372036854775807, 0, 'dispatched', 0, 0, 'system')"
    )
    .bind(job_id)
    .bind(agent_id)
    .execute(pool)
    .await
    .unwrap();
}

/// Regression: a cron dispatched to a powered-off agent must be recorded as
/// `"skipped"` in `cron_jobs.last_status` — not left as `"dispatched"` or
/// (worst case) falsely marked `"success"` by the scheduler. Violating this
/// breaks the observability guarantee in `docs/DEVELOPMENT.md §1.2`.
#[tokio::test]
async fn test_cron_on_disabled_agent_is_recorded_as_skipped() {
    let agent_id = "agent.disabled";
    let (handler, pool, _contexts, mut event_rx) = build_cron_test_handler(agent_id, false).await;

    let job_id = "cron.agent.disabled.test1";
    insert_dispatched_cron_job(&pool, job_id, agent_id).await;

    let mut metadata = HashMap::new();
    metadata.insert("cron_job_id".into(), job_id.to_string());
    metadata.insert("target_agent_id".into(), agent_id.to_string());

    let msg = ClotoMessage {
        id: "msg-skip".into(),
        source: MessageSource::System,
        target_agent: Some(agent_id.to_string()),
        content: "hello".into(),
        timestamp: chrono::Utc::now(),
        metadata,
    };

    handler.handle_message(msg).await.unwrap();

    let (status, last_error): (String, Option<String>) =
        sqlx::query_as("SELECT last_status, last_error FROM cron_jobs WHERE id = ?")
            .bind(job_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(status, "skipped", "disabled agent should yield 'skipped'");
    assert!(
        last_error.as_deref().unwrap_or("").contains("powered off"),
        "last_error should explain why the cron was skipped, got: {:?}",
        last_error
    );

    while event_rx.try_recv().is_ok() {}
}

/// Regression: `active_cron_contexts` must be cleaned up even when the
/// agent-disabled early return path is taken. Before the fix, the entry
/// leaked and caused future non-cron messages to the same agent to be
/// treated as if they were inside a cron execution context.
#[tokio::test]
async fn test_active_cron_contexts_cleaned_on_disabled_agent() {
    let agent_id = "agent.disabled_leak";
    let (handler, pool, contexts, mut event_rx) = build_cron_test_handler(agent_id, false).await;

    let job_id = "cron.agent.disabled_leak.test2";
    insert_dispatched_cron_job(&pool, job_id, agent_id).await;

    let mut metadata = HashMap::new();
    metadata.insert("cron_job_id".into(), job_id.to_string());
    metadata.insert("target_agent_id".into(), agent_id.to_string());

    let msg = ClotoMessage {
        id: "msg-leak".into(),
        source: MessageSource::System,
        target_agent: Some(agent_id.to_string()),
        content: "hello".into(),
        timestamp: chrono::Utc::now(),
        metadata,
    };

    handler.handle_message(msg).await.unwrap();

    assert!(
        contexts.is_empty(),
        "active_cron_contexts must be empty after early return, got {} entries",
        contexts.len()
    );

    while event_rx.try_recv().is_ok() {}
}

/// Regression: the cron status must always transition away from
/// `"dispatched"` (the scheduler's initial placeholder) once
/// `handle_message` returns. Leaving it at `"dispatched"` would make an
/// already-executed job indistinguishable from an in-flight one.
#[tokio::test]
async fn test_cron_status_transitions_away_from_dispatched() {
    let agent_id = "agent.enabled_no_engine";
    let (handler, pool, _contexts, mut event_rx) = build_cron_test_handler(agent_id, true).await;

    let job_id = "cron.agent.enabled_no_engine.test3";
    insert_dispatched_cron_job(&pool, job_id, agent_id).await;

    let mut metadata = HashMap::new();
    metadata.insert("cron_job_id".into(), job_id.to_string());
    metadata.insert("target_agent_id".into(), agent_id.to_string());

    let msg = ClotoMessage {
        id: "msg-transition".into(),
        source: MessageSource::System,
        target_agent: Some(agent_id.to_string()),
        content: "hello".into(),
        timestamp: chrono::Utc::now(),
        metadata,
    };

    let _ = handler.handle_message(msg).await;

    let status: String = sqlx::query_scalar("SELECT last_status FROM cron_jobs WHERE id = ?")
        .bind(job_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_ne!(
        status, "dispatched",
        "cron must not remain in 'dispatched' after handle_message returns"
    );

    while event_rx.try_recv().is_ok() {}
}
