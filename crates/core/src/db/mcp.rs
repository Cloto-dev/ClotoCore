use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::db_timeout;

// ============================================================
// Access Control Enums
// ============================================================

/// Type of access control entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EntryType {
    Capability,
    ServerGrant,
    ToolGrant,
}

/// Permission level for an access control entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PermissionLevel {
    Allow,
    Deny,
}

/// Default policy for an MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DefaultPolicy {
    OptIn,
    OptOut,
}

impl DefaultPolicy {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OptIn => "opt-in",
            Self::OptOut => "opt-out",
        }
    }

    #[must_use]
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "opt-out" => Self::OptOut,
            _ => Self::OptIn,
        }
    }

    #[must_use]
    pub fn default_permission(&self) -> PermissionLevel {
        match self {
            Self::OptOut => PermissionLevel::Allow,
            Self::OptIn => PermissionLevel::Deny,
        }
    }
}

/// Source of a chat message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageSource {
    User,
    Agent,
    System,
}

impl MessageSource {
    #[must_use]
    pub fn from_str_validated(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "agent" => Some(Self::Agent),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

// ============================================================
// MCP Dynamic Server Persistence
// ============================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct McpServerRecord {
    pub name: String,
    pub command: String,
    pub args: String,
    pub script_content: Option<String>,
    pub description: Option<String>,
    pub created_at: i64,
    pub is_active: bool,
    pub env: String, // JSON-serialized HashMap<String, String>
}

pub async fn save_mcp_server(pool: &SqlitePool, record: &McpServerRecord) -> anyhow::Result<()> {
    db_timeout(
        sqlx::query(
            "INSERT INTO mcp_servers \
             (name, command, args, script_content, description, created_at, is_active, env, default_policy) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'opt-out') \
             ON CONFLICT(name) DO UPDATE SET \
               command = excluded.command, \
               args = excluded.args, \
               script_content = excluded.script_content, \
               description = excluded.description, \
               is_active = excluded.is_active, \
               env = excluded.env",
        )
        .bind(&record.name)
        .bind(&record.command)
        .bind(&record.args)
        .bind(&record.script_content)
        .bind(&record.description)
        .bind(record.created_at)
        .bind(record.is_active)
        .bind(&record.env)
        .execute(pool),
    )
    .await?;
    Ok(())
}

pub async fn load_active_mcp_servers(pool: &SqlitePool) -> anyhow::Result<Vec<McpServerRecord>> {
    db_timeout(
        sqlx::query_as::<_, McpServerRecord>(
            "SELECT name, command, args, script_content, description, created_at, is_active, env \
             FROM mcp_servers WHERE is_active = 1 ORDER BY created_at ASC",
        )
        .fetch_all(pool),
    )
    .await
}

pub async fn deactivate_mcp_server(pool: &SqlitePool, name: &str) -> anyhow::Result<()> {
    db_timeout(
        sqlx::query("UPDATE mcp_servers SET is_active = 0 WHERE name = ?")
            .bind(name)
            .execute(pool),
    )
    .await?;
    Ok(())
}

// ============================================================
// MCP Access Control (MCP_SERVER_UI_DESIGN.md §3)
// ============================================================

/// Access control entry for MCP tool-level permissions.
/// Maps to `mcp_access_control` table.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AccessControlEntry {
    pub id: Option<i64>,
    pub entry_type: EntryType,
    pub agent_id: String,
    pub server_id: String,
    pub tool_name: Option<String>,
    pub permission: PermissionLevel,
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
    // Ensure the server exists in mcp_servers (config-loaded servers may not be persisted yet)
    sqlx::query(
        "INSERT OR IGNORE INTO mcp_servers (name, command, args, created_at) VALUES (?, 'config-loaded', '[]', strftime('%s', 'now'))",
    )
    .bind(&entry.server_id)
    .execute(pool)
    .await?;

    db_timeout(
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
        .fetch_one(pool),
    )
    .await
}

/// Get all access control entries for a specific MCP server (tree view data).
pub async fn get_access_entries_for_server(
    pool: &SqlitePool,
    server_id: &str,
) -> anyhow::Result<Vec<AccessControlEntry>> {
    db_timeout(
        sqlx::query_as::<_, AccessControlEntry>(
            "SELECT id, entry_type, agent_id, server_id, tool_name, permission, granted_by, granted_at, expires_at, justification, metadata \
             FROM mcp_access_control WHERE server_id = ? ORDER BY agent_id, entry_type, tool_name",
        )
        .bind(server_id)
        .fetch_all(pool),
    )
    .await
}

/// Get all access control entries for a specific agent (by-agent view).
pub async fn get_access_entries_for_agent(
    pool: &SqlitePool,
    agent_id: &str,
) -> anyhow::Result<Vec<AccessControlEntry>> {
    db_timeout(
        sqlx::query_as::<_, AccessControlEntry>(
            "SELECT id, entry_type, agent_id, server_id, tool_name, permission, granted_by, granted_at, expires_at, justification, metadata \
             FROM mcp_access_control WHERE agent_id = ? ORDER BY server_id, entry_type, tool_name",
        )
        .bind(agent_id)
        .fetch_all(pool),
    )
    .await
}

/// Bulk update access control entries for a server.
/// Deletes all non-capability entries for the server and inserts the new ones in a transaction.
pub async fn put_access_entries(
    pool: &SqlitePool,
    server_id: &str,
    entries: &[AccessControlEntry],
) -> anyhow::Result<()> {
    let secs = super::db_timeout_secs();
    tokio::time::timeout(std::time::Duration::from_secs(secs), async {
        let mut tx = pool.begin().await.map_err(|e| anyhow::anyhow!("Failed to begin transaction: {}", e))?;

        // Ensure the server exists in mcp_servers (config-loaded servers may not be persisted yet)
        sqlx::query(
            "INSERT OR IGNORE INTO mcp_servers (name, command, args, created_at) VALUES (?, 'config-loaded', '[]', strftime('%s', 'now'))",
        )
        .bind(server_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to ensure server record: {}", e))?;

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
            if entry.entry_type == EntryType::Capability {
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
    .map_err(|_| anyhow::anyhow!("Database operation timed out after {}s", secs))?
}

/// Delete an access control entry for an agent/server/entry_type combination.
/// If `tool_name` is Some, additionally filters by tool_name.
pub async fn delete_access_entry(
    pool: &SqlitePool,
    agent_id: &str,
    server_id: &str,
    entry_type: &str,
    tool_name: Option<&str>,
) -> anyhow::Result<u64> {
    let rows = if let Some(tn) = tool_name {
        db_timeout(
            sqlx::query(
                "DELETE FROM mcp_access_control \
                 WHERE agent_id = ? AND server_id = ? AND entry_type = ? AND tool_name = ?",
            )
            .bind(agent_id)
            .bind(server_id)
            .bind(entry_type)
            .bind(tn)
            .execute(pool),
        )
        .await?
    } else {
        db_timeout(
            sqlx::query(
                "DELETE FROM mcp_access_control \
                 WHERE agent_id = ? AND server_id = ? AND entry_type = ? AND tool_name IS NULL",
            )
            .bind(agent_id)
            .bind(server_id)
            .bind(entry_type)
            .execute(pool),
        )
        .await?
    };
    Ok(rows.rows_affected())
}

/// Resolve tool access for an agent.
/// Priority: tool_grant > server_grant > default_policy
pub async fn resolve_tool_access(
    pool: &SqlitePool,
    agent_id: &str,
    server_id: &str,
    tool_name: &str,
) -> anyhow::Result<PermissionLevel> {
    // 1. Check for explicit tool_grant
    let tool_grant = db_timeout(
        sqlx::query_scalar::<_, String>(
            "SELECT permission FROM mcp_access_control \
             WHERE agent_id = ? AND server_id = ? AND tool_name = ? AND entry_type = 'tool_grant' \
             AND (expires_at IS NULL OR expires_at > datetime('now')) \
             LIMIT 1",
        )
        .bind(agent_id)
        .bind(server_id)
        .bind(tool_name)
        .fetch_optional(pool),
    )
    .await?;

    if let Some(ref perm) = tool_grant {
        if perm == "allow" {
            return Ok(PermissionLevel::Allow);
        }
        return Ok(PermissionLevel::Deny);
    }

    // 2. Check for server_grant
    let server_grant = db_timeout(
        sqlx::query_scalar::<_, String>(
            "SELECT permission FROM mcp_access_control \
             WHERE agent_id = ? AND server_id = ? AND entry_type = 'server_grant' AND tool_name IS NULL \
             AND (expires_at IS NULL OR expires_at > datetime('now')) \
             LIMIT 1",
        )
        .bind(agent_id)
        .bind(server_id)
        .fetch_optional(pool),
    )
    .await?;

    if let Some(ref perm) = server_grant {
        if perm == "allow" {
            return Ok(PermissionLevel::Allow);
        }
        return Ok(PermissionLevel::Deny);
    }

    // 3. Fall back to server default_policy
    let policy = db_timeout(
        sqlx::query_scalar::<_, String>(
            "SELECT default_policy FROM mcp_servers WHERE name = ? LIMIT 1",
        )
        .bind(server_id)
        .fetch_optional(pool),
    )
    .await?;

    Ok(DefaultPolicy::from_str_lossy(policy.as_deref().unwrap_or("opt-in")).default_permission())
}

/// Get access summary for a server's tools (Summary Bar data).
/// Returns (tool_name, allowed_count, denied_count, inherited_count).
pub async fn get_access_summary(
    pool: &SqlitePool,
    server_id: &str,
) -> anyhow::Result<Vec<(String, i64, i64, i64)>> {
    // This query counts explicit grants per tool.
    // "inherited" means agents that have a server_grant but no tool_grant.
    let rows = db_timeout(
        sqlx::query_as::<_, (String, i64, i64)>(
            "SELECT tool_name, \
             SUM(CASE WHEN permission = 'allow' THEN 1 ELSE 0 END) as allowed, \
             SUM(CASE WHEN permission = 'deny' THEN 1 ELSE 0 END) as denied \
             FROM mcp_access_control \
             WHERE server_id = ? AND entry_type = 'tool_grant' AND tool_name IS NOT NULL \
             GROUP BY tool_name",
        )
        .bind(server_id)
        .fetch_all(pool),
    )
    .await?;

    // Count agents with server_grant but no tool_grant (inherited)
    let server_grant_count = db_timeout(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(DISTINCT agent_id) FROM mcp_access_control \
             WHERE server_id = ? AND entry_type = 'server_grant'",
        )
        .bind(server_id)
        .fetch_one(pool),
    )
    .await?;

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
    let result = db_timeout(
        sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, i64, bool, String, String)>(
            "SELECT name, command, args, script_content, description, created_at, is_active, default_policy, env \
             FROM mcp_servers WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(pool),
    )
    .await?;

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
            env,
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
                    env,
                },
                default_policy,
            )
        },
    ))
}

/// Update MCP server default_policy.
/// Returns the number of rows affected (0 if server not in DB).
pub async fn update_mcp_server_default_policy(
    pool: &SqlitePool,
    name: &str,
    default_policy: &str,
) -> anyhow::Result<u64> {
    Ok(db_timeout(
        sqlx::query("UPDATE mcp_servers SET default_policy = ? WHERE name = ?")
            .bind(default_policy)
            .bind(name)
            .execute(pool),
    )
    .await?
    .rows_affected())
}

/// Update MCP server environment variables (JSON-serialized HashMap).
pub async fn update_mcp_server_env(
    pool: &SqlitePool,
    name: &str,
    env_json: &str,
) -> anyhow::Result<u64> {
    Ok(db_timeout(
        sqlx::query("UPDATE mcp_servers SET env = ? WHERE name = ?")
            .bind(env_json)
            .bind(name)
            .execute(pool),
    )
    .await?
    .rows_affected())
}

// ============================================================
// Marketplace Server Persistence
// ============================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MarketplaceServerRecord {
    pub name: String,
    pub source: String,
    pub installed_version: Option<String>,
    pub marketplace_id: Option<String>,
    pub is_active: bool,
}

/// Save a marketplace-installed server to the DB.
pub async fn save_marketplace_server(
    pool: &SqlitePool,
    name: &str,
    command: &str,
    args: &str,
    description: Option<&str>,
    env_json: &str,
    version: &str,
    marketplace_id: &str,
) -> anyhow::Result<()> {
    db_timeout(
        sqlx::query(
            "INSERT INTO mcp_servers \
             (name, command, args, description, created_at, is_active, env, default_policy, \
              source, installed_version, marketplace_id) \
             VALUES (?, ?, ?, ?, unixepoch(), 1, ?, 'opt-out', 'marketplace', ?, ?) \
             ON CONFLICT(name) DO UPDATE SET \
               command = excluded.command, \
               args = excluded.args, \
               description = excluded.description, \
               env = excluded.env, \
               is_active = 1, \
               source = 'marketplace', \
               installed_version = excluded.installed_version, \
               marketplace_id = excluded.marketplace_id",
        )
        .bind(name)
        .bind(command)
        .bind(args)
        .bind(description)
        .bind(env_json)
        .bind(version)
        .bind(marketplace_id)
        .execute(pool),
    )
    .await?;
    Ok(())
}

/// Load all marketplace-installed servers.
pub async fn get_marketplace_servers(
    pool: &SqlitePool,
) -> anyhow::Result<Vec<MarketplaceServerRecord>> {
    db_timeout(
        sqlx::query_as::<_, MarketplaceServerRecord>(
            "SELECT name, source, installed_version, marketplace_id, is_active \
             FROM mcp_servers WHERE source = 'marketplace' ORDER BY name ASC",
        )
        .fetch_all(pool),
    )
    .await
}

/// Set marketplace-specific fields on an existing mcp_servers record.
pub async fn set_marketplace_fields(
    pool: &SqlitePool,
    name: &str,
    version: &str,
    marketplace_id: &str,
) -> anyhow::Result<()> {
    db_timeout(
        sqlx::query(
            "UPDATE mcp_servers SET source = 'marketplace', \
             installed_version = ?, marketplace_id = ? \
             WHERE name = ?",
        )
        .bind(version)
        .bind(marketplace_id)
        .bind(name)
        .execute(pool),
    )
    .await?;
    Ok(())
}

/// Update the installed version of a marketplace server.
pub async fn update_marketplace_server_version(
    pool: &SqlitePool,
    name: &str,
    version: &str,
) -> anyhow::Result<u64> {
    Ok(db_timeout(
        sqlx::query(
            "UPDATE mcp_servers SET installed_version = ? WHERE name = ? AND source = 'marketplace'",
        )
        .bind(version)
        .bind(name)
        .execute(pool),
    )
    .await?
    .rows_affected())
}

/// Hard-delete a marketplace-installed server from the DB, including access control entries.
pub async fn delete_marketplace_server(pool: &SqlitePool, name: &str) -> anyhow::Result<()> {
    // Clean up orphaned access control entries first
    db_timeout(
        sqlx::query("DELETE FROM mcp_access_control WHERE server_id = ?")
            .bind(name)
            .execute(pool),
    )
    .await?;

    let result = db_timeout(
        sqlx::query("DELETE FROM mcp_servers WHERE name = ? AND source = 'marketplace'")
            .bind(name)
            .execute(pool),
    )
    .await?;

    if result.rows_affected() == 0 {
        anyhow::bail!("No marketplace server found with name '{}'", name);
    }
    Ok(())
}

/// Insert a config-loaded MCP server into the DB so its settings can be persisted.
pub async fn ensure_mcp_server_in_db(
    pool: &SqlitePool,
    name: &str,
    command: &str,
    args: &str,
    default_policy: &str,
) -> anyhow::Result<()> {
    db_timeout(
        sqlx::query(
            "INSERT INTO mcp_servers (name, command, args, created_at, is_active, default_policy) \
             VALUES (?, ?, ?, unixepoch(), 1, ?) \
             ON CONFLICT(name) DO UPDATE SET default_policy = excluded.default_policy",
        )
        .bind(name)
        .bind(command)
        .bind(args)
        .bind(default_policy)
        .execute(pool),
    )
    .await?;
    Ok(())
}
