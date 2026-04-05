use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use super::db_timeout;

/// Maximum number of audit log write retries.
const AUDIT_MAX_RETRIES: u32 = 3;

/// Base delay in milliseconds for audit log retry backoff.
const AUDIT_RETRY_BASE_MS: u64 = 100;

/// Audit log entry structure for security event tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogEntry {
    pub timestamp: DateTime<Utc>,
    pub event_type: String,
    pub actor_id: Option<String>,
    pub target_id: Option<String>,
    pub permission: Option<String>,
    pub result: String,
    pub reason: String,
    pub metadata: Option<serde_json::Value>,
    pub trace_id: Option<String>,
}

/// Compute the canonical data string for chain hashing.
fn canonical_data(timestamp: &str, entry: &AuditLogEntry) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        timestamp,
        entry.event_type,
        entry.actor_id.as_deref().unwrap_or(""),
        entry.target_id.as_deref().unwrap_or(""),
        entry.result,
    )
}

/// Compute a Merkle chain hash: SHA-256(previous_hash | canonical_data).
fn compute_chain_hash(previous: Option<&str>, data: &str) -> String {
    let mut hasher = Sha256::new();
    if let Some(prev) = previous {
        hasher.update(prev.as_bytes());
        hasher.update(b"|");
    }
    hasher.update(data.as_bytes());
    hex::encode(hasher.finalize())
}

/// Write an audit log entry to the database with Merkle chain hash.
pub async fn write_audit_log(pool: &SqlitePool, entry: AuditLogEntry) -> anyhow::Result<()> {
    let timeout_secs = super::db_timeout_secs();
    tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        write_audit_log_inner(pool, entry),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Audit log write timed out after {}s", timeout_secs))?
}

async fn write_audit_log_inner(pool: &SqlitePool, entry: AuditLogEntry) -> anyhow::Result<()> {
    let timestamp = entry.timestamp.to_rfc3339();
    let metadata_str = entry.metadata.as_ref().map(ToString::to_string);
    let data = canonical_data(&timestamp, &entry);

    // Single transaction: fetch previous hash + insert with new hash
    let mut tx = pool.begin().await?;

    let prev_hash: Option<String> =
        sqlx::query_scalar("SELECT chain_hash FROM audit_logs ORDER BY id DESC LIMIT 1")
            .fetch_optional(&mut *tx)
            .await?
            .flatten();

    let chain_hash = compute_chain_hash(prev_hash.as_deref(), &data);

    sqlx::query(
        "INSERT INTO audit_logs (timestamp, event_type, actor_id, target_id, permission, result, reason, metadata, trace_id, chain_hash)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&timestamp)
    .bind(&entry.event_type)
    .bind(&entry.actor_id)
    .bind(&entry.target_id)
    .bind(&entry.permission)
    .bind(&entry.result)
    .bind(&entry.reason)
    .bind(&metadata_str)
    .bind(&entry.trace_id)
    .bind(&chain_hash)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(())
}

/// Spawn a background task to write an audit log entry with retry.
/// M-06: Retries up to 3 times with backoff instead of fire-and-forget.
pub fn spawn_audit_log(pool: SqlitePool, entry: AuditLogEntry) {
    tokio::spawn(async move {
        for attempt in 0..AUDIT_MAX_RETRIES {
            match write_audit_log(&pool, entry.clone()).await {
                Ok(()) => return,
                Err(e) => {
                    tracing::error!(attempt = attempt + 1, "Failed to write audit log: {}", e);
                    if attempt < AUDIT_MAX_RETRIES - 1 {
                        tokio::time::sleep(std::time::Duration::from_millis(
                            AUDIT_RETRY_BASE_MS * (u64::from(attempt) + 1),
                        ))
                        .await;
                    }
                }
            }
        }
        tracing::error!(
            "Audit log entry permanently lost after {} attempts",
            AUDIT_MAX_RETRIES
        );
    });
}

/// Query audit logs since a given ID or timestamp (for MGP audit replay).
/// Returns `(id, AuditLogEntry)` tuples where `id` serves as the global seq.
pub async fn query_audit_logs_since(
    pool: &SqlitePool,
    since_id: Option<i64>,
    since_timestamp: Option<&str>,
    limit: i64,
) -> anyhow::Result<Vec<(i64, AuditLogEntry)>> {
    #[allow(clippy::type_complexity)]
    type Row = (
        i64,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );

    let rows: Vec<Row> = if let Some(sid) = since_id {
        db_timeout(
            sqlx::query_as::<_, Row>(
                "SELECT id, timestamp, event_type, actor_id, target_id, permission, result, reason, metadata, trace_id, chain_hash \
                 FROM audit_logs WHERE id > ? ORDER BY id ASC LIMIT ?"
            )
            .bind(sid)
            .bind(limit)
            .fetch_all(pool),
        )
        .await?
    } else if let Some(ts) = since_timestamp {
        db_timeout(
            sqlx::query_as::<_, Row>(
                "SELECT id, timestamp, event_type, actor_id, target_id, permission, result, reason, metadata, trace_id, chain_hash \
                 FROM audit_logs WHERE timestamp > ? ORDER BY timestamp ASC LIMIT ?"
            )
            .bind(ts)
            .bind(limit)
            .fetch_all(pool),
        )
        .await?
    } else {
        db_timeout(
            sqlx::query_as::<_, Row>(
                "SELECT id, timestamp, event_type, actor_id, target_id, permission, result, reason, metadata, trace_id, chain_hash \
                 FROM audit_logs ORDER BY id ASC LIMIT ?"
            )
            .bind(limit)
            .fetch_all(pool),
        )
        .await?
    };

    let mut logs = Vec::new();
    for (
        id,
        timestamp,
        event_type,
        actor,
        target,
        perm,
        result,
        reason,
        metadata,
        trace,
        _chain_hash,
    ) in rows
    {
        logs.push((
            id,
            AuditLogEntry {
                timestamp: DateTime::parse_from_rfc3339(&timestamp)?.with_timezone(&Utc),
                event_type,
                actor_id: actor,
                target_id: target,
                permission: perm,
                result,
                reason,
                metadata: metadata.and_then(|s| serde_json::from_str(&s).ok()),
                trace_id: trace,
            },
        ));
    }

    Ok(logs)
}

/// Query audit logs from the database (most recent first)
pub async fn query_audit_logs(pool: &SqlitePool, limit: i64) -> anyhow::Result<Vec<AuditLogEntry>> {
    // Bug #7: Add timeout to prevent indefinite hangs on database locks
    #[allow(clippy::type_complexity)]
    let query_future = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, Option<String>, String, String, Option<String>, Option<String>)>(
            "SELECT timestamp, event_type, actor_id, target_id, permission, result, reason, metadata, trace_id
             FROM audit_logs
             ORDER BY timestamp DESC
             LIMIT ?"
        )
        .bind(limit)
        .fetch_all(pool);

    let rows = db_timeout(query_future).await?;

    let mut logs = Vec::new();
    for (timestamp, event_type, actor, target, perm, result, reason, metadata, trace) in rows {
        logs.push(AuditLogEntry {
            timestamp: DateTime::parse_from_rfc3339(&timestamp)?.with_timezone(&Utc),
            event_type,
            actor_id: actor,
            target_id: target,
            permission: perm,
            result,
            reason,
            metadata: metadata.and_then(|s| serde_json::from_str(&s).ok()),
            trace_id: trace,
        });
    }

    Ok(logs)
}
