use sqlx::SqlitePool;

use super::db_timeout;

/// Check if a command is trusted for a specific agent (exact match in DB).
pub async fn is_command_trusted(
    pool: &SqlitePool,
    agent_id: &str,
    full_command: &str,
) -> anyhow::Result<bool> {
    let query_future = sqlx::query_scalar::<_, i32>(
        "SELECT COUNT(*) FROM trusted_commands WHERE agent_id = ? AND pattern = ? AND pattern_type = 'exact'",
    )
    .bind(agent_id)
    .bind(full_command)
    .fetch_one(pool);

    let count = db_timeout(query_future).await?;
    Ok(count > 0)
}

/// Delete all trusted commands for an agent (used during agent deletion).
pub async fn delete_trusted_commands_for_agent(
    pool: &SqlitePool,
    agent_id: &str,
) -> anyhow::Result<u64> {
    let query_future = sqlx::query("DELETE FROM trusted_commands WHERE agent_id = ?")
        .bind(agent_id)
        .execute(pool);

    let result = db_timeout(query_future).await?;
    Ok(result.rows_affected())
}

/// Add a trusted command entry (exact match).
pub async fn add_trusted_command(
    pool: &SqlitePool,
    agent_id: &str,
    pattern: &str,
) -> anyhow::Result<()> {
    let query_future = sqlx::query(
        "INSERT OR IGNORE INTO trusted_commands (agent_id, pattern, pattern_type) VALUES (?, ?, 'exact')",
    )
    .bind(agent_id)
    .bind(pattern)
    .execute(pool);

    db_timeout(query_future).await?;
    Ok(())
}
