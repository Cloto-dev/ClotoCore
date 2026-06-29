use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use super::db_timeout;

/// Maximum number of audit log write retries.
const AUDIT_MAX_RETRIES: u32 = 3;

/// Base delay in milliseconds for audit log retry backoff.
const AUDIT_RETRY_BASE_MS: u64 = 100;

/// Process-lifetime counters so `/api/metrics` can expose whether the audit
/// pipeline is quietly losing entries. Today's cron incident was blind for
/// 14 hours because silent audit failures looked exactly like no events at
/// all — this counter turns that back into an observable signal.
static AUDIT_WRITES_OK: AtomicU64 = AtomicU64::new(0);
static AUDIT_WRITES_FAILED: AtomicU64 = AtomicU64::new(0);

/// Returns `(ok_count, failed_count)` for the audit log write pipeline.
#[must_use]
pub fn audit_write_counters() -> (u64, u64) {
    (
        AUDIT_WRITES_OK.load(Ordering::Relaxed),
        AUDIT_WRITES_FAILED.load(Ordering::Relaxed),
    )
}

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

    // Single transaction: fetch previous hash + insert with new hash.
    //
    // bug-412: take the write lock up front with BEGIN IMMEDIATE. The default
    // `pool.begin()` issues BEGIN DEFERRED, which starts read-only — the SELECT
    // below takes a read snapshot and the INSERT then upgrades read->write. Two
    // concurrent audit writers both snapshot the same tail, and the second's
    // upgrade fails with SQLITE_BUSY_SNAPSHOT: a snapshot conflict that
    // `busy_timeout` does NOT retry (it only waits out lock contention, not a
    // lost snapshot), so the row is silently dropped — the mechanism behind the
    // "audit silent for 14h" incident. BEGIN IMMEDIATE acquires the write lock
    // at transaction start, so concurrent writers serialize (waiting out
    // busy_timeout) instead of racing a read->write upgrade.
    //
    // bug-415: this MUST stay a sqlx `Transaction` (via begin_with), not a raw
    // `BEGIN IMMEDIATE` on a manually-acquired connection. `write_audit_log`
    // wraps this in a tokio timeout; if it cancels between BEGIN and COMMIT, the
    // Transaction's Drop queues a ROLLBACK so the connection returns to the pool
    // clean. A manual connection would be returned still holding the write lock
    // (sqlx's SQLite pool release does not roll back), wedging every later audit
    // write — exactly the blackout this code is meant to prevent.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

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
///
/// Every permanent loss is counted (see `audit_write_counters`) and logged
/// with the entry's `event_type` / `actor_id` / `target_id` so a failing
/// batch can be diagnosed from the kernel log even without the DB row.
pub fn spawn_audit_log(pool: SqlitePool, entry: AuditLogEntry) {
    tokio::spawn(async move {
        for attempt in 0..AUDIT_MAX_RETRIES {
            match write_audit_log(&pool, entry.clone()).await {
                Ok(()) => {
                    AUDIT_WRITES_OK.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(e) => {
                    tracing::error!(
                        attempt = attempt + 1,
                        event_type = %entry.event_type,
                        actor_id = entry.actor_id.as_deref().unwrap_or(""),
                        target_id = entry.target_id.as_deref().unwrap_or(""),
                        "Failed to write audit log: {}",
                        e
                    );
                    if attempt < AUDIT_MAX_RETRIES - 1 {
                        tokio::time::sleep(std::time::Duration::from_millis(
                            AUDIT_RETRY_BASE_MS * (u64::from(attempt) + 1),
                        ))
                        .await;
                    }
                }
            }
        }
        AUDIT_WRITES_FAILED.fetch_add(1, Ordering::Relaxed);
        tracing::error!(
            event_type = %entry.event_type,
            actor_id = entry.actor_id.as_deref().unwrap_or(""),
            target_id = entry.target_id.as_deref().unwrap_or(""),
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
                metadata: metadata.and_then(|s| match serde_json::from_str(&s) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        tracing::warn!(err = %e, "Audit log metadata is not valid JSON, dropping");
                        None
                    }
                }),
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
    use std::time::Duration;

    /// Build a temp FILE-backed pool that mirrors the production SQLite config
    /// (`lib.rs`): WAL journal + 10s busy_timeout. WAL is required to exercise
    /// the SQLITE_BUSY_SNAPSHOT path bug-412 targets — sqlx 0.8 does NOT default
    /// SQLite to WAL, so a plain pool would run in DELETE-journal mode where the
    /// bug cannot reproduce and the test would pass on unfixed code too. A FILE
    /// db (not `:memory:`) lets the multiple pool connections share one database.
    /// Returns `(pool, path)`; caller must `cleanup(pool, path)` when done.
    async fn wal_pool(tag: &str) -> (SqlitePool, std::path::PathBuf) {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let uniq = SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "cloto_audit_{}_{}_{}.db",
            tag,
            std::process::id(),
            uniq
        ));
        let _ = std::fs::remove_file(&path);
        let opts = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await
            .unwrap();
        crate::db::init_db(&pool, "test", None).await.unwrap();
        (pool, path)
    }

    async fn cleanup(pool: SqlitePool, path: std::path::PathBuf) {
        pool.close().await;
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    fn entry(tag: &str) -> AuditLogEntry {
        AuditLogEntry {
            timestamp: Utc::now(),
            event_type: format!("EVT_{tag}"),
            actor_id: Some("actor".into()),
            target_id: Some("target".into()),
            permission: Some("Perm".into()),
            result: "SUCCESS".into(),
            reason: "test".into(),
            metadata: None,
            trace_id: Some(format!("trace-{tag}")),
        }
    }

    /// bug-412: concurrent audit writes must all persist. Pre-fix, BEGIN
    /// DEFERRED let two writers snapshot the same tail and the second's
    /// read->write upgrade failed with SQLITE_BUSY_SNAPSHOT (which busy_timeout
    /// does not retry), silently dropping rows. BEGIN IMMEDIATE serializes
    /// writers so every row lands. Runs against a WAL file db (see `wal_pool`)
    /// so the bug's WAL-only snapshot path is actually exercised.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_audit_writes_all_persist() {
        let (pool, path) = wal_pool("bug412").await;

        const WRITERS: usize = 24;
        let mut handles = Vec::new();
        for i in 0..WRITERS {
            let pool = pool.clone();
            handles.push(tokio::spawn(async move {
                write_audit_log(&pool, entry(&format!("{i}"))).await
            }));
        }
        for h in handles {
            h.await
                .unwrap()
                .expect("each concurrent audit write must succeed");
        }

        let logs = query_audit_logs(&pool, 1000).await.unwrap();
        assert_eq!(
            logs.len(),
            WRITERS,
            "all concurrent audit writes must persist (none lost to BUSY_SNAPSHOT)"
        );

        cleanup(pool, path).await;
    }

    /// bug-415: cancelling an audit write mid-transaction must NOT poison the
    /// pool. `write_audit_log` runs under a tokio timeout; if it cancels between
    /// BEGIN IMMEDIATE and COMMIT, the sqlx `Transaction` Drop must roll back so
    /// the connection returns to the pool without holding the write lock. Pre-fix
    /// (manual acquire + raw BEGIN IMMEDIATE) the connection was returned still
    /// in a transaction, wedging every later write. Here we cancel many writes at
    /// a 1ns timeout, then assert normal writes still succeed and persist.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cancelled_write_does_not_poison_pool() {
        let (pool, path) = wal_pool("bug415").await;

        // Force cancellation mid-write: a 1ns timeout fires almost immediately,
        // dropping write_audit_log_inner's future (often after BEGIN, before
        // COMMIT). Repeat to make a hung write lock near-certain if Drop didn't
        // roll back.
        for i in 0..20 {
            let _ = tokio::time::timeout(
                Duration::from_nanos(1),
                write_audit_log(&pool, entry(&format!("cancel-{i}"))),
            )
            .await;
        }

        // The pool must still be usable: a normal write succeeds and persists.
        // If a cancelled write had left an open transaction, this would block on
        // the write lock and fail (or time out via the inner db timeout).
        write_audit_log(&pool, entry("after-cancel"))
            .await
            .expect("write after cancelled writes must succeed (pool not poisoned)");

        let logs = query_audit_logs(&pool, 1000).await.unwrap();
        assert!(
            logs.iter().any(|l| l.event_type == "EVT_after-cancel"),
            "the post-cancellation write must be durably persisted"
        );

        cleanup(pool, path).await;
    }
}
