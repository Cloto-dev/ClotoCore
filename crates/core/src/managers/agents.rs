use sqlx::SqlitePool;
use std::collections::HashMap;
use tracing::debug;

use cloto_shared::AgentMetadata;

#[derive(sqlx::FromRow)]
struct AgentRow {
    id: String,
    name: String,
    description: String,
    enabled: bool,
    last_seen: i64,
    default_engine_id: String,
    required_capabilities: sqlx::types::Json<Vec<cloto_shared::CapabilityType>>,
    metadata: sqlx::types::Json<HashMap<String, String>>,
    power_password_hash: Option<String>,
    agent_type: String,
}

#[derive(Clone)]
pub struct AgentManager {
    pub(crate) pool: SqlitePool,
    heartbeat_threshold_ms: i64,
}

impl AgentManager {
    #[must_use]
    pub fn new(pool: SqlitePool, heartbeat_threshold_ms: i64) -> Self {
        Self {
            pool,
            heartbeat_threshold_ms,
        }
    }

    fn row_to_metadata(&self, row: AgentRow) -> AgentMetadata {
        let has_pw = row.power_password_hash.is_some();
        let mut meta = row.metadata.0;
        if has_pw {
            meta.insert("has_power_password".to_string(), "true".to_string());
        }
        // Avatar/VRM presence flags derived from metadata (P4: data lives in metadata JSON)
        if meta.contains_key("avatar_path") {
            meta.insert("has_avatar".to_string(), "true".to_string());
        }
        if meta.contains_key("vrm_path") {
            meta.insert("has_vrm".to_string(), "true".to_string());
        }
        let mut agent = AgentMetadata {
            id: row.id,
            name: row.name,
            description: row.description,
            enabled: row.enabled,
            last_seen: row.last_seen,
            status: String::new(),
            default_engine_id: Some(row.default_engine_id),
            required_capabilities: row.required_capabilities.0,
            metadata: meta,
            agent_type: row.agent_type,
        };
        agent.resolve_status(self.heartbeat_threshold_ms);
        agent
    }

    pub async fn get_agent_config(
        &self,
        agent_id: &str,
    ) -> anyhow::Result<(AgentMetadata, String)> {
        let row: AgentRow = sqlx::query_as(
            "SELECT id, name, description, enabled, last_seen, default_engine_id, \
             required_capabilities, metadata, power_password_hash, agent_type FROM agents WHERE id = ?",
        )
        .bind(agent_id)
        .fetch_one(&self.pool)
        .await?;

        let engine_id = row.default_engine_id.clone();
        let metadata = self.row_to_metadata(row);
        Ok((metadata, engine_id))
    }

    pub async fn list_agents(&self) -> anyhow::Result<Vec<AgentMetadata>> {
        let rows: Vec<AgentRow> = sqlx::query_as(
            "SELECT id, name, description, enabled, last_seen, default_engine_id, \
             required_capabilities, metadata, power_password_hash, agent_type \
             FROM agents WHERE agent_type = 'agent'",
        )
        .fetch_all(&self.pool)
        .await?;

        let agents: Vec<AgentMetadata> =
            rows.into_iter().map(|r| self.row_to_metadata(r)).collect();

        for agent in &agents {
            debug!(
                "Agent {} engine is {:?}",
                agent.name, agent.default_engine_id
            );
        }

        Ok(agents)
    }

    pub async fn create_agent(
        &self,
        name: &str,
        description: &str,
        default_engine: &str,
        metadata: HashMap<String, String>,
        required_capabilities: Vec<cloto_shared::CapabilityType>,
        password: Option<&str>,
    ) -> anyhow::Result<String> {
        // K-01: Return the actual DB id_str instead of a mismatched ClotoId
        // Sanitize: keep alphanumeric, CJK, underscores, hyphens; replace everything else
        let sanitized: String = name
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' || c == '-' || c > '\u{2E7F}' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let id_str = format!("agent.{}", sanitized);
        let metadata_json = serde_json::to_string(&metadata)?;
        let capabilities_json = serde_json::to_string(&required_capabilities)?;
        let now_ms = chrono::Utc::now().timestamp_millis();

        let password_hash = if let Some(pw) = password {
            if pw.is_empty() {
                None
            } else {
                Some(Self::hash_password(pw)?)
            }
        } else {
            None
        };

        sqlx::query(
            "INSERT INTO agents (id, name, description, default_engine_id, status, \
             enabled, last_seen, metadata, required_capabilities, power_password_hash, agent_type) \
             VALUES (?, ?, ?, ?, 'online', 1, ?, ?, ?, ?, 'agent')",
        )
        .bind(&id_str)
        .bind(name)
        .bind(description)
        .bind(default_engine)
        .bind(now_ms)
        .bind(metadata_json)
        .bind(capabilities_json)
        .bind(&password_hash)
        .execute(&self.pool)
        .await?;

        Ok(id_str)
    }

    /// Update the last_seen timestamp for an agent (passive heartbeat).
    pub async fn touch_last_seen(&self, agent_id: &str) -> anyhow::Result<()> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        sqlx::query("UPDATE agents SET last_seen = ? WHERE id = ?")
            .bind(now_ms)
            .bind(agent_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Set the enabled state of an agent (power on/off).
    pub async fn set_enabled(&self, agent_id: &str, enabled: bool) -> anyhow::Result<()> {
        let now_ms = if enabled {
            chrono::Utc::now().timestamp_millis()
        } else {
            0
        };
        sqlx::query("UPDATE agents SET enabled = ?, last_seen = ? WHERE id = ?")
            .bind(enabled)
            .bind(now_ms)
            .bind(agent_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Get the stored password hash for an agent.
    pub async fn get_password_hash(&self, agent_id: &str) -> anyhow::Result<Option<String>> {
        let row: (Option<String>,) =
            sqlx::query_as("SELECT power_password_hash FROM agents WHERE id = ?")
                .bind(agent_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }

    /// Hash a plaintext password using Argon2id.
    pub fn hash_password(password: &str) -> anyhow::Result<String> {
        use argon2::password_hash::SaltString;
        use argon2::{Argon2, PasswordHasher};
        use rand::rngs::OsRng;

        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| anyhow::anyhow!("Password hashing failed: {}", e))?;
        Ok(hash.to_string())
    }

    /// Verify a plaintext password against a stored Argon2id hash.
    pub fn verify_password(password: &str, hash: &str) -> anyhow::Result<bool> {
        use argon2::password_hash::PasswordHash;
        use argon2::{Argon2, PasswordVerifier};

        let parsed_hash =
            PasswordHash::new(hash).map_err(|e| anyhow::anyhow!("Invalid password hash: {}", e))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    }

    /// Return the set of MCP server IDs this agent has access to (server_grant + allow).
    pub async fn get_granted_server_ids(&self, agent_id: &str) -> anyhow::Result<Vec<String>> {
        let entries = crate::db::get_access_entries_for_agent(&self.pool, agent_id).await?;
        Ok(entries
            .into_iter()
            .filter(|e| e.entry_type == "server_grant" && e.permission == "allow")
            .map(|e| e.server_id)
            .collect())
    }

    /// Set the avatar path and description for an agent (stored in metadata JSON).
    pub async fn set_avatar(
        &self,
        agent_id: &str,
        avatar_path: &str,
        avatar_description: Option<&str>,
    ) -> anyhow::Result<()> {
        let desc_val = avatar_description.unwrap_or("");
        sqlx::query(
            "UPDATE agents SET metadata = json_set(\
             COALESCE(metadata, '{}'), '$.avatar_path', ?, '$.avatar_description', ?) \
             WHERE id = ?",
        )
        .bind(avatar_path)
        .bind(desc_val)
        .bind(agent_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Clear the avatar for an agent (removes from metadata JSON).
    pub async fn clear_avatar(&self, agent_id: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE agents SET metadata = json_remove(\
             COALESCE(metadata, '{}'), '$.avatar_path', '$.avatar_description') \
             WHERE id = ?",
        )
        .bind(agent_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Get just the avatar path for serving (from metadata JSON).
    pub async fn get_avatar_path(&self, agent_id: &str) -> anyhow::Result<Option<String>> {
        let row: (Option<String>,) = sqlx::query_as(
            "SELECT json_extract(metadata, '$.avatar_path') FROM agents WHERE id = ?",
        )
        .bind(agent_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    /// Set the VRM model path for an agent (stored in metadata JSON).
    pub async fn set_vrm(&self, agent_id: &str, vrm_path: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE agents SET metadata = json_set(\
             COALESCE(metadata, '{}'), '$.vrm_path', ?) \
             WHERE id = ?",
        )
        .bind(vrm_path)
        .bind(agent_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Clear the VRM model for an agent (removes from metadata JSON).
    pub async fn clear_vrm(&self, agent_id: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE agents SET metadata = json_remove(\
             COALESCE(metadata, '{}'), '$.vrm_path') \
             WHERE id = ?",
        )
        .bind(agent_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Get just the VRM model path for serving (from metadata JSON).
    pub async fn get_vrm_path(&self, agent_id: &str) -> anyhow::Result<Option<String>> {
        let row: (Option<String>,) = sqlx::query_as(
            "SELECT json_extract(metadata, '$.vrm_path') FROM agents WHERE id = ?",
        )
        .bind(agent_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    /// Delete an agent and all associated data (chat messages, attachments,
    /// MCP access control entries, trusted commands).
    ///
    /// Note: CPersona memory data cleanup is handled separately by the caller
    /// (handler layer) via MCP tool call, since the agent manager does not
    /// have access to the MCP client manager.
    pub async fn delete_agent(&self, agent_id: &str) -> anyhow::Result<()> {
        // Clean up avatar/VRM files from disk (paths stored in metadata JSON)
        if let Ok(Some(path)) = self.get_avatar_path(agent_id).await {
            let _ = tokio::fs::remove_file(&path).await;
        }
        if let Ok(Some(path)) = self.get_vrm_path(agent_id).await {
            let _ = tokio::fs::remove_file(&path).await;
        }

        // chat_attachments cascade from chat_messages (ON DELETE CASCADE in schema)
        sqlx::query("DELETE FROM chat_messages WHERE agent_id = ?")
            .bind(agent_id)
            .execute(&self.pool)
            .await?;

        // Clean up MCP access control entries (no FK to agents table)
        sqlx::query("DELETE FROM mcp_access_control WHERE agent_id = ?")
            .bind(agent_id)
            .execute(&self.pool)
            .await?;

        // Clean up trusted commands (no FK to agents table)
        crate::db::trusted_commands::delete_trusted_commands_for_agent(&self.pool, agent_id)
            .await?;

        let result = sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(agent_id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(cloto_shared::ClotoError::AgentNotFound(agent_id.to_string()).into());
        }
        Ok(())
    }

    pub async fn update_agent_config(
        &self,
        agent_id: &str,
        name: Option<&str>,
        description: Option<&str>,
        default_engine_id: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> anyhow::Result<()> {
        let metadata_json = metadata.map(|m| serde_json::to_string(&m)).transpose()?;
        sqlx::query(
            "UPDATE agents SET metadata = COALESCE(?, metadata), \
             name = COALESCE(?, name), \
             description = COALESCE(?, description), \
             default_engine_id = COALESCE(?, default_engine_id) \
             WHERE id = ?",
        )
        .bind(&metadata_json)
        .bind(name)
        .bind(description)
        .bind(&default_engine_id)
        .bind(agent_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
