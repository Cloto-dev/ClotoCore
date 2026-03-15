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

use super::{check_auth, ok_data, spawn_admin_audit};

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
    #[serde(default)]
    pub metadata: Option<HashMap<String, String>>,
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
pub async fn get_agents(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;
    let agents = state.agent_manager.list_agents().await?;
    ok_data(serde_json::json!(agents))
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

    // Collect server IDs to grant before metadata is moved
    let mut servers_to_grant = vec![payload.default_engine.clone()];
    if let Some(mem) = metadata.get("preferred_memory") {
        if !mem.is_empty() {
            servers_to_grant.push(mem.clone());
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

    // Auto-grant access to the selected engine and memory servers
    let now = chrono::Utc::now().to_rfc3339();
    for server_id in &servers_to_grant {
        let entry = crate::db::mcp::AccessControlEntry {
            id: None,
            entry_type: "server_grant".to_string(),
            agent_id: agent_id.clone(),
            server_id: server_id.clone(),
            tool_name: None,
            permission: "allow".to_string(),
            granted_by: Some("system".to_string()),
            granted_at: now.clone(),
            expires_at: None,
            justification: Some("Auto-granted during agent creation".to_string()),
            metadata: None,
        };
        if let Err(e) = crate::db::mcp::save_access_control_entry(&state.pool, &entry).await {
            warn!(server_id, error = %e, "Failed to auto-grant server access during agent creation");
        }
    }

    ok_data(serde_json::json!({ "id": agent_id }))
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
    ok_data(serde_json::json!({}))
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

    // Look up the agent's preferred memory server BEFORE deletion
    let memory_server_id = state
        .agent_manager
        .get_agent_config(&id)
        .await
        .ok()
        .and_then(|(meta, _)| meta.metadata.get("preferred_memory").cloned())
        .filter(|s| !s.is_empty());

    state.agent_manager.delete_agent(&id).await?;

    // Clean up CPersona memory data (best-effort, don't fail the delete)
    if let Some(ref mem_server) = memory_server_id {
        let args = serde_json::json!({ "agent_id": id });
        match state
            .mcp_manager
            .call_server_tool(mem_server, "delete_agent_data", args)
            .await
        {
            Ok(result) => {
                tracing::info!(
                    agent_id = %id,
                    memory_server = %mem_server,
                    "CPersona agent data cleanup: {:?}",
                    result
                );
            }
            Err(e) => {
                warn!(
                    agent_id = %id,
                    memory_server = %mem_server,
                    error = %e,
                    "Failed to clean up CPersona agent data (agent already deleted from core DB)"
                );
            }
        }
    }

    ok_data(serde_json::json!({}))
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

    ok_data(serde_json::json!({ "enabled": payload.enabled }))
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

    // Attempt vision analysis (graceful degradation, 30s timeout for Ollama model load + inference)
    let avatar_description = match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        analyze_avatar(&state, &avatar_path_str),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            tracing::warn!(
                "Avatar vision analysis timed out after 30s, storing without description"
            );
            None
        }
    };

    state
        .agent_manager
        .set_avatar(&id, &avatar_path_str, avatar_description.as_deref())
        .await?;

    ok_data(serde_json::json!({
        "avatar_path": avatar_path_str,
        "avatar_description": avatar_description,
    }))
}

/// Analyze avatar image via Vision capability MCP server.
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
                   This will be used as the agent's visual identity description.",
        "mode": "vision"
    });

    match state
        .mcp_manager
        .call_capability_tool(
            crate::managers::CapabilityType::Vision,
            "analyze_image",
            args,
            None,
        )
        .await
    {
        Ok(result) => {
            for content in &result.content {
                if let crate::managers::mcp_protocol::ToolContent::Text { text } = content {
                    // Try parsing JSON response
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
                        // Reject error responses (e.g. Ollama not running)
                        if json.get("error").is_some() {
                            warn!("Vision server returned error: {}", text);
                            return None;
                        }
                        if let Some(response) = json.get("response").and_then(|r| r.as_str()) {
                            if !response.is_empty() {
                                return Some(response.to_string());
                            }
                        }
                    }
                    // Don't store raw unparsed text as description
                    return None;
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

// ============================================================
// VRM Model Management
// ============================================================

/// Maximum VRM file size: 50 MB
const VRM_MAX_BYTES: usize = 50 * 1024 * 1024;

/// Upload a VRM model for an agent.
///
/// **Route:** `POST /api/agents/:id/vrm`
///
/// Accepts raw VRM bytes (model/gltf-binary).
pub async fn upload_vrm(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    if body.len() > VRM_MAX_BYTES {
        return Err(AppError::Validation(format!(
            "VRM file too large ({} bytes, max {} bytes)",
            body.len(),
            VRM_MAX_BYTES
        )));
    }

    // Verify agent exists
    state.agent_manager.get_agent_config(&id).await?;

    // Save to disk
    let vrm_path = state.data_dir.join("vrm").join(format!("{}.vrm", id));
    let vrm_path_str = vrm_path.to_string_lossy().to_string();
    tokio::fs::write(&vrm_path, &body)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to write VRM file: {}", e)))?;

    state.agent_manager.set_vrm(&id, &vrm_path_str).await?;

    ok_data(serde_json::json!({ "vrm_path": vrm_path_str }))
}

/// Serve an agent's VRM model.
///
/// **Route:** `GET /api/agents/:id/vrm`
///
/// No authentication required (read-only).
pub async fn get_vrm(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let vrm_path = state
        .agent_manager
        .get_vrm_path(&id)
        .await?
        .ok_or_else(|| AppError::Validation("No VRM model set".to_string()))?;

    let data = tokio::fs::read(&vrm_path)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to read VRM file: {}", e)))?;

    Ok((
        StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static("model/gltf-binary"),
            ),
            (
                axum::http::header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=3600"),
            ),
        ],
        data,
    ))
}

/// Delete an agent's VRM model.
///
/// **Route:** `DELETE /api/agents/:id/vrm`
pub async fn delete_vrm(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    if let Ok(Some(path)) = state.agent_manager.get_vrm_path(&id).await {
        let _ = tokio::fs::remove_file(&path).await;
    }

    state.agent_manager.clear_vrm(&id).await?;

    ok_data(serde_json::json!({}))
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

    ok_data(serde_json::json!({}))
}

// ── Viseme Generation ──

#[derive(Deserialize)]
pub struct VisemeRequest {
    pub text: String,
}

/// Generate a viseme timeline from text for lip-sync animation.
///
/// **Route:** `POST /api/agents/:id/visemes`
///
/// # Request Body
/// ```json
/// { "text": "こんにちは" }
/// ```
///
/// # Response
/// ```json
/// { "data": { "entries": [...], "total_duration_ms": 600 } }
/// ```
pub async fn generate_visemes(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(_agent_id): Path<String>,
    Json(body): Json<VisemeRequest>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let timeline = crate::viseme::generate_timeline(&body.text);

    ok_data(serde_json::json!(timeline))
}

/// GET /api/speech/:filename — Serve synthesized WAV files from data/speech/.
pub async fn serve_speech_file(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(filename): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    check_auth(&state, &headers)?;

    // Validate filename: alphanumeric + underscores + dots only, must end with .wav
    if !filename.ends_with(".wav")
        || filename.contains("..")
        || filename.contains('/')
        || filename.contains('\\')
        || !filename
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
    {
        return Err(AppError::Validation("Invalid filename".to_string()));
    }

    let speech_dir = crate::config::exe_dir().join("data").join("speech");
    let file_path = speech_dir.join(&filename);

    // Ensure the resolved path is within the speech directory
    let canonical = file_path
        .canonicalize()
        .map_err(|_| AppError::Validation("File not found".to_string()))?;
    let canonical_dir = speech_dir
        .canonicalize()
        .map_err(|_| AppError::Validation("Speech directory not found".to_string()))?;
    if !canonical.starts_with(&canonical_dir) {
        return Err(AppError::Validation("Access denied".to_string()));
    }

    let data = tokio::fs::read(&canonical)
        .await
        .map_err(|_| AppError::Validation("File not found".to_string()))?;

    Ok((
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "audio/wav"),
            (axum::http::header::CACHE_CONTROL, "no-cache"),
        ],
        data,
    ))
}
