use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use std::sync::Arc;

use super::{check_auth, ok_data};
use crate::handlers::command_approval::CommandApprovalDecision;
use crate::{AppError, AppResult, AppState};

/// POST /api/commands/:approval_id/approve
/// One-time allow: stores exact command match in DB for future sessions.
pub async fn approve_command(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(approval_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    // Extract command info before removing from pending map
    if let Some((_, sender)) = state.pending_command_approvals.remove(&approval_id) {
        // Store exact match in DB (the kernel gate extracted the command, but we need
        // the approval metadata — retrieve from a separate store or let the kernel do it).
        // For simplicity, the kernel stores the exact match after receiving Approve.
        let _ = sender.send(CommandApprovalDecision::Approve);
        ok_data(serde_json::json!({}))
    } else {
        Err(AppError::NotFound(format!(
            "Approval request '{}' not found or already resolved",
            approval_id
        )))
    }
}

/// POST /api/commands/:approval_id/trust
/// Trust the command name for this session (in-memory only).
pub async fn trust_command(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(approval_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    if let Some((_, sender)) = state.pending_command_approvals.remove(&approval_id) {
        let _ = sender.send(CommandApprovalDecision::Trust);
        ok_data(serde_json::json!({}))
    } else {
        Err(AppError::NotFound(format!(
            "Approval request '{}' not found or already resolved",
            approval_id
        )))
    }
}

/// POST /api/commands/:approval_id/deny
pub async fn deny_command(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(approval_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    if let Some((_, sender)) = state.pending_command_approvals.remove(&approval_id) {
        let _ = sender.send(CommandApprovalDecision::Deny);
        ok_data(serde_json::json!({}))
    } else {
        Err(AppError::NotFound(format!(
            "Approval request '{}' not found or already resolved",
            approval_id
        )))
    }
}
