pub mod system;
pub mod assets;
pub mod update;
pub mod chat;
pub mod evolution;
pub mod skill_manager;

use axum::{
    extract::{Path, State},
    response::sse::{Event, Sse},
    Json,
    http::HeaderMap,
};
use futures::stream::Stream;
use serde::Deserialize;
use std::{collections::HashMap, convert::Infallible, sync::Arc, time::Duration};
use tracing::{info, error};
use exiv_shared::ExivMessage;

use crate::{AppState, AppResult, AppError};

pub(crate) fn check_auth(state: &AppState, headers: &HeaderMap) -> AppResult<()> {
    use subtle::ConstantTimeEq;
    if let Some(ref required_key) = state.config.admin_api_key {
        let auth_header = headers.get("X-API-Key")
            .and_then(|h| h.to_str().ok());

        let matches: bool = match auth_header {
            Some(provided) => provided.as_bytes().ct_eq(required_key.as_bytes()).into(),
            None => false,
        };
        if !matches {
            return Err(AppError::Exiv(exiv_shared::ExivError::PermissionDenied(
                exiv_shared::Permission::AdminAccess
            )));
        }
    } else {
        // In release builds, require API key to be configured
        if !cfg!(debug_assertions) {
            return Err(AppError::Exiv(exiv_shared::ExivError::PermissionDenied(
                exiv_shared::Permission::AdminAccess
            )));
        }
        // M-09: Warn loudly in debug builds when no API key is set
        tracing::warn!("⚠️  SECURITY: Admin API access without authentication (debug mode, no EXIV_API_KEY)");
        tracing::warn!("⚠️  Set EXIV_API_KEY in .env before deploying to production");
    }
    Ok(())
}

fn spawn_admin_audit(
    pool: sqlx::SqlitePool,
    event_type: &str,
    target_id: String,
    reason: String,
    permission: Option<String>,
    metadata: Option<serde_json::Value>,
    trace_id: Option<String>,
) {
    crate::db::spawn_audit_log(pool, crate::db::AuditLogEntry {
        timestamp: chrono::Utc::now(),
        event_type: event_type.to_string(),
        actor_id: Some("admin".to_string()),
        target_id: Some(target_id),
        permission,
        result: "SUCCESS".to_string(),
        reason,
        metadata,
        trace_id,
    });
}

#[derive(Debug, Deserialize)]
pub struct PluginToggleRequest {
    pub id: String,
    pub is_active: bool,
}

#[derive(Deserialize)]
pub struct CreateAgentRequest {
    pub name: String,
    pub description: String,
    pub default_engine: String,
    pub metadata: Option<HashMap<String, String>>,
    pub required_capabilities: Option<Vec<exiv_shared::CapabilityType>>,
    pub password: Option<String>,
}

#[derive(Deserialize)]
pub struct PowerToggleRequest {
    pub enabled: bool,
    pub password: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateConfigPayload {
    pub key: String,
    pub value: String,
}

#[derive(Deserialize)]
pub struct UpdateAgentRequest {
    pub default_engine_id: Option<String>,
    pub metadata: HashMap<String, String>,
}

/// List all registered agents.
///
/// **Route:** `GET /api/agents`
///
/// # Authentication
/// No authentication required (read-only).
///
/// # Response
/// Returns a JSON array of all agents with their metadata, configured engine,
/// and capabilities.
///
/// **200 OK:**
/// ```json
/// [{ "id": "agent-1", "name": "Assistant", "description": "...", "default_engine": "..." }]
/// ```
pub async fn get_agents(State(state): State<Arc<AppState>>) -> AppResult<Json<serde_json::Value>> {
    let agents = state.agent_manager.list_agents().await?;
    Ok(Json(serde_json::json!(agents)))
}

/// Create a new agent with the specified configuration.
///
/// **Route:** `POST /api/agents`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Request Body
/// ```json
/// {
///   "name": "My Agent",
///   "description": "A helpful assistant",
///   "default_engine": "engine-id",
///   "metadata": { "key": "value" },
///   "required_capabilities": ["Reasoning", "Memory"]
/// }
/// ```
///
/// # Validation Rules
/// - **name**: Required, 1-200 characters (UTF-8 byte length)
/// - **description**: Required, 1-1000 characters (UTF-8 byte length)
/// - **default_engine**: Required, must reference a valid engine ID
/// - **metadata**: Optional key-value pairs
/// - **required_capabilities**: Optional, defaults to `[Reasoning, Memory]`
///
/// # Response
/// - **200 OK:** `{ "status": "success", "id": "<generated-agent-id>" }`
/// - **400 Bad Request:** Validation error (name/description length)
/// - **403 Forbidden:** Invalid or missing API key
///
/// # Errors
/// Returns [`AppError`] if validation or database operation fails.
pub async fn create_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateAgentRequest>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    // M-07: Input validation
    if payload.name.is_empty() || payload.name.len() > 200 {
        return Err(AppError::Exiv(exiv_shared::ExivError::ValidationError(
            format!("Agent name must be 1-200 characters (got {} chars); example: \"my-agent\"",
                payload.name.len()),
        )));
    }
    // Bug #1: Add empty check for description to match name validation pattern
    if payload.description.is_empty() || payload.description.len() > 1000 {
        return Err(AppError::Exiv(exiv_shared::ExivError::ValidationError(
            format!("Agent description must be 1-1000 characters (got {} chars); example: \"A helpful assistant\"",
                payload.description.len()),
        )));
    }

    // H-04: Metadata size validation
    let metadata = payload.metadata.unwrap_or_default();
    if metadata.len() > 50 {
        return Err(AppError::Exiv(exiv_shared::ExivError::ValidationError(
            format!("Metadata must have at most 50 key-value pairs (got {})",
                metadata.len()),
        )));
    }
    for (k, v) in &metadata {
        if k.len() > 200 || v.len() > 5000 {
            return Err(AppError::Exiv(exiv_shared::ExivError::ValidationError(
                format!("Metadata key '{}' exceeds limits (key: {} chars max 200, value: {} chars max 5000)",
                    k, k.len(), v.len()),
            )));
        }
    }

    let agent_id = state
        .agent_manager
        .create_agent(
            &payload.name,
            &payload.description,
            &payload.default_engine,
            metadata,
            payload.required_capabilities.unwrap_or_else(|| vec![
                exiv_shared::CapabilityType::Reasoning,
                exiv_shared::CapabilityType::Memory
            ]),
            payload.password.as_deref(),
        )
        .await?;
    Ok(Json(serde_json::json!({ "status": "success", "id": agent_id })))
}

/// Update an existing agent's settings.
///
/// **Route:** `PUT /api/agents/:id`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Path Parameters
/// - **id**: Agent ID to update
///
/// # Request Body
/// ```json
/// {
///   "default_engine_id": "new-engine-id",
///   "metadata": { "key": "updated-value" }
/// }
/// ```
///
/// # Response
/// - **200 OK:** `{ "status": "success" }`
/// - **403 Forbidden:** Invalid or missing API key
/// - **404 Not Found:** Agent ID does not exist
pub async fn update_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<UpdateAgentRequest>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    state.agent_manager.update_agent_config(&id, payload.default_engine_id, payload.metadata).await?;
    Ok(Json(serde_json::json!({ "status": "success" })))
}

/// Toggle agent power state (enable/disable).
///
/// **Route:** `POST /api/agents/:id/power`
///
/// If the agent has a power password set, the `password` field is required.
pub async fn power_toggle(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<PowerToggleRequest>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    // Check if agent has a password
    let password_hash = state.agent_manager.get_password_hash(&id).await?;
    if let Some(ref hash) = password_hash {
        match &payload.password {
            Some(pw) => {
                if !crate::managers::AgentManager::verify_password(pw, hash)? {
                    return Err(AppError::Exiv(exiv_shared::ExivError::PermissionDenied(
                        exiv_shared::Permission::AdminAccess
                    )));
                }
            }
            None => {
                return Err(AppError::Exiv(exiv_shared::ExivError::ValidationError(
                    "Password required for this agent's power control".to_string()
                )));
            }
        }
    }

    state.agent_manager.set_enabled(&id, payload.enabled).await?;

    // Broadcast power change event via EventBus
    let envelope = crate::EnvelopedEvent::system(
        exiv_shared::ExivEventData::AgentPowerChanged {
            agent_id: id.clone(),
            enabled: payload.enabled,
        }
    );
    if let Err(e) = state.event_tx.send(envelope).await {
        error!("Failed to send power change event: {}", e);
    }

    spawn_admin_audit(
        state.pool.clone(),
        if payload.enabled { "AGENT_POWER_ON" } else { "AGENT_POWER_OFF" },
        id.clone(),
        format!("Agent {} powered {}", id, if payload.enabled { "on" } else { "off" }),
        None, None, None,
    );

    Ok(Json(serde_json::json!({
        "status": "success",
        "enabled": payload.enabled
    })))
}

/// List all registered plugins with their current settings.
///
/// **Route:** `GET /api/plugins`
///
/// # Authentication
/// No authentication required (read-only).
///
/// # Response
/// Returns a JSON array of plugin manifests merged with database settings
/// (enabled/disabled state, configuration overrides).
///
/// Each entry includes: `id`, `name`, `description`, `version`, `category`,
/// `tags`, `capabilities`, `is_active`, and `provided_tools`.
pub async fn get_plugins(State(state): State<Arc<AppState>>) -> AppResult<Json<serde_json::Value>> {
    let manifests = state.plugin_manager.list_plugins_with_settings(&state.registry).await?;
    Ok(Json(serde_json::json!(manifests)))
}

/// Get plugin configuration values.
///
/// **Route:** `GET /api/plugins/:id/config`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
/// Config may contain sensitive values (API keys, tokens).
///
/// # Response
/// - **200 OK:** JSON object of key-value configuration pairs
/// - **403 Forbidden:** Invalid or missing API key
pub async fn get_plugin_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let config = state.plugin_manager.get_config(&id).await?;
    Ok(Json(serde_json::json!(config)))
}

/// Update a single plugin configuration key-value pair.
///
/// **Route:** `PUT /api/plugins/:id/config`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Request Body
/// ```json
/// { "key": "api_key", "value": "your-api-key" }
/// ```
///
/// # Side Effects
/// - Broadcasts `ConfigUpdated` event to all subscribers
/// - Writes audit log entry with actor, target, and trace ID
///
/// # Response
/// - **200 OK:** `{ "status": "success" }`
/// - **403 Forbidden:** Invalid or missing API key
pub async fn update_plugin_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<UpdateConfigPayload>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    state
        .plugin_manager
        .update_config(&id, &payload.key, &payload.value)
        .await?;

    info!(plugin_id = %id, key = %payload.key, "⚙️ Config updated for plugin. Broadcasting update...");

    // Get latest settings and notify
    if let Ok(full_config) = state.plugin_manager.get_config(&id).await {
        let envelope = crate::EnvelopedEvent::system(exiv_shared::ExivEventData::ConfigUpdated {
            plugin_id: id.clone(),
            config: full_config,
        });
        let event = envelope.event.clone();
        // H-04: Log send errors instead of silently ignoring
        if let Err(e) = state.event_tx.send(envelope).await {
            error!("Failed to send config update event: {}", e);
        }

        spawn_admin_audit(
            state.pool.clone(), "CONFIG_UPDATED", id.clone(),
            format!("Configuration key '{}' updated", payload.key),
            None,
            Some(serde_json::json!({ "key": payload.key, "value_length": payload.value.len() })),
            Some(event.trace_id.to_string()),
        );
    }

    Ok(Json(serde_json::json!({ "status": "success" })))
}

/// Batch apply plugin enabled/disabled settings.
///
/// **Route:** `POST /api/plugins/settings`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Request Body
/// ```json
/// [
///   { "id": "plugin-1", "is_active": true },
///   { "id": "plugin-2", "is_active": false }
/// ]
/// ```
///
/// # Response
/// - **200 OK:** `true` on success
/// - **403 Forbidden:** Invalid or missing API key
pub async fn apply_plugin_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<Vec<PluginToggleRequest>>,
) -> AppResult<Json<bool>> {
    check_auth(&state, &headers)?;
    info!(
        count = payload.len(),
        "📥 Received plugin settings apply request"
    );

    let settings = payload.into_iter().map(|i| (i.id, i.is_active)).collect();

    state.plugin_manager.apply_settings(settings).await?;
    Ok(Json(true))
}

#[derive(Deserialize)]
pub struct GrantPermissionRequest {
    pub permission: exiv_shared::Permission,
}

/// Grant a permission to a plugin.
///
/// **Route:** `POST /api/plugins/:id/permissions`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Request Body
/// ```json
/// { "permission": "NetworkAccess" }
/// ```
///
/// Valid permissions: `NetworkAccess`, `FileRead`, `FileWrite`,
/// `ProcessExecution`, `VisionRead`, `AdminAccess`.
///
/// # Side Effects
/// - Broadcasts `PermissionGranted` event (triggers capability injection)
/// - Writes audit log entry
///
/// # Response
/// - **200 OK:** `{ "status": "success" }`
/// - **403 Forbidden:** Invalid or missing API key
pub async fn grant_permission_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<GrantPermissionRequest>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    info!(
        plugin_id = %id,
        permission = ?payload.permission,
        "🔐 Granting permission to plugin"
    );

    state.plugin_manager.grant_permission(&id, payload.permission.clone()).await?;

    // イベントループに通知して Capability を注入させる
    let envelope = crate::EnvelopedEvent::system(exiv_shared::ExivEventData::PermissionGranted {
        plugin_id: id.clone(),
        permission: payload.permission.clone(),
    });
    let event = envelope.event.clone();
    // H-04: Log send errors instead of silently ignoring
    if let Err(e) = state.event_tx.send(envelope).await {
        error!("Failed to send permission grant event: {}", e);
    }

    spawn_admin_audit(
        state.pool.clone(), "PERMISSION_GRANTED", id.clone(),
        "Administrator approved permission request".to_string(),
        Some(format!("{:?}", payload.permission)),
        None, Some(event.trace_id.to_string()),
    );

    Ok(Json(serde_json::json!({ "status": "success" })))
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
    let envelope = crate::EnvelopedEvent::system(exiv_shared::ExivEventData::SystemNotification(
        "Kernel is shutting down for maintenance...".to_string()
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
    Json(event_data): Json<exiv_shared::ExivEventData>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    // 🛡️ Security Check: 外部からの重要なシステムイベントの注入を禁止
    match &event_data {
        // H-15: Only allow safe event types from external sources
        // SystemNotification removed - external callers should not inject system notifications
        exiv_shared::ExivEventData::MessageReceived(_) |
        exiv_shared::ExivEventData::VisionUpdated(_) |
        exiv_shared::ExivEventData::GazeUpdated(_) => {
            // これらは許可
        },
        _ => {
            error!("🚫 SECURITY ALERT: External attempt to inject restricted event: {:?}", event_data);
            return Err(AppError::Exiv(exiv_shared::ExivError::PermissionDenied(exiv_shared::Permission::AdminAccess)));
        }
    }

    let envelope = crate::EnvelopedEvent::system(event_data);
    if let Err(e) = state.event_tx.send(envelope).await {
        error!("Failed to send external event: {}", e);
        return Err(AppError::Internal(anyhow::anyhow!("Failed to publish event")));
    }
    Ok(Json(serde_json::json!({ "status": "published" })))
}

/// Send a chat message into the system.
///
/// **Route:** `POST /api/chat`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Request Body
/// An `ExivMessage` JSON object containing the message content,
/// sender information, and optional metadata.
///
/// # Behavior
/// Wraps the message as a `MessageReceived` event and publishes
/// it to the event bus for processing by agents and plugins.
///
/// # Response
/// - **200 OK:** `{ "status": "accepted" }`
/// - **403 Forbidden:** Invalid or missing API key
/// - **500 Internal Server Error:** Event bus send failure
pub async fn chat_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(msg): Json<ExivMessage>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let envelope = crate::EnvelopedEvent::system(exiv_shared::ExivEventData::MessageReceived(msg));
    if let Err(e) = state.event_tx.send(envelope).await {
        error!("Failed to send chat message event: {}", e);
        return Err(AppError::Internal(anyhow::anyhow!("Failed to accept message")));
    }
    Ok(Json(serde_json::json!({ "status": "accepted" })))
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
                    continue;
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
            "memory_estimate_bytes": history_len * std::mem::size_of::<std::sync::Arc<exiv_shared::ExivEvent>>(),
        }
    })))
}

/// Get stored agent memories from the database.
///
/// **Route:** `GET /api/memories`
///
/// # Authentication
/// No authentication required (read-only).
///
/// # Response
/// Returns up to 100 most recent memory entries (key prefix `mem:`),
/// ordered by key descending. Each entry is parsed from stored JSON.
/// Malformed entries are silently skipped with a debug log.
pub async fn get_memories(State(state): State<Arc<AppState>>) -> AppResult<Json<serde_json::Value>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM plugin_data WHERE key LIKE 'mem:%' ORDER BY key DESC LIMIT 100"
    )
    .fetch_all(&state.pool)
    .await?;

    let memories: Vec<serde_json::Value> = rows.into_iter()
        .filter_map(|(_k, v)| serde_json::from_str(&v).map_err(|e| {
            tracing::debug!("Skipping malformed memory entry: {}", e);
            e
        }).ok())
        .collect();

    Ok(Json(serde_json::json!(memories)))
}

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
#[derive(Deserialize)]
pub struct PermissionDecisionPayload {
    // Accepted for backwards compatibility but not used for audit trail
    // (actor identity determined from auth, not user-supplied value)
    #[allow(dead_code)]
    approved_by: String,
}

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
        state.pool.clone(), "PERMISSION_REQUEST_APPROVED", request_id.clone(),
        "Human administrator approved permission request".to_string(),
        None, None, None,
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
        state.pool.clone(), "PERMISSION_REQUEST_DENIED", request_id.clone(),
        "Human administrator denied permission request".to_string(),
        None, None, None,
    );

    Ok(Json(serde_json::json!({
        "status": "success",
        "message": "Permission request denied"
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use crate::test_utils::create_test_app_state;

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

        if let Err(AppError::Exiv(exiv_shared::ExivError::PermissionDenied(perm))) = result {
            assert_eq!(perm, exiv_shared::Permission::AdminAccess);
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
