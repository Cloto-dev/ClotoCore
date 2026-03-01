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
        if let Some(ref engine_id) = job.engine_id {
            metadata.insert("engine_override".into(), engine_id.clone());
        }
        if let Some(max_iter) = job.max_iterations {
            metadata.insert("max_iterations_override".into(), max_iter.to_string());
        }

        let msg = ClotoMessage {
            id: ClotoId::new().to_string(),
            source: MessageSource::System,
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
            db::update_cron_job_run(
                pool,
                &job.id,
                now_ms,
                "error",
                Some(&e.to_string()),
                job.next_run_at,
                job.enabled,
            )
            .await
            .ok();
            continue;
        }

        info!(
            job_id = %job.id,
            agent_id = %job.agent_id,
            name = %job.name,
            "Cron job dispatched"
        );

        // Calculate next run and update job status
        let (next_run, still_enabled) = calculate_next_run(job, now_ms);
        db::update_cron_job_run(
            pool,
            &job.id,
            now_ms,
            "success",
            None,
            next_run,
            still_enabled,
        )
        .await
        .ok();
    }

    Ok(())
}

/// Calculate the next run time for a cron job.
/// Returns (next_run_at_ms, enabled).
fn calculate_next_run(job: &CronJobRow, now_ms: i64) -> (i64, bool) {
    match job.schedule_type.as_str() {
        "interval" => {
            let interval_secs: u64 = job.schedule_value.parse().unwrap_or(3600);
            let next = now_ms + (interval_secs as i64 * 1000);
            (next, true)
        }
        "once" => {
            // One-shot: disable after execution
            (i64::MAX, false)
        }
        "cron" => match cron::Schedule::from_str(&job.schedule_value) {
            Ok(schedule) => match schedule.upcoming(Utc).next() {
                Some(next_time) => (next_time.timestamp_millis(), true),
                None => {
                    warn!(job_id = %job.id, "Cron expression has no future occurrences");
                    (i64::MAX, false)
                }
            },
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
            if interval_secs < 60 {
                return Err(anyhow::anyhow!("Minimum interval is 60 seconds"));
            }
            Ok(now_ms + (interval_secs as i64 * 1000))
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
