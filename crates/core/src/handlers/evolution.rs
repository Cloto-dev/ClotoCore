//! Evolution Engine API handlers for dashboard integration.

use axum::{
    extract::{Path, Query, State},
    Json,
    http::HeaderMap,
};
use serde::Deserialize;
use std::sync::Arc;
use tracing::info;

use crate::{AppState, AppResult, AppError};
use crate::evolution::EvolutionParams;

fn get_engine(state: &AppState) -> AppResult<&Arc<crate::evolution::EvolutionEngine>> {
    state.evolution_engine.as_ref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("Evolution engine not initialized")))
}

#[derive(Deserialize)]
pub struct LimitQuery {
    pub limit: Option<usize>,
}

/// GET /api/evolution/status — Current evolution status
pub async fn get_evolution_status(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<serde_json::Value>> {
    let evo = get_engine(&state)?;
    let agent_id = &state.config.default_agent_id;
    let status = evo.get_status(agent_id).await
        .map_err(|e| AppError::Internal(e))?;
    Ok(Json(serde_json::to_value(status).unwrap_or_default()))
}

/// GET /api/evolution/generations — Generation history
pub async fn get_generation_history(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LimitQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let evo = get_engine(&state)?;
    let agent_id = &state.config.default_agent_id;
    let limit = query.limit.unwrap_or(50);
    let history = evo.get_generation_history(agent_id, limit).await
        .map_err(|e| AppError::Internal(e))?;
    Ok(Json(serde_json::to_value(history).unwrap_or_default()))
}

/// GET /api/evolution/generations/:n — Single generation record
pub async fn get_generation(
    State(state): State<Arc<AppState>>,
    Path(n): Path<u64>,
) -> AppResult<Json<serde_json::Value>> {
    let evo = get_engine(&state)?;
    let agent_id = &state.config.default_agent_id;
    let record = evo.get_generation(agent_id, n).await
        .map_err(|e| AppError::Internal(e))?;
    match record {
        Some(r) => Ok(Json(serde_json::to_value(r).unwrap_or_default())),
        None => Err(AppError::NotFound(format!("Generation {} not found", n))),
    }
}

/// GET /api/evolution/fitness — Fitness timeline
pub async fn get_fitness_timeline(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LimitQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let evo = get_engine(&state)?;
    let agent_id = &state.config.default_agent_id;
    let limit = query.limit.unwrap_or(100);
    let timeline = evo.get_fitness_timeline(agent_id, limit).await
        .map_err(|e| AppError::Internal(e))?;
    Ok(Json(serde_json::to_value(timeline).unwrap_or_default()))
}

/// GET /api/evolution/params — Get evolution parameters (auth required)
pub async fn get_evolution_params(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;
    let evo = get_engine(&state)?;
    let agent_id = &state.config.default_agent_id;
    let params = evo.get_params(agent_id).await
        .map_err(|e| AppError::Internal(e))?;
    Ok(Json(serde_json::to_value(params).unwrap_or_default()))
}

/// PUT /api/evolution/params — Update evolution parameters (auth required)
pub async fn update_evolution_params(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(params): Json<EvolutionParams>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    // Validate params
    if params.alpha <= 0.0 || params.alpha > 1.0 {
        return Err(AppError::Internal(anyhow::anyhow!("alpha must be in (0.0, 1.0]")));
    }
    if params.beta <= 0.0 || params.beta > 1.0 {
        return Err(AppError::Internal(anyhow::anyhow!("beta must be in (0.0, 1.0]")));
    }
    if params.theta_min < 0.0 || params.theta_min > 1.0 {
        return Err(AppError::Internal(anyhow::anyhow!("theta_min must be in [0.0, 1.0]")));
    }
    if params.gamma < 0.0 || params.gamma > 1.0 {
        return Err(AppError::Internal(anyhow::anyhow!("gamma must be in [0.0, 1.0]")));
    }

    let evo = get_engine(&state)?;
    let agent_id = &state.config.default_agent_id;
    evo.set_params(agent_id, &params).await
        .map_err(|e| AppError::Internal(e))?;

    info!(agent_id = %agent_id, "Evolution parameters updated via API");

    crate::db::spawn_audit_log(state.pool.clone(), crate::db::AuditLogEntry {
        timestamp: chrono::Utc::now(),
        event_type: "EVOLUTION_PARAMS_UPDATED".to_string(),
        actor_id: Some("admin".to_string()),
        target_id: Some(agent_id.to_string()),
        permission: None,
        result: "SUCCESS".to_string(),
        reason: "Evolution parameters updated via dashboard".to_string(),
        metadata: serde_json::to_value(&params).ok(),
        trace_id: None,
    });

    Ok(Json(serde_json::json!({ "status": "success" })))
}

/// GET /api/evolution/rollbacks — Rollback history
pub async fn get_rollback_history(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<serde_json::Value>> {
    let evo = get_engine(&state)?;
    let agent_id = &state.config.default_agent_id;
    let history = evo.get_rollback_history(agent_id).await
        .map_err(|e| AppError::Internal(e))?;
    Ok(Json(serde_json::to_value(history).unwrap_or_default()))
}
