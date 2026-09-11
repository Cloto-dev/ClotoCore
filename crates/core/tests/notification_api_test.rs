//! The read surface a bell talks to.
//!
//! Driven through the handlers the way a caller reaches them, because what
//! matters is what a caller can conclude from the response — not which internal
//! value produced it.
//!
//! The load-bearing test in here is the one about severity. Everything else is
//! ordinary plumbing; that one is the guard on the rule that a threshold decides
//! whether an item interrupts, never whether it can be found.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use cloto_core::db::{record_notification, NotificationItem, NotificationKind};
use cloto_core::handlers::notifications::{
    list_notifications, mark_notification_read, notification_summary, ListQuery,
};
use cloto_core::test_utils::create_test_app_state;
use cloto_core::AppState;
use cloto_shared::McpLogLevel;
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

async fn read_response(
    outcome: Result<Json<serde_json::Value>, cloto_core::AppError>,
) -> (StatusCode, serde_json::Value) {
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

async fn summary(state: &Arc<AppState>) -> serde_json::Value {
    let (status, body) =
        read_response(notification_summary(State(state.clone()), headers()).await).await;
    assert_eq!(status, StatusCode::OK, "summary: {body}");
    body["data"]["summary"].clone()
}

async fn list(state: &Arc<AppState>, unresolved: bool) -> Vec<serde_json::Value> {
    let (status, body) = read_response(
        list_notifications(
            State(state.clone()),
            headers(),
            Query(ListQuery {
                unresolved: Some(unresolved),
                limit: None,
            }),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "list: {body}");
    body["data"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[tokio::test]
async fn the_badge_counts_a_blocked_agent_at_the_quietest_severity_there_is() {
    let state = state().await;

    // `debug` is the far end of RFC 5424 — below anything a reader would ever
    // set a threshold at. If a threshold ever leaks into the counting, this row
    // is the first one it drops, and the failure it produces is the worst one
    // this design has: the agent waits, the badge reads zero, and nothing
    // anywhere says why. That is why the quietest possible level is the one
    // pinned here rather than a realistic one.
    record_notification(
        &state.pool,
        NotificationItem::new(
            "quiet-but-stuck",
            NotificationKind::Approval,
            McpLogLevel::Debug,
            "Something is holding an agent",
        )
        .blocking(),
    )
    .await
    .expect("record");

    let summary = summary(&state).await;
    assert_eq!(
        summary["waiting"], 1,
        "a threshold decides whether an item interrupts, never whether it exists"
    );
    assert_eq!(
        summary["blocking"], 1,
        "and an agent held behind it has to be visible as held"
    );
}

#[tokio::test]
async fn the_badge_leaves_out_what_nobody_has_to_answer() {
    let state = state().await;

    for (id, kind) in [
        ("a", NotificationKind::Approval),
        ("p", NotificationKind::Proposal),
        ("n", NotificationKind::Notice),
    ] {
        record_notification(
            &state.pool,
            NotificationItem::new(id, kind, McpLogLevel::Warning, "t"),
        )
        .await
        .expect("record");
    }

    let summary = summary(&state).await;
    assert_eq!(
        summary["waiting"], 2,
        "a notice is news, not a question — counting it makes the badge a thing \
         that is always on, which is a thing nobody reads"
    );
    assert_eq!(summary["blocking"], 0, "none of these is holding anything");
}

#[tokio::test]
async fn reading_an_item_does_not_take_it_off_the_badge() {
    let state = state().await;
    record_notification(
        &state.pool,
        NotificationItem::new("r", NotificationKind::Approval, McpLogLevel::Error, "t").blocking(),
    )
    .await
    .expect("record");

    let (status, body) = read_response(
        mark_notification_read(State(state.clone()), headers(), Path("r".to_string())).await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["changed"], true);

    // Second read changes nothing, which is how a caller tells "already seen"
    // from "no such item" without another request.
    let (_, body) = read_response(
        mark_notification_read(State(state.clone()), headers(), Path("r".to_string())).await,
    )
    .await;
    assert_eq!(body["data"]["changed"], false);

    let summary = summary(&state).await;
    assert_eq!(
        summary["waiting"], 1,
        "seeing that you were asked is not answering: a badge that cleared on \
         sight would say an agent is free when it is still held"
    );
    assert_eq!(summary["blocking"], 1);
}

#[tokio::test]
async fn the_listing_opens_on_what_just_happened() {
    let state = state().await;
    for id in ["first", "second", "third"] {
        record_notification(
            &state.pool,
            NotificationItem::new(id, NotificationKind::Notice, McpLogLevel::Info, id),
        )
        .await
        .expect("record");
    }

    let items = list(&state, false).await;
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["item_id"], "third", "newest first");

    cloto_core::db::resolve_notification(&state.pool, "third", "done")
        .await
        .expect("resolve");
    let waiting = list(&state, true).await;
    assert_eq!(waiting.len(), 2);
    assert!(
        waiting.iter().all(|i| i["item_id"] != "third"),
        "a settled item leaves the waiting list"
    );
}

#[tokio::test]
async fn the_routes_refuse_a_caller_with_no_key() {
    // 403, not 401: the kernel's own answer for a request that carries no admin
    // key. Measured, not assumed — the first draft of this test asserted 401.
    let state = state().await;
    let (status, _) =
        read_response(notification_summary(State(state.clone()), HeaderMap::new()).await).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = read_response(
        list_notifications(
            State(state.clone()),
            HeaderMap::new(),
            Query(ListQuery {
                unresolved: None,
                limit: None,
            }),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[test]
fn the_routes_are_registered() {
    // The router is assembled inline during boot, so a running kernel cannot be
    // asked what it registered. Handler tests above would pass just as happily
    // for handlers nothing routes to.
    let wiring = include_str!("../src/lib.rs");
    assert!(
        wiring.contains("\"/notifications\""),
        "the listing route is not registered"
    );
    assert!(
        wiring.contains("\"/notifications/summary\""),
        "the summary route is not registered — the bell polls this one"
    );
    assert!(
        wiring.contains("\"/notifications/{item_id}/read\""),
        "the mark-read route is not registered"
    );
    assert!(
        wiring.contains("handlers::notifications::notification_summary"),
        "the registered summary route does not reach the handler"
    );
    assert!(
        wiring.contains("handlers::notifications::list_notifications"),
        "the registered listing route does not reach the handler"
    );
    assert!(
        wiring.contains("handlers::notifications::mark_notification_read"),
        "the registered mark-read route does not reach the handler"
    );
}
