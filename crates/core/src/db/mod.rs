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
use tracing::{info, warn};

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

/// bug-388: migration `20260524010000` renames `memory.cpersona` → `cpersona`
/// in `mcp_servers`. A pre-0.6.6 user who installed cpersona from the
/// marketplace before first booting a >=0.6.6 build has BOTH rows, so the
/// rename UPDATE collides with the primary key (`UNIQUE constraint failed:
/// mcp_servers.name`) and the kernel aborts at boot. The migration file is
/// checksummed by sqlx and cannot be amended, so the precondition is repaired
/// here instead: when the migration has not been applied yet and both rows
/// exist, move the legacy row's access grants onto `cpersona` (skipping
/// duplicates), dedup `plugin_configs` (its `(plugin_id, config_key)` primary
/// key collides on the migration's UPDATE the same way), and drop the legacy
/// rows — every statement in the migration then no-ops.
///
/// All probes tolerate a fresh database where the tables (or the sqlx
/// ledger) don't exist yet — nothing can collide there.
async fn repair_cpersona_rename_collision(pool: &SqlitePool) -> anyhow::Result<()> {
    let rename_applied: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM _sqlx_migrations WHERE version = 20260524010000")
            .fetch_optional(pool)
            .await
            .unwrap_or(None);
    if rename_applied.is_some() {
        return Ok(());
    }

    let both_rows: Option<i64> = sqlx::query_scalar(
        "SELECT 1 WHERE EXISTS (SELECT 1 FROM mcp_servers WHERE name = 'memory.cpersona') \
           AND EXISTS (SELECT 1 FROM mcp_servers WHERE name = 'cpersona')",
    )
    .fetch_optional(pool)
    .await
    .unwrap_or(None);
    if both_rows.is_none() {
        return Ok(());
    }

    warn!(
        "bug-388 precondition detected: both 'memory.cpersona' and 'cpersona' exist in \
         mcp_servers and the rename migration has not run yet — merging the legacy row \
         into 'cpersona' before migrations"
    );

    // 1. Re-point legacy grants at `cpersona`, skipping rows that already
    //    have an equivalent grant (mcp_access_control has no UNIQUE
    //    constraint, so dedup explicitly).
    sqlx::query(
        "INSERT INTO mcp_access_control \
             (entry_type, agent_id, server_id, tool_name, permission, granted_by, granted_at, \
              expires_at, justification, metadata) \
         SELECT entry_type, agent_id, 'cpersona', tool_name, permission, granted_by, granted_at, \
                expires_at, justification, metadata \
         FROM mcp_access_control AS legacy \
         WHERE legacy.server_id = 'memory.cpersona' \
           AND NOT EXISTS ( \
             SELECT 1 FROM mcp_access_control AS cur \
             WHERE cur.server_id = 'cpersona' \
               AND cur.entry_type = legacy.entry_type \
               AND cur.agent_id = legacy.agent_id \
               AND COALESCE(cur.tool_name, '') = COALESCE(legacy.tool_name, '') \
               AND cur.permission = legacy.permission)",
    )
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM mcp_access_control WHERE server_id = 'memory.cpersona'")
        .execute(pool)
        .await?;

    // 2. plugin_configs: drop legacy rows whose config_key already exists
    //    under `cpersona` (the migration's UPDATE renames the rest cleanly).
    if let Err(e) = sqlx::query(
        "DELETE FROM plugin_configs WHERE plugin_id = 'memory.cpersona' \
           AND config_key IN (SELECT config_key FROM plugin_configs WHERE plugin_id = 'cpersona')",
    )
    .execute(pool)
    .await
    {
        // plugin_configs may predate this shape in exotic DBs — non-fatal,
        // the migration's UPDATE will surface any real conflict.
        warn!("bug-388 repair: plugin_configs dedup skipped: {e}");
    }

    // 3. Drop the legacy server row itself.
    sqlx::query("DELETE FROM mcp_servers WHERE name = 'memory.cpersona'")
        .execute(pool)
        .await?;

    info!("bug-388 repair complete: legacy 'memory.cpersona' merged into 'cpersona'");
    Ok(())
}

pub async fn init_db(
    pool: &SqlitePool,
    database_url: &str,
    memory_plugin_id: Option<&str>,
) -> anyhow::Result<()> {
    info!("Running database migrations & seeds...");

    // bug-388: repair the memory.cpersona/cpersona collision before the
    // checksummed rename migration runs (it cannot be amended in place).
    repair_cpersona_rename_collision(pool).await?;

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

    // Legacy embedded-plugin seeding: when CLOTO_MEMORY_PLUGIN_ID is set,
    // mirror the database_url into plugin_configs so a process that boots
    // without dashboard-driven setup can still resolve the memory plugin's
    // backing store. With the kernel decoupled in 0.6.6, the modern path
    // (Setup Wizard / marketplace install) writes agent.metadata.preferred_memory
    // instead and skips this seed entirely.
    if let Some(plugin_id) = memory_plugin_id {
        sqlx::query("INSERT OR REPLACE INTO plugin_configs (plugin_id, config_key, config_value) VALUES (?, 'database_url', ?)")
            .bind(plugin_id)
            .bind(database_url)
            .execute(pool).await?;
    }

    // API keys are NOT persisted to the database for security.
    // Plugins receive API keys at runtime via environment variables
    // through the config injection in PluginManager::initialize_all().

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // ── repair_cpersona_rename_collision (bug-388) ──

    /// Minimal shapes of the four tables touched by migration
    /// `20260524010000` — enough for its statements to execute verbatim.
    async fn create_bug388_tables(pool: &SqlitePool) {
        sqlx::raw_sql(
            "CREATE TABLE mcp_servers (name TEXT PRIMARY KEY); \
             CREATE TABLE mcp_access_control ( \
                 id INTEGER PRIMARY KEY AUTOINCREMENT, \
                 entry_type TEXT NOT NULL, \
                 agent_id TEXT NOT NULL, \
                 server_id TEXT NOT NULL REFERENCES mcp_servers(name) ON DELETE CASCADE, \
                 tool_name TEXT, \
                 permission TEXT NOT NULL DEFAULT 'allow', \
                 granted_by TEXT, \
                 granted_at TEXT NOT NULL, \
                 expires_at TEXT, \
                 justification TEXT, \
                 metadata TEXT); \
             CREATE TABLE plugin_configs ( \
                 plugin_id TEXT, config_key TEXT, config_value TEXT, \
                 PRIMARY KEY(plugin_id, config_key)); \
             CREATE TABLE agents (id TEXT PRIMARY KEY, metadata TEXT);",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn bug388_repair_merges_legacy_row_and_rename_migration_applies() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        create_bug388_tables(&pool).await;
        // Pre-0.6.6 ledger state: no _sqlx_migrations table at all is the
        // strongest form of "rename not applied yet".
        sqlx::raw_sql(
            "INSERT INTO mcp_servers (name) VALUES ('memory.cpersona'), ('cpersona'); \
             INSERT INTO mcp_access_control (entry_type, agent_id, server_id, permission, granted_at) \
               VALUES ('server_grant', 'agent.a', 'memory.cpersona', 'allow', 't0'), \
                      ('server_grant', 'agent.b', 'memory.cpersona', 'allow', 't0'), \
                      ('server_grant', 'agent.a', 'cpersona', 'allow', 't1'); \
             INSERT INTO plugin_configs (plugin_id, config_key, config_value) \
               VALUES ('memory.cpersona', 'database_url', 'old'), \
                      ('cpersona', 'database_url', 'new'), \
                      ('memory.cpersona', 'only_legacy', 'x');",
        )
        .execute(&pool)
        .await
        .unwrap();

        repair_cpersona_rename_collision(&pool).await.unwrap();

        // The checksummed migration must now apply verbatim without a
        // UNIQUE collision.
        sqlx::raw_sql(include_str!(
            "../../migrations/20260524010000_rename_memory_cpersona_to_cpersona.sql"
        ))
        .execute(&pool)
        .await
        .expect("rename migration must apply cleanly after repair");

        let names: Vec<String> = sqlx::query_scalar("SELECT name FROM mcp_servers ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(names, vec!["cpersona".to_string()]);

        // agent.b's legacy-only grant moved over; agent.a's duplicate was
        // not doubled (2 pre-existing + 1 migrated + the migration's own
        // default grant for agent.cloto_default).
        let grants: Vec<(String, String)> =
            sqlx::query_as("SELECT agent_id, server_id FROM mcp_access_control ORDER BY agent_id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            grants,
            vec![
                ("agent.a".to_string(), "cpersona".to_string()),
                ("agent.b".to_string(), "cpersona".to_string()),
                ("agent.cloto_default".to_string(), "cpersona".to_string()),
            ]
        );

        let configs: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT plugin_id, config_key, config_value FROM plugin_configs ORDER BY config_key",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            configs,
            vec![
                ("cpersona".into(), "database_url".into(), "new".into()),
                ("cpersona".into(), "only_legacy".into(), "x".into()),
            ]
        );
    }

    #[tokio::test]
    async fn bug388_repair_noops_when_rename_already_applied() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        create_bug388_tables(&pool).await;
        sqlx::raw_sql(
            "CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY); \
             INSERT INTO _sqlx_migrations (version) VALUES (20260524010000); \
             INSERT INTO mcp_servers (name) VALUES ('memory.cpersona'), ('cpersona');",
        )
        .execute(&pool)
        .await
        .unwrap();

        repair_cpersona_rename_collision(&pool).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mcp_servers")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 2, "post-migration rows must not be touched");
    }

    #[tokio::test]
    async fn bug388_repair_tolerates_fresh_database() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        repair_cpersona_rename_collision(&pool).await.unwrap();
    }

    // ── 20260701120000_retire_mind_prefix (an earlier decision / an earlier decision) ──

    const MIND_PREFIX_MIGRATION: &str =
        include_str!("../../migrations/20260701120000_retire_mind_prefix.sql");

    /// Minimal shapes of the three tables the mind.-prefix migration touches,
    /// with the same `mcp_access_control.server_id → mcp_servers(name)` FK
    /// (ON DELETE CASCADE) that constrains the real rename.
    async fn create_mind_prefix_tables(pool: &SqlitePool) {
        sqlx::raw_sql(
            "CREATE TABLE mcp_servers (name TEXT PRIMARY KEY, command TEXT); \
             CREATE TABLE mcp_access_control ( \
                 id INTEGER PRIMARY KEY AUTOINCREMENT, \
                 entry_type TEXT NOT NULL, \
                 agent_id TEXT NOT NULL, \
                 server_id TEXT NOT NULL REFERENCES mcp_servers(name) ON DELETE CASCADE, \
                 tool_name TEXT, \
                 permission TEXT NOT NULL DEFAULT 'allow', \
                 granted_by TEXT, \
                 granted_at TEXT NOT NULL, \
                 expires_at TEXT, \
                 justification TEXT, \
                 metadata TEXT); \
             CREATE TABLE agents (id TEXT PRIMARY KEY, default_engine_id TEXT NOT NULL);",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn mind_prefix_migration_renames_builtins_and_merges_aliases() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        create_mind_prefix_tables(&pool).await;
        // Built-in mind.local/mind.ollama servers; catalog deepseek already bare;
        // an orphan wizard-alias server row (mind.deepseek) that shadows deepseek.
        sqlx::raw_sql(
            "INSERT INTO mcp_servers (name, command) VALUES \
               ('mind.local','x'),('mind.ollama','x'),('deepseek','x'), \
               ('cpersona','x'),('mind.deepseek','config-loaded'); \
             INSERT INTO mcp_access_control (entry_type, agent_id, server_id, tool_name, granted_at) VALUES \
               ('server_grant','agentA','mind.local',NULL,'t'), \
               ('server_grant','agentA','mind.deepseek',NULL,'t'), \
               ('server_grant','agentB','deepseek',NULL,'t'), \
               ('server_grant','agentB','mind.deepseek',NULL,'t'), \
               ('tool_grant','agentB','mind.local','think','t'), \
               ('server_grant','agentC','cpersona',NULL,'t'); \
             INSERT INTO agents (id, default_engine_id) VALUES \
               ('agentA','mind.deepseek'),('agentB','deepseek'),('agentC','mind.ollama');",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Applying twice must be idempotent.
        for _ in 0..2 {
            sqlx::raw_sql(MIND_PREFIX_MIGRATION)
                .execute(&pool)
                .await
                .expect("mind-prefix migration must apply cleanly");
        }

        let servers: Vec<String> = sqlx::query_scalar("SELECT name FROM mcp_servers ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(servers, vec!["cpersona", "deepseek", "local", "ollama"]);

        // mind.* grants de-prefixed; the duplicate mind.deepseek→deepseek merged
        // into the existing bare deepseek grant; the tool_grant kept its tool_name;
        // the non-engine cpersona grant is untouched.
        let grants: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT agent_id, server_id, tool_name FROM mcp_access_control \
             ORDER BY agent_id, server_id, IFNULL(tool_name,'')",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            grants,
            vec![
                ("agentA".into(), "deepseek".into(), None),
                ("agentA".into(), "local".into(), None),
                ("agentB".into(), "deepseek".into(), None),
                ("agentB".into(), "local".into(), Some("think".into())),
                ("agentC".into(), "cpersona".into(), None),
            ]
        );

        let engines: Vec<(String, String)> =
            sqlx::query_as("SELECT id, default_engine_id FROM agents ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            engines,
            vec![
                ("agentA".into(), "deepseek".into()),
                ("agentB".into(), "deepseek".into()),
                ("agentC".into(), "ollama".into()),
            ]
        );

        // No dangling FK references after the rename.
        let fk_violations: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(fk_violations.is_empty(), "FK must stay consistent");
    }

    #[tokio::test]
    async fn mind_prefix_migration_tolerates_fresh_database() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        create_mind_prefix_tables(&pool).await;
        sqlx::raw_sql(
            "INSERT INTO mcp_servers (name, command) VALUES ('deepseek','x'); \
             INSERT INTO agents (id, default_engine_id) VALUES ('a','deepseek');",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(MIND_PREFIX_MIGRATION)
            .execute(&pool)
            .await
            .expect("no-op on a DB with no mind.* rows");
        let servers: Vec<String> = sqlx::query_scalar("SELECT name FROM mcp_servers ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(servers, vec!["deepseek"]);
    }

    // ── 20260704130000_retire_category_prefixes (an earlier decision / an earlier decision) ──

    const CATEGORY_PREFIX_MIGRATION: &str =
        include_str!("../../migrations/20260704130000_retire_category_prefixes.sql");

    /// Minimal shapes of the tables the category-prefix migration touches,
    /// with the same FK as the real schema plus the cron_jobs residue target.
    async fn create_category_prefix_tables(pool: &SqlitePool) {
        sqlx::raw_sql(
            "CREATE TABLE mcp_servers (name TEXT PRIMARY KEY, command TEXT); \
             CREATE TABLE mcp_access_control ( \
                 id INTEGER PRIMARY KEY AUTOINCREMENT, \
                 entry_type TEXT NOT NULL, \
                 agent_id TEXT NOT NULL, \
                 server_id TEXT NOT NULL REFERENCES mcp_servers(name) ON DELETE CASCADE, \
                 tool_name TEXT, \
                 permission TEXT NOT NULL DEFAULT 'allow', \
                 granted_by TEXT, \
                 granted_at TEXT NOT NULL, \
                 expires_at TEXT, \
                 justification TEXT, \
                 metadata TEXT); \
             CREATE TABLE cron_jobs (id INTEGER PRIMARY KEY, engine_id TEXT);",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn category_prefix_migration_renames_merges_and_maps_gaze() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        create_category_prefix_tables(&pool).await;
        // Real-world shape (observed in a live dev DB): seeded tool.* rows,
        // marketplace-installed bare twins (terminal), the gaze special case,
        // the legacy orphan tool.research, and one of every other prefix.
        sqlx::raw_sql(
            "INSERT INTO mcp_servers (name, command) VALUES \
               ('tool.terminal','config-loaded'),('terminal','x'), \
               ('tool.cron','x'),('tool.research','config-loaded'), \
               ('vision.capture','x'),('vision.gaze_webcam','x'), \
               ('voice.stt','x'),('io.discord','x'),('output.avatar','x'), \
               ('cpersona','x'); \
             INSERT INTO mcp_access_control (entry_type, agent_id, server_id, tool_name, granted_at) VALUES \
               ('server_grant','agentA','tool.terminal',NULL,'t'), \
               ('server_grant','agentA','terminal',NULL,'t'), \
               ('server_grant','agentA','tool.cron',NULL,'t'), \
               ('server_grant','agentB','vision.gaze_webcam',NULL,'t'), \
               ('tool_grant','agentB','vision.capture','analyze_image','t'), \
               ('server_grant','agentC','cpersona',NULL,'t'); \
             INSERT INTO cron_jobs (id, engine_id) VALUES \
               (1,'mind.deepseek'),(2,'local'),(3,NULL);",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Applying twice must be idempotent.
        for _ in 0..2 {
            sqlx::raw_sql(CATEGORY_PREFIX_MIGRATION)
                .execute(&pool)
                .await
                .expect("category-prefix migration must apply cleanly");
        }

        let servers: Vec<String> = sqlx::query_scalar("SELECT name FROM mcp_servers ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(
            servers,
            vec![
                "avatar", "capture", "cpersona", "cron", "discord", "gaze", "research", "stt",
                "terminal"
            ]
        );

        // The tool.terminal grant merged into the existing bare terminal
        // grant; gaze got the explicit mapping; the tool_grant kept its
        // tool_name; the bare cpersona grant is untouched.
        let grants: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT agent_id, server_id, tool_name FROM mcp_access_control \
             ORDER BY agent_id, server_id, IFNULL(tool_name,'')",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            grants,
            vec![
                ("agentA".into(), "cron".into(), None),
                ("agentA".into(), "terminal".into(), None),
                (
                    "agentB".into(),
                    "capture".into(),
                    Some("analyze_image".into())
                ),
                ("agentB".into(), "gaze".into(), None),
                ("agentC".into(), "cpersona".into(), None),
            ]
        );

        // an earlier decision residue: cron_jobs.engine_id de-prefixed; bare and NULL
        // values untouched.
        let engines: Vec<(i64, Option<String>)> =
            sqlx::query_as("SELECT id, engine_id FROM cron_jobs ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            engines,
            vec![
                (1, Some("deepseek".into())),
                (2, Some("local".into())),
                (3, None),
            ]
        );

        let fk_violations: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(fk_violations.is_empty(), "FK must stay consistent");
    }

    #[tokio::test]
    async fn category_prefix_migration_tolerates_fresh_database() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        create_category_prefix_tables(&pool).await;
        sqlx::raw_sql(
            "INSERT INTO mcp_servers (name, command) VALUES ('terminal','x'); \
             INSERT INTO cron_jobs (id, engine_id) VALUES (1,'deepseek');",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(CATEGORY_PREFIX_MIGRATION)
            .execute(&pool)
            .await
            .expect("no-op on a DB with no prefixed rows");
        let servers: Vec<String> = sqlx::query_scalar("SELECT name FROM mcp_servers ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(servers, vec!["terminal"]);
    }

    #[tokio::test]
    async fn test_audit_log_roundtrip() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        init_db(&pool, "sqlite::memory:", None).await.unwrap();

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
        init_db(&pool, "sqlite::memory:", None).await.unwrap();

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
        init_db(&pool, "sqlite::memory:", None).await.unwrap();

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
        init_db(&pool, "sqlite::memory:", None).await.unwrap();

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
