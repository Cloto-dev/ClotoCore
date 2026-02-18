use std::sync::Arc;
use exiv_core::AppState;
use exiv_shared::{ExivEvent, ExivEventData, ExivMessage, MessageSource};

async fn create_test_app_state() -> Arc<AppState> {
    exiv_core::test_utils::create_test_app_state(None).await
}

#[tokio::test]
async fn test_user_message_to_response_flow() {
    let state = create_test_app_state().await;

    // Create a test agent
    state.agent_manager.create_agent(
        "Test Agent",
        "A test agent",
        "mind.deepseek",
        std::collections::HashMap::new(),
        vec![exiv_shared::CapabilityType::Reasoning],
        None,
    ).await.unwrap();

    // Subscribe to broadcast events
    let mut rx = state.tx.subscribe();

    // Create and send a MessageReceived event
    let user_message = ExivMessage {
        id: "msg-123".to_string(),
        source: MessageSource::User {
            id: "user-1".to_string(),
            name: "Test User".to_string(),
        },
        target_agent: Some("agent.test".to_string()),
        content: "Hello, agent!".to_string(),
        timestamp: chrono::Utc::now(),
        metadata: std::collections::HashMap::new(),
    };

    let event = Arc::new(ExivEvent::new(
        ExivEventData::MessageReceived(user_message.clone())
    ));

    // Send event to broadcast channel
    state.tx.send(event.clone()).unwrap();

    // Receive the event (should be the same we sent)
    let received = tokio::time::timeout(
        tokio::time::Duration::from_secs(1),
        rx.recv()
    )
    .await
    .expect("Timeout waiting for event")
    .expect("Failed to receive event");

    // Verify we received the MessageReceived event
    match &received.data {
        ExivEventData::MessageReceived(msg) => {
            assert_eq!(msg.id, user_message.id);
            assert_eq!(msg.content, "Hello, agent!");
        }
        _ => panic!("Expected MessageReceived event"),
    }
}

#[tokio::test]
async fn test_permission_grant_flow() {
    let state = create_test_app_state().await;

    // Insert a pending permission request
    sqlx::query("INSERT INTO permission_requests (request_id, plugin_id, permission_type, justification, status, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind("req-123")
        .bind("test.plugin")
        .bind("NetworkAccess")
        .bind("Testing")
        .bind("pending")
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&state.pool)
        .await
        .unwrap();

    // Subscribe to broadcast events
    let mut rx = state.tx.subscribe();

    // Approve the permission
    exiv_core::update_permission_request(&state.pool, "req-123", "approved", "admin")
        .await
        .unwrap();

    // Send PermissionGranted event
    let event = Arc::new(ExivEvent::new(
        ExivEventData::PermissionGranted {
            plugin_id: "test.plugin".to_string(),
            permission: exiv_shared::Permission::NetworkAccess,
        }
    ));

    state.tx.send(event.clone()).unwrap();

    // Receive the event
    let received = tokio::time::timeout(
        tokio::time::Duration::from_secs(1),
        rx.recv()
    )
    .await
    .expect("Timeout waiting for event")
    .expect("Failed to receive event");

    // Verify we received the PermissionGranted event
    match &received.data {
        ExivEventData::PermissionGranted { plugin_id, permission } => {
            assert_eq!(plugin_id, "test.plugin");
            assert_eq!(permission, &exiv_shared::Permission::NetworkAccess);
        }
        _ => panic!("Expected PermissionGranted event"),
    }

    // Verify database was updated
    let status: String = sqlx::query_scalar("SELECT status FROM permission_requests WHERE request_id = ?")
        .bind("req-123")
        .fetch_one(&state.pool)
        .await
        .unwrap();

    assert_eq!(status, "approved");
}

#[tokio::test]
async fn test_config_update_flow() {
    let state = create_test_app_state().await;

    // Insert initial config
    sqlx::query("INSERT INTO plugin_configs (plugin_id, config_key, config_value) VALUES (?, ?, ?)")
        .bind("test.plugin")
        .bind("api_key")
        .bind("old_value")
        .execute(&state.pool)
        .await
        .unwrap();

    // Subscribe to broadcast events
    let mut rx = state.tx.subscribe();

    // Update config
    state.plugin_manager.update_config("test.plugin", "api_key", "new_value")
        .await
        .unwrap();

    // Send ConfigUpdated event
    let mut config = std::collections::HashMap::new();
    config.insert("api_key".to_string(), "new_value".to_string());

    let event = Arc::new(ExivEvent::new(
        ExivEventData::ConfigUpdated {
            plugin_id: "test.plugin".to_string(),
            config,
        }
    ));

    state.tx.send(event.clone()).unwrap();

    // Receive the event
    let received = tokio::time::timeout(
        tokio::time::Duration::from_secs(1),
        rx.recv()
    )
    .await
    .expect("Timeout waiting for event")
    .expect("Failed to receive event");

    // Verify we received the ConfigUpdated event
    match &received.data {
        ExivEventData::ConfigUpdated { plugin_id, config } => {
            assert_eq!(plugin_id, "test.plugin");
            assert_eq!(config.get("api_key").unwrap(), "new_value");
        }
        _ => panic!("Expected ConfigUpdated event"),
    }

    // Verify database was updated
    let value: String = sqlx::query_scalar("SELECT config_value FROM plugin_configs WHERE plugin_id = ? AND config_key = ?")
        .bind("test.plugin")
        .bind("api_key")
        .fetch_one(&state.pool)
        .await
        .unwrap();

    assert_eq!(value, "new_value");
}

#[tokio::test]
async fn test_agent_creation_with_memory_context() {
    let state = create_test_app_state().await;

    // Create an agent
    state.agent_manager.create_agent(
        "Memory Test Agent",
        "An agent for testing memory",
        "mind.deepseek",
        std::collections::HashMap::new(),
        vec![
            exiv_shared::CapabilityType::Reasoning,
            exiv_shared::CapabilityType::Memory,
        ],
        None,
    ).await.unwrap();

    // Get the created agent (there may be default agents from init_db)
    let agents = state.agent_manager.list_agents().await.unwrap();
    assert!(!agents.is_empty(), "Expected at least 1 agent");

    // Find our test agent
    let agent = agents.iter()
        .find(|a| a.name == "Memory Test Agent")
        .expect("Test agent not found");

    assert_eq!(agent.name, "Memory Test Agent");
    assert!(agent.required_capabilities.contains(&exiv_shared::CapabilityType::Memory));

    // Verify the agent was stored in the database
    let stored_agent: (String, String) = sqlx::query_as("SELECT name, description FROM agents WHERE name = ?")
        .bind("Memory Test Agent")
        .fetch_one(&state.pool)
        .await
        .unwrap();

    assert_eq!(stored_agent.0, "Memory Test Agent");
    assert_eq!(stored_agent.1, "An agent for testing memory");
}
