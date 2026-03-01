use axum::{extract::State, http::HeaderMap, Json};
use serde::Deserialize;
use std::sync::Arc;

use crate::{AppResult, AppState};

use super::{check_auth, spawn_admin_audit};

#[derive(Deserialize)]
pub struct PermissionDecisionPayload {}

/// Get pending permission requests awaiting human approval.
///
/// **Route:** `GET /api/permissions/pending`
///
/// # Authentication
/// No authentication required (read-only).
///
/// # Response
/// Returns a JSON array of `PermissionRequest` objects with status `"pending"`.
/// Used by the dashboard for Human-in-the-Loop permission management.
pub async fn get_pending_permissions(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<Vec<crate::PermissionRequest>>> {
    let requests = crate::get_pending_permission_requests(&state.pool).await?;
    Ok(Json(requests))
}

/// Approve a pending permission request.
///
/// **Route:** `POST /api/permissions/:request_id/approve`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Side Effects
/// - Updates request status to `"approved"` in database
/// - Writes audit log entry with actor and timestamp
///
/// # Response
/// - **200 OK:** `{ "status": "success", "message": "Permission request approved" }`
/// - **403 Forbidden:** Invalid or missing API key
pub async fn approve_permission(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(request_id): axum::extract::Path<String>,
    Json(_payload): Json<PermissionDecisionPayload>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    // Use fixed "admin" actor since auth is via single API key (not user-supplied value)
    let actor_id = "admin".to_string();
    crate::update_permission_request(&state.pool, &request_id, "approved", &actor_id).await?;

    spawn_admin_audit(
        state.pool.clone(),
        "PERMISSION_REQUEST_APPROVED",
        request_id.clone(),
        "Human administrator approved permission request".to_string(),
        None,
        None,
        None,
    );

    Ok(Json(serde_json::json!({
        "status": "success",
        "message": "Permission request approved"
    })))
}

/// Deny a pending permission request.
///
/// **Route:** `POST /api/permissions/:request_id/deny`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Side Effects
/// - Updates request status to `"denied"` in database
/// - Writes audit log entry with actor and timestamp
///
/// # Response
/// - **200 OK:** `{ "status": "success", "message": "Permission request denied" }`
/// - **403 Forbidden:** Invalid or missing API key
pub async fn deny_permission(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(request_id): axum::extract::Path<String>,
    Json(_payload): Json<PermissionDecisionPayload>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    // Use fixed "admin" actor since auth is via single API key (not user-supplied value)
    let actor_id = "admin".to_string();
    crate::update_permission_request(&state.pool, &request_id, "denied", &actor_id).await?;

    spawn_admin_audit(
        state.pool.clone(),
        "PERMISSION_REQUEST_DENIED",
        request_id.clone(),
        "Human administrator denied permission request".to_string(),
        None,
        None,
        None,
    );

    Ok(Json(serde_json::json!({
        "status": "success",
        "message": "Permission request denied"
    })))
}
