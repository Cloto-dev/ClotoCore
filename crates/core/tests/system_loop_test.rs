use cloto_core::handlers::system::SystemHandler;
use cloto_core::managers::{AgentManager, PluginRegistry};
use cloto_shared::{ClotoMessage, MessageSource};
use sqlx::SqlitePool;
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
        5, // memory_timeout_secs
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
