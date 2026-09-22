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
    answer_notification, list_notifications, mark_notification_read, notification_summary,
    raise_notification, AnswerBody, ListQuery, RaiseBody, EXTERNAL_ITEM_PREFIX,
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
    let wiring = include_str!("../../src/lib.rs");
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
    assert!(
        wiring.contains(".post(handlers::notifications::raise_notification)"),
        "nothing routes a POST to the raise handler — a producer outside the kernel has no way in"
    );
}

// ---------------------------------------------------------------------------
// Raising an item from outside the kernel.
//
// The request is built by deserializing JSON, the way the route receives it,
// so the refusals that live in the body's shape (unknown fields) are exercised
// along with the ones that live in the handler.

async fn raise(
    state: &Arc<AppState>,
    request: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let body: RaiseBody = serde_json::from_value(request).expect("a well-formed request");
    read_response(raise_notification(State(state.clone()), headers(), Json(body)).await).await
}

async fn stored(state: &Arc<AppState>) -> Vec<serde_json::Value> {
    list(state, false).await
}

#[tokio::test]
async fn a_raised_notice_arrives_as_nobodys_notice_and_leaves_the_badge_alone() {
    let state = state().await;
    let (status, body) = raise(
        &state,
        serde_json::json!({
            "item_id": "watch:2026-09-23:daily-review",
            "title": "The daily review left no record yesterday",
            "body": "Expected at 09:00; nothing ran and nothing declared a rest.",
            "severity": "warning",
            "metadata": { "job": "daily-review" },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["created"], true);
    assert_eq!(
        body["data"]["item_id"], "external:watch:2026-09-23:daily-review",
        "the answer has to name the row the caller will find"
    );

    let items = stored(&state).await;
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item["kind"], "notice", "notice is the default");
    assert_eq!(item["severity"], "warning");
    assert_eq!(
        item["agent_id"],
        serde_json::Value::Null,
        "no agent raised this, and filing it under one would make it that agent's words"
    );
    assert_eq!(item["blocking"], false);
    assert_eq!(item["metadata"]["job"], "daily-review");

    assert_eq!(
        summary(&state).await["waiting"],
        0,
        "a notice from outside is news like any other notice"
    );
}

#[tokio::test]
async fn a_raised_proposal_counts_until_the_reader_answers_it() {
    let state = state().await;
    let (status, body) = raise(
        &state,
        serde_json::json!({ "item_id": "silence-1", "kind": "proposal", "title": "A job went quiet" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        summary(&state).await["waiting"],
        1,
        "this is the one kind an outside producer can use to interrupt, and it has to"
    );

    let id = body["data"]["item_id"]
        .as_str()
        .expect("item id")
        .to_string();
    let (status, _) = read_response(
        answer_notification(
            State(state.clone()),
            headers(),
            Path(id),
            Json(AnswerBody {
                decision: "looked into it".into(),
            }),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        summary(&state).await["waiting"],
        0,
        "answering settles an outside proposal the same way it settles an agent's"
    );
}

#[tokio::test]
async fn raising_the_same_id_twice_is_one_item_and_the_first_write_stands() {
    let state = state().await;
    let request = |title: &str| serde_json::json!({ "item_id": "retry-me", "title": title });

    let (_, first) = raise(&state, request("first")).await;
    let (status, second) = raise(&state, request("second")).await;
    assert_eq!(status, StatusCode::OK, "a retry is not an error: {second}");
    assert_eq!(first["data"]["created"], true);
    assert_eq!(
        second["data"]["created"], false,
        "a retry after a lost response must be able to tell that it already landed"
    );

    let items = stored(&state).await;
    assert_eq!(
        items.len(),
        1,
        "a retry must not raise the same thing twice"
    );
    assert_eq!(items[0]["title"], "first");
}

#[tokio::test]
async fn an_outside_id_cannot_take_a_row_the_kernel_is_about_to_write() {
    let state = state().await;
    let (status, _) = raise(
        &state,
        serde_json::json!({ "item_id": "cmd-42", "title": "t" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The kernel's own producers write with a plain INSERT. If the outside
    // caller had been given the bare id, this would fail on the unique key and
    // the kernel's item — possibly an approval an agent is waiting behind —
    // would never reach the reader.
    record_notification(
        &state.pool,
        NotificationItem::new(
            "cmd-42",
            NotificationKind::Approval,
            McpLogLevel::Warning,
            "gate",
        )
        .blocking(),
    )
    .await
    .expect("the kernel's own write must still land");

    let ids: Vec<_> = stored(&state)
        .await
        .into_iter()
        .map(|i| i["item_id"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(ids.contains(&"cmd-42".to_string()), "{ids:?}");
    assert!(
        ids.contains(&format!("{EXTERNAL_ITEM_PREFIX}cmd-42")),
        "{ids:?}"
    );
}

#[tokio::test]
async fn what_an_outside_caller_cannot_raise_is_refused_and_leaves_nothing_behind() {
    let state = state().await;
    let long_title = "x".repeat(201);
    let long_body = "y".repeat(4_001);
    let big_metadata = serde_json::json!({ "blob": "z".repeat(16 * 1024) });
    for (why, request) in [
        (
            "an approval claims an agent is being held, and there is no agent",
            serde_json::json!({ "item_id": "a", "kind": "approval", "title": "t" }),
        ),
        (
            "a kind the store does not know",
            serde_json::json!({ "item_id": "a", "kind": "alarm", "title": "t" }),
        ),
        (
            "a keyed message would speak with the kernel's voice",
            serde_json::json!({
                "item_id": "a", "title": "t",
                "metadata": { "message": { "key": "kernel.shutdown", "params": {} } },
            }),
        ),
        (
            "metadata that is not an object",
            serde_json::json!({ "item_id": "a", "title": "t", "metadata": ["x"] }),
        ),
        (
            "metadata too large to be metadata",
            serde_json::json!({ "item_id": "a", "title": "t", "metadata": big_metadata }),
        ),
        (
            "a severity outside RFC 5424",
            serde_json::json!({ "item_id": "a", "title": "t", "severity": "urgent" }),
        ),
        (
            "an empty title",
            serde_json::json!({ "item_id": "a", "title": "   " }),
        ),
        (
            "a title past the limit",
            serde_json::json!({ "item_id": "a", "title": long_title }),
        ),
        (
            "a body past the limit",
            serde_json::json!({ "item_id": "a", "title": "t", "body": long_body }),
        ),
        (
            "an empty id",
            serde_json::json!({ "item_id": " ", "title": "t" }),
        ),
        (
            "an id with a character outside the set",
            serde_json::json!({ "item_id": "a/b", "title": "t" }),
        ),
        (
            "an id past the limit",
            serde_json::json!({ "item_id": "i".repeat(129), "title": "t" }),
        ),
    ] {
        let (status, body) = raise(&state, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {body}");
    }
    assert!(
        stored(&state).await.is_empty(),
        "a refused request must not have written anything first"
    );
}

#[tokio::test]
async fn the_limits_are_inclusive() {
    // The other side of the boundary the refusals above stand on: a value at
    // the limit is accepted, so a limit that drifted by one would show here.
    let state = state().await;
    let (status, body) = raise(
        &state,
        serde_json::json!({
            "item_id": "i".repeat(128),
            "title": "x".repeat(200),
            "body": "y".repeat(4_000),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[test]
fn a_field_the_route_does_not_honour_is_refused_rather_than_dropped() {
    // A caller sending `blocking` or `agent_id` must learn it was not honoured.
    // Silently dropping it would let an outside item look, to its producer,
    // like it holds an agent or speaks for one.
    for field in ["blocking", "agent_id"] {
        let mut request = serde_json::json!({ "item_id": "a", "title": "t" });
        request[field] = serde_json::json!(true);
        assert!(
            serde_json::from_value::<RaiseBody>(request).is_err(),
            "{field} was accepted"
        );
    }
}

#[tokio::test]
async fn raising_needs_the_key() {
    let state = state().await;
    let body: RaiseBody =
        serde_json::from_value(serde_json::json!({ "item_id": "a", "title": "t" })).unwrap();
    let (status, _) =
        read_response(raise_notification(State(state.clone()), HeaderMap::new(), Json(body)).await)
            .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(stored(&state).await.is_empty());
}
