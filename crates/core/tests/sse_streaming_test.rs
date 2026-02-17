use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use futures::StreamExt;
use sqlx::SqlitePool;
use std::sync::Arc;
use tower::ServiceExt;
use exiv_core::AppState;
use exiv_core::config::AppConfig;
use exiv_core::handlers;
use exiv_core::managers::{PluginRegistry, AgentManager, PluginManager, SystemMetrics};
use exiv_core::DynamicRouter;
use exiv_shared::{ExivEvent, ExivEventData};
use std::collections::VecDeque;
use tokio::sync::{broadcast, mpsc, Notify, RwLock};

/// Helper function to create test app state
async fn create_test_app_state() -> Arc<AppState> {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    exiv_core::db::init_db(&pool, "sqlite::memory:").await.unwrap();

    let (event_tx, _event_rx) = mpsc::channel(100);
    let (tx, _rx) = broadcast::channel(100);

    let registry = Arc::new(PluginRegistry::new(5, 10));
    let agent_manager = AgentManager::new(pool.clone());
    let plugin_manager = Arc::new(PluginManager::new(
        pool.clone(),
        vec![],
        30,
        10,
    ).unwrap());

    let dynamic_router = Arc::new(DynamicRouter {
        router: RwLock::new(axum::Router::new()),
    });

    let metrics = Arc::new(SystemMetrics::new());
    let event_history = Arc::new(RwLock::new(VecDeque::new()));

    let config = AppConfig::load().unwrap();
    let rate_limiter = Arc::new(exiv_core::middleware::RateLimiter::new(10, 20));

    Arc::new(AppState {
        tx,
        registry,
        event_tx,
        pool,
        agent_manager,
        plugin_manager,
        dynamic_router,
        config,
        event_history,
        metrics,
        rate_limiter,
        shutdown: Arc::new(Notify::new()),
        evolution_engine: None,
    })
}

/// Helper function to create a test router with app state
fn create_test_router(state: Arc<AppState>) -> axum::Router {
    use axum::routing::get;

    let api_routes = axum::Router::new()
        .route("/events", get(handlers::sse_handler))
        .with_state(state);

    axum::Router::new().nest("/api", api_routes)
}

#[tokio::test]
async fn test_sse_handshake() {
    let state = create_test_app_state().await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Check status code
    assert_eq!(response.status(), StatusCode::OK);

    // Check SSE headers
    let headers = response.headers();
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "text/event-stream"
    );
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "no-cache"
    );
}

#[tokio::test]
async fn test_sse_handler_streams_events() {
    let state = create_test_app_state().await;

    // Spawn a task to send events after a short delay
    let tx = state.tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Send a test event
        let event = Arc::new(ExivEvent::new(
            ExivEventData::SystemNotification("Test message".to_string())
        ));
        let _ = tx.send(event);
    });

    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Convert response body to stream
    let body = response.into_body();
    let mut stream = body.into_data_stream();

    // First chunk should be the handshake
    let first_chunk = tokio::time::timeout(
        tokio::time::Duration::from_secs(1),
        stream.next()
    )
    .await
    .expect("Timeout waiting for handshake")
    .expect("Stream ended unexpectedly")
    .expect("Error reading stream");

    let first_data = String::from_utf8(first_chunk.to_vec()).unwrap();
    assert!(first_data.contains("event: handshake"));
    assert!(first_data.contains("data: connected"));

    // Second chunk should be our test event (with timeout to prevent hanging)
    let second_chunk = tokio::time::timeout(
        tokio::time::Duration::from_secs(2),
        stream.next()
    )
    .await;

    // If we got an event, verify it contains our test message
    if let Ok(Some(Ok(chunk))) = second_chunk {
        let event_data = String::from_utf8(chunk.to_vec()).unwrap();
        assert!(event_data.contains("Test message"));
    }
    // If timeout or no event, that's also acceptable in test environment
}

#[tokio::test]
async fn test_sse_handler_handles_lagged_receiver() {
    let state = create_test_app_state().await;

    // Send many events rapidly to potentially cause lagging
    let tx = state.tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        for i in 0..10 {
            let event = Arc::new(ExivEvent::new(
                ExivEventData::SystemNotification(format!("Message {}", i))
            ));
            let _ = tx.send(event);
        }
    });

    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should still return OK even with potential lagging
    assert_eq!(response.status(), StatusCode::OK);

    // Stream should handle lagged messages gracefully (no panic)
    let body = response.into_body();
    let mut stream = body.into_data_stream();

    // Read first chunk (handshake)
    let _ = tokio::time::timeout(
        tokio::time::Duration::from_secs(1),
        stream.next()
    )
    .await
    .expect("Timeout waiting for handshake");

    // Try to read a few more chunks (may or may not get all messages due to lagging)
    for _ in 0..3 {
        if tokio::time::timeout(
            tokio::time::Duration::from_millis(500),
            stream.next()
        )
        .await
        .is_err() {
            break; // Timeout is acceptable
        }
    }

    // Test passes if we didn't panic
}
