//! The HTTP surface over the notification store.
//!
//! Three read routes, and the split between them is deliberate. The summary is
//! what a bell polls: two integers, cheap enough to ask for often. The listing
//! is what opening the bell costs. Marking one read is the only write a reader
//! makes, and it is not the same as answering — answering goes through whatever
//! gate raised the item.
//!
//! One write route for a producer that is not an agent: [`raise_notification`].
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
use crate::db::NotificationKind;
use crate::{AppError, AppResult, AppState};
use cloto_shared::McpLogLevel;

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

/// What an item raised from outside the kernel carries in front of the id its
/// producer chose.
///
/// Outside producers pick their own ids, so that a retry lands on the row it
/// already wrote. The prefix keeps those ids out of the kernel's own namespace:
/// an id the kernel is about to use, taken first from outside, would make the
/// kernel's own write collide — and an agent could end up waiting on an item
/// the reader was never shown.
pub const EXTERNAL_ITEM_PREFIX: &str = "external:";

/// Longest id an outside producer may choose: room for a job name, a date and
/// a separator or two, and no room to store a document in the key.
const MAX_ITEM_ID_CHARS: usize = 128;
/// Longest title. The title is the line a reader scans the list by; anything
/// that needs more words belongs in the body.
const MAX_TITLE_CHARS: usize = 200;
const MAX_BODY_CHARS: usize = 4_000;
/// Largest metadata object, measured as the JSON it is stored as.
const MAX_METADATA_BYTES: usize = 16 * 1024;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaiseBody {
    /// The producer's own id for this item. Sending the same id twice is one
    /// item, which is what makes a retry safe.
    pub item_id: String,
    /// `notice` (the default) or `proposal`. A notice informs and does not
    /// count on the badge; a proposal counts until the reader answers it.
    pub kind: Option<String>,
    /// An RFC 5424 level, as [`McpLogLevel`] spells it. Defaults to `notice`.
    pub severity: Option<String>,
    pub title: String,
    pub body: Option<String>,
    /// Opaque to the kernel, as metadata is everywhere else in the store.
    pub metadata: Option<serde_json::Value>,
}

/// POST /api/notifications — raise an item from outside the kernel.
///
/// For a producer that is not an agent: a supervisor that runs beside the
/// kernel and has to reach the reader when something it watches goes quiet. An
/// agent already has `mgp.operator.ask`; this is the same inbox for a caller
/// that is not one — and the only way in when the agents are what it is
/// reporting on, because a silent agent cannot be asked to say it is silent.
///
/// What an outside caller cannot do is settled by the shape of the request, not
/// by trusting the caller:
///
/// - **No `approval`.** An approval holds an agent until it is answered, and
///   only the gate that is holding the agent can raise one. From outside there
///   is no agent to hold, so the item would claim a wait that does not exist.
/// - **No `metadata.message`.** A keyed message is how the kernel says one of
///   its own sentences in the reader's language. Accepting one from outside
///   would let a caller speak with the kernel's voice. An outside item says
///   what its title and body say, in the words its producer chose.
/// - **Its ids live under [`EXTERNAL_ITEM_PREFIX`].**
/// - **Unknown fields are refused**, rather than dropped, so a caller that sends
///   `blocking` learns it was not honoured instead of believing it was.
///
/// Idempotent on the id: a retry after a lost response writes nothing and says
/// so with `created: false`, and the first write stands.
pub async fn raise_notification(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<RaiseBody>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let item = external_item(body)?;
    let item_id = item.item_id.clone();
    let created = crate::db::record_notification_once(&state.pool, item)
        .await
        .map_err(AppError::Internal)?;

    ok_data(serde_json::json!({ "item_id": item_id, "created": created }))
}

/// The row an outside request becomes, or the reason it cannot become one.
fn external_item(body: RaiseBody) -> Result<crate::db::NotificationItem, AppError> {
    let id = body.item_id.trim();
    let id_ok = !id.is_empty()
        && id.chars().count() <= MAX_ITEM_ID_CHARS
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'));
    if !id_ok {
        return Err(invalid(format!(
            "item_id must be 1 to {MAX_ITEM_ID_CHARS} characters of ASCII letters, digits, \
             '.', '_', ':' or '-'"
        )));
    }

    let kind = match body.kind.as_deref() {
        None => NotificationKind::Notice,
        Some(raw) => match serde_json::from_value(serde_json::Value::String(raw.to_string())) {
            Ok(NotificationKind::Approval) => {
                return Err(invalid(
                    "an approval holds an agent until it is answered, and only the gate \
                     holding that agent can raise one",
                ))
            }
            Ok(kind) => kind,
            Err(_) => return Err(invalid(format!("{raw:?} is not a kind of item"))),
        },
    };

    let severity = match body.severity.as_deref() {
        None => McpLogLevel::Notice,
        Some(raw) => serde_json::from_value(serde_json::Value::String(raw.to_string()))
            .map_err(|_| invalid(format!("{raw:?} is not an RFC 5424 level")))?,
    };

    let title = body.title.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
        return Err(invalid(format!(
            "title must be 1 to {MAX_TITLE_CHARS} characters"
        )));
    }

    let mut item = crate::db::NotificationItem::new(
        format!("{EXTERNAL_ITEM_PREFIX}{id}"),
        kind,
        severity,
        title,
    );

    if let Some(text) = body.body.as_deref().filter(|b| !b.trim().is_empty()) {
        if text.chars().count() > MAX_BODY_CHARS {
            return Err(invalid(format!(
                "body must be at most {MAX_BODY_CHARS} characters"
            )));
        }
        item = item.body(text);
    }

    if let Some(metadata) = body.metadata {
        let Some(fields) = metadata.as_object() else {
            return Err(invalid("metadata must be a JSON object"));
        };
        if fields.contains_key(crate::db::MESSAGE_KEY) {
            return Err(invalid(format!(
                "metadata.{} is how the kernel says its own sentences, and an item raised \
                 from outside says what its title and body say",
                crate::db::MESSAGE_KEY
            )));
        }
        if metadata.to_string().len() > MAX_METADATA_BYTES {
            return Err(invalid(format!(
                "metadata must be at most {MAX_METADATA_BYTES} bytes as JSON"
            )));
        }
        item = item.metadata(metadata);
    }

    Ok(item)
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::Validation(message.into())
}
