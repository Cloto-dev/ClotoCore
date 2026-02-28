use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};
use tracing::error;

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
