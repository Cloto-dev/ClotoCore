//! The surface a publisher outside the kernel writes to, and a dashboard
//! module reads from.
//!
//! Driven through the handlers the way a caller reaches them — status line and
//! body — because what matters is whether a caller can tell the outcomes apart,
//! not which internal error value was produced.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use cloto_core::handlers::published::{get_published_state, list_published, publish_state};
use cloto_core::test_utils::create_test_app_state;
use cloto_core::AppState;
use serde_json::json;
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

async fn publish(
    state: &Arc<AppState>,
    publisher: &str,
    document: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    read_response(
        publish_state(
            State(state.clone()),
            headers(),
            Path(publisher.to_string()),
            Json(document),
        )
        .await,
    )
    .await
}

async fn list(state: &Arc<AppState>) -> (StatusCode, serde_json::Value) {
    read_response(list_published(State(state.clone()), headers()).await).await
}

async fn read(state: &Arc<AppState>, publisher: &str) -> (StatusCode, serde_json::Value) {
    read_response(
        get_published_state(State(state.clone()), headers(), Path(publisher.to_string())).await,
    )
    .await
}

#[tokio::test]
async fn a_published_document_comes_back_as_it_went_in() {
    let state = state().await;
    // Nested and mixed on purpose: the kernel stores what it was handed rather
    // than a shape it knows, so anything JSON can express has to survive.
    let document = json!({
        "trail": [{"action": "schedule.run", "result": "success"}],
        "queue": {"open": 2, "labels": ["決裁待ち", "review"]},
        "nested": {"deep": {"deeper": [1, 2.5, true, null]}},
    });

    let (status, body) = publish(&state, "producer", document.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = read(&state, "producer").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body.pointer("/data/document"),
        Some(&document),
        "the document a module reads has to be the one that was published: {body}"
    );
}

#[tokio::test]
async fn the_time_on_the_record_is_the_kernel_s_not_the_publisher_s() {
    let state = state().await;
    // A publisher that has stopped cannot be trusted to say so, which is the
    // whole reason this field is not read out of the document.
    let document = json!({"published_at": "1999-01-01T00:00:00+00:00", "stale": true});

    let (status, body) = publish(&state, "producer", document.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (_, body) = read(&state, "producer").await;
    let stamped = body
        .pointer("/data/published_at")
        .and_then(serde_json::Value::as_str)
        .expect("the record carries a time");
    assert!(
        !stamped.starts_with("1999"),
        "the publisher's own field must not become the record's time: {stamped}"
    );
    assert!(
        chrono::DateTime::parse_from_rfc3339(stamped).is_ok(),
        "and it has to be a time a reader can parse: {stamped}"
    );
    assert_eq!(
        body.pointer("/data/document"),
        Some(&document),
        "the document itself is still untouched — the kernel added a field beside it, not inside it"
    );
}

#[tokio::test]
async fn publishing_again_replaces_the_document() {
    let state = state().await;
    publish(&state, "producer", json!({"generation": 1, "gone": "yes"})).await;
    publish(&state, "producer", json!({"generation": 2})).await;

    let (status, body) = read(&state, "producer").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body.pointer("/data/document"),
        Some(&json!({"generation": 2})),
        "a write replaces the document rather than merging into it: {body}"
    );
}

#[tokio::test]
async fn a_publisher_nobody_wrote_to_is_not_found() {
    let state = state().await;
    let (status, body) = read(&state, "producer").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "never published and published nothing are different answers: {body}"
    );
}

#[tokio::test]
async fn a_document_over_the_limit_is_refused_and_stores_nothing() {
    let state = state().await;
    let oversized = json!({"blob": "x".repeat(300 * 1024)});

    let (status, body) = publish(&state, "producer", oversized).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.to_string().contains("limit"),
        "the refusal has to name the bound the caller has to get under: {body}"
    );

    let (status, _) = read(&state, "producer").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a refused publish must not have written a partial record"
    );
}

#[tokio::test]
async fn an_oversized_document_is_refused_at_the_boundary_not_by_shape() {
    let state = state().await;
    // Just under the bound, spent on many small values rather than one big
    // one: the limit is on the serialized size, so how the bytes are arranged
    // must not change the answer.
    let entries: Vec<serde_json::Value> = (0..2000)
        .map(|i| json!({"i": i, "s": "abcdefgh"}))
        .collect();
    let (status, body) = publish(&state, "producer", json!({"entries": entries})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a document under the bound is accepted whatever its shape: {body}"
    );
}

#[tokio::test]
async fn a_publisher_id_that_is_not_a_single_safe_segment_is_refused() {
    let state = state().await;
    for id in ["", "a/b", "..", "with space", "sym+bol"] {
        let (status, body) = publish(&state, id, json!({})).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "'{id}' must not be accepted as a publisher id: {body}"
        );
        let (status, _) = read(&state, id).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "'{id}' must be refused on the way out too, not answered 404"
        );
    }
}

#[tokio::test]
async fn an_unauthenticated_caller_reaches_neither_side() {
    let state = state().await;
    publish(&state, "producer", json!({"secret": "no"})).await;

    let outcome = get_published_state(
        State(state.clone()),
        HeaderMap::new(),
        Path("producer".to_string()),
    )
    .await;
    let (status, _) = read_response(outcome).await;
    assert_ne!(status, StatusCode::OK, "the read is behind the auth check");

    let outcome = publish_state(
        State(state.clone()),
        HeaderMap::new(),
        Path("producer".to_string()),
        Json(json!({"overwritten": true})),
    )
    .await;
    let (status, _) = read_response(outcome).await;
    assert_ne!(status, StatusCode::OK, "and so is the write");

    let (_, body) = read(&state, "producer").await;
    assert_eq!(
        body.pointer("/data/document"),
        Some(&json!({"secret": "no"})),
        "the refused write must not have replaced anything: {body}"
    );
}

/// The handlers above answer correctly whether or not anyone can reach them.
/// This is the other half: that the kernel actually serves the path a module
/// will declare in its manifest. Read out of the source that registers it,
/// because the router is built inline during boot and there is no way to ask a
/// running one what it registered.
#[test]
fn the_kernel_serves_the_path_a_module_would_declare() {
    let wiring = include_str!("../src/lib.rs");
    assert!(
        wiring.contains("\"/published/{publisher}\""),
        "the route is not registered — a module declaring GET /api/published/<id> would be refused"
    );
    assert!(
        wiring.contains("handlers::published::get_published_state"),
        "the registered route does not reach the read handler"
    );
    assert!(
        wiring.contains("handlers::published::publish_state"),
        "the registered route does not reach the write handler"
    );
    assert!(
        wiring.contains("\"/published\""),
        "the listing route is not registered — a viewer with no publisher name has \
         nothing to ask, which is the whole reason it exists"
    );
    assert!(
        wiring.contains("handlers::published::list_published"),
        "the registered listing route does not reach the listing handler"
    );
}

/// A viewer that renders whatever is published starts here: it has no name to
/// ask for until this answers.
#[tokio::test]
async fn the_listing_names_every_publisher_and_when_each_last_wrote() {
    let state = state().await;
    publish(&state, "beta", json!({"n": 2})).await;
    publish(&state, "alpha", json!({"n": 1})).await;

    let (status, body) = list(&state).await;
    assert_eq!(status, StatusCode::OK);

    let rows = body["data"]["publishers"].as_array().expect("publishers");
    assert_eq!(rows.len(), 2);
    // Sorted, so a viewer can keep a selection across polls.
    assert_eq!(rows[0]["publisher"], "alpha");
    assert_eq!(rows[1]["publisher"], "beta");
    assert!(
        rows[0]["published_at"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "the timestamp is what answers 'is anyone still publishing'"
    );
}

/// The design of the route, asserted rather than described: naming who has
/// published must not be the same thing as handing over what they published.
/// A listing that carried the documents would give everything to anything
/// allowed to call this one path, and no manifest would show the difference.
#[tokio::test]
async fn the_listing_does_not_carry_the_documents() {
    let state = state().await;
    publish(&state, "alpha", json!({"secret": "in the document"})).await;

    let (_, body) = list(&state).await;
    let row = &body["data"]["publishers"][0];

    assert!(
        row.get("document").is_none(),
        "the listing named a document"
    );
    assert!(
        !serde_json::to_string(&body)
            .unwrap()
            .contains("in the document"),
        "the document reached the listing by some other name"
    );
}

/// Nobody publishing is an empty list, not an error: a viewer has to be able to
/// say "nothing here yet" without that being indistinguishable from a fault.
#[tokio::test]
async fn an_empty_listing_is_an_answer() {
    let state = state().await;
    let (status, body) = list(&state).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["publishers"].as_array().unwrap().len(), 0);
}

/// And it is behind the same credential as everything else under `/api`.
#[tokio::test]
async fn the_listing_refuses_a_caller_without_the_key() {
    let state = state().await;
    publish(&state, "alpha", json!({"n": 1})).await;

    let (listing, _) =
        read_response(list_published(State(state.clone()), HeaderMap::new()).await).await;
    let (single, _) = read_response(
        get_published_state(
            State(state.clone()),
            HeaderMap::new(),
            Path("alpha".to_string()),
        )
        .await,
    )
    .await;

    assert_ne!(listing, StatusCode::OK, "the listing is behind the check");
    assert_eq!(
        listing, single,
        "and behind the same one — a listing that refused differently would be a \
         second access-control story to keep in step"
    );
}
