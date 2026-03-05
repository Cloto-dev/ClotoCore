use axum::{extract::State, http::HeaderMap, Json};
use std::sync::Arc;
use tracing::error;

use crate::{AppError, AppResult, AppState};

use super::{check_auth, ok_data};

/// Inject an event into the event bus from external sources.
///
/// **Route:** `POST /api/events`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Security
/// Only the following event types are allowed from external sources:
/// - `MessageReceived` - Chat messages
/// - `VisionUpdated` - Vision data updates
/// - `GazeUpdated` - Gaze tracking data
///
/// All other event types are rejected with 403 to prevent
/// injection of system-critical events.
///
/// # Response
/// - **200 OK:** `{ "status": "published" }`
/// - **403 Forbidden:** Invalid API key or restricted event type
/// - **500 Internal Server Error:** Event bus send failure
pub async fn post_event_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(event_data): Json<cloto_shared::ClotoEventData>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    // 🛡️ Security Check: 外部からの重要なシステムイベントの注入を禁止
    match &event_data {
        // H-15: Only allow safe event types from external sources
        // SystemNotification removed - external callers should not inject system notifications
        cloto_shared::ClotoEventData::MessageReceived(_)
        | cloto_shared::ClotoEventData::VisionUpdated { .. }
        | cloto_shared::ClotoEventData::GazeUpdated(_) => {
            // これらは許可
        }
        _ => {
            error!(
                "🚫 SECURITY ALERT: External attempt to inject restricted event: {:?}",
                event_data
            );
            return Err(AppError::Cloto(cloto_shared::ClotoError::PermissionDenied(
                cloto_shared::Permission::AdminAccess,
            )));
        }
    }

    let envelope = crate::EnvelopedEvent::system(event_data);
    if let Err(e) = state.event_tx.send(envelope).await {
        error!("Failed to send external event: {}", e);
        return Err(AppError::Internal(anyhow::anyhow!(
            "Failed to publish event"
        )));
    }
    ok_data(serde_json::json!({}))
}
