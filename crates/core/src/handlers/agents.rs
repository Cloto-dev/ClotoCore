use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};
use tracing::{error, warn};

use crate::{AppError, AppResult, AppState};

use super::{check_auth, spawn_admin_audit};

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
pub struct UpdateAgentRequest {
    pub name: Option<String>,
    pub description: Option<String>,
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
    use super::utils::{
        AGENT_DESC_MAX, AGENT_DESC_MIN, AGENT_METADATA_KEY_MAX, AGENT_METADATA_MAX_PAIRS,
        AGENT_METADATA_VALUE_MAX, AGENT_NAME_MAX, AGENT_NAME_MIN,
    };
    if payload.name.is_empty() || payload.name.len() > AGENT_NAME_MAX {
        return Err(AppError::Cloto(cloto_shared::ClotoError::ValidationError(
            format!(
                "Agent name must be {AGENT_NAME_MIN}-{AGENT_NAME_MAX} characters (got {} chars); example: \"my-agent\"",
                payload.name.len()
            ),
        )));
    }
    if payload.description.is_empty() || payload.description.len() > AGENT_DESC_MAX {
        return Err(AppError::Cloto(cloto_shared::ClotoError::ValidationError(
            format!("Agent description must be {AGENT_DESC_MIN}-{AGENT_DESC_MAX} characters (got {} chars); example: \"A helpful assistant\"",
                payload.description.len()),
        )));
    }

    // H-04: Metadata size validation
    let metadata = payload.metadata.unwrap_or_default();
    if metadata.len() > AGENT_METADATA_MAX_PAIRS {
        return Err(AppError::Cloto(cloto_shared::ClotoError::ValidationError(
            format!(
                "Metadata must have at most {} key-value pairs (got {})",
                AGENT_METADATA_MAX_PAIRS,
                metadata.len()
            ),
        )));
    }
    for (k, v) in &metadata {
        if k.len() > AGENT_METADATA_KEY_MAX || v.len() > AGENT_METADATA_VALUE_MAX {
            return Err(AppError::Cloto(cloto_shared::ClotoError::ValidationError(
                format!("Metadata key '{}' exceeds limits (key: {} chars max {}, value: {} chars max {})",
                    k, k.len(), AGENT_METADATA_KEY_MAX, v.len(), AGENT_METADATA_VALUE_MAX),
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

    // Protect default agent name/description
    if id == state.config.default_agent_id
        && (payload.name.is_some() || payload.description.is_some())
    {
        return Err(AppError::Validation(
            "Cannot modify name or description of the default agent".to_string(),
        ));
    }

    if let Some(ref name) = payload.name {
        if name.is_empty() || name.len() > super::utils::AGENT_NAME_MAX {
            return Err(AppError::Validation(format!(
                "Agent name must be {}-{} characters (got {})",
                super::utils::AGENT_NAME_MIN,
                super::utils::AGENT_NAME_MAX,
                name.len()
            )));
        }
    }
    if let Some(ref desc) = payload.description {
        if desc.is_empty() || desc.len() > super::utils::AGENT_DESC_MAX {
            return Err(AppError::Validation(format!(
                "Agent description must be {}-{} characters (got {})",
                super::utils::AGENT_DESC_MIN,
                super::utils::AGENT_DESC_MAX,
                desc.len()
            )));
        }
    }

    state
        .agent_manager
        .update_agent_config(
            &id,
            payload.name.as_deref(),
            payload.description.as_deref(),
            payload.default_engine_id,
            payload.metadata,
        )
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
    body: Option<Json<serde_json::Value>>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    if id == state.config.default_agent_id {
        return Err(AppError::Validation(format!(
            "Cannot delete the default agent '{}'",
            id
        )));
    }

    // If agent has a password, require it for deletion
    let password = body
        .as_ref()
        .and_then(|b| b.get("password"))
        .and_then(|v| v.as_str());
    super::utils::verify_agent_password(&state, &id, password, "delete this agent").await?;

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
    super::utils::verify_agent_password(
        &state,
        &id,
        payload.password.as_deref(),
        "control this agent's power state",
    )
    .await?;

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

// ============================================================
// Avatar Management
// ============================================================

use super::utils::{ext_to_mime, mime_to_ext, AVATAR_MAX_BYTES};

/// Upload an avatar image for an agent.
///
/// **Route:** `POST /api/agents/:id/avatar`
///
/// Accepts raw image bytes with Content-Type header.
/// Optionally analyzes the image via vision.capture MCP server.
pub async fn upload_avatar(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    if id == state.config.default_agent_id {
        return Err(AppError::Validation(
            "Cannot modify avatar of the default agent".to_string(),
        ));
    }

    if body.len() > AVATAR_MAX_BYTES {
        return Err(AppError::Validation(format!(
            "Avatar image too large ({} bytes, max {} bytes)",
            body.len(),
            AVATAR_MAX_BYTES
        )));
    }

    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/png");

    let ext = mime_to_ext(content_type).ok_or_else(|| {
        AppError::Validation(format!(
            "Unsupported image type '{}'. Supported: png, jpeg, gif, webp",
            content_type
        ))
    })?;

    // Verify agent exists
    state.agent_manager.get_agent_config(&id).await?;

    // Save to disk (use data_dir for Tauri compatibility)
    let avatar_path = state
        .data_dir
        .join("avatars")
        .join(format!("{}.{}", id, ext));
    let avatar_path_str = avatar_path.to_string_lossy().to_string();
    tokio::fs::write(&avatar_path, &body)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to write avatar file: {}", e)))?;

    // Attempt vision analysis (graceful degradation)
    let avatar_description = analyze_avatar(&state, &avatar_path_str).await;

    state
        .agent_manager
        .set_avatar(&id, &avatar_path_str, avatar_description.as_deref())
        .await?;

    Ok(Json(serde_json::json!({
        "status": "success",
        "avatar_path": avatar_path_str,
        "avatar_description": avatar_description,
    })))
}

/// Analyze avatar image via vision.capture MCP server.
/// Returns None if vision server is unavailable or analysis fails.
async fn analyze_avatar(state: &AppState, avatar_path: &str) -> Option<String> {
    let abs_path = std::env::current_dir()
        .ok()?
        .join(avatar_path)
        .to_string_lossy()
        .to_string();

    let args = serde_json::json!({
        "file_path": abs_path,
        "prompt": "Describe this character/avatar image concisely. \
                   Focus on appearance, style, colors, and mood. \
                   This will be used as the agent's visual identity description."
    });

    match state
        .mcp_manager
        .call_server_tool("vision.capture", "analyze_image", args)
        .await
    {
        Ok(result) => {
            for content in &result.content {
                if let crate::managers::mcp_protocol::ToolContent::Text { text } = content {
                    // Try parsing JSON response
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
                        if let Some(response) = json.get("response").and_then(|r| r.as_str()) {
                            return Some(response.to_string());
                        }
                    }
                    return Some(text.clone());
                }
            }
            None
        }
        Err(e) => {
            warn!(error = %e, "Avatar vision analysis unavailable, storing without description");
            None
        }
    }
}

/// Serve an agent's avatar image.
///
/// **Route:** `GET /api/agents/:id/avatar`
///
/// No authentication required (read-only).
pub async fn get_avatar(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let avatar_path = state
        .agent_manager
        .get_avatar_path(&id)
        .await?
        .ok_or_else(|| AppError::Validation("No avatar set".to_string()))?;

    let data = tokio::fs::read(&avatar_path)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to read avatar file: {}", e)))?;

    let ext = avatar_path.rsplit('.').next().unwrap_or("png");
    let mime = ext_to_mime(ext);

    Ok((
        StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static(mime),
            ),
            (
                axum::http::header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=3600"),
            ),
        ],
        data,
    ))
}

/// Delete an agent's avatar.
///
/// **Route:** `DELETE /api/agents/:id/avatar`
pub async fn delete_avatar(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    if id == state.config.default_agent_id {
        return Err(AppError::Validation(
            "Cannot modify avatar of the default agent".to_string(),
        ));
    }

    if let Ok(Some(path)) = state.agent_manager.get_avatar_path(&id).await {
        let _ = tokio::fs::remove_file(&path).await;
    }

    state.agent_manager.clear_avatar(&id).await?;

    Ok(Json(serde_json::json!({ "status": "success" })))
}
