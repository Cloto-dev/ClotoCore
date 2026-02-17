use std::sync::Arc;
use std::collections::HashMap;
use sqlx::SqlitePool;
use tracing::{info, error, debug};

use exiv_shared::{
    AgentMetadata, Plugin, PluginConfig, PluginFactory, PluginManifest,
    ExivId, Permission, PluginRuntimeContext
};
use crate::capabilities::SafeHttpClient;

/// L-01: Named constant for the official SDK magic seal value
const OFFICIAL_SDK_MAGIC: u32 = 0x56455253; // "VERS" in ASCII

#[derive(sqlx::FromRow, Debug)]
pub struct PluginSetting {
    pub plugin_id: String,
    pub is_active: bool,
    pub allowed_permissions: sqlx::types::Json<Vec<Permission>>,
}

pub struct PluginRegistry {
    pub plugins: tokio::sync::RwLock<HashMap<String, Arc<dyn Plugin>>>,
    pub effective_permissions: tokio::sync::RwLock<HashMap<ExivId, Vec<Permission>>>,
    pub event_timeout_secs: u64,
    pub max_event_depth: u8,
    pub event_semaphore: Arc<tokio::sync::Semaphore>,
}

pub struct SystemMetrics {
    pub total_requests: std::sync::atomic::AtomicU64,
    pub total_memories: std::sync::atomic::AtomicU64,
    pub total_episodes: std::sync::atomic::AtomicU64,
}

impl Default for SystemMetrics {
    fn default() -> Self {
        Self {
            total_requests: std::sync::atomic::AtomicU64::new(0),
            total_memories: std::sync::atomic::AtomicU64::new(0),
            total_episodes: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl SystemMetrics {
    pub fn new() -> Self {
        Self::default()
    }
}

impl PluginRegistry {
    pub fn new(event_timeout_secs: u64, max_event_depth: u8) -> Self {
        Self {
            plugins: tokio::sync::RwLock::new(HashMap::new()),
            effective_permissions: tokio::sync::RwLock::new(HashMap::new()),
            event_timeout_secs,
            max_event_depth,
            event_semaphore: Arc::new(tokio::sync::Semaphore::new(50)),
        }
    }

    pub async fn update_effective_permissions(&self, plugin_id: ExivId, permission: Permission) {
        let mut perms_lock = self.effective_permissions.write().await;
        let perms = perms_lock.entry(plugin_id).or_default();
        if !perms.contains(&permission) {
            perms.push(permission);
        }
    }

    pub async fn list_plugins(&self) -> Vec<PluginManifest> {
        let plugins = self.plugins.read().await;
        plugins.values().map(|p| p.manifest()).collect()
    }

    pub async fn get_engine(&self, id: &str) -> Option<Arc<dyn Plugin>> {
        let plugins = self.plugins.read().await;
        plugins.get(id).cloned()
    }

    pub async fn find_memory(&self) -> Option<Arc<dyn Plugin>> {
        let plugins = self.plugins.read().await;
        for plugin in plugins.values() {
            if plugin.as_memory().is_some() {
                return Some(plugin.clone());
            }
        }
        None
    }

    /// 全てのアクティブなプラグインにイベントを配信する
    pub async fn dispatch_event(
        &self,
        envelope: crate::EnvelopedEvent,
        event_tx: &tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
    ) {
        let event = envelope.event.clone();
        let current_depth = envelope.depth;
        
        // 🚨 連鎖爆発の防止 (Guardrail #2)
        if current_depth >= self.max_event_depth {
            error!(
                event_type = ?event,
                depth = current_depth,
                "🛑 Event cascading limit reached ({}). Dropping event to prevent infinite loop.",
                self.max_event_depth
            );
            return;
        }

        let plugins = self.plugins.read().await;
        
        use futures::stream::{FuturesUnordered, StreamExt};
        use futures::FutureExt;
        let mut futures = FuturesUnordered::new();

        for (id, plugin) in plugins.iter() {
            let plugin = plugin.clone();
            let event = event.clone();
            let id = id.clone();
            let timeout_duration = std::time::Duration::from_secs(self.event_timeout_secs);
            let semaphore = self.event_semaphore.clone();

            futures.push(tokio::spawn(async move {
                let _permit = match semaphore.acquire().await {
                    Ok(p) => p,
                    Err(_) => {
                        tracing::warn!("Semaphore closed during shutdown, skipping plugin {}", id);
                        return (id, Ok(Ok(None)));
                    }
                };
                // Catch panics to prevent semaphore permit leaks
                let result = tokio::time::timeout(
                    timeout_duration,
                    async {
                        match std::panic::AssertUnwindSafe(plugin.on_event(&event))
                            .catch_unwind()
                            .await
                        {
                            Ok(r) => r,
                            Err(_) => Err(anyhow::anyhow!("Plugin panicked during on_event")),
                        }
                    }
                ).await;
                // _permit dropped here automatically (even on panic path above)
                (id, result)
            }));
        }

        // ロックを早めに解放
        drop(plugins);

        // 完了した順に結果を処理
        while let Some(join_result) = futures.next().await {
            let (id, timeout_result) = match join_result {
                Ok(pair) => pair,
                Err(e) => {
                    error!("🔥 Plugin task PANICKED or was cancelled: {}", e);
                    continue;
                }
            };

            match timeout_result {
                Ok(Ok(Some(new_event_data))) => {
                    let tx = event_tx.clone();
                    let id_clone = id.clone();
                    let trace_id = event.trace_id;
                    let semaphore = self.event_semaphore.clone();
                    tokio::spawn(redispatch_plugin_event(
                        tx,
                        id_clone,
                        trace_id,
                        new_event_data,
                        current_depth,
                        semaphore,
                    ));
                }
                Ok(Ok(None)) => {}
                Ok(Err(e)) => {
                    error!("🔌 Plugin {} on_event error: {}", id, e);
                }
                Err(_) => {
                    error!("⏱️ Plugin {} timed out during event processing", id);
                }
            }
        }
    }
}

/// Helper function to re-dispatch plugin events asynchronously
async fn redispatch_plugin_event(
    tx: tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
    plugin_id: String,
    trace_id: ExivId,
    new_event_data: exiv_shared::ExivEventData,
    current_depth: u8,
    semaphore: Arc<tokio::sync::Semaphore>,
) {
    let _permit = match semaphore.acquire().await {
        Ok(p) => p,
        Err(_) => {
            tracing::warn!("Semaphore closed during shutdown, skipping redispatch for {}", plugin_id);
            return;
        }
    };
    let issuer_id = ExivId::from_name(&plugin_id);
    let envelope = crate::EnvelopedEvent {
        event: Arc::new(exiv_shared::ExivEvent::with_trace(trace_id, new_event_data)),
        issuer: Some(issuer_id),
        correlation_id: Some(trace_id),
        depth: current_depth + 1,
    };
    if let Err(e) = tx.send(envelope).await {
        error!("🔌 Failed to re-dispatch plugin event: {}", e);
    }
}

pub struct PluginManager {
    pub pool: SqlitePool, // Made public for handlers if needed, or keep methods here
    factories: HashMap<String, Arc<dyn PluginFactory>>,
    http_client: Arc<SafeHttpClient>,
    store: Arc<crate::db::SqliteDataStore>,
    event_timeout_secs: u64,
    max_event_depth: u8,
    pub event_tx: Option<tokio::sync::mpsc::Sender<crate::EnvelopedEvent>>,
    pub bridge_semaphore: Arc<tokio::sync::Semaphore>,
}

impl PluginManager {
    pub fn new(pool: SqlitePool, allowed_hosts: Vec<String>, event_timeout_secs: u64, max_event_depth: u8) -> anyhow::Result<Self> {
        Ok(Self {
            pool: pool.clone(),
            factories: HashMap::new(),
            http_client: Arc::new(SafeHttpClient::new(allowed_hosts)?),
            store: Arc::new(crate::db::SqliteDataStore::new(pool)),
            event_timeout_secs,
            max_event_depth,
            event_tx: None,
            bridge_semaphore: Arc::new(tokio::sync::Semaphore::new(20)),
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
            config_map.entry("mind.deepseek".to_string()).or_default()
                .entry("api_key".to_string()).or_insert(key);
        }
        if let Ok(key) = std::env::var("CEREBRAS_API_KEY") {
            config_map.entry("mind.cerebras".to_string()).or_default()
                .entry("api_key".to_string()).or_insert(key);
        }

        // M-11: Track failed plugins for summary reporting
        let mut failed_plugins = Vec::new();
        for setting in settings {
            let pid = setting.plugin_id.clone();
            let config_values = config_map.remove(&pid).unwrap_or_default();

            if let Err(e) = self.bootstrap_plugin(setting, config_values, &mut registry).await {
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

    async fn fetch_plugin_configs(&self) -> anyhow::Result<(Vec<PluginSetting>, HashMap<String, HashMap<String, String>>)> {
        let settings: Vec<PluginSetting> =
            sqlx::query_as("SELECT plugin_id, is_active, allowed_permissions FROM plugin_settings WHERE is_active = 1")
                .fetch_all(&self.pool)
                .await?;

        let config_rows: Vec<(String, String, String)> =
            // H-07: Removed arbitrary LIMIT to ensure all plugin configs are loaded
            sqlx::query_as("SELECT plugin_id, config_key, config_value FROM plugin_configs")
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
        let factory = self.factories.get(plugin_id_str)
            .or_else(|| {
                if plugin_id_str.starts_with("python.") {
                    debug!("Fallback: Using 'bridge.python' factory for plugin ID '{}'", plugin_id_str);
                    self.factories.get("bridge.python")
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow::anyhow!("No factory found for enabled plugin: {}", plugin_id_str))?;

        let config = PluginConfig {
            id: plugin_id_str.clone(),
            config_values,
        };

        info!(plugin_id = %plugin_id_str, "🔌 Initializing plugin");
        let plugin = factory.create(config).await?;
        let manifest = plugin.manifest();
        
        // 🛂 入国審査 (Magic Seal Validation)
        if manifest.magic_seal != OFFICIAL_SDK_MAGIC {
            return Err(anyhow::anyhow!("Access Denied: Plugin '{}' is not compiled with official SDK", manifest.name));
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
            tokio::spawn(async move {
                while let Some(data) = p_rx.recv().await {
                    let _permit = match semaphore.acquire().await {
                        Ok(p) => p,
                        Err(_) => {
                            tracing::warn!("Semaphore closed during shutdown, stopping event bridge");
                            break;
                        }
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
            });
        }

        let context = PluginRuntimeContext {
            effective_permissions: permissions.clone(),
            store: Arc::new(crate::db::ScopedDataStore::new(self.store.clone(), plugin_id_str.clone())),
            event_tx: p_tx,
        };
        
        // 💉 Capability Injection
        let network_capability = if permissions.contains(&Permission::NetworkAccess) {
            self.get_capability_for_permission(&Permission::NetworkAccess)
                .map(|c| {
                    let exiv_shared::PluginCapability::Network(net) = c;
                    net
                })
        } else {
            None
        };

        plugin.on_plugin_init(context, network_capability).await?;
        
        // H-05: Atomic plugin registration - acquire both locks before inserting
        {
            let mut plugins = registry.plugins.write().await;
            let mut perms_lock = registry.effective_permissions.write().await;
            plugins.insert(plugin_id_str.clone(), plugin.clone());
            perms_lock.insert(ExivId::from_name(&manifest.id), permissions);
        }

        Ok(())
    }

    pub fn get_capability_for_permission(&self, permission: &Permission) -> Option<exiv_shared::PluginCapability> {
        match permission {
            Permission::NetworkAccess => Some(exiv_shared::PluginCapability::Network(self.http_client.clone())),
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

    pub async fn list_plugins_with_settings(&self, registry: &PluginRegistry) -> anyhow::Result<Vec<PluginManifest>> {
        let rows: Vec<PluginSetting> = sqlx::query_as(
            "SELECT plugin_id, is_active, allowed_permissions FROM plugin_settings LIMIT 100"
        )
            .fetch_all(&self.pool)
            .await?;
        
        let settings: HashMap<String, bool> = rows.into_iter()
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

    pub async fn grant_permission(&self, plugin_id: &str, permission: exiv_shared::Permission) -> anyhow::Result<()> {
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
            )"
        )
        .bind(&perm_json)
        .bind(plugin_id)
        .bind(&perm_json)
        .execute(&self.pool).await?;
        Ok(())
    }
}

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

    pub async fn get_agent_config(&self, agent_id: &str) -> anyhow::Result<(AgentMetadata, String)> {
        let row: AgentRow = sqlx::query_as(
            "SELECT id, name, description, enabled, last_seen, default_engine_id, \
             required_capabilities, metadata, power_password_hash FROM agents WHERE id = ?"
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
             required_capabilities, metadata, power_password_hash FROM agents"
        )
        .fetch_all(&self.pool)
        .await?;

        let agents: Vec<AgentMetadata> = rows.into_iter().map(Self::row_to_metadata).collect();

        for agent in &agents {
            debug!("Agent {} engine is {:?}", agent.name, agent.default_engine_id);
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
            if pw.is_empty() { None } else { Some(Self::hash_password(pw)?) }
        } else {
            None
        };

        sqlx::query(
            "INSERT INTO agents (id, name, description, default_engine_id, status, \
             enabled, last_seen, metadata, required_capabilities, power_password_hash) \
             VALUES (?, ?, ?, ?, 'online', 1, ?, ?, ?, ?)"
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
        let now_ms = if enabled { chrono::Utc::now().timestamp_millis() } else { 0 };
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
        let row: (Option<String>,) = sqlx::query_as(
            "SELECT power_password_hash FROM agents WHERE id = ?"
        )
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

        let parsed_hash = PasswordHash::new(hash)
            .map_err(|e| anyhow::anyhow!("Invalid password hash: {}", e))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
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
