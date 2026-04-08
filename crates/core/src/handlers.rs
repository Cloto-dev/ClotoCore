//! HTTP API handler routing and sub-handler modules.
//!
//! Each sub-module corresponds to a REST API domain (agents, chat, MCP, etc.).
//! All handlers use the `ok_data()` / `json_data()` response helpers for
//! consistent envelope formatting.

pub mod agents;
pub mod assets;
pub mod chat;
pub(crate) mod command_approval;
pub mod commands;
pub mod cron;
pub(crate) mod engine_routing;
pub mod events;
pub mod health;
pub mod llm;
pub mod marketplace;
pub mod mcp;
pub mod permissions;
pub mod response;
pub mod setup;
pub mod system;
pub mod utils;

pub use response::{json_data, ok_data};

// Re-export all handler functions so that existing `handlers::*` paths in lib.rs continue to work.
pub use agents::{
    create_agent, delete_agent, delete_avatar, delete_vrm, generate_visemes, get_agents,
    get_avatar, get_vrm, power_toggle, serve_speech_file, update_agent, upload_avatar, upload_vrm,
};
pub use chat::chat_handler;
pub use commands::{approve_command, deny_command, trust_command};
pub use cron::{
    create_cron_job, delete_cron_job, list_cron_jobs, run_cron_job_now, toggle_cron_job,
};
pub use events::post_event_handler;
pub use llm::{delete_llm_provider_key, list_llm_providers, set_llm_provider_key};
pub use marketplace::{
    batch_install_handler, catalog_handler, install_handler, marketplace_progress_handler,
    uninstall_handler,
};
pub use mcp::{
    apply_plugin_settings, call_mcp_tool, create_mcp_server, delete_mcp_server, get_agent_access,
    get_max_cron_generation, get_mcp_server_access, get_mcp_server_settings, get_plugin_config,
    get_plugin_permissions, get_plugins, get_yolo_mode, grant_permission_handler, list_mcp_servers,
    put_mcp_server_access, restart_mcp_server, revoke_permission_handler, set_max_cron_generation,
    set_yolo_mode, start_mcp_server, stop_mcp_server, update_mcp_server_settings,
    update_plugin_config,
};
pub use permissions::{approve_permission, deny_permission, get_pending_permissions};

/// GET /api/system/version
/// Returns current Cloto version and build target (public, no auth).
pub async fn version_handler() -> axum::Json<serde_json::Value> {
    json_data(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "build_target": env!("TARGET"),
    }))
}

/// GET /api/system/health — lightweight liveness check (no auth required)
pub async fn health_handler() -> axum::Json<serde_json::Value> {
    json_data(serde_json::json!({
        "status": "ok"
    }))
}

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::sse::{Event, Sse},
    Json,
};
use futures::stream::Stream;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tracing::{error, info};

use crate::{AppError, AppResult, AppState};

/// Authenticate via `X-API-Key` header OR `?token=` query parameter.
/// SSE's `EventSource` API cannot set custom headers, so the dashboard
/// passes the API key as a query parameter instead (bug-157).
pub(crate) fn check_auth_with_query(
    state: &AppState,
    headers: &HeaderMap,
    query: &std::collections::HashMap<String, String>,
) -> AppResult<()> {
    // Try header first, fall back to query token
    let from_header = headers.get("X-API-Key").and_then(|h| h.to_str().ok());
    let from_query = query.get("token").map(String::as_str);
    let provided = from_header.or(from_query);

    if let Some(ref required_key) = state.config.admin_api_key {
        use subtle::ConstantTimeEq;
        let matches: bool = match provided {
            Some(p) => p.as_bytes().ct_eq(required_key.as_bytes()).into(),
            None => false,
        };
        if !matches {
            return Err(AppError::Cloto(cloto_shared::ClotoError::PermissionDenied(
                cloto_shared::Permission::AdminAccess,
            )));
        }
        if let Some(p) = provided {
            let hash = crate::db::hash_api_key(p);
            match state.revoked_keys.try_read() {
                Ok(revoked) => {
                    if revoked.contains(&hash) {
                        tracing::warn!("🚫 Rejected revoked API key");
                        return Err(AppError::Cloto(cloto_shared::ClotoError::PermissionDenied(
                            cloto_shared::Permission::AdminAccess,
                        )));
                    }
                }
                Err(_) => {
                    tracing::warn!(
                        "Failed to acquire revoked_keys lock — skipping revocation check"
                    );
                }
            }
        }
    } else {
        #[cfg(debug_assertions)]
        {
            let skip_auth = std::env::var("CLOTO_DEBUG_SKIP_AUTH")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            if !skip_auth {
                return Err(AppError::Cloto(cloto_shared::ClotoError::PermissionDenied(
                    cloto_shared::Permission::AdminAccess,
                )));
            }
            tracing::warn!(
                "⚠️  SECURITY: Admin API access without authentication (CLOTO_DEBUG_SKIP_AUTH=true)"
            );
            tracing::warn!("⚠️  Set CLOTO_API_KEY in .env before deploying to production");
        }
        #[cfg(not(debug_assertions))]
        {
            return Err(AppError::Cloto(cloto_shared::ClotoError::PermissionDenied(
                cloto_shared::Permission::AdminAccess,
            )));
        }
    }
    Ok(())
}

pub(crate) fn check_auth(state: &AppState, headers: &HeaderMap) -> AppResult<()> {
    check_auth_with_query(state, headers, &std::collections::HashMap::new())
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

    // P9: Drain all MCP servers before shutting down
    let mcp = state.mcp_manager.clone();
    let shutdown = state.shutdown.clone();
    let setup_flag = state.setup_in_progress.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Drain all MCP servers concurrently (5s per server, 10s global cap)
        let server_ids: Vec<String> = {
            let state = mcp.state.read().await;
            state.servers.keys().cloned().collect()
        };
        let drain_futures: Vec<_> = server_ids
            .iter()
            .map(|sid| {
                let mcp = mcp.clone();
                let sid = sid.clone();
                async move {
                    if let Err(e) = mcp.drain_server(&sid, "kernel shutdown", 5000).await {
                        tracing::warn!(server = %sid, error = %e, "MCP drain failed during shutdown");
                    }
                }
            })
            .collect();
        if tokio::time::timeout(
            Duration::from_secs(10),
            futures::future::join_all(drain_futures),
        )
        .await
        .is_err()
        {
            tracing::error!(
                "MCP shutdown drain timed out after 10s — {} server(s) may not have shut down cleanly",
                server_ids.len()
            );
        }

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

        // Reset setup_in_progress flag to prevent stale lock on restart (bug-366)
        setup_flag.store(false, std::sync::atomic::Ordering::SeqCst);

        info!("👋 Kernel shutting down gracefully.");
        shutdown.notify_waiters();
    });

    ok_data(serde_json::json!({}))
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
#[allow(clippy::implicit_hasher)]
pub async fn sse_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    check_auth_with_query(&state, &headers, &query)?;

    let last_event_id: Option<u64> = headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());

    // Subscribe BEFORE reading history (prevents gap between replay and live)
    let mut rx = state.tx.subscribe();
    let history = state.event_history.clone();

    let stream = async_stream::stream! {
        yield Ok(Event::default().event("handshake").data("connected"));

        // Replay missed events from history
        if let Some(last_id) = last_event_id {
            let guard = history.read().await;
            let replay: Vec<_> = guard.iter()
                .filter(|se| se.seq_id > last_id)
                .cloned()
                .collect();
            drop(guard);
            for se in replay {
                if let Ok(json) = serde_json::to_string(&*se.event) {
                    yield Ok(Event::default().id(se.seq_id.to_string()).data(json));
                }
            }
        }

        loop {
            match rx.recv().await {
                Ok(seq_event) => {
                    if let Ok(json) = serde_json::to_string(&*seq_event.event) {
                        yield Ok(Event::default()
                            .id(seq_event.seq_id.to_string())
                            .data(json));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("SSE stream lagged by {} messages", n);
                    yield Ok(Event::default().event("lagged").data(n.to_string()));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
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
pub async fn get_history(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let history = state.event_history.read().await;
    let history_vec: Vec<serde_json::Value> = history
        .iter()
        .map(|se| {
            let mut obj = serde_json::to_value(&*se.event).unwrap_or_default();
            if let serde_json::Value::Object(ref mut map) = obj {
                map.insert("seq_id".into(), serde_json::json!(se.seq_id));
            }
            obj
        })
        .collect();
    ok_data(serde_json::json!(history_vec))
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
pub async fn get_metrics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let history_len = state.event_history.read().await.len();
    let max_size = state.config.event_history_size;

    ok_data(serde_json::json!({
        "total_requests": state.metrics.total_requests.load(std::sync::atomic::Ordering::Relaxed),
        "total_memories": state.metrics.total_memories.load(std::sync::atomic::Ordering::Relaxed),
        "total_episodes": state.metrics.total_episodes.load(std::sync::atomic::Ordering::Relaxed),
        "ram_usage": "Unknown", // Future implementation
        "event_history": {
            "current_size": history_len,
            "max_size": max_size,
            "memory_estimate_bytes": history_len * std::mem::size_of::<crate::events::SequencedEvent>(),
        }
    }))
}

/// Parse the first text content from an MCP tool result as JSON.
fn parse_mcp_tool_result(
    result: &crate::managers::mcp_protocol::CallToolResult,
) -> Option<serde_json::Value> {
    if let Some(crate::managers::mcp_protocol::ToolContent::Text { text }) = result.content.first()
    {
        serde_json::from_str::<serde_json::Value>(text).ok()
    } else {
        None
    }
}

/// Call a memory MCP tool via capability dispatch, returning parsed JSON or a fallback on error.
async fn call_memory_tool_with_fallback(
    state: &AppState,
    tool: &str,
    args: serde_json::Value,
    fallback: serde_json::Value,
) -> AppResult<Json<serde_json::Value>> {
    match state
        .mcp_manager
        .call_capability_tool(crate::managers::CapabilityType::Memory, tool, args, None)
        .await
    {
        Ok(result) => ok_data(parse_mcp_tool_result(&result).unwrap_or(fallback)),
        Err(e) => {
            tracing::warn!("Memory tool {} failed: {}", tool, e);
            ok_data(fallback)
        }
    }
}

/// **Route:** `GET /api/memories`
///
/// Returns recent memories enriched with lock status and server capabilities.
pub async fn get_memories(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let args = serde_json::json!({ "agent_id": "", "limit": 100 });

    // Fetch memories from MCP server
    let mut data = match state
        .mcp_manager
        .call_capability_tool(
            crate::managers::CapabilityType::Memory,
            "list_memories",
            args,
            None,
        )
        .await
    {
        Ok(result) => parse_mcp_tool_result(&result)
            .unwrap_or(serde_json::json!({ "memories": [], "count": 0 })),
        Err(e) => {
            tracing::warn!("Memory tool list_memories failed: {}", e);
            serde_json::json!({ "memories": [], "count": 0 })
        }
    };

    // Enrich with kernel-level lock status for servers without native lock support
    if let Some(memories) = data.get_mut("memories").and_then(|m| m.as_array_mut()) {
        // Resolve memory server ID for kernel lock table queries
        let server_id = state
            .mcp_manager
            .resolve_capability_server(crate::managers::CapabilityType::Memory)
            .await
            .unwrap_or_default();

        let kernel_locks: std::collections::HashSet<i64> =
            sqlx::query_scalar("SELECT memory_id FROM memory_locks WHERE server_id = ?")
                .bind(&server_id)
                .fetch_all(&state.pool)
                .await
                .unwrap_or_default()
                .into_iter()
                .collect();

        for mem in memories.iter_mut() {
            let mem_id = mem
                .get("id")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(-1);
            if mem
                .get("locked")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                // Server-level lock
                if let Some(o) = mem.as_object_mut() {
                    o.insert("lock_level".into(), "server".into());
                }
            } else if kernel_locks.contains(&mem_id) {
                // Kernel fallback lock
                if let Some(o) = mem.as_object_mut() {
                    o.insert("locked".into(), true.into());
                    o.insert("lock_level".into(), "kernel".into());
                }
            }
        }
    }

    // Add capability detection for the dashboard
    let cap = crate::managers::CapabilityType::Memory;
    let capabilities = serde_json::json!({
        "update_memory": state.mcp_manager.has_capability_tool(cap, "update_memory").await,
        "lock_memory": state.mcp_manager.has_capability_tool(cap, "lock_memory").await,
        "unlock_memory": state.mcp_manager.has_capability_tool(cap, "unlock_memory").await,
    });
    data.as_object_mut()
        .map(|o| o.insert("capabilities".into(), capabilities));

    ok_data(data)
}

/// **Route:** `GET /api/episodes`
///
/// # Response
/// Returns recent episodes from CPersona memory server.
pub async fn get_episodes(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let args = serde_json::json!({ "agent_id": "", "limit": 50 });
    call_memory_tool_with_fallback(
        &state,
        "list_episodes",
        args,
        serde_json::json!({ "episodes": [], "count": 0 }),
    )
    .await
}

/// **Route:** `DELETE /api/memories/:id`
pub async fn delete_memory(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    // Check kernel-level lock before forwarding to server
    if is_kernel_locked(&state, id).await {
        return Err(AppError::Validation(
            "Memory is locked and cannot be deleted".into(),
        ));
    }

    let args = serde_json::json!({ "memory_id": id });
    let result = state
        .mcp_manager
        .call_capability_tool(
            crate::managers::CapabilityType::Memory,
            "delete_memory",
            args,
            None,
        )
        .await
        .map_err(AppError::Internal)?;
    ok_data(parse_mcp_tool_result(&result).unwrap_or(serde_json::json!({})))
}

/// **Route:** `DELETE /api/episodes/:id`
pub async fn delete_episode(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let args = serde_json::json!({ "episode_id": id });
    let result = state
        .mcp_manager
        .call_capability_tool(
            crate::managers::CapabilityType::Memory,
            "delete_episode",
            args,
            None,
        )
        .await
        .map_err(AppError::Internal)?;
    ok_data(parse_mcp_tool_result(&result).unwrap_or(serde_json::json!({})))
}

/// **Route:** `POST /api/memories/import`
///
/// Import memories from JSONL data via CPersona's `import_memories` tool.
/// Body: `{ "data": "...jsonl lines...", "agent_id": "optional-remap" }`
pub async fn import_memories(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let data = body
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Validation("missing 'data' field".into()))?;

    let target_agent_id = body.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");

    // Write JSONL to a temp file for the MCP tool
    let tmp_path = std::env::temp_dir()
        .join(format!("cloto-import-{}.jsonl", std::process::id()))
        .to_string_lossy()
        .to_string();

    std::fs::write(&tmp_path, data)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to write temp file: {}", e)))?;

    let args = serde_json::json!({
        "input_path": tmp_path,
        "target_agent_id": target_agent_id,
        "dry_run": false,
    });

    let result = state
        .mcp_manager
        .call_capability_tool(
            crate::managers::CapabilityType::Memory,
            "import_memories",
            args,
            None,
        )
        .await;

    // Clean up temp file regardless of result
    let _ = std::fs::remove_file(&tmp_path);

    let result = result.map_err(AppError::Internal)?;
    ok_data(parse_mcp_tool_result(&result).unwrap_or(serde_json::json!({"ok": false})))
}

/// Check if a memory is locked in the kernel fallback table.
async fn is_kernel_locked(state: &AppState, memory_id: i64) -> bool {
    sqlx::query_scalar::<_, i32>("SELECT 1 FROM memory_locks WHERE memory_id = ? LIMIT 1")
        .bind(memory_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
        .is_some()
}

/// **Route:** `PUT /api/memories/:id`
///
/// Update memory content. Requires the MCP server to support `update_memory`.
pub async fn update_memory(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    if is_kernel_locked(&state, id).await {
        return Err(AppError::Validation(
            "Memory is locked and cannot be edited".into(),
        ));
    }

    let content = body
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Validation("missing 'content' field".into()))?;

    if !state
        .mcp_manager
        .has_capability_tool(crate::managers::CapabilityType::Memory, "update_memory")
        .await
    {
        return Err(AppError::Validation(
            "Memory server does not support editing".into(),
        ));
    }

    let args = serde_json::json!({ "memory_id": id, "content": content });
    let result = state
        .mcp_manager
        .call_capability_tool(
            crate::managers::CapabilityType::Memory,
            "update_memory",
            args,
            None,
        )
        .await
        .map_err(AppError::Internal)?;
    ok_data(parse_mcp_tool_result(&result).unwrap_or(serde_json::json!({})))
}

/// **Route:** `POST /api/memories/:id/lock`
///
/// Lock a memory. Delegates to MCP server if supported, otherwise uses kernel fallback.
pub async fn lock_memory(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let cap = crate::managers::CapabilityType::Memory;
    if state
        .mcp_manager
        .has_capability_tool(cap, "lock_memory")
        .await
    {
        // Server-level lock
        let args = serde_json::json!({ "memory_id": id });
        let result = state
            .mcp_manager
            .call_capability_tool(cap, "lock_memory", args, None)
            .await
            .map_err(AppError::Internal)?;
        let mut data = parse_mcp_tool_result(&result).unwrap_or(serde_json::json!({}));
        data.as_object_mut()
            .map(|o| o.insert("lock_level".into(), "server".into()));
        ok_data(data)
    } else {
        // Kernel fallback lock
        let server_id = state
            .mcp_manager
            .resolve_capability_server(cap)
            .await
            .unwrap_or_default();

        sqlx::query("INSERT OR IGNORE INTO memory_locks (server_id, memory_id) VALUES (?, ?)")
            .bind(&server_id)
            .bind(id)
            .execute(&state.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to lock memory: {}", e)))?;

        ok_data(serde_json::json!({ "ok": true, "locked_id": id, "lock_level": "kernel" }))
    }
}

/// **Route:** `POST /api/memories/:id/unlock`
///
/// Unlock a memory. Delegates to MCP server if supported, otherwise removes kernel fallback.
pub async fn unlock_memory(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let cap = crate::managers::CapabilityType::Memory;
    if state
        .mcp_manager
        .has_capability_tool(cap, "unlock_memory")
        .await
    {
        // Server-level unlock
        let args = serde_json::json!({ "memory_id": id });
        let result = state
            .mcp_manager
            .call_capability_tool(cap, "unlock_memory", args, None)
            .await
            .map_err(AppError::Internal)?;
        let mut data = parse_mcp_tool_result(&result).unwrap_or(serde_json::json!({}));
        data.as_object_mut()
            .map(|o| o.insert("lock_level".into(), "server".into()));
        ok_data(data)
    } else {
        // Remove kernel fallback lock
        sqlx::query("DELETE FROM memory_locks WHERE memory_id = ?")
            .bind(id)
            .execute(&state.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to unlock memory: {}", e)))?;

        ok_data(serde_json::json!({ "ok": true, "unlocked_id": id, "lock_level": "kernel" }))
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
    {
        let mut revoked = state.revoked_keys.write().await;
        revoked.insert(hash);
    }

    tracing::warn!("🔑 API key invalidated — system-wide access revoked for this key");

    ok_data(serde_json::json!({
        "message": "API key has been revoked. All future requests with this key will be rejected. Restart with a new CLOTO_API_KEY to restore access."
    }))
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
    async fn test_check_auth_no_key_configured_requires_env_var() {
        // H-01: No API key configured requires CLOTO_DEBUG_SKIP_AUTH to skip auth
        let state = create_test_app_state(None).await;
        let headers = HeaderMap::new();

        // Without env var → denied
        std::env::remove_var("CLOTO_DEBUG_SKIP_AUTH");
        let result = check_auth(&state, &headers);
        assert!(result.is_err());

        // With CLOTO_DEBUG_SKIP_AUTH=1 → allowed
        std::env::set_var("CLOTO_DEBUG_SKIP_AUTH", "1");
        let result = check_auth(&state, &headers);
        assert!(result.is_ok());

        // Cleanup
        std::env::remove_var("CLOTO_DEBUG_SKIP_AUTH");
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
