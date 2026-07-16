use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

use super::registry::{PluginRegistry, PluginSetting};
use crate::capabilities::SafeHttpClient;
use cloto_shared::Permission;

pub struct PluginManager {
    pub pool: SqlitePool,
    http_client: Arc<SafeHttpClient>,
    event_timeout_secs: u64,
    max_event_depth: u8,
    event_concurrency_limit: usize,
    pub event_tx: Option<tokio::sync::mpsc::Sender<crate::EnvelopedEvent>>,
    pub plugin_semaphore: Arc<tokio::sync::Semaphore>,
    pub shutdown: Arc<tokio::sync::Notify>,
}

impl PluginManager {
    pub fn new(
        pool: SqlitePool,
        allowed_hosts: Vec<String>,
        event_timeout_secs: u64,
        max_event_depth: u8,
        event_concurrency_limit: usize,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            pool,
            http_client: Arc::new(SafeHttpClient::new(allowed_hosts)?),
            event_timeout_secs,
            max_event_depth,
            event_concurrency_limit,
            event_tx: None,
            plugin_semaphore: Arc::new(tokio::sync::Semaphore::new(event_concurrency_limit)),
            shutdown: Arc::new(tokio::sync::Notify::new()),
        })
    }

    pub fn set_event_tx(&mut self, tx: tokio::sync::mpsc::Sender<crate::EnvelopedEvent>) {
        self.event_tx = Some(tx);
    }

    /// Initialize the plugin registry (no Rust SDK plugins — all external plugins are MCP).
    pub async fn initialize_all(&self) -> anyhow::Result<PluginRegistry> {
        let registry = PluginRegistry::new(
            self.event_timeout_secs,
            self.max_event_depth,
            self.event_concurrency_limit,
        );
        info!("✅ Plugin registry initialized (MCP-only mode)");
        Ok(registry)
    }

    /// L5: Get a clone of the shared SafeHttpClient Arc for runtime host addition.
    #[must_use]
    pub fn http_client(&self) -> Arc<SafeHttpClient> {
        self.http_client.clone()
    }

    /// bug-459: per-plugin sandbox root under `data/plugin_sandbox/<plugin_id>/`.
    /// Sanitizes `plugin_id` to a single safe path component so a crafted id
    /// cannot escape the sandbox root via path separators or `..`.
    fn plugin_sandbox_dir(plugin_id: &str) -> std::path::PathBuf {
        let safe: String = plugin_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        // Reject empty / "." / ".." (which sanitization above leaves intact) so
        // the join always yields a real child directory of `data/plugin_sandbox`.
        let component = if safe.trim_matches('.').is_empty() {
            "_unnamed".to_string()
        } else {
            safe
        };
        std::path::PathBuf::from("data/plugin_sandbox").join(component)
    }

    #[must_use]
    pub fn get_capability_for_permission(
        &self,
        plugin_id: &str,
        permission: &Permission,
    ) -> Option<cloto_shared::PluginCapability> {
        match permission {
            Permission::NetworkAccess => Some(cloto_shared::PluginCapability::Network(
                self.http_client.clone(),
            )),
            Permission::FileRead => {
                // bug-459: per-plugin read-only sandbox so two plugins that both
                // hold file permissions cannot read each other's files.
                let base = Self::plugin_sandbox_dir(plugin_id);
                Some(cloto_shared::PluginCapability::File(std::sync::Arc::new(
                    crate::capabilities::SandboxedFileCapability::read_only(base),
                )))
            }
            Permission::FileWrite => {
                // bug-459: per-plugin read+write sandbox, isolated from every
                // other plugin's sandbox directory.
                let base = Self::plugin_sandbox_dir(plugin_id);
                Some(cloto_shared::PluginCapability::File(std::sync::Arc::new(
                    crate::capabilities::SandboxedFileCapability::read_write(base),
                )))
            }
            Permission::ProcessExecution => {
                // Empty allowlist by default — callers must configure permitted commands
                Some(cloto_shared::PluginCapability::Process(
                    std::sync::Arc::new(crate::capabilities::AllowedProcessCapability::new(vec![])),
                ))
            }
            _ => None,
        }
    }

    pub async fn get_config(&self, plugin_id: &str) -> anyhow::Result<HashMap<String, String>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT config_key, config_value FROM plugin_configs WHERE plugin_id = ? LIMIT 100",
        )
        .bind(plugin_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().collect())
    }

    pub async fn update_config(
        &self,
        plugin_id: &str,
        key: &str,
        value: &str,
    ) -> anyhow::Result<()> {
        sqlx::query("INSERT OR REPLACE INTO plugin_configs (plugin_id, config_key, config_value) VALUES (?, ?, ?)")
            .bind(plugin_id)
            .bind(key)
            .bind(value)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn list_plugins_with_settings(
        &self,
        registry: &PluginRegistry,
    ) -> anyhow::Result<Vec<cloto_shared::PluginManifest>> {
        let rows: Vec<PluginSetting> = sqlx::query_as(
            "SELECT plugin_id, is_active, allowed_permissions FROM plugin_settings LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await?;

        let settings: HashMap<String, bool> = rows
            .into_iter()
            .map(|s| (s.plugin_id, s.is_active))
            .collect();

        let mut manifests = registry.list_plugins().await;
        for m in &mut manifests {
            if let Some(&active) = settings.get(&m.id) {
                m.is_active = active;
            }
        }
        Ok(manifests)
    }

    pub async fn apply_settings(&self, settings: Vec<(String, bool)>) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        for (id, active) in settings {
            sqlx::query("UPDATE plugin_settings SET is_active = ? WHERE plugin_id = ?")
                .bind(active)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Return the current effective permissions for a plugin from the DB.
    pub async fn get_permissions(
        &self,
        plugin_id: &str,
    ) -> anyhow::Result<Vec<cloto_shared::Permission>> {
        let row: Option<(sqlx::types::Json<Vec<cloto_shared::Permission>>,)> =
            sqlx::query_as("SELECT allowed_permissions FROM plugin_settings WHERE plugin_id = ?")
                .bind(plugin_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(j,)| j.0).unwrap_or_default())
    }

    /// Remove a single permission from a plugin's allowed_permissions in the DB and in-memory.
    pub async fn revoke_permission(
        &self,
        plugin_id: &str,
        permission: &cloto_shared::Permission,
        registry: &PluginRegistry,
    ) -> anyhow::Result<()> {
        // bug-466: revoke atomically like grant_permission (H-08) — a
        // read-modify-write here loses concurrent revocations of *other*
        // permissions (both callers read the same snapshot, the later UPDATE
        // overwrites the earlier one). A single SQL statement that filters the
        // target out via json_each avoids the TOCTOU window. The EXISTS guard
        // makes rows_affected==0 mean "not granted" so the error path is kept.
        let perm_json = serde_json::to_string(permission)?;
        let result = sqlx::query(
            "UPDATE plugin_settings SET allowed_permissions = (
                SELECT json_group_array(value) FROM json_each(allowed_permissions)
                WHERE value <> json_extract(json(?), '$')
            ) WHERE plugin_id = ?
            AND EXISTS (
                SELECT 1 FROM json_each(allowed_permissions)
                WHERE value = json_extract(json(?), '$')
            )",
        )
        .bind(&perm_json)
        .bind(plugin_id)
        .bind(&perm_json)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(anyhow::anyhow!(
                "Permission '{:?}' is not granted to plugin '{}'",
                permission,
                plugin_id
            ));
        }

        // Update in-memory effective permissions
        let plugin_cloto_id = cloto_shared::ClotoId::from_name(plugin_id);
        let mut reg_state = registry.state.write().await;
        if let Some(p) = reg_state.effective_permissions.get_mut(&plugin_cloto_id) {
            p.retain(|x| x != permission);
        }
        Ok(())
    }

    pub async fn grant_permission(
        &self,
        plugin_id: &str,
        permission: cloto_shared::Permission,
    ) -> anyhow::Result<()> {
        // H-08: Single atomic SQL statement to prevent TOCTOU race in permission grant
        let perm_json = serde_json::to_string(&permission)?;
        sqlx::query(
            "UPDATE plugin_settings SET allowed_permissions = json_insert(
                allowed_permissions,
                '$[' || json_array_length(allowed_permissions) || ']',
                json(?)
            ) WHERE plugin_id = ?
            AND NOT EXISTS (
                SELECT 1 FROM json_each(allowed_permissions)
                WHERE value = json_extract(json(?), '$')
            )",
        )
        .bind(&perm_json)
        .bind(plugin_id)
        .bind(&perm_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
