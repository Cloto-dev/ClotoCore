//! The read surface over the notification store.
//!
//! Three routes, and the split between them is deliberate. The summary is what
//! a bell polls: two integers, cheap enough to ask for often. The listing is
//! what opening the bell costs. Marking one read is the only write a reader
//! makes, and it is not the same as answering — answering goes through whatever
//! gate raised the item.
//!
//! **No severity filter anywhere in here.** A threshold decides whether an item
//! interrupts the reader, never whether the reader can find out it exists.
//! Filtering on the server would put that distinction one refactor away from
//! collapsing, and the failure is silent: the agent waits, the badge stays at
//! zero, and nothing anywhere says why.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};

use super::{check_auth, ok_data};
use crate::{AppError, AppResult, AppState};

/// Most items one listing will return.
///
/// The bell shows the recent past, not an archive: a reader scrolling a
/// thousand rows in a popover is a reader who needed a different screen.
const MAX_LIMIT: i64 = 200;
const DEFAULT_LIMIT: i64 = 50;

#[derive(Debug, serde::Deserialize)]
pub struct ListQuery {
    /// Only items nobody has settled yet. Defaults to true — what is still
    /// waiting is the question the bell exists to answer.
    pub unresolved: Option<bool>,
    pub limit: Option<i64>,
}

/// GET /api/notifications — what is waiting, newest first.
pub async fn list_notifications(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let unresolved = query.unresolved.unwrap_or(true);

    let items = crate::db::list_notifications(&state.pool, limit, unresolved)
        .await
        .map_err(AppError::Internal)?;

    ok_data(serde_json::json!({ "items": items }))
}

/// GET /api/notifications/summary — the two numbers a bell renders.
pub async fn notification_summary(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let summary = crate::db::notification_summary(&state.pool)
        .await
        .map_err(AppError::Internal)?;

    ok_data(serde_json::json!({ "summary": summary }))
}

/// POST /api/notifications/{item_id}/read — this reader has seen it.
///
/// Returns whether the call changed anything, so a caller can tell "already
/// read" from "no such item" without a second request. Reading does not settle
/// the item and does not stop it blocking: those belong to whoever answers it.
pub async fn mark_notification_read(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(item_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let changed = crate::db::mark_notification_read(&state.pool, &item_id)
        .await
        .map_err(AppError::Internal)?;

    ok_data(serde_json::json!({ "item_id": item_id, "changed": changed }))
}

#[derive(Debug, serde::Deserialize)]
pub struct AnswerBody {
    /// What the operator said, in their own words. Stored verbatim: the agent
    /// that asked reads this back, and a normalized yes/no would throw away the
    /// part of the answer worth asking a person for.
    pub decision: String,
}

/// POST /api/notifications/{item_id}/answer — the operator replies.
///
/// Separate from `/read` because seeing a question is not answering it, and the
/// two have different consequences: reading moves nothing, answering settles the
/// item, takes it off the bell, and is what `mgp.operator.replies` hands back to
/// the agent that asked.
///
/// `changed: false` means the item was already settled. That is reported rather
/// than treated as an error, because two people answering the same question at
/// once is ordinary, and the first answer standing is the right outcome.
pub async fn answer_notification(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(item_id): Path<String>,
    Json(body): Json<AnswerBody>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let decision = body.decision.trim();
    if decision.is_empty() {
        return Err(AppError::Validation(
            "an answer with no words in it is not an answer".to_string(),
        ));
    }

    let changed = crate::db::resolve_notification(&state.pool, &item_id, decision)
        .await
        .map_err(AppError::Internal)?;

    ok_data(serde_json::json!({ "item_id": item_id, "changed": changed }))
}
