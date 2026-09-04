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

    /// Returns `true` if a row with the given id exists in the `agents` table.
    /// Lightweight existence probe used by handlers that need to reject
    /// references to unknown agents with a clear validation error instead of
    /// letting the downstream foreign-key constraint fail as a 500.
    pub async fn agent_exists(&self, agent_id: &str) -> anyhow::Result<bool> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM agents WHERE id = ? LIMIT 1")
            .bind(agent_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
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
            .filter(|e| {
                e.entry_type == crate::db::mcp::EntryType::ServerGrant
                    && e.permission == crate::db::mcp::PermissionLevel::Allow
            })
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
        let updated_at = chrono::Utc::now().timestamp_millis().to_string();
        sqlx::query(
            "UPDATE agents SET metadata = json_set(\
             COALESCE(metadata, '{}'), '$.avatar_path', ?, '$.avatar_description', ?, '$.avatar_updated_at', ?) \
             WHERE id = ?",
        )
        .bind(avatar_path)
        .bind(desc_val)
        .bind(&updated_at)
        .bind(agent_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Clear the avatar for an agent (removes from metadata JSON).
    pub async fn clear_avatar(&self, agent_id: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE agents SET metadata = json_remove(\
             COALESCE(metadata, '{}'), '$.avatar_path', '$.avatar_description', '$.has_avatar', '$.avatar_updated_at') \
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
        let row: (Option<String>,) =
            sqlx::query_as("SELECT json_extract(metadata, '$.vrm_path') FROM agents WHERE id = ?")
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

        // Wrap all DB deletions in a transaction for consistency
        let mut tx = self.pool.begin().await?;

        // chat_attachments cascade from chat_messages (ON DELETE CASCADE in schema)
        sqlx::query("DELETE FROM chat_messages WHERE agent_id = ?")
            .bind(agent_id)
            .execute(&mut *tx)
            .await?;

        // Clean up MCP access control entries (no FK to agents table)
        sqlx::query("DELETE FROM mcp_access_control WHERE agent_id = ?")
            .bind(agent_id)
            .execute(&mut *tx)
            .await?;

        // Clean up trusted commands (no FK to agents table)
        sqlx::query("DELETE FROM trusted_commands WHERE agent_id = ?")
            .bind(agent_id)
            .execute(&mut *tx)
            .await?;

        let result = sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(agent_id)
            .execute(&mut *tx)
            .await?;

        if result.rows_affected() == 0 {
            return Err(cloto_shared::ClotoError::AgentNotFound(agent_id.to_string()).into());
        }

        tx.commit().await?;
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
        // Preserve fields managed by dedicated APIs (avatar, VRM) that use
        // json_set/json_remove for partial updates. Without this, COALESCE
        // full-replacement would overwrite these fields.
        // Only re-inject each field if it exists in the current row (IS NOT NULL),
        // otherwise json_set would insert a JSON null which breaks deserialization.
        // bug-476: avatar_updated_at (the frontend cache-bust key, written by
        // set_avatar) must be preserved through the COALESCE full-replace just
        // like avatar_path/avatar_description/vrm_path — otherwise it is dropped
        // on every unrelated agent update.
        let result = sqlx::query(
            "UPDATE agents SET metadata = CASE \
               WHEN ?1 IS NOT NULL THEN ( \
                 SELECT CASE WHEN json_extract(metadata, '$.avatar_updated_at') IS NOT NULL \
                   THEN json_set(m4, '$.avatar_updated_at', json_extract(metadata, '$.avatar_updated_at')) \
                   ELSE m4 END \
                 FROM ( \
                   SELECT CASE WHEN json_extract(metadata, '$.vrm_path') IS NOT NULL \
                     THEN json_set(m3, '$.vrm_path', json_extract(metadata, '$.vrm_path')) \
                     ELSE m3 END AS m4 \
                   FROM ( \
                     SELECT CASE WHEN json_extract(metadata, '$.avatar_description') IS NOT NULL \
                       THEN json_set(m2, '$.avatar_description', json_extract(metadata, '$.avatar_description')) \
                       ELSE m2 END AS m3 \
                     FROM ( \
                       SELECT CASE WHEN json_extract(metadata, '$.avatar_path') IS NOT NULL \
                         THEN json_set(?1, '$.avatar_path', json_extract(metadata, '$.avatar_path')) \
                         ELSE ?1 END AS m2 \
                     ) \
                   ) \
                 ) \
               ) \
               ELSE metadata END, \
             name = COALESCE(?2, name), \
             description = COALESCE(?3, description), \
             default_engine_id = COALESCE(?4, default_engine_id) \
             WHERE id = ?5",
        )
        .bind(&metadata_json)
        .bind(name)
        .bind(description)
        .bind(&default_engine_id)
        .bind(agent_id)
        .execute(&self.pool)
        .await?;
        // bug-477: a 0-row UPDATE means the agent id doesn't exist — surface a
        // 404-equivalent instead of a silent Ok (delete_agent already does this).
        if result.rows_affected() == 0 {
            return Err(cloto_shared::ClotoError::AgentNotFound(agent_id.to_string()).into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Characterization tests for `AgentManager`.
    //!
    //! Everything here runs against a real in-memory SQLite with the production
    //! migrations applied, so the SQL (json_set / json_remove / COALESCE
    //! re-injection) is exercised rather than mocked.

    use super::*;
    use sqlx::SqlitePool;

    async fn manager_with_threshold(heartbeat_threshold_ms: i64) -> AgentManager {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", None)
            .await
            .unwrap();
        AgentManager::new(pool, heartbeat_threshold_ms)
    }

    async fn manager() -> AgentManager {
        manager_with_threshold(90_000).await
    }

    async fn new_agent(mgr: &AgentManager, name: &str) -> String {
        mgr.create_agent(name, "desc", "engine.test", HashMap::new(), vec![], None)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn creating_an_agent_derives_its_id_from_the_name_and_the_row_round_trips() {
        let mgr = manager().await;
        let id = mgr
            .create_agent(
                "Test Agent",
                "a description",
                "engine.test",
                HashMap::from([("k".to_string(), "v".to_string())]),
                vec![],
                None,
            )
            .await
            .unwrap();

        assert_eq!(id, "agent.test_agent", "space becomes an underscore");
        assert!(mgr.agent_exists(&id).await.unwrap());

        let (meta, engine) = mgr.get_agent_config(&id).await.unwrap();
        assert_eq!(meta.id, id);
        assert_eq!(meta.name, "Test Agent", "the display name keeps its case");
        assert_eq!(meta.description, "a description");
        assert_eq!(engine, "engine.test");
        assert_eq!(meta.metadata.get("k").map(String::as_str), Some("v"));
        assert!(meta.enabled, "a freshly created agent is enabled");
        assert!(
            !meta.metadata.contains_key("has_power_password"),
            "no password was set, so no flag is derived"
        );
    }

    #[tokio::test]
    async fn id_sanitisation_replaces_ascii_punctuation_but_keeps_characters_above_u2e7f() {
        // Quirk: the `c > '\u{2E7F}'` clause is what lets CJK *punctuation*
        // through unchanged, while ASCII punctuation becomes '_'. Two names that
        // differ only in ASCII punctuation therefore collide on one id.
        let mgr = manager().await;

        let cjk = mgr
            .create_agent("さくら。Bot", "d", "e", HashMap::new(), vec![], None)
            .await
            .unwrap();
        assert_eq!(cjk, "agent.さくら。bot");

        let first = new_agent(&mgr, "Ops!").await;
        assert_eq!(first, "agent.ops_");
        let collision = mgr
            .create_agent("Ops?", "d", "e", HashMap::new(), vec![], None)
            .await;
        assert!(
            collision.is_err(),
            "a second name sanitising to the same id must hit the primary key"
        );
    }

    #[tokio::test]
    async fn an_enabled_agent_is_online_inside_the_heartbeat_window_and_degraded_outside_it() {
        let mgr = manager_with_threshold(90_000).await;
        let id = new_agent(&mgr, "Heartbeat").await;

        let (fresh, _) = mgr.get_agent_config(&id).await.unwrap();
        assert_eq!(fresh.status, "online");

        // Push last_seen ten minutes into the past: still enabled, now stale.
        sqlx::query("UPDATE agents SET last_seen = ? WHERE id = ?")
            .bind(chrono::Utc::now().timestamp_millis() - 600_000)
            .bind(&id)
            .execute(&mgr.pool)
            .await
            .unwrap();
        let (stale, _) = mgr.get_agent_config(&id).await.unwrap();
        assert_eq!(stale.status, "degraded");
        assert!(stale.enabled, "staleness does not disable the agent");

        // A heartbeat brings it back.
        mgr.touch_last_seen(&id).await.unwrap();
        let (touched, _) = mgr.get_agent_config(&id).await.unwrap();
        assert_eq!(touched.status, "online");

        // A wider threshold reclassifies the very same row.
        let wide = AgentManager::new(mgr.pool.clone(), 3_600_000);
        sqlx::query("UPDATE agents SET last_seen = ? WHERE id = ?")
            .bind(chrono::Utc::now().timestamp_millis() - 600_000)
            .bind(&id)
            .execute(&mgr.pool)
            .await
            .unwrap();
        let (relaxed, _) = wide.get_agent_config(&id).await.unwrap();
        assert_eq!(
            relaxed.status, "online",
            "the threshold, not the row, decides liveness"
        );
    }

    #[tokio::test]
    async fn powering_an_agent_off_zeroes_last_seen_and_reports_offline() {
        let mgr = manager().await;
        let id = new_agent(&mgr, "Power").await;

        mgr.set_enabled(&id, false).await.unwrap();
        let (off, _) = mgr.get_agent_config(&id).await.unwrap();
        assert!(!off.enabled);
        assert_eq!(off.last_seen, 0, "power-off clears the heartbeat");
        assert_eq!(off.status, "offline");

        mgr.set_enabled(&id, true).await.unwrap();
        let (on, _) = mgr.get_agent_config(&id).await.unwrap();
        assert!(on.enabled);
        assert!(on.last_seen > 0, "power-on stamps a fresh heartbeat");
        assert_eq!(on.status, "online");
    }

    #[tokio::test]
    async fn avatar_and_vrm_paths_round_trip_and_clearing_removes_the_derived_flags() {
        let mgr = manager().await;
        let id = new_agent(&mgr, "Looks").await;

        mgr.set_avatar(&id, "data/avatars/a.png", Some("a smiling face"))
            .await
            .unwrap();
        mgr.set_vrm(&id, "data/vrm/a.vrm").await.unwrap();

        assert_eq!(
            mgr.get_avatar_path(&id).await.unwrap().as_deref(),
            Some("data/avatars/a.png")
        );
        assert_eq!(
            mgr.get_vrm_path(&id).await.unwrap().as_deref(),
            Some("data/vrm/a.vrm")
        );
        let (with_media, _) = mgr.get_agent_config(&id).await.unwrap();
        assert_eq!(
            with_media.metadata.get("has_avatar").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            with_media.metadata.get("has_vrm").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            with_media
                .metadata
                .get("avatar_description")
                .map(String::as_str),
            Some("a smiling face")
        );
        assert!(
            with_media.metadata.contains_key("avatar_updated_at"),
            "set_avatar stamps the cache-bust key"
        );

        mgr.clear_avatar(&id).await.unwrap();
        mgr.clear_vrm(&id).await.unwrap();

        assert!(mgr.get_avatar_path(&id).await.unwrap().is_none());
        assert!(mgr.get_vrm_path(&id).await.unwrap().is_none());
        let (cleared, _) = mgr.get_agent_config(&id).await.unwrap();
        assert!(!cleared.metadata.contains_key("has_avatar"));
        assert!(!cleared.metadata.contains_key("has_vrm"));
        assert!(!cleared.metadata.contains_key("avatar_updated_at"));
    }

    #[tokio::test]
    async fn a_metadata_replacing_update_preserves_the_avatar_and_vrm_fields() {
        let mgr = manager().await;
        let id = new_agent(&mgr, "Preserve").await;
        mgr.set_avatar(&id, "data/avatars/p.png", Some("portrait"))
            .await
            .unwrap();
        mgr.set_vrm(&id, "data/vrm/p.vrm").await.unwrap();
        let (before, _) = mgr.get_agent_config(&id).await.unwrap();
        let stamp = before.metadata.get("avatar_updated_at").cloned().unwrap();

        mgr.update_agent_config(
            &id,
            Some("Renamed"),
            Some("new description"),
            Some("engine.other".to_string()),
            Some(HashMap::from([("persona".to_string(), "calm".to_string())])),
        )
        .await
        .unwrap();

        let (after, engine) = mgr.get_agent_config(&id).await.unwrap();
        assert_eq!(after.name, "Renamed");
        assert_eq!(after.description, "new description");
        assert_eq!(engine, "engine.other");
        assert_eq!(
            after.metadata.get("persona").map(String::as_str),
            Some("calm")
        );
        assert_eq!(
            after.metadata.get("avatar_path").map(String::as_str),
            Some("data/avatars/p.png")
        );
        assert_eq!(
            after.metadata.get("avatar_description").map(String::as_str),
            Some("portrait")
        );
        assert_eq!(
            after.metadata.get("vrm_path").map(String::as_str),
            Some("data/vrm/p.vrm")
        );
        assert_eq!(
            after.metadata.get("avatar_updated_at"),
            Some(&stamp),
            "the cache-bust key survives an unrelated update (bug-476)"
        );
    }

    #[tokio::test]
    async fn omitting_a_field_from_an_update_leaves_it_untouched() {
        let mgr = manager().await;
        let id = new_agent(&mgr, "Partial").await;

        mgr.update_agent_config(&id, Some("Only Name"), None, None, None)
            .await
            .unwrap();

        let (after, engine) = mgr.get_agent_config(&id).await.unwrap();
        assert_eq!(after.name, "Only Name");
        assert_eq!(
            after.description, "desc",
            "None means COALESCE keeps the old value"
        );
        assert_eq!(engine, "engine.test");
    }

    #[tokio::test]
    async fn updating_or_deleting_an_agent_that_does_not_exist_is_reported_as_not_found() {
        let mgr = manager().await;

        let update_err = mgr
            .update_agent_config("agent.ghost", Some("x"), None, None, None)
            .await
            .expect_err("a zero-row update must not read as success");
        assert!(
            update_err.to_string().contains("agent.ghost"),
            "{update_err}"
        );

        let delete_err = mgr
            .delete_agent("agent.ghost")
            .await
            .expect_err("deleting nothing must not read as success");
        assert!(
            delete_err.to_string().contains("agent.ghost"),
            "{delete_err}"
        );
    }

    #[tokio::test]
    async fn deleting_an_agent_also_removes_its_chat_history_and_trusted_commands() {
        let mgr = manager().await;
        let id = new_agent(&mgr, "Doomed").await;
        let survivor = new_agent(&mgr, "Bystander").await;

        for agent in [&id, &survivor] {
            crate::db::add_trusted_command(&mgr.pool, agent, "ls -la")
                .await
                .unwrap();
        }

        mgr.delete_agent(&id).await.unwrap();

        assert!(!mgr.agent_exists(&id).await.unwrap());
        assert!(
            !crate::db::is_command_trusted(&mgr.pool, &id, "ls -la")
                .await
                .unwrap(),
            "the deleted agent's trust list is gone"
        );
        assert!(
            crate::db::is_command_trusted(&mgr.pool, &survivor, "ls -la")
                .await
                .unwrap(),
            "another agent's trust list is untouched"
        );
    }

    #[tokio::test]
    async fn the_manager_layer_will_delete_the_default_seeded_agent_without_complaint() {
        // Quirk worth knowing before a headless rewrite: "the default agent
        // cannot be deleted" is enforced only in the HTTP handler. Any caller
        // that reaches AgentManager directly bypasses that protection.
        let mgr = manager().await;
        let default_id = "agent.cloto_default"; // AppConfig's DEFAULT_AGENT_ID fallback
        assert!(
            mgr.agent_exists(default_id).await.unwrap(),
            "the migrations seed this agent"
        );

        mgr.delete_agent(default_id).await.unwrap();

        assert!(!mgr.agent_exists(default_id).await.unwrap());
    }

    #[tokio::test]
    async fn listing_agents_hides_rows_whose_type_is_not_agent_but_getting_them_by_id_does_not() {
        let mgr = manager().await;
        let id = new_agent(&mgr, "Typed").await;
        sqlx::query("UPDATE agents SET agent_type = 'engine' WHERE id = ?")
            .bind(&id)
            .execute(&mgr.pool)
            .await
            .unwrap();

        let listed: Vec<String> = mgr
            .list_agents()
            .await
            .unwrap()
            .into_iter()
            .map(|a| a.id)
            .collect();
        assert!(
            !listed.contains(&id),
            "list_agents filters on agent_type = 'agent'"
        );
        assert!(
            mgr.get_agent_config(&id).await.is_ok(),
            "get_agent_config applies no such filter"
        );
    }

    #[tokio::test]
    async fn a_power_password_is_stored_hashed_and_surfaces_only_as_a_boolean_flag() {
        let mgr = manager().await;
        let id = mgr
            .create_agent(
                "Locked",
                "d",
                "e",
                HashMap::new(),
                vec![],
                Some("correct horse"),
            )
            .await
            .unwrap();

        let hash = mgr
            .get_password_hash(&id)
            .await
            .unwrap()
            .expect("a hash was stored");
        assert!(!hash.contains("correct horse"), "the plaintext is not kept");
        assert!(AgentManager::verify_password("correct horse", &hash).unwrap());
        assert!(!AgentManager::verify_password("wrong horse", &hash).unwrap());

        let (meta, _) = mgr.get_agent_config(&id).await.unwrap();
        assert_eq!(
            meta.metadata.get("has_power_password").map(String::as_str),
            Some("true")
        );

        // An empty password string is treated as "no password at all".
        let open = mgr
            .create_agent("Open", "d", "e", HashMap::new(), vec![], Some(""))
            .await
            .unwrap();
        assert!(mgr.get_password_hash(&open).await.unwrap().is_none());
    }
}
