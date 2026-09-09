//! State kept here by a publisher outside the kernel, for a dashboard module to
//! read.
//!
//! A runtime module (`handlers::modules`) runs in a frame with no origin of its
//! own, so it holds no credential and can only ask the host to proxy a `GET` it
//! declared in its manifest. That makes the kernel's read surface the only way
//! data reaches a module — and nothing produced outside the kernel was on it.
//! The alternative was a route per producer, which would put a product name in
//! a microkernel's public API (ARCHITECTURE §1.1, §1.2) and leave a route to
//! delete once that producer's own infrastructure is retired.
//!
//! So the kernel holds one document per publisher and hands it back. Three
//! decisions keep that from growing into a subsystem:
//!
//! * **The document is opaque.** The kernel does not model, validate or version
//!   what is in it beyond "it is JSON". Its shape is a contract between the
//!   publisher and the module that renders it; typing it here would move that
//!   contract into the kernel and make every field a producer adds a kernel
//!   change.
//! * **One document, replaced on write — not a log.** A history invites a
//!   retention policy, pagination and archival, none of which the kernel is the
//!   right owner of. Producers that keep a history already keep it themselves;
//!   what is missing is the current picture.
//! * **`published_at` is the kernel's clock**, not a field in the document. The
//!   question an operator actually asks of a screen like this is "is anyone
//!   still publishing" — and a producer that has stopped cannot be trusted to
//!   report that it has.
//!
//! Not broadcast on the event stream: a document is arbitrary content from
//! outside, the stream is replayed to every authenticated subscriber, and the
//! masking that protects plugin config (`handlers::mcp::update_plugin_config`)
//! is a key-name rule that means nothing against a free-form document.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use cloto_shared::PluginDataStore as _;

use super::{check_auth, ok_data};
use crate::{AppError, AppResult, AppState};

/// Namespace these documents occupy in the generic key/value store, alongside
/// the kernel's own (`managers::llm_proxy` keeps its record under one too). The
/// publisher id is the key.
const STORE_ID: &str = "cloto.published";

/// Largest document accepted, measured on its serialized form.
///
/// A screen's worth of state, with room to spare. The bound exists because the
/// store is the kernel's database and a publisher writing into it is not the
/// operator: without one, a producer with a loop in it grows that database
/// until something else fails.
const MAX_DOCUMENT_BYTES: usize = 256 * 1024;

/// Reject anything that cannot be a single, safe path segment.
///
/// The same rule module ids follow, for the same reason: the id appears in a
/// URL and is compared against what a module declared, and this set survives
/// both without encoding rules of its own.
fn is_valid_publisher(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// What is stored, and what a reader gets back.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PublishedState {
    /// RFC 3339, from the kernel's clock at the moment of the write.
    pub published_at: String,
    /// Whatever the publisher sent, unread.
    pub document: serde_json::Value,
}

/// POST /api/published/{publisher} — replace this publisher's document.
pub async fn publish_state(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(publisher): Path<String>,
    Json(document): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    if !is_valid_publisher(&publisher) {
        return Err(AppError::Validation(format!(
            "publisher '{publisher}' is not a valid id: letters, digits, '-' and '_', at most 64"
        )));
    }

    // Measured on the serialized form, which is what is stored — not on the
    // request body, which may be formatted any way at all.
    let size = serde_json::to_string(&document)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("could not serialize the document: {e}")))?
        .len();
    if size > MAX_DOCUMENT_BYTES {
        return Err(AppError::Validation(format!(
            "document is {size} bytes, over the {MAX_DOCUMENT_BYTES} byte limit"
        )));
    }

    let published_at = chrono::Utc::now().to_rfc3339();
    let record = PublishedState {
        published_at: published_at.clone(),
        document,
    };
    let value = serde_json::to_value(&record)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("could not serialize the record: {e}")))?;
    crate::db::SqliteDataStore::new(state.pool.clone())
        .set_json(STORE_ID, &publisher, value)
        .await
        .map_err(AppError::Internal)?;

    ok_data(serde_json::json!({ "publisher": publisher, "published_at": published_at }))
}

/// GET /api/published/{publisher} — read it back.
///
/// A publisher nobody has written to is a 404 rather than an empty document: a
/// module that cannot tell "never published" from "published nothing" would
/// render an organisation with nothing in it as if that were the news.
pub async fn get_published_state(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(publisher): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    if !is_valid_publisher(&publisher) {
        return Err(AppError::Validation(format!(
            "publisher '{publisher}' is not a valid id: letters, digits, '-' and '_', at most 64"
        )));
    }

    let stored = crate::db::SqliteDataStore::new(state.pool.clone())
        .get_json(STORE_ID, &publisher)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound(format!("nothing published under '{publisher}'")))?;

    // A row that will not parse is a fault on this side, not an absent
    // publisher: answering 404 would send the operator looking at the producer.
    let record: PublishedState = serde_json::from_value(stored).map_err(|e| {
        AppError::Internal(anyhow::anyhow!(
            "the stored document for '{publisher}' could not be read: {e}"
        ))
    })?;

    ok_data(serde_json::json!({
        "publisher": publisher,
        "published_at": record.published_at,
        "document": record.document,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publisher_ids_are_single_safe_segments() {
        assert!(is_valid_publisher("cil"));
        assert!(is_valid_publisher("a-b_C9"));
        assert!(!is_valid_publisher(""), "an empty id names nothing");
        assert!(
            !is_valid_publisher("a/b"),
            "a separator would leave the segment"
        );
        assert!(!is_valid_publisher(".."), "and so would a traversal");
        assert!(
            !is_valid_publisher(&"x".repeat(65)),
            "over the length bound"
        );
    }
}
