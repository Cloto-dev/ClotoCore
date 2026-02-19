//! Evolution Engine API handlers for dashboard integration.

use axum::{
    extract::{Path, Query, State},
    Json,
    http::HeaderMap,
};
use serde::Deserialize;
use std::sync::Arc;
use tracing::info;

use crate::{AppState, AppResult, AppError, EnvelopedEvent};
use crate::evolution::{EvolutionParams, FitnessScores, AgentSnapshot, AutonomyLevel};

fn get_engine(state: &AppState) -> AppResult<&Arc<crate::evolution::EvolutionEngine>> {
    state.evolution_engine.as_ref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("Evolution engine not initialized")))
}

fn to_json<T: serde::Serialize>(value: &T) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::to_value(value)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Serialization failed: {}", e)))?))
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
        .map_err(AppError::Internal)?;
    to_json(&status)
}

/// GET /api/evolution/generations — Generation history
pub async fn get_generation_history(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LimitQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let evo = get_engine(&state)?;
    let agent_id = &state.config.default_agent_id;
    let limit = query.limit.unwrap_or(50).min(500);
    let history = evo.get_generation_history(agent_id, limit).await
        .map_err(AppError::Internal)?;
    to_json(&history)
}

/// GET /api/evolution/generations/:n — Single generation record
pub async fn get_generation(
    State(state): State<Arc<AppState>>,
    Path(n): Path<u64>,
) -> AppResult<Json<serde_json::Value>> {
    let evo = get_engine(&state)?;
    let agent_id = &state.config.default_agent_id;
    let record = evo.get_generation(agent_id, n).await
        .map_err(AppError::Internal)?;
    match record {
        Some(r) => to_json(&r),
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
    let limit = query.limit.unwrap_or(100).min(1000);
    let timeline = evo.get_fitness_timeline(agent_id, limit).await
        .map_err(AppError::Internal)?;
    to_json(&timeline)
}

/// GET /api/evolution/params — Get evolution parameters (read-only, no auth required)
pub async fn get_evolution_params(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<serde_json::Value>> {
    let evo = get_engine(&state)?;
    let agent_id = &state.config.default_agent_id;
    let params = evo.get_params(agent_id).await
        .map_err(AppError::Internal)?;
    to_json(&params)
}

/// PUT /api/evolution/params — Update evolution parameters (auth required)
pub async fn update_evolution_params(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(params): Json<EvolutionParams>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    // Validate params (NaN/Inf are rejected by is_finite checks)
    if !params.alpha.is_finite() || params.alpha <= 0.0 || params.alpha > 1.0 {
        return Err(AppError::Validation("alpha must be in (0.0, 1.0] and finite".to_string()));
    }
    if !params.beta.is_finite() || params.beta <= 0.0 || params.beta > 1.0 {
        return Err(AppError::Validation("beta must be in (0.0, 1.0] and finite".to_string()));
    }
    if !params.theta_min.is_finite() || params.theta_min < 0.0 || params.theta_min > 1.0 {
        return Err(AppError::Validation("theta_min must be in [0.0, 1.0] and finite".to_string()));
    }
    if !params.gamma.is_finite() || params.gamma < 0.0 || params.gamma > 1.0 {
        return Err(AppError::Validation("gamma must be in [0.0, 1.0] and finite".to_string()));
    }
    if params.min_interactions == 0 {
        return Err(AppError::Validation("min_interactions must be > 0".to_string()));
    }
    let w = &params.weights;
    let all_weights = [w.cognitive, w.behavioral, w.safety, w.autonomy, w.meta_learning];
    if all_weights.iter().any(|v| !v.is_finite()) {
        return Err(AppError::Validation("all weights must be finite numbers".to_string()));
    }
    if all_weights.iter().any(|v| *v < 0.0) {
        return Err(AppError::Validation("all weights must be non-negative".to_string()));
    }
    let weight_sum = w.cognitive + w.behavioral + w.safety + w.autonomy + w.meta_learning;
    if (weight_sum - 1.0).abs() > 0.01 {
        return Err(AppError::Validation(format!("weights must sum to ~1.0 (got {:.4})", weight_sum)));
    }

    let evo = get_engine(&state)?;
    let agent_id = &state.config.default_agent_id;
    evo.set_params(agent_id, &params).await
        .map_err(AppError::Internal)?;

    info!(agent_id = %agent_id, "Evolution parameters updated via API");

    crate::db::spawn_audit_log(state.pool.clone(), crate::db::AuditLogEntry {
        timestamp: chrono::Utc::now(),
        event_type: "EVOLUTION_PARAMS_UPDATED".to_string(),
        actor_id: Some("admin".to_string()), // TODO: extract from auth token when auth system supports user identification
        target_id: Some(agent_id.clone()),
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
        .map_err(AppError::Internal)?;
    to_json(&history)
}

// ── Evaluate endpoint ──

#[derive(Deserialize)]
pub struct EvaluateScores {
    pub cognitive: f64,
    pub behavioral: f64,
    pub safety: f64,
    pub autonomy: f64,
    pub meta_learning: f64,
}

#[derive(Deserialize)]
pub struct EvaluateRequest {
    pub scores: EvaluateScores,
    pub snapshot: Option<AgentSnapshot>,
}

/// Build an AgentSnapshot from the current PluginRegistry state.
async fn build_snapshot_from_registry(state: &AppState) -> AgentSnapshot {
    crate::events::build_snapshot_from_registry_inner(&state.registry).await
}

/// POST /api/evolution/evaluate — Submit fitness scores and trigger evaluation (auth required)
pub async fn evaluate_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<EvaluateRequest>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    // Validate score ranges
    for (name, val) in [
        ("cognitive", req.scores.cognitive),
        ("behavioral", req.scores.behavioral),
        ("safety", req.scores.safety),
        ("autonomy", req.scores.autonomy),
        ("meta_learning", req.scores.meta_learning),
    ] {
        if !val.is_finite() || !(0.0..=1.0).contains(&val) {
            return Err(AppError::Validation(
                format!("{} must be in [0.0, 1.0] and finite, got {}", name, val),
            ));
        }
    }

    let scores = FitnessScores {
        cognitive: req.scores.cognitive,
        behavioral: req.scores.behavioral,
        safety: req.scores.safety,
        autonomy: AutonomyLevel::from_normalized(req.scores.autonomy),
        meta_learning: req.scores.meta_learning,
    };

    let snapshot = match req.snapshot {
        Some(s) => s,
        None => build_snapshot_from_registry(&state).await,
    };

    let evo = get_engine(&state)?;
    let agent_id = &state.config.default_agent_id;

    let events = evo.evaluate(agent_id, scores, snapshot).await
        .map_err(AppError::Internal)?;

    // Dispatch events to the event bus for SSE subscribers
    let event_summaries: Vec<serde_json::Value> = events.iter().map(|e| {
        serde_json::to_value(e).unwrap_or_default()
    }).collect();

    for event_data in events {
        let envelope = EnvelopedEvent::system(event_data);
        let _ = state.event_tx.send(envelope).await;
    }

    info!(agent_id = %agent_id, event_count = event_summaries.len(), "Evolution evaluate called via API");

    Ok(Json(serde_json::json!({
        "status": "success",
        "events": event_summaries,
    })))
}
