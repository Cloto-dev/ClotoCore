use sqlx::SqlitePool;
use tracing::info;
use async_trait::async_trait;
use std::sync::Arc;
use exiv_shared::PluginDataStore;
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};
use tokio::time::{timeout, Duration};

// Bug #7: Database operation timeout to prevent indefinite hangs
const DB_TIMEOUT_SECS: u64 = 10;

pub struct SqliteDataStore {
    pool: SqlitePool,
}

impl SqliteDataStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PluginDataStore for SqliteDataStore {
    async fn set_json(&self, plugin_id: &str, key: &str, value: serde_json::Value) -> anyhow::Result<()> {
        // Input validation
        if key.contains('\0') {
            return Err(anyhow::anyhow!("Key must not contain null bytes"));
        }
        if key.len() > 255 {
            return Err(anyhow::anyhow!("Key exceeds maximum length (255 characters)"));
        }

        let val_str = serde_json::to_string(&value)?;

        // Bug #7: Add timeout to prevent indefinite hangs on database locks
        let query_future = sqlx::query("INSERT OR REPLACE INTO plugin_data (plugin_id, key, value) VALUES (?, ?, ?)")
            .bind(plugin_id)
            .bind(key)
            .bind(val_str)
            .execute(&self.pool);

        // Bug A: Fixed error handling pattern - single ? operator after map_err
        timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
            .await
            .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
            .map_err(|e| anyhow::anyhow!("Database operation failed: {}", e))?;

        Ok(())
    }

    async fn get_json(&self, plugin_id: &str, key: &str) -> anyhow::Result<Option<serde_json::Value>> {
        // Input validation
        if key.contains('\0') {
            return Err(anyhow::anyhow!("Key must not contain null bytes"));
        }
        if key.len() > 255 {
            return Err(anyhow::anyhow!("Key exceeds maximum length (255 characters)"));
        }

        // Bug #7: Add timeout to prevent indefinite hangs on database locks
        let query_future = sqlx::query_as::<_, (String,)>("SELECT value FROM plugin_data WHERE plugin_id = ? AND key = ?")
            .bind(plugin_id)
            .bind(key)
            .fetch_optional(&self.pool);

        // Bug A: Fixed error handling pattern - single ? operator after each map_err
        let row: Option<(String,)> = timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
            .await
            .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
            .map_err(|e| anyhow::anyhow!("Database query failed: {}", e))?;

        if let Some((val_str,)) = row {
            let val = serde_json::from_str(&val_str)?;
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    async fn get_all_json(&self, plugin_id: &str, key_prefix: &str) -> anyhow::Result<Vec<(String, serde_json::Value)>> {
        // Input validation: prevent malicious characters
        if key_prefix.contains('\0') {
            return Err(anyhow::anyhow!("Key prefix must not contain null bytes"));
        }
        if key_prefix.len() > 255 {
            return Err(anyhow::anyhow!("Key prefix exceeds maximum length (255 characters)"));
        }

        // Escape LIKE special characters to prevent pattern injection
        let escaped_prefix = key_prefix.replace('%', "\\%").replace('_', "\\_");
        let pattern = format!("{}%", escaped_prefix);

        // Bug #7: Add timeout to prevent indefinite hangs on database locks
        let query_future = sqlx::query_as::<_, (String, String)>("SELECT key, value FROM plugin_data WHERE plugin_id = ? AND key LIKE ? ESCAPE '\\' ORDER BY key DESC")
            .bind(plugin_id)
            .bind(pattern)
            .fetch_all(&self.pool);

        // Bug A: Fixed error handling pattern - single ? operator after each map_err
        let rows: Vec<(String, String)> = timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
            .await
            .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
            .map_err(|e| anyhow::anyhow!("Database query failed: {}", e))?;

        let mut results = Vec::new();
        for (key, val_str) in rows {
            let val = serde_json::from_str(&val_str)
                .map_err(|e| anyhow::anyhow!("Failed to parse JSON for key '{}': {}", key, e))?;
            results.push((key, val));
        }
        Ok(results)
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
    async fn set_json(&self, _plugin_id: &str, key: &str, value: serde_json::Value) -> anyhow::Result<()> {
        // Ignore the argument plugin_id and forcibly use our own ID
        self.inner.set_json(&self.plugin_id, key, value).await
    }

    async fn get_json(&self, _plugin_id: &str, key: &str) -> anyhow::Result<Option<serde_json::Value>> {
        self.inner.get_json(&self.plugin_id, key).await
    }

    async fn get_all_json(&self, _plugin_id: &str, key_prefix: &str) -> anyhow::Result<Vec<(String, serde_json::Value)>> {
        self.inner.get_all_json(&self.plugin_id, key_prefix).await
    }
}

pub async fn init_db(pool: &SqlitePool, database_url: &str) -> anyhow::Result<()> {
    info!("Running database migrations & seeds...");

    // Run migrations from migrations/ directory
    // Bug C: Wrap migration with timeout to prevent indefinite startup hangs (30s for schema changes)
    const MIGRATION_TIMEOUT_SECS: u64 = 30;
    let migration_future = sqlx::migrate!("./migrations").run(pool);
    timeout(Duration::from_secs(MIGRATION_TIMEOUT_SECS), migration_future)
        .await
        .map_err(|_| anyhow::anyhow!("Database migrations timed out after {}s", MIGRATION_TIMEOUT_SECS))?
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
                    tracing::error!(
                        attempt = attempt + 1,
                        "Failed to write audit log: {}",
                        e
                    );
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_millis(
                            100 * (attempt as u64 + 1),
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
pub async fn query_audit_logs(
    pool: &SqlitePool,
    limit: i64,
) -> anyhow::Result<Vec<AuditLogEntry>> {
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
    for (request_id, created_at, plugin_id, permission_type, target_resource, justification, status, approved_by, approved_at, expires_at, metadata) in rows {
        requests.push(PermissionRequest {
            request_id,
            created_at: DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&Utc),
            plugin_id,
            permission_type,
            target_resource,
            justification,
            status,
            approved_by,
            approved_at: approved_at.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
            expires_at: expires_at.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
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
         WHERE request_id = ? AND status = 'pending'"
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

// ─── Chat Persistence Layer ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageRow {
    pub id: String,
    pub agent_id: String,
    pub user_id: String,
    pub source: String,
    pub content: String,        // JSON string of ContentBlock[]
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
         VALUES (?, ?, ?, ?, ?, ?, ?)"
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

    let rows: Vec<(String, String, String, String, String, Option<String>, i64)> = if let Some(before) = before_ts {
        let query_future = sqlx::query_as::<_, (String, String, String, String, String, Option<String>, i64)>(
            "SELECT id, agent_id, user_id, source, content, metadata, created_at
             FROM chat_messages
             WHERE agent_id = ? AND user_id = ? AND created_at < ?
             ORDER BY created_at DESC
             LIMIT ?"
        )
        .bind(agent_id)
        .bind(user_id)
        .bind(before)
        .bind(limit)
        .fetch_all(pool);

        timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
            .await
            .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
            .map_err(|e| anyhow::anyhow!("Database query failed: {}", e))?
    } else {
        let query_future = sqlx::query_as::<_, (String, String, String, String, String, Option<String>, i64)>(
            "SELECT id, agent_id, user_id, source, content, metadata, created_at
             FROM chat_messages
             WHERE agent_id = ? AND user_id = ?
             ORDER BY created_at DESC
             LIMIT ?"
        )
        .bind(agent_id)
        .bind(user_id)
        .bind(limit)
        .fetch_all(pool);

        timeout(Duration::from_secs(DB_TIMEOUT_SECS), query_future)
            .await
            .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", DB_TIMEOUT_SECS))?
            .map_err(|e| anyhow::anyhow!("Database query failed: {}", e))?
    };

    let messages = rows.into_iter().map(|(id, agent_id, user_id, source, content, metadata, created_at)| {
        ChatMessageRow { id, agent_id, user_id, source, content, metadata, created_at }
    }).collect();

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
        "SELECT id FROM chat_messages WHERE agent_id = ? AND user_id = ?"
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
    let delete_future = sqlx::query(
        "DELETE FROM chat_messages WHERE agent_id = ? AND user_id = ?"
    )
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

    let attachments = rows.into_iter().map(|(id, message_id, filename, mime_type, size_bytes, storage_type, inline_data, disk_path, created_at)| {
        AttachmentRow { id, message_id, filename, mime_type, size_bytes, storage_type, inline_data, disk_path, created_at }
    }).collect();

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

    Ok(row.map(|(id, message_id, filename, mime_type, size_bytes, storage_type, inline_data, disk_path, created_at)| {
        AttachmentRow { id, message_id, filename, mime_type, size_bytes, storage_type, inline_data, disk_path, created_at }
    }))
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
        update_permission_request(&pool, "req-001", "approved", "admin").await.unwrap();

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
