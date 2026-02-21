use sqlx::SqlitePool;
use std::collections::HashMap;
use tracing::debug;

use exiv_shared::AgentMetadata;

#[derive(sqlx::FromRow)]
struct AgentRow {
    id: String,
    name: String,
    description: String,
    enabled: bool,
    last_seen: i64,
    default_engine_id: String,
    required_capabilities: sqlx::types::Json<Vec<exiv_shared::CapabilityType>>,
    metadata: sqlx::types::Json<HashMap<String, String>>,
    power_password_hash: Option<String>,
}

#[derive(Clone)]
pub struct AgentManager {
    pool: SqlitePool,
}

impl AgentManager {
    /// Heartbeat threshold: 90 seconds. Agents not heard from in this window are "degraded".
    pub const HEARTBEAT_THRESHOLD_MS: i64 = 90_000;

    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_metadata(row: AgentRow) -> AgentMetadata {
        let has_pw = row.power_password_hash.is_some();
        let mut meta = row.metadata.0;
        if has_pw {
            meta.insert("has_power_password".to_string(), "true".to_string());
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
            plugin_bindings: vec![],
            metadata: meta,
        };
        agent.resolve_status(Self::HEARTBEAT_THRESHOLD_MS);
        agent
    }

    pub async fn get_agent_config(
        &self,
        agent_id: &str,
    ) -> anyhow::Result<(AgentMetadata, String)> {
        let row: AgentRow = sqlx::query_as(
            "SELECT id, name, description, enabled, last_seen, default_engine_id, \
             required_capabilities, metadata, power_password_hash FROM agents WHERE id = ?",
        )
        .bind(agent_id)
        .fetch_one(&self.pool)
        .await?;

        let engine_id = row.default_engine_id.clone();
        let metadata = Self::row_to_metadata(row);
        Ok((metadata, engine_id))
    }

    pub async fn list_agents(&self) -> anyhow::Result<Vec<AgentMetadata>> {
        let rows: Vec<AgentRow> = sqlx::query_as(
            "SELECT id, name, description, enabled, last_seen, default_engine_id, \
             required_capabilities, metadata, power_password_hash FROM agents",
        )
        .fetch_all(&self.pool)
        .await?;

        let agents: Vec<AgentMetadata> = rows.into_iter().map(Self::row_to_metadata).collect();

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
        required_capabilities: Vec<exiv_shared::CapabilityType>,
        password: Option<&str>,
    ) -> anyhow::Result<String> {
        // K-01: Return the actual DB id_str instead of a mismatched ExivId
        let id_str = format!("agent.{}", name.to_lowercase().replace(' ', "_"));
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
             enabled, last_seen, metadata, required_capabilities, power_password_hash) \
             VALUES (?, ?, ?, ?, 'online', 1, ?, ?, ?, ?)",
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

    /// Return the plugin list for an agent from the agent_plugins table.
    pub async fn get_agent_plugins(
        &self,
        agent_id: &str,
    ) -> anyhow::Result<Vec<crate::db::AgentPluginRow>> {
        crate::db::get_agent_plugins(&self.pool, agent_id).await
    }

    /// Replace an agent's plugin list. Also updates default_engine_id and preferred_memory
    /// by inspecting the plugin manifests via the provided registry.
    pub async fn set_agent_plugins(
        &self,
        agent_id: &str,
        plugins: &[(String, i32, i32)],
        registry: &crate::managers::PluginRegistry,
    ) -> anyhow::Result<()> {
        crate::db::set_agent_plugins(&self.pool, agent_id, plugins).await?;

        // Derive default_engine_id and preferred_memory from the new plugin list.
        // Priority for default_engine_id: mind.* LLM engines > other Reasoning engines.
        let manifests = registry.list_plugins().await;
        let mut llm_engine_id: Option<String> = None; // mind.* preferred
        let mut fallback_engine_id: Option<String> = None;
        let mut memory_id: Option<String> = None;
        for (plugin_id, _, _) in plugins {
            if let Some(m) = manifests.iter().find(|m| &m.id == plugin_id) {
                if m.service_type == exiv_shared::ServiceType::Reasoning {
                    if llm_engine_id.is_none() && plugin_id.starts_with("mind.") {
                        llm_engine_id = Some(plugin_id.clone());
                    } else if fallback_engine_id.is_none() && !plugin_id.starts_with("mind.") {
                        fallback_engine_id = Some(plugin_id.clone());
                    }
                }
                if memory_id.is_none() && m.service_type == exiv_shared::ServiceType::Memory {
                    memory_id = Some(plugin_id.clone());
                }
            }
        }
        let engine_id = llm_engine_id.or(fallback_engine_id);

        // Update agents table
        let (meta, _) = self.get_agent_config(agent_id).await?;
        let mut metadata = meta.metadata.clone();
        if let Some(ref mid) = memory_id {
            metadata.insert("preferred_memory".to_string(), mid.clone());
        }
        self.update_agent_config(agent_id, engine_id, metadata)
            .await?;
        Ok(())
    }

    /// Delete an agent and all associated data (chat messages, attachments via cascade).
    pub async fn delete_agent(&self, agent_id: &str) -> anyhow::Result<()> {
        // chat_attachments cascade from chat_messages (ON DELETE CASCADE in schema)
        sqlx::query("DELETE FROM chat_messages WHERE agent_id = ?")
            .bind(agent_id)
            .execute(&self.pool)
            .await?;

        let result = sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(agent_id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(exiv_shared::ExivError::AgentNotFound(agent_id.to_string()).into());
        }
        Ok(())
    }

    pub async fn update_agent_config(
        &self,
        agent_id: &str,
        default_engine_id: Option<String>,
        metadata: HashMap<String, String>,
    ) -> anyhow::Result<()> {
        let metadata_json = serde_json::to_string(&metadata)?;
        if let Some(engine_id) = default_engine_id {
            sqlx::query("UPDATE agents SET metadata = ?, default_engine_id = ? WHERE id = ?")
                .bind(metadata_json)
                .bind(engine_id)
                .bind(agent_id)
                .execute(&self.pool)
                .await?;
        } else {
            sqlx::query("UPDATE agents SET metadata = ? WHERE id = ?")
                .bind(metadata_json)
                .bind(agent_id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }
}
