use async_trait::async_trait;
use chrono::{DateTime, Utc};
use exiv_shared::PluginDataStore;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::time::{timeout, Duration};
use tracing::info;

// Bug #7: Database operation timeout to prevent indefinite hangs
const DB_TIMEOUT_SECS: u64 = 10;

pub struct SqliteDataStore {
    pool: SqlitePool,
}

impl SqliteDataStore {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PluginDataStore for SqliteDataStore {
    async fn set_json(
        &self,
        plugin_id: &str,
        key: &str,
        value: serde_json::Value,
    ) -> anyhow::Result<()> {
        // Input validation
        if plugin_id.contains('\0') || plugin_id.len() > 255 {
            return Err(anyhow::anyhow!(
                "plugin_id must not contain null bytes and must be <= 255 chars"
            ));
        }
        if key.contains('\0') {
            return Err(anyhow::anyhow!("Key must not contain null bytes"));
        }
        if key.len() > 255 {
            return Err(anyhow::anyhow!(
                "Key exceeds maximum length (255 characters)"
            ));
        }

        let val_str = serde_json::to_string(&value)?;

        // Bug #7: Add timeout to prevent indefinite hangs on database locks
        let query_future = sqlx::query(
            "INSERT OR REPLACE INTO plugin_data (plugin_id, key, value) VALUES (?, ?, ?)",
        )
        .bind(plugin_id)
        .bind(key)
        .bind(val_str)
        .execute(&self.pool);

        // Bug A: Fixed error handling pattern - single ? operator after map_err
        timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
            .await
            .map_err(|_| {
                anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS)
            })?
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to save key '{}' for plugin '{}': {}",
                    key,
                    plugin_id,
                    e
                )
            })?;

        Ok(())
    }

    async fn get_json(
        &self,
        plugin_id: &str,
        key: &str,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        // Input validation
        if plugin_id.contains('\0') || plugin_id.len() > 255 {
            return Err(anyhow::anyhow!(
                "plugin_id must not contain null bytes and must be <= 255 chars"
            ));
        }
        if key.contains('\0') {
            return Err(anyhow::anyhow!("Key must not contain null bytes"));
        }
        if key.len() > 255 {
            return Err(anyhow::anyhow!(
                "Key exceeds maximum length (255 characters)"
            ));
        }

        // Bug #7: Add timeout to prevent indefinite hangs on database locks
        let query_future = sqlx::query_as::<_, (String,)>(
            "SELECT value FROM plugin_data WHERE plugin_id = ? AND key = ?",
        )
        .bind(plugin_id)
        .bind(key)
        .fetch_optional(&self.pool);

        // Bug A: Fixed error handling pattern - single ? operator after each map_err
        let row: Option<(String,)> = timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
            .await
            .map_err(|_| {
                anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS)
            })?
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to get key '{}' for plugin '{}': {}",
                    key,
                    plugin_id,
                    e
                )
            })?;

        if let Some((val_str,)) = row {
            let val = serde_json::from_str(&val_str)?;
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    async fn get_all_json(
        &self,
        plugin_id: &str,
        key_prefix: &str,
    ) -> anyhow::Result<Vec<(String, serde_json::Value)>> {
        // Input validation: prevent malicious characters
        if key_prefix.contains('\0') {
            return Err(anyhow::anyhow!("Key prefix must not contain null bytes"));
        }
        if key_prefix.len() > 255 {
            return Err(anyhow::anyhow!(
                "Key prefix exceeds maximum length (255 characters)"
            ));
        }

        // Escape LIKE special characters to prevent pattern injection
        let escaped_prefix = key_prefix.replace('%', "\\%").replace('_', "\\_");
        let pattern = format!("{}%", escaped_prefix);

        const DEFAULT_MAX_RESULTS: i64 = 1_000;

        // Bug #7: Add timeout to prevent indefinite hangs on database locks
        // Fetch DEFAULT_MAX_RESULTS + 1 to detect overflow without fetching all rows.
        let query_future = sqlx::query_as::<_, (String, String)>(
            "SELECT key, value FROM plugin_data WHERE plugin_id = ? AND key LIKE ? ESCAPE '\\' \
             ORDER BY key DESC LIMIT ?",
        )
        .bind(plugin_id)
        .bind(pattern)
        .bind(DEFAULT_MAX_RESULTS + 1)
        .fetch_all(&self.pool);

        // Bug A: Fixed error handling pattern - single ? operator after each map_err
        let mut rows: Vec<(String, String)> =
            timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
                .await
                .map_err(|_| {
                    anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS)
                })?
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to list keys with prefix '{}' for plugin '{}': {}",
                        key_prefix,
                        plugin_id,
                        e
                    )
                })?;

        if rows.len() > DEFAULT_MAX_RESULTS as usize {
            rows.truncate(DEFAULT_MAX_RESULTS as usize);
            tracing::warn!(
                plugin_id = %plugin_id,
                key_prefix = %key_prefix,
                limit = DEFAULT_MAX_RESULTS,
                "get_all_json: result set truncated to {} entries to prevent memory exhaustion",
                DEFAULT_MAX_RESULTS
            );
        }

        let mut results = Vec::new();
        for (key, val_str) in rows {
            let val = serde_json::from_str(&val_str)
                .map_err(|e| anyhow::anyhow!("Failed to parse JSON for key '{}': {}", key, e))?;
            results.push((key, val));
        }
        Ok(results)
    }

    /// Atomically increment a counter stored in `plugin_data`.
    ///
    /// Values are stored as TEXT (e.g., "1", "2") in the `value` column, matching
    /// the schema of `set_json` (which stores `serde_json::to_string` output).
    /// Both representations are compatible: `serde_json::from_str("1")` produces
    /// `Number(1)`, which `get_json` and `get_latest_generation` can parse as u64.
    async fn increment_counter(&self, plugin_id: &str, key: &str) -> anyhow::Result<i64> {
        if key.contains('\0') {
            return Err(anyhow::anyhow!("Key must not contain null bytes"));
        }
        if key.len() > 255 {
            return Err(anyhow::anyhow!(
                "Key exceeds maximum length (255 characters)"
            ));
        }

        // Atomic UPSERT: INSERT or UPDATE in a single SQL statement
        // The RETURNING clause gives us the new value without a second query
        let query_future = sqlx::query_as::<_, (String,)>(
            "INSERT INTO plugin_data (plugin_id, key, value) VALUES (?, ?, '1') \
             ON CONFLICT(plugin_id, key) DO UPDATE SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT) \
             RETURNING value"
        )
            .bind(plugin_id)
            .bind(key)
            .fetch_one(&self.pool);

        let (val_str,) = timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
            .await
            .map_err(|_| {
                anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS)
            })?
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to increment counter '{}' for plugin '{}': {}",
                    key,
                    plugin_id,
                    e
                )
            })?;

        val_str
            .parse::<i64>()
            .map_err(|e| anyhow::anyhow!("Failed to parse counter value '{}': {}", val_str, e))
    }
}

/// Proxy that restricts operations to a specific plugin ID (Security Guardrail)
pub struct ScopedDataStore {
    inner: Arc<dyn PluginDataStore>,
    plugin_id: String,
}

impl ScopedDataStore {
    pub fn new(inner: Arc<dyn PluginDataStore>, plugin_id: String) -> Self {
        Self { inner, plugin_id }
    }
}

#[async_trait]
impl PluginDataStore for ScopedDataStore {
    async fn set_json(
        &self,
        _plugin_id: &str,
        key: &str,
        value: serde_json::Value,
    ) -> anyhow::Result<()> {
        // Ignore the argument plugin_id and forcibly use our own ID
        self.inner.set_json(&self.plugin_id, key, value).await
    }

    async fn get_json(
        &self,
        _plugin_id: &str,
        key: &str,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        self.inner.get_json(&self.plugin_id, key).await
    }

    async fn get_all_json(
        &self,
        _plugin_id: &str,
        key_prefix: &str,
    ) -> anyhow::Result<Vec<(String, serde_json::Value)>> {
        self.inner.get_all_json(&self.plugin_id, key_prefix).await
    }

    async fn increment_counter(&self, _plugin_id: &str, key: &str) -> anyhow::Result<i64> {
        self.inner.increment_counter(&self.plugin_id, key).await
    }
}

pub async fn init_db(pool: &SqlitePool, database_url: &str) -> anyhow::Result<()> {
    info!("Running database migrations & seeds...");

    // Run migrations from migrations/ directory
    // Bug C: Wrap migration with timeout to prevent indefinite startup hangs (30s for schema changes)
    const MIGRATION_TIMEOUT_SECS: u64 = 30;
    let migration_future = sqlx::migrate!("./migrations").run(pool);
    timeout(
        Duration::from_secs(MIGRATION_TIMEOUT_SECS),
        migration_future,
    )
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "Database migrations timed out after {}s",
            MIGRATION_TIMEOUT_SECS
        )
    })?
    .map_err(|e| anyhow::anyhow!("Database migration failed: {}", e))?;

    info!("Applying runtime configurations...");

    // Configs that depend on runtime environment
    sqlx::query("INSERT OR REPLACE INTO plugin_configs (plugin_id, config_key, config_value) VALUES ('core.ks22', 'database_url', ?)")
        .bind(database_url)
        .execute(pool).await?;

    // API keys are NOT persisted to the database for security.
    // Plugins receive API keys at runtime via environment variables
    // through the config injection in PluginManager::initialize_all().

    Ok(())
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

/// Write an audit log entry to the database
pub async fn write_audit_log(pool: &SqlitePool, entry: AuditLogEntry) -> anyhow::Result<()> {
    let timestamp = entry.timestamp.to_rfc3339();
    let metadata_str = entry.metadata.map(|v| v.to_string());

    // Bug #7: Add timeout to prevent indefinite hangs on database locks
    let query_future = sqlx::query(
        "INSERT INTO audit_logs (timestamp, event_type, actor_id, target_id, permission, result, reason, metadata, trace_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
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
    .execute(pool);

    // Bug A: Fixed error handling pattern - single ? operator after each map_err
    timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
        .await
        .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
        .map_err(|e| anyhow::anyhow!("Database operation failed: {}", e))?;

    Ok(())
}

/// Spawn a background task to write an audit log entry with retry.
/// M-06: Retries up to 3 times with backoff instead of fire-and-forget.
pub fn spawn_audit_log(pool: SqlitePool, entry: AuditLogEntry) {
    tokio::spawn(async move {
        for attempt in 0..3u32 {
            match write_audit_log(&pool, entry.clone()).await {
                Ok(()) => return,
                Err(e) => {
                    tracing::error!(attempt = attempt + 1, "Failed to write audit log: {}", e);
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_millis(
                            100 * (u64::from(attempt) + 1),
                        ))
                        .await;
                    }
                }
            }
        }
        tracing::error!("Audit log entry permanently lost after 3 attempts");
    });
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

    // Bug A: Fixed error handling pattern - single ? operator after each map_err
    let rows = timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
        .await
        .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
        .map_err(|e| anyhow::anyhow!("Database query failed: {}", e))?;

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

/// Permission request entry for human-in-the-loop workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub request_id: String,
    pub created_at: DateTime<Utc>,
    pub plugin_id: String,
    pub permission_type: String,
    pub target_resource: Option<String>,
    pub justification: String,
    pub status: String,
    pub approved_by: Option<String>,
    pub approved_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub metadata: Option<serde_json::Value>,
}

/// Create a new permission request
pub async fn create_permission_request(
    pool: &SqlitePool,
    request: PermissionRequest,
) -> anyhow::Result<()> {
    let created_at = request.created_at.to_rfc3339();
    let expires_at = request.expires_at.map(|dt| dt.to_rfc3339());
    let metadata_str = request.metadata.map(|v| v.to_string());

    // Bug #7: Add timeout to prevent indefinite hangs on database locks
    let query_future = sqlx::query(
        "INSERT INTO permission_requests (request_id, created_at, plugin_id, permission_type, target_resource, justification, status, expires_at, metadata)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&request.request_id)
    .bind(&created_at)
    .bind(&request.plugin_id)
    .bind(&request.permission_type)
    .bind(&request.target_resource)
    .bind(&request.justification)
    .bind(&request.status)
    .bind(&expires_at)
    .bind(&metadata_str)
    .execute(pool);

    // Bug A: Fixed error handling pattern - single ? operator after each map_err
    timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
        .await
        .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
        .map_err(|e| anyhow::anyhow!("Database operation failed: {}", e))?;

    Ok(())
}

/// Query pending permission requests
pub async fn get_pending_permission_requests(
    pool: &SqlitePool,
) -> anyhow::Result<Vec<PermissionRequest>> {
    // Bug #7: Add timeout to prevent indefinite hangs on database locks
    #[allow(clippy::type_complexity)]
    let query_future = sqlx::query_as::<_, (String, String, String, String, Option<String>, String, String, Option<String>, Option<String>, Option<String>, Option<String>)>(
            "SELECT request_id, created_at, plugin_id, permission_type, target_resource, justification, status, approved_by, approved_at, expires_at, metadata
             FROM permission_requests
             WHERE status = 'pending'
             ORDER BY created_at DESC"
        )
        .fetch_all(pool);

    // Bug A: Fixed error handling pattern - single ? operator after each map_err
    let rows = timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
        .await
        .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
        .map_err(|e| anyhow::anyhow!("Database query failed: {}", e))?;

    let mut requests = Vec::new();
    for (
        request_id,
        created_at,
        plugin_id,
        permission_type,
        target_resource,
        justification,
        status,
        approved_by,
        approved_at,
        expires_at,
        metadata,
    ) in rows
    {
        requests.push(PermissionRequest {
            request_id,
            created_at: DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&Utc),
            plugin_id,
            permission_type,
            target_resource,
            justification,
            status,
            approved_by,
            approved_at: approved_at.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            expires_at: expires_at.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            metadata: metadata.and_then(|s| serde_json::from_str(&s).ok()),
        });
    }

    Ok(requests)
}

/// Update permission request status (approve/deny)
/// Only transitions from 'pending' status are allowed to prevent double-approval
pub async fn update_permission_request(
    pool: &SqlitePool,
    request_id: &str,
    status: &str,
    approved_by: &str,
) -> anyhow::Result<()> {
    // Whitelist allowed status transitions
    if !["approved", "denied"].contains(&status) {
        return Err(anyhow::anyhow!(
            "Invalid status value: '{}'. Must be 'approved' or 'denied'",
            status
        ));
    }

    let approved_at = Utc::now().to_rfc3339();

    // Bug #7: Add timeout to prevent indefinite hangs on database locks
    let query_future = sqlx::query(
        "UPDATE permission_requests
         SET status = ?, approved_by = ?, approved_at = ?
         WHERE request_id = ? AND status = 'pending'",
    )
    .bind(status)
    .bind(approved_by)
    .bind(&approved_at)
    .bind(request_id)
    .execute(pool);

    // Bug A: Fixed error handling pattern - single ? operator after each map_err
    let result = timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
        .await
        .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
        .map_err(|e| anyhow::anyhow!("Database operation failed: {}", e))?;

    if result.rows_affected() == 0 {
        return Err(anyhow::anyhow!(
            "Permission request '{}' not found or already processed",
            request_id
        ));
    }

    Ok(())
}

/// Check if a specific permission is already approved for a plugin/server.
/// Returns true if an approved, non-expired permission exists.
pub async fn is_permission_approved(
    pool: &SqlitePool,
    plugin_id: &str,
    permission_type: &str,
) -> anyhow::Result<bool> {
    let query_future = sqlx::query_scalar::<_, i32>(
        "SELECT COUNT(*) FROM permission_requests
         WHERE plugin_id = ? AND permission_type = ? AND status = 'approved'
           AND (expires_at IS NULL OR expires_at > datetime('now'))",
    )
    .bind(plugin_id)
    .bind(permission_type)
    .fetch_one(pool);

    let count = timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
        .await
        .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
        .map_err(|e| anyhow::anyhow!("Database query failed: {}", e))?;

    Ok(count > 0)
}

// ─── Chat Persistence Layer ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageRow {
    pub id: String,
    pub agent_id: String,
    pub user_id: String,
    pub source: String,
    pub content: String, // JSON string of ContentBlock[]
    pub metadata: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentRow {
    pub id: String,
    pub message_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub storage_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_data: Option<Vec<u8>>,
    pub disk_path: Option<String>,
    pub created_at: i64,
}

/// Save a chat message to the database
pub async fn save_chat_message(pool: &SqlitePool, msg: &ChatMessageRow) -> anyhow::Result<()> {
    let query_future = sqlx::query(
        "INSERT INTO chat_messages (id, agent_id, user_id, source, content, metadata, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&msg.id)
    .bind(&msg.agent_id)
    .bind(&msg.user_id)
    .bind(&msg.source)
    .bind(&msg.content)
    .bind(&msg.metadata)
    .bind(msg.created_at)
    .execute(pool);

    timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
        .await
        .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
        .map_err(|e| anyhow::anyhow!("Database operation failed: {}", e))?;

    Ok(())
}

/// Get chat messages with cursor-based pagination (ordered by created_at DESC)
pub async fn get_chat_messages(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: &str,
    before_ts: Option<i64>,
    limit: i64,
) -> anyhow::Result<Vec<ChatMessageRow>> {
    let limit = limit.min(200);

    let rows: Vec<(String, String, String, String, String, Option<String>, i64)> =
        if let Some(before) = before_ts {
            let query_future =
                sqlx::query_as::<_, (String, String, String, String, String, Option<String>, i64)>(
                    "SELECT id, agent_id, user_id, source, content, metadata, created_at
             FROM chat_messages
             WHERE agent_id = ? AND user_id = ? AND created_at < ?
             ORDER BY created_at DESC
             LIMIT ?",
                )
                .bind(agent_id)
                .bind(user_id)
                .bind(before)
                .bind(limit)
                .fetch_all(pool);

            timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
                .await
                .map_err(|_| {
                    anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS)
                })?
                .map_err(|e| anyhow::anyhow!("Database query failed: {}", e))?
        } else {
            let query_future =
                sqlx::query_as::<_, (String, String, String, String, String, Option<String>, i64)>(
                    "SELECT id, agent_id, user_id, source, content, metadata, created_at
             FROM chat_messages
             WHERE agent_id = ? AND user_id = ?
             ORDER BY created_at DESC
             LIMIT ?",
                )
                .bind(agent_id)
                .bind(user_id)
                .bind(limit)
                .fetch_all(pool);

            timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
                .await
                .map_err(|_| {
                    anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS)
                })?
                .map_err(|e| anyhow::anyhow!("Database query failed: {}", e))?
        };

    let messages = rows
        .into_iter()
        .map(
            |(id, agent_id, user_id, source, content, metadata, created_at)| ChatMessageRow {
                id,
                agent_id,
                user_id,
                source,
                content,
                metadata,
                created_at,
            },
        )
        .collect();

    Ok(messages)
}

/// Delete all chat messages (and cascade to attachments) for an agent/user pair
pub async fn delete_chat_messages(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: &str,
) -> anyhow::Result<u64> {
    // First get message IDs for disk attachment cleanup
    let ids_future = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM chat_messages WHERE agent_id = ? AND user_id = ?",
    )
    .bind(agent_id)
    .bind(user_id)
    .fetch_all(pool);

    let msg_ids: Vec<String> = timeout(Duration::from_secs(DB_TIMEOUT_SECS), ids_future)
        .await
        .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
        .map_err(|e| anyhow::anyhow!("Database query failed: {}", e))?
        .into_iter()
        .map(|(id,)| id)
        .collect();

    // Get disk paths for cleanup
    let disk_paths = get_disk_attachment_paths(pool, &msg_ids).await?;

    // Delete messages (attachments cascade via ON DELETE CASCADE)
    let delete_future = sqlx::query("DELETE FROM chat_messages WHERE agent_id = ? AND user_id = ?")
        .bind(agent_id)
        .bind(user_id)
        .execute(pool);

    let result = timeout(Duration::from_secs(DB_TIMEOUT_SECS), delete_future)
        .await
        .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
        .map_err(|e| anyhow::anyhow!("Database operation failed: {}", e))?;

    // Clean up disk files (best-effort)
    for path in disk_paths {
        let _ = tokio::fs::remove_file(&path).await;
    }

    Ok(result.rows_affected())
}

/// Save a chat attachment
pub async fn save_attachment(pool: &SqlitePool, att: &AttachmentRow) -> anyhow::Result<()> {
    let query_future = sqlx::query(
        "INSERT INTO chat_attachments (id, message_id, filename, mime_type, size_bytes, storage_type, inline_data, disk_path, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&att.id)
    .bind(&att.message_id)
    .bind(&att.filename)
    .bind(&att.mime_type)
    .bind(att.size_bytes)
    .bind(&att.storage_type)
    .bind(&att.inline_data)
    .bind(&att.disk_path)
    .bind(att.created_at)
    .execute(pool);

    timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
        .await
        .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
        .map_err(|e| anyhow::anyhow!("Database operation failed: {}", e))?;

    Ok(())
}

/// Get attachments for a specific message
pub async fn get_attachments_for_message(
    pool: &SqlitePool,
    message_id: &str,
) -> anyhow::Result<Vec<AttachmentRow>> {
    let query_future = sqlx::query_as::<_, (String, String, String, String, i64, String, Option<Vec<u8>>, Option<String>, i64)>(
        "SELECT id, message_id, filename, mime_type, size_bytes, storage_type, inline_data, disk_path, created_at
         FROM chat_attachments
         WHERE message_id = ?"
    )
    .bind(message_id)
    .fetch_all(pool);

    let rows = timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
        .await
        .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
        .map_err(|e| anyhow::anyhow!("Database query failed: {}", e))?;

    let attachments = rows
        .into_iter()
        .map(
            |(
                id,
                message_id,
                filename,
                mime_type,
                size_bytes,
                storage_type,
                inline_data,
                disk_path,
                created_at,
            )| {
                AttachmentRow {
                    id,
                    message_id,
                    filename,
                    mime_type,
                    size_bytes,
                    storage_type,
                    inline_data,
                    disk_path,
                    created_at,
                }
            },
        )
        .collect();

    Ok(attachments)
}

/// Get an attachment by ID
pub async fn get_attachment_by_id(
    pool: &SqlitePool,
    attachment_id: &str,
) -> anyhow::Result<Option<AttachmentRow>> {
    let query_future = sqlx::query_as::<_, (String, String, String, String, i64, String, Option<Vec<u8>>, Option<String>, i64)>(
        "SELECT id, message_id, filename, mime_type, size_bytes, storage_type, inline_data, disk_path, created_at
         FROM chat_attachments
         WHERE id = ?"
    )
    .bind(attachment_id)
    .fetch_optional(pool);

    let row = timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
        .await
        .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
        .map_err(|e| anyhow::anyhow!("Database query failed: {}", e))?;

    Ok(row.map(
        |(
            id,
            message_id,
            filename,
            mime_type,
            size_bytes,
            storage_type,
            inline_data,
            disk_path,
            created_at,
        )| {
            AttachmentRow {
                id,
                message_id,
                filename,
                mime_type,
                size_bytes,
                storage_type,
                inline_data,
                disk_path,
                created_at,
            }
        },
    ))
}

/// Helper: get disk paths for attachments belonging to given message IDs
async fn get_disk_attachment_paths(
    pool: &SqlitePool,
    message_ids: &[String],
) -> anyhow::Result<Vec<String>> {
    if message_ids.is_empty() {
        return Ok(vec![]);
    }
    // Build placeholders for IN clause
    let placeholders: Vec<&str> = message_ids.iter().map(|_| "?").collect();
    let sql = format!(
        "SELECT disk_path FROM chat_attachments WHERE message_id IN ({}) AND storage_type = 'disk' AND disk_path IS NOT NULL",
        placeholders.join(",")
    );

    let mut query = sqlx::query_as::<_, (String,)>(&sql);
    for id in message_ids {
        query = query.bind(id);
    }

    let rows = timeout(Duration::from_secs(DB_TIMEOUT_SECS), query.fetch_all(pool))
        .await
        .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
        .map_err(|e| anyhow::anyhow!("Database query failed: {}", e))?;

    Ok(rows.into_iter().map(|(path,)| path).collect())
}

// ============================================================
// MCP Dynamic Server Persistence
// ============================================================

#[derive(Debug, Clone)]
pub struct McpServerRecord {
    pub name: String,
    pub command: String,
    pub args: String,
    pub script_content: Option<String>,
    pub description: Option<String>,
    pub created_at: i64,
    pub is_active: bool,
}

pub async fn save_mcp_server(pool: &SqlitePool, record: &McpServerRecord) -> anyhow::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(DB_TIMEOUT_SECS), async {
        sqlx::query(
            "INSERT OR REPLACE INTO mcp_servers \
             (name, command, args, script_content, description, created_at, is_active) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&record.name)
        .bind(&record.command)
        .bind(&record.args)
        .bind(&record.script_content)
        .bind(&record.description)
        .bind(record.created_at)
        .bind(record.is_active)
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to save MCP server: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout saving MCP server"))?
}

pub async fn load_active_mcp_servers(pool: &SqlitePool) -> anyhow::Result<Vec<McpServerRecord>> {
    tokio::time::timeout(std::time::Duration::from_secs(DB_TIMEOUT_SECS), async {
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                Option<String>,
                Option<String>,
                i64,
                bool,
            ),
        >(
            "SELECT name, command, args, script_content, description, created_at, is_active \
             FROM mcp_servers WHERE is_active = 1 ORDER BY created_at ASC",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load MCP servers: {}", e))?;

        Ok(rows
            .into_iter()
            .map(
                |(name, command, args, script_content, description, created_at, is_active)| {
                    McpServerRecord {
                        name,
                        command,
                        args,
                        script_content,
                        description,
                        created_at,
                        is_active,
                    }
                },
            )
            .collect())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout loading MCP servers"))?
}

pub async fn deactivate_mcp_server(pool: &SqlitePool, name: &str) -> anyhow::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(DB_TIMEOUT_SECS), async {
        sqlx::query("UPDATE mcp_servers SET is_active = 0 WHERE name = ?")
            .bind(name)
            .execute(pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to deactivate MCP server: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout deactivating MCP server"))?
}

// ============================================================
// MCP Access Control (MCP_SERVER_UI_DESIGN.md §3)
// ============================================================

/// Access control entry for MCP tool-level permissions.
/// Maps to `mcp_access_control` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessControlEntry {
    pub id: Option<i64>,
    pub entry_type: String, // "capability" | "server_grant" | "tool_grant"
    pub agent_id: String,
    pub server_id: String,
    pub tool_name: Option<String>,
    pub permission: String, // "allow" | "deny"
    pub granted_by: Option<String>,
    pub granted_at: String,
    pub expires_at: Option<String>,
    pub justification: Option<String>,
    pub metadata: Option<String>,
}

/// Save a single access control entry.
pub async fn save_access_control_entry(
    pool: &SqlitePool,
    entry: &AccessControlEntry,
) -> anyhow::Result<i64> {
    let result = timeout(Duration::from_secs(DB_TIMEOUT_SECS), async {
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO mcp_access_control \
             (entry_type, agent_id, server_id, tool_name, permission, granted_by, granted_at, expires_at, justification, metadata) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING id",
        )
        .bind(&entry.entry_type)
        .bind(&entry.agent_id)
        .bind(&entry.server_id)
        .bind(&entry.tool_name)
        .bind(&entry.permission)
        .bind(&entry.granted_by)
        .bind(&entry.granted_at)
        .bind(&entry.expires_at)
        .bind(&entry.justification)
        .bind(&entry.metadata)
        .fetch_one(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to save access control entry: {}", e))
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout saving access control entry"))??;

    Ok(result)
}

/// Get all access control entries for a specific MCP server (tree view data).
pub async fn get_access_entries_for_server(
    pool: &SqlitePool,
    server_id: &str,
) -> anyhow::Result<Vec<AccessControlEntry>> {
    let rows = timeout(Duration::from_secs(DB_TIMEOUT_SECS), async {
        sqlx::query_as::<_, (i64, String, String, String, Option<String>, String, Option<String>, String, Option<String>, Option<String>, Option<String>)>(
            "SELECT id, entry_type, agent_id, server_id, tool_name, permission, granted_by, granted_at, expires_at, justification, metadata \
             FROM mcp_access_control WHERE server_id = ? ORDER BY agent_id, entry_type, tool_name",
        )
        .bind(server_id)
        .fetch_all(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load access entries: {}", e))
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout loading access entries"))??;

    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                entry_type,
                agent_id,
                server_id,
                tool_name,
                permission,
                granted_by,
                granted_at,
                expires_at,
                justification,
                metadata,
            )| {
                AccessControlEntry {
                    id: Some(id),
                    entry_type,
                    agent_id,
                    server_id,
                    tool_name,
                    permission,
                    granted_by,
                    granted_at,
                    expires_at,
                    justification,
                    metadata,
                }
            },
        )
        .collect())
}

/// Get all access control entries for a specific agent (by-agent view).
pub async fn get_access_entries_for_agent(
    pool: &SqlitePool,
    agent_id: &str,
) -> anyhow::Result<Vec<AccessControlEntry>> {
    let rows = timeout(Duration::from_secs(DB_TIMEOUT_SECS), async {
        sqlx::query_as::<_, (i64, String, String, String, Option<String>, String, Option<String>, String, Option<String>, Option<String>, Option<String>)>(
            "SELECT id, entry_type, agent_id, server_id, tool_name, permission, granted_by, granted_at, expires_at, justification, metadata \
             FROM mcp_access_control WHERE agent_id = ? ORDER BY server_id, entry_type, tool_name",
        )
        .bind(agent_id)
        .fetch_all(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load access entries for agent: {}", e))
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout loading access entries for agent"))??;

    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                entry_type,
                agent_id,
                server_id,
                tool_name,
                permission,
                granted_by,
                granted_at,
                expires_at,
                justification,
                metadata,
            )| {
                AccessControlEntry {
                    id: Some(id),
                    entry_type,
                    agent_id,
                    server_id,
                    tool_name,
                    permission,
                    granted_by,
                    granted_at,
                    expires_at,
                    justification,
                    metadata,
                }
            },
        )
        .collect())
}

/// Bulk update access control entries for a server.
/// Deletes all non-capability entries for the server and inserts the new ones in a transaction.
pub async fn put_access_entries(
    pool: &SqlitePool,
    server_id: &str,
    entries: &[AccessControlEntry],
) -> anyhow::Result<()> {
    timeout(Duration::from_secs(DB_TIMEOUT_SECS), async {
        let mut tx = pool.begin().await.map_err(|e| anyhow::anyhow!("Failed to begin transaction: {}", e))?;

        // Delete existing server_grant and tool_grant entries (preserve capability entries)
        sqlx::query(
            "DELETE FROM mcp_access_control WHERE server_id = ? AND entry_type IN ('server_grant', 'tool_grant')",
        )
        .bind(server_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to delete old access entries: {}", e))?;

        // Insert new entries
        for entry in entries {
            if entry.entry_type == "capability" {
                continue; // Don't overwrite capability entries via bulk update
            }
            sqlx::query(
                "INSERT INTO mcp_access_control \
                 (entry_type, agent_id, server_id, tool_name, permission, granted_by, granted_at, expires_at, justification, metadata) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&entry.entry_type)
            .bind(&entry.agent_id)
            .bind(&entry.server_id)
            .bind(&entry.tool_name)
            .bind(&entry.permission)
            .bind(&entry.granted_by)
            .bind(&entry.granted_at)
            .bind(&entry.expires_at)
            .bind(&entry.justification)
            .bind(&entry.metadata)
            .execute(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to insert access entry: {}", e))?;
        }

        tx.commit().await.map_err(|e| anyhow::anyhow!("Failed to commit transaction: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout updating access entries"))?
}

/// Resolve tool access for an agent.
/// Priority: tool_grant > server_grant > default_policy
pub async fn resolve_tool_access(
    pool: &SqlitePool,
    agent_id: &str,
    server_id: &str,
    tool_name: &str,
) -> anyhow::Result<String> {
    // 1. Check for explicit tool_grant
    let tool_grant = timeout(Duration::from_secs(DB_TIMEOUT_SECS), async {
        sqlx::query_scalar::<_, String>(
            "SELECT permission FROM mcp_access_control \
             WHERE agent_id = ? AND server_id = ? AND tool_name = ? AND entry_type = 'tool_grant' \
             AND (expires_at IS NULL OR expires_at > datetime('now')) \
             LIMIT 1",
        )
        .bind(agent_id)
        .bind(server_id)
        .bind(tool_name)
        .fetch_optional(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to check tool grant: {}", e))
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout checking tool grant"))??;

    if let Some(permission) = tool_grant {
        return Ok(permission);
    }

    // 2. Check for server_grant
    let server_grant = timeout(Duration::from_secs(DB_TIMEOUT_SECS), async {
        sqlx::query_scalar::<_, String>(
            "SELECT permission FROM mcp_access_control \
             WHERE agent_id = ? AND server_id = ? AND entry_type = 'server_grant' AND tool_name IS NULL \
             AND (expires_at IS NULL OR expires_at > datetime('now')) \
             LIMIT 1",
        )
        .bind(agent_id)
        .bind(server_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to check server grant: {}", e))
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout checking server grant"))??;

    if let Some(permission) = server_grant {
        return Ok(permission);
    }

    // 3. Fall back to server default_policy
    let policy = timeout(Duration::from_secs(DB_TIMEOUT_SECS), async {
        sqlx::query_scalar::<_, String>(
            "SELECT default_policy FROM mcp_servers WHERE name = ? LIMIT 1",
        )
        .bind(server_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to check default policy: {}", e))
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout checking default policy"))??;

    match policy.as_deref() {
        Some("opt-out") => Ok("allow".to_string()),
        _ => Ok("deny".to_string()), // opt-in = deny by default
    }
}

/// Get access summary for a server's tools (Summary Bar data).
/// Returns (tool_name, allowed_count, denied_count, inherited_count).
pub async fn get_access_summary(
    pool: &SqlitePool,
    server_id: &str,
) -> anyhow::Result<Vec<(String, i64, i64, i64)>> {
    // This query counts explicit grants per tool.
    // "inherited" means agents that have a server_grant but no tool_grant.
    let rows = timeout(Duration::from_secs(DB_TIMEOUT_SECS), async {
        sqlx::query_as::<_, (String, i64, i64)>(
            "SELECT tool_name, \
             SUM(CASE WHEN permission = 'allow' THEN 1 ELSE 0 END) as allowed, \
             SUM(CASE WHEN permission = 'deny' THEN 1 ELSE 0 END) as denied \
             FROM mcp_access_control \
             WHERE server_id = ? AND entry_type = 'tool_grant' AND tool_name IS NOT NULL \
             GROUP BY tool_name",
        )
        .bind(server_id)
        .fetch_all(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get access summary: {}", e))
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout getting access summary"))??;

    // Count agents with server_grant but no tool_grant (inherited)
    let server_grant_count = timeout(Duration::from_secs(DB_TIMEOUT_SECS), async {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(DISTINCT agent_id) FROM mcp_access_control \
             WHERE server_id = ? AND entry_type = 'server_grant'",
        )
        .bind(server_id)
        .fetch_one(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to count server grants: {}", e))
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout counting server grants"))??;

    Ok(rows
        .into_iter()
        .map(|(tool_name, allowed, denied)| {
            let explicit = allowed + denied;
            let inherited = (server_grant_count - explicit).max(0);
            (tool_name, allowed, denied, inherited)
        })
        .collect())
}

/// Get MCP server settings (including default_policy from the extended mcp_servers table).
pub async fn get_mcp_server_settings(
    pool: &SqlitePool,
    name: &str,
) -> anyhow::Result<Option<(McpServerRecord, String)>> {
    let result = timeout(Duration::from_secs(DB_TIMEOUT_SECS), async {
        sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, i64, bool, String)>(
            "SELECT name, command, args, script_content, description, created_at, is_active, default_policy \
             FROM mcp_servers WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get MCP server settings: {}", e))
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout getting MCP server settings"))??;

    Ok(result.map(
        |(
            name,
            command,
            args,
            script_content,
            description,
            created_at,
            is_active,
            default_policy,
        )| {
            (
                McpServerRecord {
                    name,
                    command,
                    args,
                    script_content,
                    description,
                    created_at,
                    is_active,
                },
                default_policy,
            )
        },
    ))
}

/// Update MCP server default_policy.
pub async fn update_mcp_server_default_policy(
    pool: &SqlitePool,
    name: &str,
    default_policy: &str,
) -> anyhow::Result<()> {
    timeout(Duration::from_secs(DB_TIMEOUT_SECS), async {
        let result = sqlx::query("UPDATE mcp_servers SET default_policy = ? WHERE name = ?")
            .bind(default_policy)
            .bind(name)
            .execute(pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to update default policy: {}", e))?;

        if result.rows_affected() == 0 {
            return Err(anyhow::anyhow!("MCP server '{}' not found", name));
        }
        Ok(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout updating default policy"))?
}

// ============================================================
// Revoked API Keys
// ============================================================

/// Compute a deterministic fingerprint of a key for revocation storage.
/// Uses DefaultHasher with a fixed salt (not crypto-grade, but sufficient
/// for revocation purposes on a local LAN-only dashboard).
pub fn hash_api_key(key: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::fmt::Write as _;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    key.len().hash(&mut h);
    b"exiv-revoke-salt-2026".hash(&mut h);
    let val = h.finish();
    let mut out = String::new();
    write!(out, "{:016x}{:016x}", val, val ^ 0xdeadbeef_cafebabe).unwrap();
    out
}

pub async fn revoke_api_key(pool: &SqlitePool, key: &str) -> anyhow::Result<()> {
    let key_hash = hash_api_key(key);
    let now = chrono::Utc::now().timestamp_millis();
    tokio::time::timeout(std::time::Duration::from_secs(DB_TIMEOUT_SECS), async {
        sqlx::query("INSERT OR IGNORE INTO revoked_keys (key_hash, revoked_at) VALUES (?, ?)")
            .bind(&key_hash)
            .bind(now)
            .execute(pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to revoke API key: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout revoking API key"))?
}

pub async fn load_revoked_key_hashes(pool: &SqlitePool) -> anyhow::Result<Vec<String>> {
    tokio::time::timeout(std::time::Duration::from_secs(DB_TIMEOUT_SECS), async {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT key_hash FROM revoked_keys")
            .fetch_all(pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load revoked key hashes: {}", e))?;
        Ok(rows.into_iter().map(|(h,)| h).collect())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout loading revoked keys"))?
}

// ── Agent Plugins ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AgentPluginRow {
    pub plugin_id: String,
    pub pos_x: i32,
    pub pos_y: i32,
}

/// Return the ordered plugin list for an agent.
pub async fn get_agent_plugins(
    pool: &SqlitePool,
    agent_id: &str,
) -> anyhow::Result<Vec<AgentPluginRow>> {
    let rows: Vec<(String, i32, i32)> = sqlx::query_as(
        "SELECT plugin_id, pos_x, pos_y FROM agent_plugins WHERE agent_id = ? ORDER BY pos_y, pos_x"
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(plugin_id, pos_x, pos_y)| AgentPluginRow {
            plugin_id,
            pos_x,
            pos_y,
        })
        .collect())
}

/// Replace an agent's entire plugin list atomically.
pub async fn set_agent_plugins(
    pool: &SqlitePool,
    agent_id: &str,
    plugins: &[(String, i32, i32)], // (plugin_id, pos_x, pos_y)
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM agent_plugins WHERE agent_id = ?")
        .bind(agent_id)
        .execute(&mut *tx)
        .await?;
    for (plugin_id, pos_x, pos_y) in plugins {
        sqlx::query(
            "INSERT INTO agent_plugins (agent_id, plugin_id, pos_x, pos_y) VALUES (?, ?, ?, ?)",
        )
        .bind(agent_id)
        .bind(plugin_id)
        .bind(pos_x)
        .bind(pos_y)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn is_api_key_revoked(pool: &SqlitePool, key: &str) -> anyhow::Result<bool> {
    let key_hash = hash_api_key(key);
    tokio::time::timeout(std::time::Duration::from_secs(DB_TIMEOUT_SECS), async {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT key_hash FROM revoked_keys WHERE key_hash = ?")
                .bind(&key_hash)
                .fetch_optional(pool)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to check revoked keys: {}", e))?;
        Ok(row.is_some())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout checking revoked keys"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_audit_log_roundtrip() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        init_db(&pool, "sqlite::memory:").await.unwrap();

        let entry = AuditLogEntry {
            timestamp: Utc::now(),
            event_type: "PERMISSION_GRANTED".to_string(),
            actor_id: Some("plugin.test".to_string()),
            target_id: Some("file.txt".to_string()),
            permission: Some("FileWrite".to_string()),
            result: "SUCCESS".to_string(),
            reason: "User approved".to_string(),
            metadata: Some(serde_json::json!({"approval_id": "123"})),
            trace_id: Some("trace-001".to_string()),
        };

        write_audit_log(&pool, entry.clone()).await.unwrap();

        let logs = query_audit_logs(&pool, 10).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].event_type, "PERMISSION_GRANTED");
        assert_eq!(logs[0].actor_id, Some("plugin.test".to_string()));
        assert_eq!(logs[0].permission, Some("FileWrite".to_string()));
        assert_eq!(logs[0].result, "SUCCESS");
    }

    #[tokio::test]
    async fn test_audit_log_ordering() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        init_db(&pool, "sqlite::memory:").await.unwrap();

        // Insert multiple entries
        for i in 1..=5 {
            let entry = AuditLogEntry {
                timestamp: Utc::now(),
                event_type: format!("EVENT_{}", i),
                actor_id: None,
                target_id: None,
                permission: None,
                result: "SUCCESS".to_string(),
                reason: format!("Test entry {}", i),
                metadata: None,
                trace_id: None,
            };
            write_audit_log(&pool, entry).await.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        let logs = query_audit_logs(&pool, 3).await.unwrap();
        assert_eq!(logs.len(), 3);
        // Most recent first
        assert_eq!(logs[0].event_type, "EVENT_5");
        assert_eq!(logs[1].event_type, "EVENT_4");
        assert_eq!(logs[2].event_type, "EVENT_3");
    }

    #[tokio::test]
    async fn test_permission_request_lifecycle() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        init_db(&pool, "sqlite::memory:").await.unwrap();

        // Create a permission request
        let request = PermissionRequest {
            request_id: "req-001".to_string(),
            created_at: Utc::now(),
            plugin_id: "test.plugin".to_string(),
            permission_type: "FileWrite".to_string(),
            target_resource: Some("/tmp/test.txt".to_string()),
            justification: "Need to write test results".to_string(),
            status: "pending".to_string(),
            approved_by: None,
            approved_at: None,
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            metadata: Some(serde_json::json!({"priority": "high"})),
        };

        create_permission_request(&pool, request).await.unwrap();

        // Query pending requests
        let pending = get_pending_permission_requests(&pool).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request_id, "req-001");
        assert_eq!(pending[0].status, "pending");

        // Approve the request
        update_permission_request(&pool, "req-001", "approved", "admin")
            .await
            .unwrap();

        // Verify no longer in pending list
        let pending_after = get_pending_permission_requests(&pool).await.unwrap();
        assert_eq!(pending_after.len(), 0);
    }

    #[tokio::test]
    async fn test_multiple_permission_requests() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        init_db(&pool, "sqlite::memory:").await.unwrap();

        // Create multiple requests
        for i in 1..=3 {
            let request = PermissionRequest {
                request_id: format!("req-{:03}", i),
                created_at: Utc::now(),
                plugin_id: format!("plugin.{}", i),
                permission_type: "NetworkAccess".to_string(),
                target_resource: Some(format!("https://api{}.example.com", i)),
                justification: format!("API call {}", i),
                status: "pending".to_string(),
                approved_by: None,
                approved_at: None,
                expires_at: None,
                metadata: None,
            };
            create_permission_request(&pool, request).await.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        let pending = get_pending_permission_requests(&pool).await.unwrap();
        assert_eq!(pending.len(), 3);
        // Most recent first
        assert_eq!(pending[0].request_id, "req-003");
        assert_eq!(pending[1].request_id, "req-002");
        assert_eq!(pending[2].request_id, "req-001");
    }
}
