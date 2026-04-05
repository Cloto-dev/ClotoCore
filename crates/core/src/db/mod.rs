//! Database layer for ClotoCore kernel.
//!
//! SQLite-backed persistence split into domain modules: audit logging,
//! permissions, chat messages, MCP server state, API keys, CRON jobs,
//! LLM routing, and trusted commands.

pub mod api_keys;
pub mod audit;
pub mod chat;
pub mod cron;
pub mod health;
pub mod llm;
pub mod mcp;
pub mod permissions;
pub mod trusted_commands;

pub use api_keys::*;
pub use audit::*;
pub use chat::*;
pub use cron::*;
pub use llm::*;
pub use mcp::*;
pub use permissions::*;
pub use trusted_commands::*;

use async_trait::async_trait;
use cloto_shared::PluginDataStore;
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::time::{timeout, Duration};
use tracing::info;

// Bug #7: Database operation timeout to prevent indefinite hangs
const DEFAULT_DB_TIMEOUT_SECS: u64 = 10;
static DB_TIMEOUT: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

/// Set the database timeout (called once at startup from config).
pub fn set_db_timeout(secs: u64) {
    let _ = DB_TIMEOUT.set(secs);
}

pub(super) fn db_timeout_secs() -> u64 {
    *DB_TIMEOUT.get().unwrap_or(&DEFAULT_DB_TIMEOUT_SECS)
}

/// Execute a database operation with standard timeout and error handling.
/// Consolidates the repeated timeout+error pattern (bug-148).
pub(crate) async fn db_timeout<T, F>(future: F) -> anyhow::Result<T>
where
    F: std::future::Future<Output = Result<T, sqlx::Error>>,
{
    let secs = db_timeout_secs();
    timeout(Duration::from_secs(secs), future)
        .await
        .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", secs))?
        .map_err(|e| anyhow::anyhow!("Database operation failed: {}", e))
}

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

        db_timeout(query_future).await?;

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

        let row: Option<(String,)> = db_timeout(query_future).await?;

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

        let mut rows: Vec<(String, String)> = db_timeout(query_future).await?;

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

        let (val_str,) = db_timeout(query_future).await?;

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

pub async fn init_db(
    pool: &SqlitePool,
    database_url: &str,
    memory_plugin_id: &str,
) -> anyhow::Result<()> {
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
    sqlx::query("INSERT OR REPLACE INTO plugin_configs (plugin_id, config_key, config_value) VALUES (?, 'database_url', ?)")
        .bind(memory_plugin_id)
        .bind(database_url)
        .execute(pool).await?;

    // API keys are NOT persisted to the database for security.
    // Plugins receive API keys at runtime via environment variables
    // through the config injection in PluginManager::initialize_all().

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn test_audit_log_roundtrip() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        init_db(&pool, "sqlite::memory:", "memory.cpersona")
            .await
            .unwrap();

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
        init_db(&pool, "sqlite::memory:", "memory.cpersona")
            .await
            .unwrap();

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
        init_db(&pool, "sqlite::memory:", "memory.cpersona")
            .await
            .unwrap();

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
        init_db(&pool, "sqlite::memory:", "memory.cpersona")
            .await
            .unwrap();

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
