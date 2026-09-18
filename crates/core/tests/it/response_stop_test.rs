//! Stopping a reply while it is being produced (`POST /api/chat/{agent_id}/stop`).
//!
//! What a stop has to do, each tested where it can be seen: the route answers
//! whether there was a reply to stop; a stopped turn stores nothing of the
//! reply and sends `ResponseStopped` instead of `ThoughtResponse`; and the
//! user's own message is stored even when the stop came before the turn began.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use cloto_core::handlers::chat::{stop_response, StopResponseRequest};
use cloto_core::handlers::system::SystemHandler;
use cloto_core::managers::response_stop::ResponseStops;
use cloto_core::managers::{AgentManager, McpClientManager, PluginRegistry};
use cloto_core::test_utils::create_test_app_state;
use cloto_core::AppState;
use cloto_shared::{ClotoEventData, ClotoMessage, MessageSource};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

const API_KEY: &str = "test-key";
const AGENT: &str = "agent.a";

fn headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("X-API-Key", API_KEY.parse().unwrap());
    h
}

async fn call_stop(
    state: &Arc<AppState>,
    headers: HeaderMap,
    agent: &str,
    message_id: &str,
) -> (StatusCode, serde_json::Value) {
    let response = match stop_response(
        State(state.clone()),
        headers,
        Path(agent.to_string()),
        Json(StopResponseRequest {
            source_message_id: message_id.to_string(),
        }),
    )
    .await
    {
        Ok(json) => json.into_response(),
        Err(err) => err.into_response(),
    };
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
async fn the_route_stops_only_a_reply_the_agent_is_producing() {
    let state = create_test_app_state(Some(API_KEY.into())).await;

    let (status, body) = call_stop(&state, headers(), AGENT, "m1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["stopped"], false, "nothing is running yet");

    let _registration = state.response_stops.register("m1", AGENT);
    let (_, body) = call_stop(&state, headers(), "agent.other", "m1").await;
    assert_eq!(body["data"]["stopped"], false, "another agent stopped it");

    let (_, body) = call_stop(&state, headers(), AGENT, "m1").await;
    assert_eq!(body["data"]["stopped"], true);
}

#[tokio::test]
async fn the_route_wants_the_admin_key() {
    let state = create_test_app_state(Some(API_KEY.into())).await;
    let _registration = state.response_stops.register("m1", AGENT);
    let (status, _) = call_stop(&state, HeaderMap::new(), AGENT, "m1").await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "stopped without a key: {status}"
    );
    // And it did not stop anything on the way.
    let (_, body) = call_stop(&state, headers(), AGENT, "m1").await;
    assert_eq!(body["data"]["stopped"], true);
}

/// A handler nobody routes passes every test above. The router is assembled
/// inline during boot, so the wiring is asserted where it is written.
#[test]
fn the_route_is_registered() {
    let wiring = include_str!("../../src/lib.rs");
    assert!(
        wiring.contains(r#".route("/chat/{agent_id}/stop", post(handlers::chat::stop_response))"#),
        "POST /api/chat/{{agent_id}}/stop is not routed"
    );
    // The registry the route stops is the one the turns register in.
    assert!(wiring.contains("h.set_response_stops(response_stops.clone());"));
}

/// The event loop registers a reply when its message is queued — before it
/// waits for the agent's previous turn — and hands the turn that registration.
#[test]
fn a_reply_is_registered_before_it_waits_its_turn() {
    let events = include_str!("../../src/events.rs");
    let registered = events
        .find(".register(&msg.id, &handler.target_agent_of(&msg))")
        .expect("the event loop does not register replies");
    let waits = events[registered..]
        .find("sem.acquire().await")
        .expect("no wait for the agent's turn after registering");
    let handled = events[registered + waits..]
        .find(".handle_message_stoppable(msg, registration.stopped())")
        .expect("the turn is not handed its registration");
    assert!(waits > 0 && handled > 0);
}

async fn handler_on(
    pool: SqlitePool,
    stops: ResponseStops,
) -> (SystemHandler, mpsc::Receiver<cloto_core::EnvelopedEvent>) {
    let mcp = Arc::new(McpClientManager::new(pool.clone(), false, 120, 30));
    let registry = Arc::new(PluginRegistry::new(5, 10, 50, mcp));
    let agent_manager = AgentManager::new(pool.clone(), 90_000);
    let (event_tx, event_rx) = mpsc::channel(64);
    let metrics = Arc::new(cloto_core::managers::SystemMetrics::new());
    let mut handler = SystemHandler::new(
        registry,
        agent_manager,
        AGENT.to_string(),
        event_tx,
        10,
        metrics,
        vec![],
        "consensus:".to_string(),
        16,
        30,
        Arc::new(dashmap::DashMap::new()),
        Arc::new(dashmap::DashMap::new()),
        pool,
        Arc::new(dashmap::DashMap::new()),
        5,
        false,
    );
    handler.set_response_stops(stops);
    (handler, event_rx)
}

fn user_message() -> ClotoMessage {
    let mut metadata = HashMap::new();
    metadata.insert("target_agent_id".to_string(), AGENT.to_string());
    ClotoMessage {
        id: cloto_shared::ClotoId::new().to_string(),
        source: MessageSource::User {
            id: "default".into(),
            name: "User".into(),
        },
        target_agent: Some(AGENT.to_string()),
        content: "hello".to_string(),
        timestamp: chrono::Utc::now(),
        metadata,
    }
}

async fn rows(pool: &SqlitePool, source: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM chat_messages WHERE agent_id = ? AND source = ?")
        .bind(AGENT)
        .bind(source)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn drain(rx: &mut mpsc::Receiver<cloto_core::EnvelopedEvent>) -> Vec<ClotoEventData> {
    let mut out = Vec::new();
    while let Ok(e) = rx.try_recv() {
        out.push(e.event.data.clone());
    }
    out
}

async fn agent_row(state: &Arc<AppState>) {
    sqlx::query("INSERT INTO agents (id, name, description, status, default_engine_id, required_capabilities, metadata, enabled) VALUES (?, 'A', 'd', 'online', 'engine.none', '[]', '{}', 1)")
        .bind(AGENT)
        .execute(&state.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn an_unstopped_turn_stores_its_reply_and_answers_with_a_thought() {
    // The control for the test below: no engine is registered, so the reply
    // is an error — stored and sent all the same.
    let state = create_test_app_state(Some(API_KEY.into())).await;
    agent_row(&state).await;
    let stops = ResponseStops::new();
    let (handler, mut rx) = handler_on(state.pool.clone(), stops.clone()).await;
    let msg = user_message();
    let registration = stops.register(&msg.id, AGENT);

    handler
        .handle_message_stoppable(msg.clone(), registration.stopped())
        .await
        .unwrap();

    assert_eq!(rows(&state.pool, "user").await, 1);
    assert_eq!(
        rows(&state.pool, "agent").await,
        1,
        "the reply was not stored"
    );
    let events = drain(&mut rx);
    assert!(events.iter().any(|e| matches!(e, ClotoEventData::ThoughtResponse { source_message_id, .. } if *source_message_id == msg.id)));
    assert!(!events
        .iter()
        .any(|e| matches!(e, ClotoEventData::ResponseStopped { .. })));
}

#[tokio::test]
async fn a_stopped_turn_keeps_the_users_message_stores_no_reply_and_says_it_stopped() {
    let state = create_test_app_state(Some(API_KEY.into())).await;
    agent_row(&state).await;
    let stops = ResponseStops::new();
    let (handler, mut rx) = handler_on(state.pool.clone(), stops.clone()).await;
    let msg = user_message();
    let registration = stops.register(&msg.id, AGENT);
    // Stopped while still queued, before the turn began.
    assert!(stops.stop(AGENT, &msg.id));

    handler
        .handle_message_stoppable(msg.clone(), registration.stopped())
        .await
        .unwrap();

    assert_eq!(
        rows(&state.pool, "user").await,
        1,
        "the user's message was lost"
    );
    assert_eq!(
        rows(&state.pool, "agent").await,
        0,
        "a stopped reply was stored"
    );
    let events = drain(&mut rx);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ClotoEventData::ThoughtResponse { .. })),
        "a stopped turn still answered"
    );
    assert!(events.iter().any(|e| matches!(
        e,
        ClotoEventData::ResponseStopped { agent_id, source_message_id }
            if agent_id == AGENT && *source_message_id == msg.id
    )));
}
