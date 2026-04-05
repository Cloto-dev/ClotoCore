use sqlx::SqlitePool;

use super::db_timeout;

// ── Cron Job Scheduler ──

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct CronJobRow {
    pub id: String,
    pub agent_id: String,
    pub name: String,
    pub enabled: bool,
    pub schedule_type: String,
    pub schedule_value: String,
    pub engine_id: Option<String>,
    pub message: String,
    pub next_run_at: i64,
    pub last_run_at: Option<i64>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub max_iterations: Option<i32>,
    pub created_at: String,
    pub hide_prompt: bool,
    pub cron_generation: i32,
    pub source_type: String,
    pub creator_user_id: Option<String>,
    pub creator_user_name: Option<String>,
}

pub async fn list_cron_jobs(pool: &SqlitePool) -> anyhow::Result<Vec<CronJobRow>> {
    let rows = db_timeout(sqlx::query_as::<_, CronJobRow>(
        "SELECT id, agent_id, name, enabled, schedule_type, schedule_value, engine_id, message, next_run_at, last_run_at, last_status, last_error, max_iterations, created_at, hide_prompt, cron_generation, source_type, creator_user_id, creator_user_name FROM cron_jobs ORDER BY created_at DESC"
    ).fetch_all(pool)).await?;
    Ok(rows)
}

pub async fn list_cron_jobs_for_agent(
    pool: &SqlitePool,
    agent_id: &str,
) -> anyhow::Result<Vec<CronJobRow>> {
    let rows = db_timeout(sqlx::query_as::<_, CronJobRow>(
        "SELECT id, agent_id, name, enabled, schedule_type, schedule_value, engine_id, message, next_run_at, last_run_at, last_status, last_error, max_iterations, created_at, hide_prompt, cron_generation, source_type, creator_user_id, creator_user_name FROM cron_jobs WHERE agent_id = ? ORDER BY created_at DESC"
    ).bind(agent_id).fetch_all(pool)).await?;
    Ok(rows)
}

pub async fn get_due_cron_jobs(pool: &SqlitePool, now_ms: i64) -> anyhow::Result<Vec<CronJobRow>> {
    let rows = db_timeout(sqlx::query_as::<_, CronJobRow>(
        "SELECT id, agent_id, name, enabled, schedule_type, schedule_value, engine_id, message, next_run_at, last_run_at, last_status, last_error, max_iterations, created_at, hide_prompt, cron_generation, source_type, creator_user_id, creator_user_name FROM cron_jobs WHERE enabled = 1 AND next_run_at <= ? ORDER BY next_run_at ASC"
    ).bind(now_ms).fetch_all(pool)).await?;
    Ok(rows)
}

pub async fn create_cron_job(pool: &SqlitePool, job: &CronJobRow) -> anyhow::Result<()> {
    db_timeout(sqlx::query(
        "INSERT INTO cron_jobs (id, agent_id, name, enabled, schedule_type, schedule_value, engine_id, message, next_run_at, max_iterations, hide_prompt, cron_generation, source_type, creator_user_id, creator_user_name) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&job.id)
    .bind(&job.agent_id)
    .bind(&job.name)
    .bind(job.enabled)
    .bind(&job.schedule_type)
    .bind(&job.schedule_value)
    .bind(&job.engine_id)
    .bind(&job.message)
    .bind(job.next_run_at)
    .bind(job.max_iterations)
    .bind(job.hide_prompt)
    .bind(job.cron_generation)
    .bind(&job.source_type)
    .bind(&job.creator_user_id)
    .bind(&job.creator_user_name)
    .execute(pool)).await?;
    Ok(())
}

pub async fn update_cron_job_run(
    pool: &SqlitePool,
    id: &str,
    last_run_at: i64,
    last_status: &str,
    last_error: Option<&str>,
    next_run_at: i64,
    enabled: bool,
) -> anyhow::Result<()> {
    db_timeout(sqlx::query(
        "UPDATE cron_jobs SET last_run_at = ?, last_status = ?, last_error = ?, next_run_at = ?, enabled = ? WHERE id = ?"
    )
    .bind(last_run_at)
    .bind(last_status)
    .bind(last_error)
    .bind(next_run_at)
    .bind(enabled)
    .bind(id)
    .execute(pool)).await?;
    Ok(())
}

pub async fn delete_cron_job(pool: &SqlitePool, id: &str) -> anyhow::Result<()> {
    let result = db_timeout(sqlx::query("DELETE FROM cron_jobs WHERE id = ?")
        .bind(id)
        .execute(pool)).await?;
    if result.rows_affected() == 0 {
        return Err(anyhow::anyhow!("Cron job '{}' not found", id));
    }
    Ok(())
}

pub async fn set_cron_job_enabled(
    pool: &SqlitePool,
    id: &str,
    enabled: bool,
) -> anyhow::Result<()> {
    let result = db_timeout(sqlx::query("UPDATE cron_jobs SET enabled = ? WHERE id = ?")
        .bind(enabled)
        .bind(id)
        .execute(pool)).await?;
    if result.rows_affected() == 0 {
        return Err(anyhow::anyhow!("Cron job '{}' not found", id));
    }
    Ok(())
}

pub async fn get_cron_job_generation(pool: &SqlitePool, job_id: &str) -> anyhow::Result<i32> {
    let row: (i32,) = db_timeout(sqlx::query_as("SELECT cron_generation FROM cron_jobs WHERE id = ?")
        .bind(job_id)
        .fetch_one(pool)).await?;
    Ok(row.0)
}
