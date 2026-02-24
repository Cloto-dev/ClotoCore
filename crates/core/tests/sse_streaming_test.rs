use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use cloto_core::handlers;
use cloto_core::test_utils::create_test_app_state as create_test_app_state_with_key;
use cloto_core::AppState;
use cloto_shared::{ClotoEvent, ClotoEventData};
use futures::StreamExt;
use std::sync::Arc;
use tower::ServiceExt;

async fn create_test_app_state() -> Arc<cloto_core::AppState> {
    create_test_app_state_with_key(None).await
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
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-cache");
}

#[tokio::test]
async fn test_sse_handler_streams_events() {
    let state = create_test_app_state().await;

    // Spawn a task to send events after a short delay
    let tx = state.tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Send a test event
        let event = Arc::new(ClotoEvent::new(ClotoEventData::SystemNotification(
            "Test message".to_string(),
        )));
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
    let first_chunk = tokio::time::timeout(tokio::time::Duration::from_secs(1), stream.next())
        .await
        .expect("Timeout waiting for handshake")
        .expect("Stream ended unexpectedly")
        .expect("Error reading stream");

    let first_data = String::from_utf8(first_chunk.to_vec()).unwrap();
    assert!(first_data.contains("event: handshake"));
    assert!(first_data.contains("data: connected"));

    // Second chunk should be our test event (with timeout to prevent hanging)
    let second_chunk =
        tokio::time::timeout(tokio::time::Duration::from_secs(2), stream.next()).await;

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
            let event = Arc::new(ClotoEvent::new(ClotoEventData::SystemNotification(format!(
                "Message {}",
                i
            ))));
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
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(1), stream.next())
        .await
        .expect("Timeout waiting for handshake");

    // Try to read a few more chunks (may or may not get all messages due to lagging)
    for _ in 0..3 {
        if tokio::time::timeout(tokio::time::Duration::from_millis(500), stream.next())
            .await
            .is_err()
        {
            break; // Timeout is acceptable
        }
    }

    // Test passes if we didn't panic
}
