use axum::{extract::State, Json};
use std::sync::Arc;
use tracing::info;

use crate::{AppError, AppResult, AppState};

use super::check_auth;

/// GET /api/cron/jobs[?agent_id=X]
#[allow(clippy::implicit_hasher)]
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
    .map_err(AppError::Internal)?;
    Ok(Json(
        serde_json::json!({ "jobs": jobs, "count": jobs.len() }),
    ))
}

/// POST /api/cron/jobs
pub async fn create_cron_job(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let agent_id = payload["agent_id"]
        .as_str()
        .ok_or_else(|| AppError::Validation("agent_id is required".into()))?;
    let name = payload["name"]
        .as_str()
        .ok_or_else(|| AppError::Validation("name is required".into()))?;
    let schedule_type = payload["schedule_type"].as_str().ok_or_else(|| {
        AppError::Validation("schedule_type is required (interval|cron|once)".into())
    })?;
    let schedule_value = payload["schedule_value"]
        .as_str()
        .ok_or_else(|| AppError::Validation("schedule_value is required".into()))?;
    let message = payload["message"]
        .as_str()
        .ok_or_else(|| AppError::Validation("message is required".into()))?;

    // Validate schedule and compute initial next_run_at
    let next_run_at =
        crate::managers::scheduler::calculate_initial_next_run(schedule_type, schedule_value)
            .map_err(|e| AppError::Validation(e.to_string()))?;

    // CRON recursion depth check: if called from within a cron execution context,
    // enforce generation limit (kernel auto-sets generation, ignoring any LLM-provided value)
    let cron_generation = if let Some(ctx) = state.active_cron_contexts.get(&agent_id.to_string()) {
        let child_gen = ctx.generation + 1;
        let max_gen = state
            .max_cron_generation
            .load(std::sync::atomic::Ordering::Relaxed) as i32;
        if child_gen > max_gen {
            return Err(AppError::Validation(format!(
                "CRON recursion depth limit exceeded (generation {} > max {})",
                child_gen, max_gen
            )));
        }
        child_gen
    } else {
        0
    };

    let job_id = format!("cron.{}.{}", agent_id, cloto_shared::ClotoId::new());
    let engine_id = payload["engine_id"].as_str().map(String::from);
    let max_iterations = payload["max_iterations"].as_i64().map(|v| v as i32);
    let hide_prompt = payload["hide_prompt"].as_bool().unwrap_or(false);

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
        hide_prompt,
        cron_generation,
    };

    crate::db::create_cron_job(&state.pool, &job)
        .await
        .map_err(AppError::Internal)?;

    info!(job_id = %job_id, agent_id = %agent_id, name = %name, "Cron job created");

    Ok(Json(
        serde_json::json!({ "id": job_id, "next_run_at": next_run_at }),
    ))
}

/// DELETE /api/cron/jobs/:id
pub async fn delete_cron_job(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    crate::db::delete_cron_job(&state.pool, &job_id)
        .await
        .map_err(|e| AppError::Validation(e.to_string()))?;
    info!(job_id = %job_id, "Cron job deleted");
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
    let enabled = payload["enabled"]
        .as_bool()
        .ok_or_else(|| AppError::Validation("enabled (bool) is required".into()))?;
    crate::db::set_cron_job_enabled(&state.pool, &job_id, enabled)
        .await
        .map_err(|e| AppError::Validation(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "status": "ok", "enabled": enabled }),
    ))
}

/// POST /api/cron/jobs/:id/run — trigger immediate execution
pub async fn run_cron_job_now(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    // Fetch the job
    let jobs = crate::db::list_cron_jobs(&state.pool)
        .await
        .map_err(AppError::Internal)?;
    let job = jobs
        .into_iter()
        .find(|j| j.id == job_id)
        .ok_or_else(|| AppError::NotFound(format!("Cron job '{}' not found", job_id)))?;

    // Build and dispatch the message immediately
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("target_agent_id".into(), job.agent_id.clone());
    metadata.insert("cron_job_id".into(), job.id.clone());
    metadata.insert("cron_source".into(), "manual".into());
    if let Some(ref engine_id) = job.engine_id {
        metadata.insert("engine_override".into(), engine_id.clone());
    }
    if job.hide_prompt {
        metadata.insert("skip_user_persist".into(), "true".into());
    }

    let msg = cloto_shared::ClotoMessage {
        id: cloto_shared::ClotoId::new().to_string(),
        source: cloto_shared::MessageSource::System,
        target_agent: Some(job.agent_id.clone()),
        content: job.message.clone(),
        timestamp: chrono::Utc::now(),
        metadata,
    };

    let envelope =
        crate::EnvelopedEvent::system(cloto_shared::ClotoEventData::MessageReceived(msg));

    state
        .event_tx
        .send(envelope)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to dispatch: {}", e)))?;

    info!(job_id = %job_id, "Cron job manually triggered");

    // Audit log for hidden-prompt cron jobs (observability guarantee)
    if job.hide_prompt {
        super::spawn_admin_audit(
            state.pool.clone(),
            "CRON_HIDDEN_DISPATCH",
            job.agent_id.clone(),
            format!("Cron job '{}' manually dispatched with hide_prompt", job.name),
            None,
            Some(serde_json::json!({
                "job_id": job.id,
                "message": job.message,
                "generation": job.cron_generation,
                "source": "manual",
            })),
            None,
        );
    }

    Ok(Json(serde_json::json!({ "status": "dispatched" })))
}
