pub mod agents;
pub mod assets;
pub mod chat;
pub mod cron;
pub mod events;
pub mod llm;
pub mod mcp;
pub mod permissions;
pub mod system;

// Re-export all handler functions so that existing `handlers::*` paths in lib.rs continue to work.
pub use agents::{create_agent, delete_agent, get_agents, power_toggle, update_agent};
pub use chat::chat_handler;
pub use cron::{
    create_cron_job, delete_cron_job, list_cron_jobs, run_cron_job_now, toggle_cron_job,
};
pub use events::post_event_handler;
pub use llm::{delete_llm_provider_key, list_llm_providers, set_llm_provider_key};
pub use mcp::{
    apply_plugin_settings, create_mcp_server, delete_mcp_server, get_agent_access,
    get_mcp_server_access, get_mcp_server_settings, get_plugin_config, get_plugin_permissions,
    get_plugins, get_yolo_mode, grant_permission_handler, list_mcp_servers, put_mcp_server_access,
    restart_mcp_server, revoke_permission_handler, set_yolo_mode, start_mcp_server,
    stop_mcp_server, update_mcp_server_settings, update_plugin_config,
};
pub use permissions::{approve_permission, deny_permission, get_pending_permissions};

/// GET /api/system/version
/// Returns current Cloto version and build target (public, no auth).
pub async fn version_handler() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "build_target": env!("TARGET"),
    }))
}

/// GET /api/system/health — lightweight liveness check (no auth required)
pub async fn health_handler() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok"
    }))
}

use axum::{
    extract::State,
    http::HeaderMap,
    response::sse::{Event, Sse},
    Json,
};
use futures::stream::Stream;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tracing::{error, info};

use crate::{AppError, AppResult, AppState};

pub(crate) fn check_auth(state: &AppState, headers: &HeaderMap) -> AppResult<()> {
    use subtle::ConstantTimeEq;
    if let Some(ref required_key) = state.config.admin_api_key {
        let auth_header = headers.get("X-API-Key").and_then(|h| h.to_str().ok());

        let matches: bool = match auth_header {
            Some(provided) => provided.as_bytes().ct_eq(required_key.as_bytes()).into(),
            None => false,
        };
        if !matches {
            return Err(AppError::Cloto(cloto_shared::ClotoError::PermissionDenied(
                cloto_shared::Permission::AdminAccess,
            )));
        }
        // Check revocation: reject key even if it matches, if it has been invalidated
        if let Some(provided) = auth_header {
            let hash = crate::db::hash_api_key(provided);
            if let Ok(revoked) = state.revoked_keys.read() {
                if revoked.contains(&hash) {
                    tracing::warn!("🚫 Rejected revoked API key");
                    return Err(AppError::Cloto(cloto_shared::ClotoError::PermissionDenied(
                        cloto_shared::Permission::AdminAccess,
                    )));
                }
            }
        }
    } else {
        // In release builds, require API key to be configured
        if !cfg!(debug_assertions) {
            return Err(AppError::Cloto(cloto_shared::ClotoError::PermissionDenied(
                cloto_shared::Permission::AdminAccess,
            )));
        }
        // M-09: Warn loudly in debug builds when no API key is set
        tracing::warn!(
            "⚠️  SECURITY: Admin API access without authentication (debug mode, no CLOTO_API_KEY)"
        );
        tracing::warn!("⚠️  Set CLOTO_API_KEY in .env before deploying to production");
    }
    Ok(())
}

pub(crate) fn spawn_admin_audit(
    pool: sqlx::SqlitePool,
    event_type: &str,
    target_id: String,
    reason: String,
    permission: Option<String>,
    metadata: Option<serde_json::Value>,
    trace_id: Option<String>,
) {
    crate::db::spawn_audit_log(
        pool,
        crate::db::AuditLogEntry {
            timestamp: chrono::Utc::now(),
            event_type: event_type.to_string(),
            actor_id: Some("admin".to_string()),
            target_id: Some(target_id),
            permission,
            result: "SUCCESS".to_string(),
            reason,
            metadata,
            trace_id,
        },
    );
}

/// Initiate graceful system shutdown.
///
/// **Route:** `POST /api/system/shutdown`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Behavior
/// 1. Broadcasts `SystemNotification` shutdown message
/// 2. Creates `.maintenance` file (atomic write via tmp + rename)
/// 3. Signals shutdown after 1-second delay (allows response delivery)
///
/// Guardian process can detect `.maintenance` file and handle restart logic.
///
/// # Response
/// - **200 OK:** `{ "status": "shutting_down" }`
/// - **403 Forbidden:** Invalid or missing API key
pub async fn shutdown_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    info!("🛑 Shutdown requested. Broadcasting notification...");

    // Send system notification
    let envelope = crate::EnvelopedEvent::system(cloto_shared::ClotoEventData::SystemNotification(
        "Kernel is shutting down for maintenance...".to_string(),
    ));
    // H-04: Log send errors instead of silently ignoring
    if let Err(e) = state.event_tx.send(envelope).await {
        error!("Failed to send shutdown notification event: {}", e);
    }

    // Execute shutdown asynchronously (after returning response)
    let shutdown = state.shutdown.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;

        // 🚧 Signal maintenance mode (atomic write to prevent symlink attacks)
        let maint = crate::config::exe_dir().join(".maintenance");
        let suffix: u64 = rand::random();
        let maint_tmp = crate::config::exe_dir().join(format!(".maintenance_{:016x}.tmp", suffix));
        match std::fs::write(&maint_tmp, "active")
            .and_then(|()| std::fs::rename(&maint_tmp, &maint))
        {
            Ok(()) => info!("🚧 Maintenance mode engaged."),
            Err(e) => error!("❌ Failed to create .maintenance file: {}", e),
        }

        info!("👋 Kernel shutting down gracefully.");
        shutdown.notify_one();
    });

    Ok(Json(serde_json::json!({ "status": "shutting_down" })))
}

/// Server-Sent Events (SSE) stream for real-time event delivery.
///
/// **Route:** `GET /api/events/stream`
///
/// # Authentication
/// No authentication required (subscriber-only).
///
/// # Behavior
/// 1. Sends initial `handshake` event with data `"connected"`
/// 2. Streams all events from the broadcast channel as JSON
/// 3. Sends keep-alive every 15 seconds to prevent connection timeout
/// 4. Handles lag by warning and continuing (events may be dropped)
///
/// # Connection
/// Clients should use `EventSource` API or equivalent SSE client.
/// Connection closes when the broadcast channel is closed.
pub async fn sse_handler(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.tx.subscribe();
    let stream = async_stream::stream! {
        yield Ok(Event::default().event("handshake").data("connected"));
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Ok(json) = serde_json::to_string(&event) {
                        yield Ok(Event::default().data(json));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("SSE stream lagged by {} messages", n);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    };
    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// Get recent event history from the in-memory ring buffer.
///
/// **Route:** `GET /api/history`
///
/// # Authentication
/// No authentication required (read-only).
///
/// # Response
/// Returns a JSON array of recent events (most recent first),
/// limited by the configured `event_history_size`.
pub async fn get_history(State(state): State<Arc<AppState>>) -> AppResult<Json<serde_json::Value>> {
    let history = state.event_history.read().await;
    let history_vec: Vec<_> = history.iter().collect();
    Ok(Json(serde_json::json!(history_vec)))
}

/// Get system metrics and health information.
///
/// **Route:** `GET /api/metrics`
///
/// # Authentication
/// No authentication required (read-only).
///
/// # Response
/// ```json
/// {
///   "total_requests": 42,
///   "total_memories": 10,
///   "total_episodes": 5,
///   "event_history": { "current_size": 100, "max_size": 1000, "memory_estimate_bytes": 800 }
/// }
/// ```
pub async fn get_metrics(State(state): State<Arc<AppState>>) -> AppResult<Json<serde_json::Value>> {
    let history_len = state.event_history.read().await.len();
    let max_size = state.config.event_history_size;

    Ok(Json(serde_json::json!({
        "total_requests": state.metrics.total_requests.load(std::sync::atomic::Ordering::Relaxed),
        "total_memories": state.metrics.total_memories.load(std::sync::atomic::Ordering::Relaxed),
        "total_episodes": state.metrics.total_episodes.load(std::sync::atomic::Ordering::Relaxed),
        "ram_usage": "Unknown", // Future implementation
        "event_history": {
            "current_size": history_len,
            "max_size": max_size,
            "memory_estimate_bytes": history_len * std::mem::size_of::<std::sync::Arc<cloto_shared::ClotoEvent>>(),
        }
    })))
}

/// Get stored agent memories via KS22 MCP server.
///
/// **Route:** `GET /api/memories`
///
/// # Authentication
/// No authentication required (read-only).
///
/// # Response
/// Returns recent memories from KS22 memory server.
pub async fn get_memories(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<serde_json::Value>> {
    let args = serde_json::json!({ "agent_id": "", "limit": 100 });
    match state
        .mcp_manager
        .call_server_tool("memory.ks22", "list_memories", args)
        .await
    {
        Ok(result) => {
            if let Some(crate::managers::mcp_protocol::ToolContent::Text { text }) =
                result.content.first()
            {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(text) {
                    return Ok(Json(data));
                }
            }
            Ok(Json(serde_json::json!({ "memories": [], "count": 0 })))
        }
        Err(e) => {
            tracing::warn!("KS22 list_memories failed: {}", e);
            Ok(Json(serde_json::json!({ "memories": [], "count": 0 })))
        }
    }
}

/// Get archived episodes via KS22 MCP server.
///
/// **Route:** `GET /api/episodes`
///
/// # Response
/// Returns recent episodes from KS22 memory server.
pub async fn get_episodes(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<serde_json::Value>> {
    let args = serde_json::json!({ "agent_id": "", "limit": 50 });
    match state
        .mcp_manager
        .call_server_tool("memory.ks22", "list_episodes", args)
        .await
    {
        Ok(result) => {
            if let Some(crate::managers::mcp_protocol::ToolContent::Text { text }) =
                result.content.first()
            {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(text) {
                    return Ok(Json(data));
                }
            }
            Ok(Json(serde_json::json!({ "episodes": [], "count": 0 })))
        }
        Err(e) => {
            tracing::warn!("KS22 list_episodes failed: {}", e);
            Ok(Json(serde_json::json!({ "episodes": [], "count": 0 })))
        }
    }
}

// ============================================================
// API Key Invalidation
// ============================================================

pub async fn invalidate_api_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let provided_key = headers
        .get("X-API-Key")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| AppError::Validation("X-API-Key header required".to_string()))?;

    // Persist to DB
    crate::db::revoke_api_key(&state.pool, provided_key)
        .await
        .map_err(AppError::Internal)?;

    // Update in-memory cache
    let hash = crate::db::hash_api_key(provided_key);
    if let Ok(mut revoked) = state.revoked_keys.write() {
        revoked.insert(hash);
    }

    tracing::warn!("🔑 API key invalidated — system-wide access revoked for this key");

    Ok(Json(serde_json::json!({
        "status": "invalidated",
        "message": "API key has been revoked. All future requests with this key will be rejected. Restart with a new CLOTO_API_KEY to restore access."
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::create_test_app_state;
    use axum::http::HeaderValue;

    #[tokio::test]
    async fn test_check_auth_with_valid_api_key() {
        let state = create_test_app_state(Some("test-secret-key".to_string())).await;
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", HeaderValue::from_static("test-secret-key"));

        let result = check_auth(&state, &headers);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_check_auth_with_invalid_api_key() {
        let state = create_test_app_state(Some("test-secret-key".to_string())).await;
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", HeaderValue::from_static("wrong-key"));

        let result = check_auth(&state, &headers);
        assert!(result.is_err());

        if let Err(AppError::Cloto(cloto_shared::ClotoError::PermissionDenied(perm))) = result {
            assert_eq!(perm, cloto_shared::Permission::AdminAccess);
        } else {
            panic!("Expected PermissionDenied error");
        }
    }

    #[tokio::test]
    async fn test_check_auth_with_missing_header() {
        let state = create_test_app_state(Some("test-secret-key".to_string())).await;
        let headers = HeaderMap::new();

        let result = check_auth(&state, &headers);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_check_auth_no_key_configured_debug_mode() {
        // In debug mode (cfg!(debug_assertions) = true), no API key allows access
        let state = create_test_app_state(None).await;
        let headers = HeaderMap::new();

        let result = check_auth(&state, &headers);

        #[cfg(debug_assertions)]
        assert!(result.is_ok());

        #[cfg(not(debug_assertions))]
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_check_auth_empty_api_key_header() {
        let state = create_test_app_state(Some("test-secret-key".to_string())).await;
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", HeaderValue::from_static(""));

        let result = check_auth(&state, &headers);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_check_auth_case_sensitive() {
        let state = create_test_app_state(Some("test-secret-key".to_string())).await;
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", HeaderValue::from_static("TEST-SECRET-KEY"));

        let result = check_auth(&state, &headers);
        assert!(result.is_err(), "API key should be case-sensitive");
    }
}
