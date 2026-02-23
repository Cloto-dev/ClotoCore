use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

use super::registry::{PluginRegistry, PluginSetting};
use crate::capabilities::SafeHttpClient;
use exiv_shared::Permission;

pub struct PluginManager {
    pub pool: SqlitePool,
    http_client: Arc<SafeHttpClient>,
    event_timeout_secs: u64,
    max_event_depth: u8,
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
    ) -> anyhow::Result<Self> {
        Ok(Self {
            pool,
            http_client: Arc::new(SafeHttpClient::new(allowed_hosts)?),
            event_timeout_secs,
            max_event_depth,
            event_tx: None,
            plugin_semaphore: Arc::new(tokio::sync::Semaphore::new(20)),
            shutdown: Arc::new(tokio::sync::Notify::new()),
        })
    }

    pub fn set_event_tx(&mut self, tx: tokio::sync::mpsc::Sender<crate::EnvelopedEvent>) {
        self.event_tx = Some(tx);
    }

    /// Initialize the plugin registry (no Rust SDK plugins — all external plugins are MCP).
    pub async fn initialize_all(&self) -> anyhow::Result<PluginRegistry> {
        let registry = PluginRegistry::new(self.event_timeout_secs, self.max_event_depth);
        info!("✅ Plugin registry initialized (MCP-only mode)");
        Ok(registry)
    }

    /// L5: Get a clone of the shared SafeHttpClient Arc for runtime host addition.
    #[must_use]
    pub fn http_client(&self) -> Arc<SafeHttpClient> {
        self.http_client.clone()
    }

    #[must_use]
    pub fn get_capability_for_permission(
        &self,
        permission: &Permission,
    ) -> Option<exiv_shared::PluginCapability> {
        match permission {
            Permission::NetworkAccess => Some(exiv_shared::PluginCapability::Network(
                self.http_client.clone(),
            )),
            Permission::FileRead => {
                // Read-only sandbox: plugins can read from the data/ directory
                let base = std::path::PathBuf::from("data/plugin_sandbox");
                Some(exiv_shared::PluginCapability::File(std::sync::Arc::new(
                    crate::capabilities::SandboxedFileCapability::read_only(base),
                )))
            }
            Permission::FileWrite => {
                // Read+write sandbox
                let base = std::path::PathBuf::from("data/plugin_sandbox");
                Some(exiv_shared::PluginCapability::File(std::sync::Arc::new(
                    crate::capabilities::SandboxedFileCapability::read_write(base),
                )))
            }
            Permission::ProcessExecution => {
                // Empty allowlist by default — callers must configure permitted commands
                Some(exiv_shared::PluginCapability::Process(std::sync::Arc::new(
                    crate::capabilities::AllowedProcessCapability::new(vec![]),
                )))
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
    ) -> anyhow::Result<Vec<exiv_shared::PluginManifest>> {
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
    ) -> anyhow::Result<Vec<exiv_shared::Permission>> {
        let row: Option<(sqlx::types::Json<Vec<exiv_shared::Permission>>,)> =
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
        permission: &exiv_shared::Permission,
        registry: &PluginRegistry,
    ) -> anyhow::Result<()> {
        // Reload current list, remove the target, write back atomically
        let mut perms = self.get_permissions(plugin_id).await?;
        let before = perms.len();
        perms.retain(|p| p != permission);
        if perms.len() == before {
            return Err(anyhow::anyhow!(
                "Permission '{:?}' is not granted to plugin '{}'",
                permission,
                plugin_id
            ));
        }
        let updated = serde_json::to_string(&perms)?;
        sqlx::query("UPDATE plugin_settings SET allowed_permissions = ? WHERE plugin_id = ?")
            .bind(&updated)
            .bind(plugin_id)
            .execute(&self.pool)
            .await?;

        // Update in-memory effective permissions
        let plugin_exiv_id = exiv_shared::ExivId::from_name(plugin_id);
        let mut perms_lock = registry.effective_permissions.write().await;
        if let Some(p) = perms_lock.get_mut(&plugin_exiv_id) {
            p.retain(|x| x != permission);
        }
        Ok(())
    }

    pub async fn grant_permission(
        &self,
        plugin_id: &str,
        permission: exiv_shared::Permission,
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
