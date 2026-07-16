use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use sqlx::SqlitePool;
use tokio::sync::{mpsc, Notify};
use tracing::{debug, error, info, warn};

use cloto_shared::{ClotoEvent, ClotoEventData, ClotoId, ClotoMessage, MessageSource};

use crate::db::{self, CronJobRow};
use crate::EnvelopedEvent;

/// Upper bound for an `interval` cron schedule, in seconds (1 year).
///
/// bug-405: without a cap, a large `schedule_value` makes `interval_secs * 1000`
/// overflow `i64`, wrapping `next_run_at` to a past/garbage value so the job is
/// perpetually "due" and re-dispatches a full agentic loop every tick (a
/// self-inflicted availability/cost DoS).
const MAX_INTERVAL_SECS: u64 = 365 * 24 * 60 * 60;

/// Minimum interval between recurring runs, in seconds.
const MIN_INTERVAL_SECS: u64 = 60;

/// Spawn the cron scheduler background task.
///
/// Every `check_interval_secs` seconds, queries `cron_jobs` for due jobs
/// and dispatches them as `MessageReceived` events through the existing
/// agentic loop pipeline.
pub fn spawn_cron_task(
    pool: SqlitePool,
    event_tx: mpsc::Sender<EnvelopedEvent>,
    check_interval_secs: u64,
    shutdown: Arc<Notify>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(check_interval_secs));
        info!(
            "Cron scheduler started (check interval: {}s)",
            check_interval_secs
        );

        loop {
            tokio::select! {
                () = shutdown.notified() => {
                    info!("Cron scheduler shutting down");
                    break;
                }
                _ = interval.tick() => {
                    if let Err(e) = tick(&pool, &event_tx).await {
                        error!("Cron scheduler tick error: {}", e);
                    }
                }
            }
        }
    });
}

#[allow(clippy::too_many_lines)]
async fn tick(pool: &SqlitePool, event_tx: &mpsc::Sender<EnvelopedEvent>) -> anyhow::Result<()> {
    let now_ms = Utc::now().timestamp_millis();
    let due_jobs = db::get_due_cron_jobs(pool, now_ms).await?;

    if due_jobs.is_empty() {
        return Ok(());
    }

    debug!("Cron scheduler: {} due job(s)", due_jobs.len());

    for job in &due_jobs {
        // Build a synthetic ClotoMessage to feed into the existing agentic loop
        let mut metadata = HashMap::new();
        metadata.insert("target_agent_id".into(), job.agent_id.clone());
        metadata.insert("cron_job_id".into(), job.id.clone());
        metadata.insert("cron_source".into(), "scheduler".into());
        metadata.insert("cron_job_name".into(), format!("CRON: {}", job.name));
        let dialogue_id = format!("cron-{}", job.id);
        if !job.hide_prompt {
            metadata.insert("cron_dialogue_id".into(), dialogue_id.clone());
        }
        if let Some(ref engine_id) = job.engine_id {
            metadata.insert("engine_override".into(), engine_id.clone());
        }
        if let Some(max_iter) = job.max_iterations {
            metadata.insert("max_iterations_override".into(), max_iter.to_string());
        }
        if job.hide_prompt {
            metadata.insert("skip_user_persist".into(), "true".into());
        }

        let source = match job.source_type.as_str() {
            "user" => MessageSource::User {
                id: job
                    .creator_user_id
                    .clone()
                    .unwrap_or_else(|| "default".into()),
                name: job
                    .creator_user_name
                    .clone()
                    .unwrap_or_else(|| "User".into()),
            },
            _ => MessageSource::System,
        };

        let msg = ClotoMessage {
            id: ClotoId::new().to_string(),
            source,
            target_agent: Some(job.agent_id.clone()),
            content: job.message.clone(),
            timestamp: Utc::now(),
            metadata,
        };

        let envelope = EnvelopedEvent {
            event: Arc::new(ClotoEvent::new(ClotoEventData::MessageReceived(msg))),
            issuer: None,
            correlation_id: None,
            depth: 0,
        };

        if let Err(e) = event_tx.send(envelope).await {
            error!("Cron scheduler: failed to dispatch job '{}': {}", job.id, e);
            // bug-467: log rather than silently drop the status-write failure.
            if let Err(db_err) = db::update_cron_job_run(
                pool,
                &job.id,
                now_ms,
                "error",
                Some(&e.to_string()),
                job.next_run_at,
                job.enabled,
            )
            .await
            {
                tracing::error!(
                    job_id = %job.id,
                    error = %db_err,
                    "Failed to persist cron job error-state"
                );
            }
            continue;
        }

        info!(
            job_id = %job.id,
            agent_id = %job.agent_id,
            name = %job.name,
            "Cron job dispatched"
        );

        // Emit AgentDialogue "pending" for Actions panel (skip hidden-prompt jobs)
        if !job.hide_prompt {
            let data = ClotoEventData::AgentDialogue {
                dialogue_id: dialogue_id.clone(),
                caller_agent_id: "system.cron".to_string(),
                caller_agent_name: format!("CRON: {}", job.name),
                target_agent_id: job.agent_id.clone(),
                target_agent_name: job.agent_id.clone(),
                prompt: job.message.clone(),
                engine_id: job.engine_id.clone().unwrap_or_default(),
                response: None,
                chain_depth: 0,
                status: "pending".to_string(),
            };
            let _ = event_tx.send(EnvelopedEvent::system(data)).await;
        }

        // Audit log for hidden-prompt cron jobs (observability guarantee)
        if job.hide_prompt {
            crate::db::spawn_audit_log(
                pool.clone(),
                crate::db::AuditLogEntry {
                    timestamp: Utc::now(),
                    event_type: "CRON_HIDDEN_DISPATCH".into(),
                    actor_id: Some("scheduler".into()),
                    target_id: Some(job.agent_id.clone()),
                    permission: None,
                    result: "dispatched".into(),
                    reason: format!("Cron job '{}' dispatched with hide_prompt", job.name),
                    metadata: Some(serde_json::json!({
                        "job_id": job.id,
                        "message": job.message,
                        "generation": job.cron_generation,
                    })),
                    trace_id: None,
                },
            );
        }

        // Calculate next run and mark as dispatched.
        // The final status ("success" / "error" / "skipped") is written by
        // `handle_message` once the agentic loop resolves — "dispatched" here
        // only records that the event was successfully placed on the bus.
        let (next_run, still_enabled) = calculate_next_run(job, now_ms);
        // bug-467: this write is the ONLY thing that advances the job past its
        // due window — swallowing its error with `.ok()` lets a transient DB
        // failure leave a `once` job enabled (re-fires next tick) or a recurring
        // job stuck re-dispatching. Log it so the re-fire has a diagnostic trail.
        if let Err(e) = db::update_cron_job_run(
            pool,
            &job.id,
            now_ms,
            "dispatched",
            None,
            next_run,
            still_enabled,
        )
        .await
        {
            tracing::error!(
                job_id = %job.id,
                error = %e,
                "Failed to persist cron job run-state after dispatch — job may re-fire next tick"
            );
        }
    }

    Ok(())
}

/// Calculate the next run time for a cron job.
/// Returns (next_run_at_ms, enabled).
fn calculate_next_run(job: &CronJobRow, now_ms: i64) -> (i64, bool) {
    match job.schedule_type.as_str() {
        "interval" => {
            // bug-405: clamp + saturating arithmetic so a malformed/oversized
            // stored interval can't wrap `next_run_at` into the past.
            let interval_secs = job
                .schedule_value
                .parse::<u64>()
                .unwrap_or(3600)
                .clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS);
            let next = now_ms.saturating_add(interval_secs.cast_signed().saturating_mul(1000));
            (next, true)
        }
        "once" => {
            // One-shot: disable after execution
            (i64::MAX, false)
        }
        "cron" => match cron::Schedule::from_str(&job.schedule_value) {
            Ok(schedule) => {
                if let Some(next_time) = schedule.upcoming(Utc).next() {
                    (next_time.timestamp_millis(), true)
                } else {
                    warn!(job_id = %job.id, "Cron expression has no future occurrences");
                    (i64::MAX, false)
                }
            }
            Err(e) => {
                error!(job_id = %job.id, error = %e, "Invalid cron expression: {}", job.schedule_value);
                (i64::MAX, false)
            }
        },
        other => {
            error!(job_id = %job.id, "Unknown schedule type: {}", other);
            (i64::MAX, false)
        }
    }
}

/// Calculate the initial next_run_at for a new cron job.
pub fn calculate_initial_next_run(
    schedule_type: &str,
    schedule_value: &str,
) -> anyhow::Result<i64> {
    let now_ms = Utc::now().timestamp_millis();
    match schedule_type {
        "interval" => {
            let interval_secs: u64 = schedule_value
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid interval: must be seconds (integer)"))?;
            if interval_secs < MIN_INTERVAL_SECS {
                return Err(anyhow::anyhow!("Minimum interval is 60 seconds"));
            }
            // bug-405: reject oversized intervals up front so `* 1000` can't
            // overflow i64 and produce a perpetually-due job.
            if interval_secs > MAX_INTERVAL_SECS {
                return Err(anyhow::anyhow!(
                    "Maximum interval is {MAX_INTERVAL_SECS} seconds (1 year)"
                ));
            }
            Ok(now_ms.saturating_add(interval_secs.cast_signed().saturating_mul(1000)))
        }
        "once" => {
            let dt = chrono::DateTime::parse_from_rfc3339(schedule_value)
                .map_err(|e| anyhow::anyhow!("Invalid ISO 8601 datetime: {}", e))?;
            let target_ms = dt.timestamp_millis();
            if target_ms <= now_ms {
                return Err(anyhow::anyhow!("Scheduled time must be in the future"));
            }
            Ok(target_ms)
        }
        "cron" => {
            let schedule = cron::Schedule::from_str(schedule_value)
                .map_err(|e| anyhow::anyhow!("Invalid cron expression: {}", e))?;
            match schedule.upcoming(Utc).next() {
                Some(next) => Ok(next.timestamp_millis()),
                None => Err(anyhow::anyhow!("Cron expression has no future occurrences")),
            }
        }
        _ => Err(anyhow::anyhow!(
            "Unknown schedule_type: must be 'interval', 'cron', or 'once'"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval_job(value: &str) -> CronJobRow {
        CronJobRow {
            id: "job-1".to_string(),
            agent_id: "agent.test".to_string(),
            name: "t".to_string(),
            enabled: true,
            schedule_type: "interval".to_string(),
            schedule_value: value.to_string(),
            engine_id: None,
            message: String::new(),
            next_run_at: 0,
            last_run_at: None,
            last_status: None,
            last_error: None,
            max_iterations: None,
            created_at: String::new(),
            hide_prompt: false,
            cron_generation: 0,
            source_type: "user".to_string(),
            creator_user_id: None,
            creator_user_name: None,
        }
    }

    #[test]
    fn initial_interval_rejects_oversized_value() {
        // bug-405: an interval whose `* 1000` would overflow i64 must be rejected,
        // not wrapped into a perpetually-due past timestamp.
        let err = calculate_initial_next_run("interval", "99999999999999999")
            .expect_err("oversized interval must be rejected");
        assert!(err.to_string().contains("Maximum interval"));
    }

    #[test]
    fn initial_interval_rejects_too_small() {
        assert!(calculate_initial_next_run("interval", "30").is_err());
    }

    #[test]
    fn initial_interval_accepts_valid() {
        let now = Utc::now().timestamp_millis();
        let next = calculate_initial_next_run("interval", "3600").unwrap();
        assert!(next > now, "next run must be in the future");
    }

    #[test]
    fn recurring_interval_clamps_oversized_value() {
        // bug-405: even if a row with an oversized/garbage interval reaches the
        // recurring path, the next run must stay strictly in the future.
        let now = Utc::now().timestamp_millis();
        let (next, enabled) = calculate_next_run(&interval_job("99999999999999999"), now);
        assert!(enabled);
        assert!(
            next > now,
            "clamped next run must be in the future, got {next}"
        );
    }
}
