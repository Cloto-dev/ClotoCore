use axum::{extract::State, Json};
use std::sync::Arc;

use crate::{AppError, AppResult, AppState};

use super::{check_auth, ok_data, spawn_admin_audit};

/// Maximum allowed length for `model_id` (characters after trimming).
const MODEL_ID_MAX_LEN: usize = 200;

/// GET /api/llm/providers
pub async fn list_llm_providers(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let providers = crate::db::list_llm_providers(&state.pool)
        .await
        .map_err(AppError::Internal)?;
    // Mask API keys in response
    let masked: Vec<serde_json::Value> = providers
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "display_name": p.display_name,
                "api_url": p.api_url,
                "has_key": !p.api_key.is_empty(),
                "model_id": p.model_id,
                "timeout_secs": p.timeout_secs,
                "enabled": p.enabled,
            })
        })
        .collect();
    ok_data(serde_json::json!({ "providers": masked }))
}

/// POST /api/llm/providers/:id/key
pub async fn set_llm_provider_key(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let api_key = payload["api_key"]
        .as_str()
        .ok_or_else(|| AppError::Validation("api_key is required".into()))?;
    crate::db::set_llm_provider_key(&state.pool, &provider_id, api_key)
        .await
        .map_err(|e| AppError::Validation(e.to_string()))?;
    tracing::info!(provider = %provider_id, "LLM provider API key updated");
    ok_data(serde_json::json!({}))
}

/// POST /api/llm/providers/:id/model
///
/// Updates the `model_id` for a provider, recording the change in
/// `llm_provider_model_history`. For `mind.ollama`, also relays the change to
/// the running MCP server's `switch_model` tool so the active model updates
/// without a kernel restart.
pub async fn set_llm_provider_model(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let raw = payload["model_id"]
        .as_str()
        .ok_or_else(|| AppError::Validation("model_id is required".into()))?;
    let model_id = raw.trim();

    if model_id.is_empty() {
        return Err(AppError::Validation("model_id must not be empty".into()));
    }
    if model_id.len() > MODEL_ID_MAX_LEN {
        return Err(AppError::Validation(format!(
            "model_id exceeds max length {}",
            MODEL_ID_MAX_LEN
        )));
    }
    if model_id.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "model_id must not contain control characters or newlines".into(),
        ));
    }

    let old_model = crate::db::set_llm_provider_model(&state.pool, &provider_id, model_id)
        .await
        .map_err(|e| AppError::Validation(e.to_string()))?;

    // mind.ollama reads OLLAMA_MODEL only at startup, so relay the change to
    // its switch_model tool. Failure here is non-fatal (DB is already updated;
    // post-connect sync will catch up on the next (re)start).
    if provider_id == "ollama" {
        let mcp_mgr = state.mcp_manager.clone();
        let model_owned = model_id.to_string();
        tokio::spawn(async move {
            match mcp_mgr
                .call_server_tool(
                    "mind.ollama",
                    "switch_model",
                    serde_json::json!({ "model": model_owned }),
                )
                .await
            {
                Ok(_) => tracing::info!(model = %model_owned, "mind.ollama switch_model relayed"),
                Err(e) => tracing::warn!(
                    error = %e,
                    "mind.ollama switch_model relay failed (DB updated; next connect will resync)"
                ),
            }
        });
    }

    spawn_admin_audit(
        state.pool.clone(),
        "LLM_PROVIDER_MODEL_UPDATED",
        provider_id.clone(),
        format!("Model changed from '{}' to '{}'", old_model, model_id),
        None,
        Some(serde_json::json!({ "old_model_id": old_model, "new_model_id": model_id })),
        None,
    );

    tracing::info!(provider = %provider_id, model = %model_id, "LLM provider model updated");
    ok_data(serde_json::json!({}))
}

/// DELETE /api/llm/providers/:id/key
pub async fn delete_llm_provider_key(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    crate::db::delete_llm_provider_key(&state.pool, &provider_id)
        .await
        .map_err(AppError::Internal)?;
    tracing::info!(provider = %provider_id, "LLM provider API key deleted");
    ok_data(serde_json::json!({}))
}
