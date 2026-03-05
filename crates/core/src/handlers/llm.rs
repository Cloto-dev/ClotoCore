use axum::{extract::State, Json};
use std::sync::Arc;

use crate::{AppError, AppResult, AppState};

use super::{check_auth, ok_data};

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
