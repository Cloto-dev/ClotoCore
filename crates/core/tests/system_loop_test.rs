use cloto_core::handlers::system::SystemHandler;
use cloto_core::managers::{AgentManager, PluginRegistry};
use cloto_shared::{ClotoEvent, ClotoMessage, MessageSource, Plugin};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_system_handler_loop_prevention() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    cloto_core::db::init_db(&pool, "sqlite::memory:")
        .await
        .unwrap();

    let agent_id = "agent.test";
    sqlx::query("INSERT INTO agents (id, name, description, status, default_engine_id, required_capabilities, metadata, enabled) VALUES (?, 'Test Agent', 'Desc', 'online', 'engine.test', '[\"Reasoning\", \"Memory\"]', '{}', 1)")
        .bind(agent_id)
        .execute(&pool).await.unwrap();

    let registry = Arc::new(PluginRegistry::new(5, 10));
    let agent_manager = AgentManager::new(pool);
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
    );

    // 1. Test User Message → triggers handle_message (agentic loop)
    //    Without a registered engine, the loop errors gracefully.
    //    The key assertion: on_event does NOT panic.
    let user_msg = ClotoMessage::new(
        MessageSource::User {
            id: "user1".into(),
            name: "User".into(),
        },
        "Hello".into(),
    );
    let user_event = ClotoEvent::new(cloto_shared::ClotoEventData::MessageReceived(user_msg));

    let result = handler.on_event(&user_event).await;
    assert!(
        result.is_ok(),
        "User message should be handled without panic"
    );

    // Drain any events produced by user message (e.g. error ThoughtResponse)
    while event_rx.try_recv().is_ok() {}

    // 2. Test Agent Message (Should NOT trigger processing at all)
    let agent_msg = ClotoMessage::new(
        MessageSource::Agent {
            id: agent_id.into(),
        },
        "Response".into(),
    );
    let agent_event = ClotoEvent::new(cloto_shared::ClotoEventData::MessageReceived(agent_msg));

    let _ = handler.on_event(&agent_event).await.unwrap();

    // Check that NO event was sent to the channel for agent messages
    let result = event_rx.try_recv();
    assert!(
        result.is_err(),
        "Should NOT have received any event for agent message"
    );
}
