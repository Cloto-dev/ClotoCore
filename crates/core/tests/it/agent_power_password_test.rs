//! Changing the password that guards an agent's power switch.
//!
//! The password exists so that holding the dashboard is not enough to stop or
//! delete an agent. The route that changes it is therefore the one place the
//! guard could be walked around: if a change went through on the admin key
//! alone, whoever has the key would replace the password and then use the new
//! one. Every test here is about that — what the route refuses, and that a
//! refusal leaves the old password working.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use cloto_core::handlers::agents::{set_power_password, PowerPasswordRequest};
use cloto_core::managers::AgentManager;
use cloto_core::test_utils::create_test_app_state;
use cloto_core::AppState;
use std::collections::HashMap;
use std::sync::Arc;

const API_KEY: &str = "test-key";

async fn state() -> Arc<AppState> {
    create_test_app_state(Some(API_KEY.into())).await
}

fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("X-API-Key", API_KEY.parse().unwrap());
    headers
}

async fn agent(state: &Arc<AppState>, password: Option<&str>) -> String {
    state
        .agent_manager
        .create_agent("Guarded", "d", "e", HashMap::new(), vec![], password)
        .await
        .expect("create agent")
}

async fn change(
    state: &Arc<AppState>,
    headers: HeaderMap,
    id: &str,
    current: Option<&str>,
    new: &str,
) -> (StatusCode, serde_json::Value) {
    let outcome = set_power_password(
        State(state.clone()),
        headers,
        Path(id.to_string()),
        Json(PowerPasswordRequest {
            current_password: current.map(str::to_string),
            new_password: new.to_string(),
        }),
    )
    .await;
    let response = match outcome {
        Ok(json) => json.into_response(),
        Err(err) => err.into_response(),
    };
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

/// Whether `candidate` is the password the agent currently has.
async fn opens(state: &Arc<AppState>, id: &str, candidate: &str) -> bool {
    match state.agent_manager.get_password_hash(id).await.unwrap() {
        Some(hash) => AgentManager::verify_password(candidate, &hash).unwrap(),
        None => false,
    }
}

#[tokio::test]
async fn an_agent_without_a_password_gets_one() {
    let state = state().await;
    let id = agent(&state, None).await;

    let (status, body) = change(&state, headers(), &id, None, "first").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["has_power_password"], true);
    assert!(opens(&state, &id, "first").await);
    assert!(!opens(&state, &id, "other").await);
}

#[tokio::test]
async fn the_wrong_current_password_changes_nothing() {
    let state = state().await;
    let id = agent(&state, Some("old")).await;

    let (status, _) = change(&state, headers(), &id, Some("guess"), "new").await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        opens(&state, &id, "old").await,
        "the old password still works"
    );
    assert!(
        !opens(&state, &id, "new").await,
        "the new one was not stored"
    );
}

#[tokio::test]
async fn a_missing_current_password_changes_nothing() {
    let state = state().await;
    let id = agent(&state, Some("old")).await;

    let (status, _) = change(&state, headers(), &id, None, "new").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(opens(&state, &id, "old").await);
    assert!(!opens(&state, &id, "new").await);
}

#[tokio::test]
async fn the_right_current_password_replaces_it() {
    let state = state().await;
    let id = agent(&state, Some("old")).await;

    let (status, body) = change(&state, headers(), &id, Some("old"), "new").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(opens(&state, &id, "new").await);
    assert!(!opens(&state, &id, "old").await, "the old password is gone");
}

#[tokio::test]
async fn an_empty_new_password_removes_it_and_only_with_the_current_one() {
    let state = state().await;
    let id = agent(&state, Some("old")).await;

    let (refused, _) = change(&state, headers(), &id, Some("guess"), "").await;
    assert_eq!(refused, StatusCode::FORBIDDEN);
    assert!(
        opens(&state, &id, "old").await,
        "a refused removal removes nothing"
    );

    let (status, body) = change(&state, headers(), &id, Some("old"), "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["has_power_password"], false);
    assert_eq!(
        state.agent_manager.get_password_hash(&id).await.unwrap(),
        None,
        "removed, not replaced by a hash of the empty string"
    );
}

#[tokio::test]
async fn the_admin_key_is_required() {
    let state = state().await;
    let id = agent(&state, None).await;

    let (status, _) = change(&state, HeaderMap::new(), &id, None, "first").await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        state.agent_manager.get_password_hash(&id).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn an_unknown_agent_is_not_found() {
    let state = state().await;

    let (status, _) = change(&state, headers(), "agent.nobody", None, "first").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A handler nobody routes passes every test above. The router is assembled
/// inline during boot, so the wiring is asserted where it is written.
#[test]
fn the_route_is_registered() {
    let wiring = include_str!("../../src/lib.rs");
    assert!(
        wiring.contains(concat!("\"/agents/{id}/power-", "password\"")),
        "the power-password route is not registered"
    );
    assert!(
        wiring.contains(concat!("post(handlers::set_power_", "password)")),
        "the power-password route does not reach its handler"
    );
}
