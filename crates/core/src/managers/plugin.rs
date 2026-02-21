use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info};

use super::registry::{PluginRegistry, PluginSetting};
use crate::capabilities::SafeHttpClient;
use exiv_shared::{ExivId, Permission, PluginConfig, PluginFactory, PluginRuntimeContext};

/// L-01: Named constant for the official SDK magic seal value
const OFFICIAL_SDK_MAGIC: u32 = 0x56455253; // "VERS" in ASCII

pub struct PluginManager {
    pub pool: SqlitePool, // Made public for handlers if needed, or keep methods here
    factories: HashMap<String, Arc<dyn PluginFactory>>,
    http_client: Arc<SafeHttpClient>,
    store: Arc<crate::db::SqliteDataStore>,
    event_timeout_secs: u64,
    max_event_depth: u8,
    pub event_tx: Option<tokio::sync::mpsc::Sender<crate::EnvelopedEvent>>,
    pub bridge_semaphore: Arc<tokio::sync::Semaphore>,
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
            pool: pool.clone(),
            factories: HashMap::new(),
            http_client: Arc::new(SafeHttpClient::new(allowed_hosts)?),
            store: Arc::new(crate::db::SqliteDataStore::new(pool)),
            event_timeout_secs,
            max_event_depth,
            event_tx: None,
            bridge_semaphore: Arc::new(tokio::sync::Semaphore::new(20)),
            shutdown: Arc::new(tokio::sync::Notify::new()),
        })
    }

    pub fn set_event_tx(&mut self, tx: tokio::sync::mpsc::Sender<crate::EnvelopedEvent>) {
        self.event_tx = Some(tx);
    }

    pub fn register_factory(&mut self, factory: Arc<dyn PluginFactory>) {
        self.factories.insert(factory.name().to_string(), factory);
    }

    /// Register all built-in plugins (Principle #1: Core Minimalism / Dynamic Bootstrap)
    pub fn register_builtins(&mut self) {
        info!("🔍 Scanning for plugins via inventory...");

        let mut discovered_count = 0;
        for registrar in exiv_shared::inventory::iter::<exiv_shared::PluginRegistrar> {
            let factory = (registrar.factory)();
            info!("📦 Discovered plugin factory: {}", factory.name());
            self.register_factory(factory);
            discovered_count += 1;
        }

        if discovered_count == 0 {
            error!("⚠️ No plugin factories discovered! Check that:");
            error!("   1. Plugin crates are added to exiv_core/Cargo.toml");
            error!("   2. Plugin crates are imported in exiv_core/src/lib.rs");
            error!("   3. Full rebuild was performed (cargo clean && cargo build)");
        } else {
            info!("✅ Discovered {} plugin factories", discovered_count);
        }
    }

    pub async fn initialize_all(&self) -> anyhow::Result<PluginRegistry> {
        let mut registry = PluginRegistry::new(self.event_timeout_secs, self.max_event_depth);
        let (settings, mut config_map) = self.fetch_plugin_configs().await?;

        // Inject API keys from environment variables at runtime (never persisted to DB)
        if let Ok(key) = std::env::var("DEEPSEEK_API_KEY") {
            config_map
                .entry("mind.deepseek".to_string())
                .or_default()
                .entry("api_key".to_string())
                .or_insert(key);
        }
        if let Ok(key) = std::env::var("CEREBRAS_API_KEY") {
            config_map
                .entry("mind.cerebras".to_string())
                .or_default()
                .entry("api_key".to_string())
                .or_insert(key);
        }

        // M-11: Track failed plugins for summary reporting
        let mut failed_plugins = Vec::new();
        for setting in settings {
            let pid = setting.plugin_id.clone();
            let config_values = config_map.remove(&pid).unwrap_or_default();

            if let Err(e) = self
                .bootstrap_plugin(setting, config_values, &mut registry)
                .await
            {
                error!(plugin_id = %pid, error = %e, "❌ Failed to bootstrap plugin");
                failed_plugins.push(pid);
            }
        }

        if !failed_plugins.is_empty() {
            tracing::warn!(
                count = failed_plugins.len(),
                plugins = ?failed_plugins,
                "⚠️ {} plugin(s) failed to initialize",
                failed_plugins.len()
            );
        }

        Ok(registry)
    }

    async fn fetch_plugin_configs(
        &self,
    ) -> anyhow::Result<(Vec<PluginSetting>, HashMap<String, HashMap<String, String>>)> {
        let settings: Vec<PluginSetting> =
            sqlx::query_as("SELECT plugin_id, is_active, allowed_permissions FROM plugin_settings WHERE is_active = 1")
                .fetch_all(&self.pool)
                .await?;

        let config_rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT plugin_id, config_key, config_value FROM plugin_configs LIMIT 10000",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut config_map: HashMap<String, HashMap<String, String>> = HashMap::new();
        for (pid, k, v) in config_rows {
            config_map.entry(pid).or_default().insert(k, v);
        }

        Ok((settings, config_map))
    }

    async fn bootstrap_plugin(
        &self,
        setting: PluginSetting,
        config_values: HashMap<String, String>,
        registry: &mut PluginRegistry,
    ) -> anyhow::Result<()> {
        let plugin_id_str = &setting.plugin_id;

        // 🔍 Factory lookup with fallback for Python plugins (Principle #2: Capability over Type)
        let factory = self
            .factories
            .get(plugin_id_str)
            .or_else(|| {
                if plugin_id_str.starts_with("python.") {
                    debug!(
                        "Fallback: Using 'bridge.python' factory for plugin ID '{}'",
                        plugin_id_str
                    );
                    self.factories.get("bridge.python")
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                anyhow::anyhow!("No factory found for enabled plugin: {}", plugin_id_str)
            })?;

        let config = PluginConfig {
            id: plugin_id_str.clone(),
            config_values,
        };

        info!(plugin_id = %plugin_id_str, "🔌 Initializing plugin");
        let plugin = factory.create(config).await?;
        let manifest = plugin.manifest();

        // 🛂 入国審査 (Magic Seal Validation)
        if manifest.magic_seal != OFFICIAL_SDK_MAGIC {
            return Err(anyhow::anyhow!(
                "Access Denied: Plugin '{}' is not compiled with official SDK",
                manifest.name
            ));
        }

        info!(
            plugin_id = %plugin_id_str,
            manifest_name = %manifest.name,
            sdk_version = %manifest.sdk_version,
            "✅ Plugin verified"
        );

        // 🔐 権限の注入 (Principles #5)
        let permissions = setting.allowed_permissions.0;

        // 🔌 Create a per-plugin async event bridge
        let (p_tx, mut p_rx) = tokio::sync::mpsc::channel::<exiv_shared::ExivEventData>(100);
        if let Some(main_tx) = &self.event_tx {
            let main_tx = main_tx.clone();
            let pid = ExivId::from_name(plugin_id_str);
            let semaphore = self.bridge_semaphore.clone();
            let shutdown = self.shutdown.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        () = shutdown.notified() => {
                            tracing::info!("Plugin event bridge shutting down for {}", pid);
                            break;
                        }
                        maybe_data = p_rx.recv() => {
                            let data = match maybe_data {
                                Some(d) => d,
                                None => break,
                            };
                            let _permit = if let Ok(p) = semaphore.acquire().await { p } else {
                                tracing::warn!("Semaphore closed during shutdown, stopping event bridge");
                                break;
                            };
                            let envelope = crate::EnvelopedEvent {
                                event: Arc::new(exiv_shared::ExivEvent::new(data)),
                                issuer: Some(pid),
                                correlation_id: None,
                                depth: 0,
                            };
                            if let Err(e) = main_tx.send(envelope).await {
                                error!("🔌 Failed to forward async plugin event from {}: {}", pid, e);
                            }
                        }
                    }
                }
            });
        }

        let context = PluginRuntimeContext {
            effective_permissions: permissions.clone(),
            store: Arc::new(crate::db::ScopedDataStore::new(
                self.store.clone(),
                plugin_id_str.clone(),
            )),
            event_tx: p_tx,
        };

        // 🔐 Bootstrap validation: warn if required_permissions exceed allowed_permissions
        for req_perm in &manifest.required_permissions {
            if !permissions.contains(req_perm) {
                tracing::warn!(
                    plugin_id = %plugin_id_str,
                    missing_permission = ?req_perm,
                    "⚠️  Plugin requires {:?} but permission is NOT granted. \
                     Plugin will start with reduced capabilities.",
                    req_perm
                );
            }
        }

        // 💉 Capability Injection: inject all granted capabilities
        let network_capability = if permissions.contains(&Permission::NetworkAccess) {
            self.get_capability_for_permission(&Permission::NetworkAccess)
                .map(|c| {
                    let exiv_shared::PluginCapability::Network(net) = c else {
                        unreachable!()
                    };
                    net
                })
        } else {
            None
        };

        plugin.on_plugin_init(context, network_capability).await?;

        // Inject additional capabilities for granted permissions
        for cap_perm in &[
            Permission::FileRead,
            Permission::FileWrite,
            Permission::ProcessExecution,
        ] {
            if permissions.contains(cap_perm) {
                if let Some(cap) = self.get_capability_for_permission(cap_perm) {
                    if let Err(e) = plugin.on_capability_injected(cap).await {
                        tracing::warn!(
                            plugin_id = %plugin_id_str,
                            permission = ?cap_perm,
                            error = %e,
                            "Failed to inject capability for {:?}",
                            cap_perm
                        );
                    }
                }
            }
        }

        // H-05: Atomic plugin registration - acquire both locks before inserting
        {
            let mut plugins = registry.plugins.write().await;
            let mut perms_lock = registry.effective_permissions.write().await;
            plugins.insert(plugin_id_str.clone(), plugin.clone());
            perms_lock.insert(ExivId::from_name(&manifest.id), permissions);
        }

        Ok(())
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

    /// L5: Register a Python Bridge plugin at runtime into a live PluginRegistry.
    /// Runtime plugins are ephemeral (not persisted to DB) and use the `python.runtime.*` namespace.
    pub async fn register_runtime_plugin(
        &self,
        plugin_id: &str,
        config_values: HashMap<String, String>,
        permissions: Vec<Permission>,
        registry: &PluginRegistry,
    ) -> anyhow::Result<()> {
        // 1. Namespace validation
        if !plugin_id.starts_with("python.runtime.") {
            return Err(anyhow::anyhow!(
                "Runtime plugin ID must start with 'python.runtime.', got: {}",
                plugin_id
            ));
        }

        // 2. Duplicate check
        {
            let plugins = registry.plugins.read().await;
            if plugins.contains_key(plugin_id) {
                return Err(anyhow::anyhow!(
                    "Plugin '{}' is already registered",
                    plugin_id
                ));
            }
        }

        // 3. Use bridge.python factory
        let factory = self
            .factories
            .get("bridge.python")
            .ok_or_else(|| anyhow::anyhow!("bridge.python factory not found"))?;

        let config = PluginConfig {
            id: plugin_id.to_string(),
            config_values,
        };

        info!(plugin_id = %plugin_id, "🔌 L5: Runtime plugin registration");
        let plugin = factory.create(config).await?;
        let manifest = plugin.manifest();

        // 4. Magic Seal validation
        if manifest.magic_seal != OFFICIAL_SDK_MAGIC {
            return Err(anyhow::anyhow!(
                "Access Denied: Runtime plugin '{}' is not compiled with official SDK",
                plugin_id
            ));
        }

        // 5. Per-plugin event bridge (same pattern as bootstrap_plugin)
        let (p_tx, mut p_rx) = tokio::sync::mpsc::channel::<exiv_shared::ExivEventData>(100);
        if let Some(main_tx) = &self.event_tx {
            let main_tx = main_tx.clone();
            let pid = ExivId::from_name(plugin_id);
            let semaphore = self.bridge_semaphore.clone();
            let shutdown = self.shutdown.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        () = shutdown.notified() => {
                            tracing::info!("Runtime plugin event bridge shutting down for {}", pid);
                            break;
                        }
                        maybe_data = p_rx.recv() => {
                            let data = match maybe_data {
                                Some(d) => d,
                                None => break,
                            };
                            let _permit = match semaphore.acquire().await {
                                Ok(p) => p,
                                Err(_) => break,
                            };
                            let envelope = crate::EnvelopedEvent {
                                event: Arc::new(exiv_shared::ExivEvent::new(data)),
                                issuer: Some(pid),
                                correlation_id: None,
                                depth: 0,
                            };
                            let _ = main_tx.send(envelope).await;
                        }
                    }
                }
            });
        }

        // 6. PluginRuntimeContext with scoped data store
        let context = PluginRuntimeContext {
            effective_permissions: permissions.clone(),
            store: Arc::new(crate::db::ScopedDataStore::new(
                self.store.clone(),
                plugin_id.to_string(),
            )),
            event_tx: p_tx,
        };

        // 7. Capability injection
        let network_capability = if permissions.contains(&Permission::NetworkAccess) {
            self.get_capability_for_permission(&Permission::NetworkAccess)
                .map(|c| {
                    let exiv_shared::PluginCapability::Network(net) = c else {
                        unreachable!()
                    };
                    net
                })
        } else {
            None
        };

        // 8. Initialize (triggers Python subprocess handshake)
        plugin.on_plugin_init(context, network_capability).await?;

        // 9. Atomic registration
        {
            let mut plugins = registry.plugins.write().await;
            let mut perms_lock = registry.effective_permissions.write().await;
            plugins.insert(plugin_id.to_string(), plugin);
            perms_lock.insert(ExivId::from_name(plugin_id), permissions);
        }

        info!(plugin_id = %plugin_id, "✅ L5: Runtime plugin registered successfully");
        Ok(())
    }
}
