pub mod assets;
pub mod chat;
pub mod system;

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
    extract::{Path, State},
    http::HeaderMap,
    response::sse::{Event, Sse},
    Json,
};
use cloto_shared::ClotoMessage;
use futures::stream::Stream;
use serde::Deserialize;
use std::{collections::HashMap, convert::Infallible, sync::Arc, time::Duration};
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

fn spawn_admin_audit(
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
    pub required_capabilities: Option<Vec<cloto_shared::CapabilityType>>,
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
        return Err(AppError::Cloto(cloto_shared::ClotoError::ValidationError(
            format!(
                "Agent name must be 1-200 characters (got {} chars); example: \"my-agent\"",
                payload.name.len()
            ),
        )));
    }
    // Bug #1: Add empty check for description to match name validation pattern
    if payload.description.is_empty() || payload.description.len() > 1000 {
        return Err(AppError::Cloto(cloto_shared::ClotoError::ValidationError(
            format!("Agent description must be 1-1000 characters (got {} chars); example: \"A helpful assistant\"",
                payload.description.len()),
        )));
    }

    // H-04: Metadata size validation
    let metadata = payload.metadata.unwrap_or_default();
    if metadata.len() > 50 {
        return Err(AppError::Cloto(cloto_shared::ClotoError::ValidationError(
            format!(
                "Metadata must have at most 50 key-value pairs (got {})",
                metadata.len()
            ),
        )));
    }
    for (k, v) in &metadata {
        if k.len() > 200 || v.len() > 5000 {
            return Err(AppError::Cloto(cloto_shared::ClotoError::ValidationError(
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
            payload.required_capabilities.unwrap_or_else(|| {
                vec![
                    cloto_shared::CapabilityType::Reasoning,
                    cloto_shared::CapabilityType::Memory,
                ]
            }),
            payload.password.as_deref(),
        )
        .await?;
    Ok(Json(
        serde_json::json!({ "status": "success", "id": agent_id }),
    ))
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
    state
        .agent_manager
        .update_agent_config(&id, payload.default_engine_id, payload.metadata)
        .await?;
    Ok(Json(serde_json::json!({ "status": "success" })))
}

/// Delete an agent and all its data.
///
/// **Route:** `DELETE /api/agents/:id`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Protection
/// The default agent (configured via `DEFAULT_AGENT_ID`) cannot be deleted.
///
/// # Response
/// - **200 OK:** `{ "status": "success" }`
/// - **403 Forbidden:** Attempt to delete the default agent, or invalid API key
/// - **404 Not Found:** Agent ID does not exist
pub async fn delete_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    if id == state.config.default_agent_id {
        return Err(AppError::Validation(format!(
            "Cannot delete the default agent '{}'",
            id
        )));
    }

    state.agent_manager.delete_agent(&id).await?;
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
                    return Err(AppError::Cloto(cloto_shared::ClotoError::PermissionDenied(
                        cloto_shared::Permission::AdminAccess,
                    )));
                }
            }
            None => {
                return Err(AppError::Cloto(cloto_shared::ClotoError::ValidationError(
                    "Password required for this agent's power control".to_string(),
                )));
            }
        }
    }

    state
        .agent_manager
        .set_enabled(&id, payload.enabled)
        .await?;

    // Broadcast power change event via EventBus
    let envelope = crate::EnvelopedEvent::system(cloto_shared::ClotoEventData::AgentPowerChanged {
        agent_id: id.clone(),
        enabled: payload.enabled,
    });
    if let Err(e) = state.event_tx.send(envelope).await {
        error!("Failed to send power change event: {}", e);
    }

    spawn_admin_audit(
        state.pool.clone(),
        if payload.enabled {
            "AGENT_POWER_ON"
        } else {
            "AGENT_POWER_OFF"
        },
        id.clone(),
        format!(
            "Agent {} powered {}",
            id,
            if payload.enabled { "on" } else { "off" }
        ),
        None,
        None,
        None,
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
    let manifests = state
        .plugin_manager
        .list_plugins_with_settings(&state.registry)
        .await?;
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
        let envelope = crate::EnvelopedEvent::system(cloto_shared::ClotoEventData::ConfigUpdated {
            plugin_id: id.clone(),
            config: full_config,
        });
        let event = envelope.event.clone();
        // H-04: Log send errors instead of silently ignoring
        if let Err(e) = state.event_tx.send(envelope).await {
            error!("Failed to send config update event: {}", e);
        }

        spawn_admin_audit(
            state.pool.clone(),
            "CONFIG_UPDATED",
            id.clone(),
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
    pub permission: cloto_shared::Permission,
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

    state
        .plugin_manager
        .grant_permission(&id, payload.permission.clone())
        .await?;

    // イベントループに通知して Capability を注入させる
    let envelope = crate::EnvelopedEvent::system(cloto_shared::ClotoEventData::PermissionGranted {
        plugin_id: id.clone(),
        permission: payload.permission.clone(),
    });
    let event = envelope.event.clone();
    // H-04: Log send errors instead of silently ignoring
    if let Err(e) = state.event_tx.send(envelope).await {
        error!("Failed to send permission grant event: {}", e);
    }

    spawn_admin_audit(
        state.pool.clone(),
        "PERMISSION_GRANTED",
        id.clone(),
        "Administrator approved permission request".to_string(),
        Some(format!("{:?}", payload.permission)),
        None,
        Some(event.trace_id.to_string()),
    );

    Ok(Json(serde_json::json!({ "status": "success" })))
}

/// Get the current effective permissions for a plugin.
///
/// **Route:** `GET /api/plugins/:id/permissions`
pub async fn get_plugin_permissions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let perms = state.plugin_manager.get_permissions(&id).await?;
    let list: Vec<String> = perms.iter().map(|p| format!("{:?}", p)).collect();
    Ok(Json(
        serde_json::json!({ "plugin_id": id, "permissions": list }),
    ))
}

#[derive(Deserialize)]
pub struct RevokePermissionRequest {
    pub permission: cloto_shared::Permission,
}

/// Revoke a permission from a plugin.
///
/// **Route:** `DELETE /api/plugins/:id/permissions`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Request Body
/// ```json
/// { "permission": "NetworkAccess" }
/// ```
pub async fn revoke_permission_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<RevokePermissionRequest>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    info!(plugin_id = %id, permission = ?payload.permission, "🔓 Revoking permission from plugin");

    state
        .plugin_manager
        .revoke_permission(&id, &payload.permission, &state.registry)
        .await?;

    spawn_admin_audit(
        state.pool.clone(),
        "PERMISSION_REVOKED",
        id.clone(),
        "Administrator revoked permission".to_string(),
        Some(format!("{:?}", payload.permission)),
        None,
        None,
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
        | cloto_shared::ClotoEventData::VisionUpdated(_)
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
/// An `ClotoMessage` JSON object containing the message content,
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
    Json(msg): Json<ClotoMessage>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let envelope = crate::EnvelopedEvent::system(cloto_shared::ClotoEventData::MessageReceived(msg));
    if let Err(e) = state.event_tx.send(envelope).await {
        error!("Failed to send chat message event: {}", e);
        return Err(AppError::Internal(anyhow::anyhow!(
            "Failed to accept message"
        )));
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
            if let Some(crate::managers::mcp_protocol::ToolContent::Text { text }) = result.content.first() {
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
            if let Some(crate::managers::mcp_protocol::ToolContent::Text { text }) = result.content.first() {
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

// ============================================================
// MCP Dynamic Server Management
// ============================================================

pub async fn create_mcp_server(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Validation("Missing required field: name".into()))?;

    // Name validation
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::Validation(
            "Server name must be 1-64 characters".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::Validation(
            "Server name must contain only alphanumeric characters, underscores, and hyphens"
                .into(),
        ));
    }

    // Determine command/args: either explicit or auto-generated from code
    let (command, args, script_content) =
        if let Some(code) = body.get("code").and_then(|v| v.as_str()) {
            let description = body
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("A dynamically generated MCP server.");

            // Auto-generate MCP server Python script
            let script = format!(
                r#""""MCP Server: {name} — {description}"""
from mcp.server import Server
from mcp.server.stdio import stdio_server

app = Server("{name}")

{code}

async def main():
    async with stdio_server() as (read, write):
        await app.run(read, write)

if __name__ == "__main__":
    import asyncio
    asyncio.run(main())
"#,
                name = name,
                description = description.replace('"', r#"\""#),
                code = code,
            );

            // Write script to scripts/ directory
            let script_filename = format!("mcp_{}.py", name);
            let scripts_dir = std::path::Path::new("scripts");
            if !scripts_dir.exists() {
                std::fs::create_dir_all(scripts_dir).map_err(|e| {
                    AppError::Internal(anyhow::anyhow!("Failed to create scripts directory: {}", e))
                })?;
            }
            std::fs::write(scripts_dir.join(&script_filename), &script).map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to write MCP server script: {}", e))
            })?;

            let python = if cfg!(windows) { "python" } else { "python3" };
            (
                python.to_string(),
                vec![format!("scripts/{}", script_filename)],
                Some(script),
            )
        } else {
            // Explicit command/args
            let command = body
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AppError::Validation("Missing 'command' or 'code' field".into()))?
                .to_string();

            let args: Vec<String> = body
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            (command, args, None)
        };

    // Add server via McpClientManager (handles connection + DB persistence)
    let tool_names = state
        .mcp_manager
        .add_dynamic_server(
            name.to_string(),
            command.clone(),
            args.clone(),
            script_content,
            body.get("description")
                .and_then(|v| v.as_str())
                .map(String::from),
        )
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to add MCP server: {}", e)))?;

    tracing::info!(name = %name, tools = ?tool_names, "🔌 Dynamic MCP server added");

    Ok(Json(serde_json::json!({
        "status": "created",
        "name": name,
        "tools": tool_names,
    })))
}

pub async fn list_mcp_servers(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let servers = state.mcp_manager.list_servers().await;

    Ok(Json(serde_json::json!({
        "servers": servers,
        "count": servers.len(),
    })))
}

pub async fn delete_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    // Remove from McpClientManager (handles disconnect + DB deactivation)
    // Config-loaded servers cannot be deleted — return 400 with guidance
    state
        .mcp_manager
        .remove_dynamic_server(&name)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("config-loaded") {
                AppError::Validation(msg)
            } else {
                AppError::Internal(anyhow::anyhow!("{}", e))
            }
        })?;

    // Remove auto-generated script file if it exists
    let script_path = std::path::Path::new("scripts").join(format!("mcp_{}.py", name));
    if script_path.exists() {
        let _ = std::fs::remove_file(&script_path);
    }

    tracing::info!(name = %name, "🗑️ MCP server removed");

    Ok(Json(serde_json::json!({
        "status": "deleted",
        "name": name,
    })))
}

// ============================================================
// MCP Server Settings & Access Control (MCP_SERVER_UI_DESIGN.md §4)
// ============================================================

/// GET /api/mcp/servers/:name/settings
pub async fn get_mcp_server_settings(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let settings = crate::db::get_mcp_server_settings(&state.pool, &name)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

    // Get in-memory config env (from mcp.toml or runtime) as defaults
    let config_env = state.mcp_manager.get_server_env(&name).await;

    if let Some((record, default_policy)) = settings {
        // Merge: in-memory config env as base, DB env overrides
        let db_env: HashMap<String, String> =
            serde_json::from_str(&record.env).unwrap_or_default();
        let mut merged = config_env;
        for (k, v) in &db_env {
            merged.insert(k.clone(), v.clone());
        }
        // Mask only sensitive values (KEY, SECRET, TOKEN, PASSWORD)
        let masked_env: HashMap<String, String> = merged
            .iter()
            .map(|(k, v)| {
                let upper = k.to_uppercase();
                let is_secret = upper.contains("KEY")
                    || upper.contains("SECRET")
                    || upper.contains("TOKEN")
                    || upper.contains("PASSWORD")
                    || upper.contains("CREDENTIAL");
                (k.clone(), if is_secret { "***".to_string() } else { v.clone() })
            })
            .collect();

        Ok(Json(serde_json::json!({
            "server_id": record.name,
            "default_policy": default_policy,
            "config": {},
            "env": masked_env,
            "auto_restart": false,
            "command": record.command,
            "args": serde_json::from_str::<Vec<String>>(&record.args).unwrap_or_default(),
            "description": record.description,
        })))
    } else {
        // Fallback: config-loaded servers not yet in DB — use in-memory env
        let servers = state.mcp_manager.list_servers().await;
        if let Some(server) = servers.iter().find(|s| s.id == name) {
            let masked_env: HashMap<String, String> = config_env
                .iter()
                .map(|(k, v)| {
                    let upper = k.to_uppercase();
                    let is_secret = upper.contains("KEY")
                        || upper.contains("SECRET")
                        || upper.contains("TOKEN")
                        || upper.contains("PASSWORD")
                        || upper.contains("CREDENTIAL");
                    (k.clone(), if is_secret { "***".to_string() } else { v.clone() })
                })
                .collect();
            Ok(Json(serde_json::json!({
                "server_id": server.id,
                "default_policy": "opt-in",
                "config": {},
                "env": masked_env,
                "auto_restart": false,
                "command": server.command,
                "args": server.args,
                "description": null,
            })))
        } else {
            Err(AppError::Validation(format!(
                "MCP server '{}' not found",
                name
            )))
        }
    }
}

/// PUT /api/mcp/servers/:name/settings
pub async fn update_mcp_server_settings(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    if let Some(policy) = body.get("default_policy").and_then(|v| v.as_str()) {
        if !["opt-in", "opt-out"].contains(&policy) {
            return Err(AppError::Validation(
                "default_policy must be 'opt-in' or 'opt-out'".into(),
            ));
        }
        let rows = crate::db::update_mcp_server_default_policy(&state.pool, &name, policy)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

        if rows == 0 {
            // Config-loaded server (from mcp.toml) — not yet in DB.
            // Look up in-memory server info and persist it.
            let servers = state.mcp_manager.list_servers().await;
            if let Some(server) = servers.iter().find(|s| s.id == name) {
                let args_json = serde_json::to_string(&server.args).unwrap_or_else(|_| "[]".to_string());
                crate::db::ensure_mcp_server_in_db(
                    &state.pool,
                    &name,
                    &server.command,
                    &args_json,
                    policy,
                )
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;
            } else {
                return Err(AppError::Validation(format!(
                    "MCP server '{}' not found",
                    name
                )));
            }
        }
    }

    // Handle env updates
    if let Some(env_obj) = body.get("env").and_then(|v| v.as_object()) {
        // Load existing env from DB to preserve unchanged values (sent as "***")
        let existing_env: HashMap<String, String> = if let Ok(Some((record, _))) =
            crate::db::get_mcp_server_settings(&state.pool, &name).await
        {
            serde_json::from_str(&record.env).unwrap_or_default()
        } else {
            HashMap::new()
        };

        let mut merged_env: HashMap<String, String> = HashMap::new();
        for (key, value) in env_obj {
            if let Some(val_str) = value.as_str() {
                if val_str == "***" {
                    // Preserve existing value
                    if let Some(existing_val) = existing_env.get(key) {
                        merged_env.insert(key.clone(), existing_val.clone());
                    }
                } else if !val_str.is_empty() {
                    // New or updated value
                    merged_env.insert(key.clone(), val_str.to_string());
                }
                // Empty string = remove the key (omit from merged_env)
            }
        }

        // Ensure server is in DB before updating env
        let rows = crate::db::update_mcp_server_env(
            &state.pool,
            &name,
            &serde_json::to_string(&merged_env).unwrap_or_else(|_| "{}".to_string()),
        )
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

        if rows == 0 {
            // Config-loaded server not yet in DB — persist it first
            let servers = state.mcp_manager.list_servers().await;
            if let Some(server) = servers.iter().find(|s| s.id == name) {
                let args_json =
                    serde_json::to_string(&server.args).unwrap_or_else(|_| "[]".to_string());
                crate::db::ensure_mcp_server_in_db(
                    &state.pool,
                    &name,
                    &server.command,
                    &args_json,
                    "opt-in",
                )
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;
                // Retry env update
                crate::db::update_mcp_server_env(
                    &state.pool,
                    &name,
                    &serde_json::to_string(&merged_env).unwrap_or_else(|_| "{}".to_string()),
                )
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;
            }
        }

        // Update in-memory config and restart server
        if let Err(e) = state
            .mcp_manager
            .update_server_env(&name, merged_env)
            .await
        {
            tracing::warn!("Failed to restart server after env update: {}", e);
        }
    }

    spawn_admin_audit(
        state.pool.clone(),
        "MCP_SERVER_SETTINGS_UPDATED",
        name.clone(),
        "MCP server settings updated".to_string(),
        None,
        None,
        None,
    );

    Ok(Json(serde_json::json!({
        "status": "success",
        "message": format!("Settings updated for server '{}'", name)
    })))
}

/// GET /api/mcp/servers/:name/access
pub async fn get_mcp_server_access(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let entries = crate::db::get_access_entries_for_server(&state.pool, &name)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

    // Get server's default_policy
    let settings = crate::db::get_mcp_server_settings(&state.pool, &name)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

    let default_policy = settings.as_ref().map_or("opt-in", |(_, dp)| dp.as_str());

    // Get tools from running server
    let tools: Vec<String> = {
        let servers = state.mcp_manager.list_servers().await;
        servers
            .iter()
            .find(|s| s.id == name)
            .map(|s| s.tools.clone())
            .unwrap_or_default()
    };

    Ok(Json(serde_json::json!({
        "server_id": name,
        "default_policy": default_policy,
        "tools": tools,
        "entries": entries,
    })))
}

/// PUT /api/mcp/servers/:name/access
pub async fn put_mcp_server_access(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let entries_val = body
        .get("entries")
        .ok_or_else(|| AppError::Validation("Missing required field: entries".into()))?;

    let entries: Vec<crate::db::AccessControlEntry> =
        serde_json::from_value(entries_val.clone())
            .map_err(|e| AppError::Validation(format!("Invalid entries format: {}", e)))?;

    // Validate all entries reference this server
    for entry in &entries {
        if entry.server_id != name {
            return Err(AppError::Validation(format!(
                "Entry server_id '{}' does not match route server '{}'",
                entry.server_id, name
            )));
        }
        if !["server_grant", "tool_grant"].contains(&entry.entry_type.as_str()) {
            return Err(AppError::Validation(format!(
                "Cannot bulk-update entry_type '{}'; only server_grant and tool_grant allowed",
                entry.entry_type
            )));
        }
    }

    crate::db::put_access_entries(&state.pool, &name, &entries)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

    spawn_admin_audit(
        state.pool.clone(),
        "MCP_ACCESS_UPDATED",
        name.clone(),
        format!("Access control updated with {} entries", entries.len()),
        None,
        None,
        None,
    );

    Ok(Json(serde_json::json!({
        "status": "success",
        "message": format!("Access control updated for server '{}'", name),
        "count": entries.len(),
    })))
}

/// GET /api/mcp/access/by-agent/:agent_id
pub async fn get_agent_access(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let entries = crate::db::get_access_entries_for_agent(&state.pool, &agent_id)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;

    Ok(Json(serde_json::json!({
        "agent_id": agent_id,
        "entries": entries,
    })))
}

/// POST /api/mcp/servers/:name/restart
pub async fn restart_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let tools = state
        .mcp_manager
        .restart_server(&name)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to restart server: {}", e)))?;

    spawn_admin_audit(
        state.pool.clone(),
        "MCP_SERVER_RESTARTED",
        name.clone(),
        "MCP server restarted".to_string(),
        None,
        None,
        None,
    );

    info!(name = %name, "🔄 MCP server restarted");

    Ok(Json(serde_json::json!({
        "status": "restarted",
        "name": name,
        "tools": tools,
    })))
}

/// POST /api/mcp/servers/:name/start
pub async fn start_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let tools = state
        .mcp_manager
        .start_server(&name)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to start server: {}", e)))?;

    spawn_admin_audit(
        state.pool.clone(),
        "MCP_SERVER_STARTED",
        name.clone(),
        "MCP server started".to_string(),
        None,
        None,
        None,
    );

    info!(name = %name, "▶️ MCP server started");

    Ok(Json(serde_json::json!({
        "status": "started",
        "name": name,
        "tools": tools,
    })))
}

/// POST /api/mcp/servers/:name/stop
pub async fn stop_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    state
        .mcp_manager
        .stop_server(&name)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to stop server: {}", e)))?;

    spawn_admin_audit(
        state.pool.clone(),
        "MCP_SERVER_STOPPED",
        name.clone(),
        "MCP server stopped".to_string(),
        None,
        None,
        None,
    );

    info!(name = %name, "⏹️ MCP server stopped");

    Ok(Json(serde_json::json!({
        "status": "stopped",
        "name": name,
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

// ============================================================
// YOLO Mode API
// ============================================================

/// GET /api/settings/yolo
pub async fn get_yolo_mode(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let enabled = state
        .mcp_manager
        .yolo_mode
        .load(std::sync::atomic::Ordering::Relaxed);
    Ok(Json(serde_json::json!({ "enabled": enabled })))
}

/// PUT /api/settings/yolo
pub async fn set_yolo_mode(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    state
        .mcp_manager
        .yolo_mode
        .store(enabled, std::sync::atomic::Ordering::Relaxed);

    if enabled {
        tracing::warn!("YOLO mode enabled via API");
    } else {
        tracing::info!("YOLO mode disabled via API");
    }

    spawn_admin_audit(
        state.pool.clone(),
        "YOLO_MODE_CHANGED",
        "system".to_string(),
        format!("YOLO mode set to {}", enabled),
        None,
        None,
        None,
    );

    Ok(Json(
        serde_json::json!({ "status": "ok", "enabled": enabled }),
    ))
}

// ── Cron Job Management (Layer 2: Autonomous Trigger) ──

/// GET /api/cron/jobs[?agent_id=X]
pub async fn list_cron_jobs(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let jobs = if let Some(agent_id) = query.get("agent_id") {
        crate::db::list_cron_jobs_for_agent(&state.pool, agent_id).await
    } else {
        crate::db::list_cron_jobs(&state.pool).await
    }
    .map_err(|e| AppError::Internal(e))?;
    Ok(Json(serde_json::json!({ "jobs": jobs, "count": jobs.len() })))
}

/// POST /api/cron/jobs
pub async fn create_cron_job(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let agent_id = payload["agent_id"].as_str()
        .ok_or_else(|| AppError::Validation("agent_id is required".into()))?;
    let name = payload["name"].as_str()
        .ok_or_else(|| AppError::Validation("name is required".into()))?;
    let schedule_type = payload["schedule_type"].as_str()
        .ok_or_else(|| AppError::Validation("schedule_type is required (interval|cron|once)".into()))?;
    let schedule_value = payload["schedule_value"].as_str()
        .ok_or_else(|| AppError::Validation("schedule_value is required".into()))?;
    let message = payload["message"].as_str()
        .ok_or_else(|| AppError::Validation("message is required".into()))?;

    // Validate schedule and compute initial next_run_at
    let next_run_at = crate::managers::scheduler::calculate_initial_next_run(schedule_type, schedule_value)
        .map_err(|e| AppError::Validation(e.to_string()))?;

    let job_id = format!("cron.{}.{}", agent_id, cloto_shared::ClotoId::new());
    let engine_id = payload["engine_id"].as_str().map(String::from);
    let max_iterations = payload["max_iterations"].as_i64().map(|v| v as i32);

    let job = crate::db::CronJobRow {
        id: job_id.clone(),
        agent_id: agent_id.to_string(),
        name: name.to_string(),
        enabled: true,
        schedule_type: schedule_type.to_string(),
        schedule_value: schedule_value.to_string(),
        engine_id,
        message: message.to_string(),
        next_run_at,
        last_run_at: None,
        last_status: None,
        last_error: None,
        max_iterations: max_iterations.or(Some(8)),
        created_at: String::new(), // set by DB default
    };

    crate::db::create_cron_job(&state.pool, &job).await
        .map_err(|e| AppError::Internal(e))?;

    tracing::info!(job_id = %job_id, agent_id = %agent_id, name = %name, "Cron job created");

    Ok(Json(serde_json::json!({ "id": job_id, "next_run_at": next_run_at })))
}

/// DELETE /api/cron/jobs/:id
pub async fn delete_cron_job(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    crate::db::delete_cron_job(&state.pool, &job_id).await
        .map_err(|e| AppError::Validation(e.to_string()))?;
    tracing::info!(job_id = %job_id, "Cron job deleted");
    Ok(Json(serde_json::json!({ "status": "deleted" })))
}

/// POST /api/cron/jobs/:id/toggle
pub async fn toggle_cron_job(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(job_id): axum::extract::Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let enabled = payload["enabled"].as_bool()
        .ok_or_else(|| AppError::Validation("enabled (bool) is required".into()))?;
    crate::db::set_cron_job_enabled(&state.pool, &job_id, enabled).await
        .map_err(|e| AppError::Validation(e.to_string()))?;
    Ok(Json(serde_json::json!({ "status": "ok", "enabled": enabled })))
}

/// POST /api/cron/jobs/:id/run — trigger immediate execution
pub async fn run_cron_job_now(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    // Fetch the job
    let jobs = crate::db::list_cron_jobs(&state.pool).await
        .map_err(|e| AppError::Internal(e))?;
    let job = jobs.into_iter().find(|j| j.id == job_id)
        .ok_or_else(|| AppError::NotFound(format!("Cron job '{}' not found", job_id)))?;

    // Build and dispatch the message immediately
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("target_agent_id".into(), job.agent_id.clone());
    metadata.insert("cron_job_id".into(), job.id.clone());
    metadata.insert("cron_source".into(), "manual".into());
    if let Some(ref engine_id) = job.engine_id {
        metadata.insert("engine_override".into(), engine_id.clone());
    }

    let msg = cloto_shared::ClotoMessage {
        id: cloto_shared::ClotoId::new().to_string(),
        source: cloto_shared::MessageSource::System,
        target_agent: Some(job.agent_id.clone()),
        content: job.message.clone(),
        timestamp: chrono::Utc::now(),
        metadata,
    };

    let envelope = crate::EnvelopedEvent::system(
        cloto_shared::ClotoEventData::MessageReceived(msg),
    );

    state.event_tx.send(envelope).await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to dispatch: {}", e)))?;

    tracing::info!(job_id = %job_id, "Cron job manually triggered");
    Ok(Json(serde_json::json!({ "status": "dispatched" })))
}

// ── LLM Provider Management (MGP §13.4 — Centralized Key Management) ──

/// GET /api/llm/providers
pub async fn list_llm_providers(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let providers = crate::db::list_llm_providers(&state.pool).await
        .map_err(|e| AppError::Internal(e))?;
    // Mask API keys in response
    let masked: Vec<serde_json::Value> = providers.iter().map(|p| {
        serde_json::json!({
            "id": p.id,
            "display_name": p.display_name,
            "api_url": p.api_url,
            "has_key": !p.api_key.is_empty(),
            "model_id": p.model_id,
            "timeout_secs": p.timeout_secs,
            "enabled": p.enabled,
        })
    }).collect();
    Ok(Json(serde_json::json!({ "providers": masked })))
}

/// POST /api/llm/providers/:id/key
pub async fn set_llm_provider_key(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let api_key = payload["api_key"].as_str()
        .ok_or_else(|| AppError::Validation("api_key is required".into()))?;
    crate::db::set_llm_provider_key(&state.pool, &provider_id, api_key).await
        .map_err(|e| AppError::Validation(e.to_string()))?;
    tracing::info!(provider = %provider_id, "LLM provider API key updated");
    Ok(Json(serde_json::json!({ "status": "ok" })))
}

/// DELETE /api/llm/providers/:id/key
pub async fn delete_llm_provider_key(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    crate::db::delete_llm_provider_key(&state.pool, &provider_id).await
        .map_err(|e| AppError::Internal(e))?;
    tracing::info!(provider = %provider_id, "LLM provider API key deleted");
    Ok(Json(serde_json::json!({ "status": "deleted" })))
}
