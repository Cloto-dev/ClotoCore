use std::sync::Arc;
use tokio::sync::mpsc;
use exiv_shared::{ExivEvent, ExivMessage, MessageSource, Plugin};
use exiv_core::handlers::system::SystemHandler;
use exiv_core::managers::{PluginRegistry, AgentManager};
use sqlx::SqlitePool;

#[tokio::test]
async fn test_system_handler_loop_prevention() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    exiv_core::db::init_db(&pool, "sqlite::memory:").await.unwrap();

    let agent_id = "agent.test";
    sqlx::query("INSERT INTO agents (id, name, description, status, default_engine_id, required_capabilities, metadata, enabled) VALUES (?, 'Test Agent', 'Desc', 'online', 'engine.test', '[\"Reasoning\", \"Memory\"]', '{}', 1)")
        .bind(agent_id)
        .execute(&pool).await.unwrap();

    let registry = Arc::new(PluginRegistry::new(5, 10));
    let agent_manager = AgentManager::new(pool);
    let (event_tx, mut event_rx) = mpsc::channel(10);

    let metrics = Arc::new(exiv_core::managers::SystemMetrics::new());
    let handler = SystemHandler::new(
        registry.clone(),
        agent_manager,
        agent_id.to_string(),
        event_tx,
        10, // memory_context_limit
        metrics,
        vec!["mind.deepseek".to_string(), "mind.cerebras".to_string()],
    );

    // 1. Test User Message (Should trigger ThoughtRequested)
    let user_msg = ExivMessage::new(
        MessageSource::User { id: "user1".into(), name: "User".into() },
        "Hello".into()
    );
    let user_event = ExivEvent::new(exiv_shared::ExivEventData::MessageReceived(user_msg));
    
    let _ = handler.on_event(&user_event).await.unwrap();
    
    let received_envelope = event_rx.try_recv().expect("Should have received ThoughtRequested");
    if let exiv_shared::ExivEventData::ThoughtRequested { .. } = &received_envelope.event.data {
        // OK
    } else {
        panic!("Expected ThoughtRequested, got {:?}", received_envelope.event);
    }

    // 2. Test Agent Message (Should NOT trigger ThoughtRequested)
    let agent_msg = ExivMessage::new(
        MessageSource::Agent { id: agent_id.into() },
        "Response".into()
    );
    let agent_event = ExivEvent::new(exiv_shared::ExivEventData::MessageReceived(agent_msg));
    
    let _ = handler.on_event(&agent_event).await.unwrap();
    
    // Check that NO event was sent to the channel
    let result = event_rx.try_recv();
    assert!(result.is_err(), "Should NOT have received any event for agent message");
}
