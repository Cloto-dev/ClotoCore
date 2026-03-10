//! MCP Client Manager — orchestrates all MCP server lifecycles.
//!
//! Responsible for spawning, stopping, and restarting MCP server processes,
//! routing tool calls to the correct server, managing manifests, and
//! forwarding kernel events as MCP notifications.

pub use super::mcp_client::{McpClient, McpNotification};
pub use super::mcp_events::CallbackHandleResult;
pub use super::mcp_types::*;

use super::mcp_mgp::{self, ToolSecurityMetadata};
use super::mcp_protocol::{McpConfigFile, McpServerConfig, ToolContent};
use super::mcp_tool_validator::validate_tool_arguments;
use super::mcp_transport;
use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, error, info, warn};

/// Approver identity used for YOLO-mode auto-approved permissions.
const YOLO_APPROVER_ID: &str = "YOLO";

// ============================================================
// McpClientManager — kernel-level MCP server orchestrator
// ============================================================

/// G1.3: Unified MCP state — single RwLock avoids fragmented locking.
pub(crate) struct McpState {
    pub servers: HashMap<String, McpServerHandle>,
    pub tool_index: HashMap<String, String>,
    pub stopped_configs: HashMap<String, (McpServerConfig, ServerSource)>,
}

pub struct McpClientManager {
    pub(crate) state: RwLock<McpState>,
    pool: SqlitePool,
    /// YOLO mode: auto-approve all MCP server permissions (ARCHITECTURE.md §5.7).
    /// Arc<AtomicBool> allows runtime toggle via API without restart.
    pub yolo_mode: Arc<AtomicBool>,
    /// Shared notification channel — all MCP servers' notifications are collected here
    notification_tx: mpsc::Sender<McpNotification>,
    notification_rx: Mutex<Option<mpsc::Receiver<McpNotification>>>,
    mcp_request_timeout_secs: u64,
    /// Kernel event bus sender — set after AppState creation for PermissionRequested emission.
    kernel_event_tx: Mutex<Option<mpsc::Sender<crate::EnvelopedEvent>>>,
    /// Lifecycle manager for restart policies (MGP §11)
    pub(super) lifecycle: super::mcp_lifecycle::LifecycleManager,
    /// Event subscription + callback manager (MGP §13)
    pub(super) events: super::mcp_events::EventManager,
    /// Rich tool index for dynamic discovery (MGP §16)
    pub(super) rich_tool_index: super::mcp_tool_discovery::ToolIndex,
    /// Per-agent session tool cache (MGP §16.7)
    pub(super) session_cache: super::mcp_tool_discovery::SessionToolCache,
    /// Stream chunk assembler for gap detection (MGP §12)
    pub(super) stream_assembler: super::mcp_streaming::StreamAssembler,
    /// Capability-based tool routing (P1 Core Minimalism)
    pub(crate) dispatcher: super::capability_dispatcher::CapabilityDispatcher,
    /// LLM proxy port for isolation NetworkScope::ProxyOnly (MGP §8-10).
    llm_proxy_port: u16,
    /// Env var keys that contain LLM API secrets — stripped from child process env.
    sensitive_env_keys: Vec<String>,
    /// Master switch for OS-level isolation (MGP §8-10).
    isolation_enabled: bool,
    /// Whether to allow unsigned MCP servers (no Magic Seal check).
    allow_unsigned: bool,
    /// Base directory for per-server sandboxes.
    sandbox_base_dir: std::path::PathBuf,
}

impl McpClientManager {
    #[must_use]
    pub fn new(pool: SqlitePool, yolo_mode: bool, mcp_request_timeout_secs: u64) -> Self {
        let (notification_tx, notification_rx) = mpsc::channel(256);
        Self {
            state: RwLock::new(McpState {
                servers: HashMap::new(),
                tool_index: HashMap::new(),
                stopped_configs: HashMap::new(),
            }),
            pool,
            yolo_mode: Arc::new(AtomicBool::new(yolo_mode)),
            notification_tx,
            notification_rx: Mutex::new(Some(notification_rx)),
            mcp_request_timeout_secs,
            lifecycle: super::mcp_lifecycle::LifecycleManager::new(),
            events: super::mcp_events::EventManager::new(),
            rich_tool_index: super::mcp_tool_discovery::ToolIndex::new(),
            session_cache: super::mcp_tool_discovery::SessionToolCache::new(),
            stream_assembler: super::mcp_streaming::StreamAssembler::new(),
            dispatcher: super::capability_dispatcher::CapabilityDispatcher::new(),
            kernel_event_tx: Mutex::new(None),
            llm_proxy_port: 8082,
            sensitive_env_keys: Vec::new(),
            isolation_enabled: true,
            allow_unsigned: true,
            sandbox_base_dir: std::path::PathBuf::from("data/mcp-sandbox"),
        }
    }

    /// Configure isolation settings from AppConfig (called once at startup).
    pub fn configure_isolation(&mut self, config: &crate::config::AppConfig) {
        self.llm_proxy_port = config.llm_proxy_port;
        self.isolation_enabled = config.isolation_enabled;
        self.allow_unsigned = config.allow_unsigned;
        self.sandbox_base_dir = config.sandbox_base_dir.clone();
        // Derive sensitive env keys from LLM provider env mappings.
        self.sensitive_env_keys = config
            .llm_provider_env_mappings
            .iter()
            .map(|(_, env_key)| env_key.clone())
            .collect();
    }

    /// Set the kernel event bus sender (called once after AppState creation).
    pub async fn set_kernel_event_tx(&self, tx: mpsc::Sender<crate::EnvelopedEvent>) {
        *self.kernel_event_tx.lock().await = Some(tx);
    }

    /// Take the notification receiver (can only be called once).
    /// The Kernel event loop uses this to forward MCP notifications to the event bus.
    pub async fn take_notification_receiver(&self) -> Option<mpsc::Receiver<McpNotification>> {
        self.notification_rx.lock().await.take()
    }

    /// Deliver a permission grant to a connected server (C1: bug-302).
    ///
    /// Sends `mgp/permission/grant` RPC and emits `PermissionGranted` event.
    pub async fn deliver_permission_grant(
        &self,
        server_id: &str,
        permission: &str,
        approved_by: &str,
    ) {
        let state = self.state.read().await;
        if let Some(handle) = state.servers.get(server_id) {
            if let Some(ref client) = handle.client {
                let grant_params = serde_json::json!({
                    "request_id": format!("perm-{}-{}", server_id, permission),
                    "grants": {permission: {"granted": true}},
                    "approved_by": approved_by,
                });
                if let Err(e) = client.call("mgp/permission/grant", Some(grant_params)).await {
                    debug!(
                        "Failed to send mgp/permission/grant to [{}]: {}",
                        server_id, e
                    );
                } else {
                    info!(
                        server = %server_id,
                        permission = %permission,
                        "Permission grant delivered to server"
                    );
                }
            }
        }
        drop(state);

        // Emit PermissionGranted event
        if let Some(tx) = self.kernel_event_tx.lock().await.as_ref() {
            let data = cloto_shared::ClotoEventData::PermissionGranted {
                plugin_id: server_id.to_string(),
                permission: permission.to_string(),
            };
            let envelope = crate::EnvelopedEvent::system(data);
            let _ = tx.send(envelope).await;
        }
    }

    /// Load server configs from mcp.toml file (if exists) and connect.
    ///
    /// Relative paths in `args` are resolved against the project root directory
    /// (detected by walking up from the config file to find `Cargo.toml`) or,
    /// in production, against the config file's parent directory.
    /// This allows `mcp.toml` to use portable paths like
    /// `"mcp-servers/terminal/server.py"` instead of absolute ones.
    pub async fn load_config_file(&self, config_path: &str) -> Result<()> {
        let configs = self.parse_config_file(config_path)?;
        self.connect_server_configs(&configs).await;
        Ok(())
    }

    /// Parse mcp.toml and resolve paths, returning server configs without connecting.
    pub fn parse_config_file(&self, config_path: &str) -> Result<Vec<McpServerConfig>> {
        let path = std::path::Path::new(config_path);
        if !path.exists() {
            info!("No MCP config file at {}, skipping", config_path);
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(path).context("Failed to read MCP config file")?;
        let config: McpConfigFile =
            toml::from_str(&content).context("Failed to parse MCP config file")?;

        // Determine the base directory for resolving relative paths.
        // In development: walk up from the config file to find the workspace root
        //   (directory containing `Cargo.toml`).
        // In production: fall back to the config file's parent directory.
        // IMPORTANT: base_dir must be absolute so that args resolve to absolute
        // paths — sandbox isolation changes cwd, breaking relative paths.
        let base_dir = Self::detect_project_root(path).unwrap_or_else(|| {
            let parent = path.parent().map_or_else(
                || std::path::PathBuf::from("."),
                std::path::Path::to_path_buf,
            );
            if parent.is_absolute() {
                parent
            } else {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join(&parent)
            }
        });

        info!(
            "Parsed {} MCP server(s) from {} (base_dir={})",
            config.servers.len(),
            config_path,
            base_dir.display()
        );

        let resolved: Vec<McpServerConfig> = config
            .servers
            .into_iter()
            .map(|mut server_config| {
                // Resolve relative command path against the base directory
                // (e.g. "target/debug/mgp-avatar" → absolute path)
                // On Windows, also try with .exe suffix for extensionless commands.
                {
                    let p = std::path::Path::new(&server_config.command);
                    if p.is_relative() && p.components().count() > 1 {
                        let resolved = base_dir.join(p);
                        if resolved.exists() {
                            server_config.command = resolved.to_string_lossy().to_string();
                        } else if cfg!(windows) && resolved.extension().is_none() {
                            let with_exe = resolved.with_extension("exe");
                            if with_exe.exists() {
                                server_config.command = with_exe.to_string_lossy().to_string();
                            }
                        }
                    }
                }

                // Resolve relative paths in args against the base directory
                server_config.args = server_config
                    .args
                    .into_iter()
                    .map(|arg| {
                        let p = std::path::Path::new(&arg);
                        if p.is_relative() {
                            let resolved = base_dir.join(p);
                            if resolved.exists() {
                                return resolved.to_string_lossy().to_string();
                            }
                        }
                        arg
                    })
                    .collect();

                // Inject CLOTO_PROJECT_DIR so sandboxed servers can resolve
                // relative paths (e.g. data/models/) against the project root.
                server_config
                    .env
                    .entry("CLOTO_PROJECT_DIR".to_string())
                    .or_insert_with(|| base_dir.to_string_lossy().to_string());

                server_config
            })
            .collect();

        Ok(resolved)
    }

    /// Connect a list of server configs, registering failures with Error status.
    pub async fn connect_server_configs(&self, configs: &[McpServerConfig]) {
        let total = configs.len();
        let mut failed = 0usize;

        for server_config in configs {
            if let Err(e) = self
                .connect_server(server_config.clone(), ServerSource::Config)
                .await
            {
                failed += 1;
                warn!(
                    id = %server_config.id,
                    error = %e,
                    "Failed to connect MCP server from config"
                );
                // Register with Error status so it appears in list_servers()
                let mut state = self.state.write().await;
                state.servers
                    .entry(server_config.id.clone())
                    .or_insert_with(|| McpServerHandle {
                        id: server_config.id.clone(),
                        config: server_config.clone(),
                        client: None,
                        tools: Vec::new(),
                        handshake: None,
                        mgp_negotiated: None,
                        status: ServerStatus::Error(e.to_string()),
                        source: ServerSource::Config,
                        audit_seq: Arc::new(AtomicU64::new(0)),
                        connected_at: None,
                        isolation_profile: None,
                    });
            }
        }

        if failed > 0 {
            warn!(
                total = total,
                failed = failed,
                "MCP config loaded with failures ({}/{} servers failed)",
                failed,
                total
            );
        }
    }

    /// Restore persisted MCP servers from the database.
    pub async fn restore_from_db(&self) -> Result<()> {
        let records = crate::db::load_active_mcp_servers(&self.pool).await?;
        if records.is_empty() {
            return Ok(());
        }

        info!("Restoring {} MCP server(s) from database", records.len());

        for record in records {
            // Skip placeholder rows inserted by migration (config-loaded servers
            // are already loaded from mcp.toml via load_config_file()).
            if record.command == "config-loaded" {
                continue;
            }

            // Skip servers already loaded from mcp.toml
            if self.state.read().await.servers.contains_key(&record.name) {
                continue;
            }

            let args: Vec<String> = serde_json::from_str(&record.args).unwrap_or_default();
            let db_env: HashMap<String, String> =
                serde_json::from_str(&record.env).unwrap_or_default();
            let config = McpServerConfig {
                id: record.name.clone(),
                command: record.command,
                args,
                env: db_env,
                transport: "stdio".to_string(),
                auto_restart: Some(true),
                required_permissions: Vec::new(),
                tool_validators: HashMap::new(),
                display_name: None,
                mgp: None,
                restart_policy: None,
                seal: None,
                isolation: None,
            };

            // Regenerate script file if needed
            if let Some(ref content) = record.script_content {
                let script_path =
                    std::path::Path::new("scripts").join(format!("mcp_{}.py", record.name));
                if !script_path.exists() {
                    let _ = std::fs::create_dir_all("scripts");
                    if let Err(e) = std::fs::write(&script_path, content) {
                        warn!(
                            error = %e,
                            name = %record.name,
                            "Failed to regenerate MCP server script"
                        );
                        continue;
                    }
                }
            }

            if let Err(e) = self
                .connect_server(config.clone(), ServerSource::Dynamic)
                .await
            {
                warn!(
                    name = %record.name,
                    error = %e,
                    "Failed to restore MCP server"
                );
                // Register with Error status so it appears in list_servers()
                let mut state = self.state.write().await;
                state.servers
                    .entry(config.id.clone())
                    .or_insert_with(|| McpServerHandle {
                        id: config.id.clone(),
                        config,
                        client: None,
                        tools: Vec::new(),
                        handshake: None,
                        mgp_negotiated: None,
                        status: ServerStatus::Error(e.to_string()),
                        source: ServerSource::Dynamic,
                        audit_seq: Arc::new(AtomicU64::new(0)),
                        connected_at: None,
                        isolation_profile: None,
                    });
            }
        }

        Ok(())
    }

    /// Connect to an MCP server with retry logic.
    #[allow(clippy::too_many_lines)]
    pub async fn connect_server(
        &self,
        config: McpServerConfig,
        source: ServerSource,
    ) -> Result<Vec<String>> {
        let id = config.id.clone();

        // Validate command against whitelist
        mcp_transport::validate_command(&config.command)?;

        // Check for duplicate — allow retry if server is in Error/Disconnected state
        {
            let state = self.state.read().await;
            if let Some(existing) = state.servers.get(&id) {
                if existing.status == ServerStatus::Connected {
                    return Err(anyhow::anyhow!("MCP server '{}' is already connected", id));
                }
                // Non-connected server will be replaced below
            }
        }

        // ──── Permission Gate (D): Check required_permissions ────
        if !config.required_permissions.is_empty() {
            if self.yolo_mode.load(Ordering::Relaxed) {
                // YOLO mode: auto-approve all permissions
                for perm in &config.required_permissions {
                    let already_approved = crate::db::is_permission_approved(&self.pool, &id, perm)
                        .await
                        .unwrap_or(false);
                    if !already_approved {
                        let request = crate::db::PermissionRequest {
                            request_id: format!("mcp-{}-{}", id, perm),
                            created_at: chrono::Utc::now(),
                            plugin_id: id.clone(),
                            permission_type: perm.clone(),
                            target_resource: None,
                            justification: format!(
                                "MCP server '{}' requires '{}' (auto-approved: YOLO mode)",
                                id, perm
                            ),
                            status: "auto-approved".to_string(),
                            approved_by: Some(YOLO_APPROVER_ID.to_string()),
                            approved_at: Some(chrono::Utc::now()),
                            expires_at: None,
                            metadata: None,
                        };
                        if let Err(e) =
                            crate::db::create_permission_request(&self.pool, request).await
                        {
                            // Ignore duplicate key errors (permission already exists)
                            debug!("Permission auto-approve note for [MCP] {}: {}", id, e);
                        }
                    }
                }
                warn!(
                    "YOLO mode: auto-approved {} permission(s) for MCP server '{}'",
                    config.required_permissions.len(),
                    id
                );
            } else {
                // Non-YOLO: check each permission, create pending requests for missing ones
                let mut pending_perms = Vec::new();
                for perm in &config.required_permissions {
                    let approved = crate::db::is_permission_approved(&self.pool, &id, perm)
                        .await
                        .unwrap_or(false);
                    if !approved {
                        pending_perms.push(perm.clone());
                        // Create a pending permission request for admin to approve
                        let request = crate::db::PermissionRequest {
                            request_id: format!("mcp-{}-{}", id, perm),
                            created_at: chrono::Utc::now(),
                            plugin_id: id.clone(),
                            permission_type: perm.clone(),
                            target_resource: None,
                            justification: format!(
                                "MCP server '{}' requires '{}' permission to operate",
                                id, perm
                            ),
                            status: "pending".to_string(),
                            approved_by: None,
                            approved_at: None,
                            expires_at: None,
                            metadata: Some(serde_json::json!({
                                "source": "mcp_permission_gate",
                                "server_command": config.command,
                            })),
                        };
                        if let Err(e) =
                            crate::db::create_permission_request(&self.pool, request).await
                        {
                            debug!("Permission request note for [MCP] {}: {}", id, e);
                        }
                    }
                }

                if !pending_perms.is_empty() {
                    // P8: Emit PermissionRequested events so SecurityGuard UI can display them
                    if let Some(tx) = self.kernel_event_tx.lock().await.as_ref() {
                        for perm in &pending_perms {
                            let data = cloto_shared::ClotoEventData::PermissionRequested {
                                plugin_id: id.clone(),
                                permission: perm.clone(),
                                reason: format!(
                                    "MCP server '{}' requires '{}' permission",
                                    id, perm
                                ),
                            };
                            let envelope = crate::EnvelopedEvent::system(data);
                            if let Err(e) = tx.send(envelope).await {
                                debug!("Failed to emit PermissionRequested event: {}", e);
                            }
                        }
                    }
                    return Err(anyhow::anyhow!(
                        "MCP server '{}' blocked: {} permission(s) pending approval: [{}]. \
                         Approve via dashboard or API, then retry.",
                        id,
                        pending_perms.len(),
                        pending_perms.join(", ")
                    ));
                }
            }
        }

        // ──── Seal Verification (MGP §8 L0) ────
        let trust_level = config
            .mgp
            .as_ref()
            .and_then(|m| m.trust_level.as_deref())
            .map(|s| serde_json::from_value::<super::mcp_mgp::TrustLevel>(
                serde_json::Value::String(s.to_string()),
            ))
            .transpose()
            .unwrap_or(None)
            .unwrap_or(super::mcp_mgp::TrustLevel::Standard);

        if let Some(ref seal_value) = config.seal {
            // Seal is present in config — verify the entry point binary.
            let entry_point = std::path::Path::new(&config.command);
            if entry_point.exists() {
                let seal_key = super::mcp_seal::load_or_generate_seal_key(
                    &self.sandbox_base_dir.parent().unwrap_or(std::path::Path::new("data")),
                )?;
                let status = super::mcp_seal::check_seal(
                    &trust_level,
                    Some(seal_value.as_str()),
                    entry_point,
                    &seal_key,
                    self.allow_unsigned,
                )?;
                match status {
                    super::mcp_seal::SealStatus::Verified => {
                        info!(id = %id, "Magic Seal verified for MCP server");
                    }
                    super::mcp_seal::SealStatus::Failed => {
                        return Err(anyhow::anyhow!(
                            "MCP server '{}': Magic Seal verification failed — binary may be tampered",
                            id
                        ));
                    }
                    _ => {}
                }
            } else {
                debug!(id = %id, "Seal configured but entry point is not a file path — skipping seal check");
            }
        } else if trust_level == super::mcp_mgp::TrustLevel::Untrusted && !self.allow_unsigned {
            return Err(anyhow::anyhow!(
                "MCP server '{}': Untrusted servers require a Magic Seal in production mode",
                id
            ));
        }

        // ──── Isolation Profile Derivation (MGP §8-10) ────
        let isolation_profile = if self.isolation_enabled {
            let approved_perms = config.required_permissions.clone();
            match super::mcp_isolation::derive_isolation_profile(
                trust_level.clone(),
                &approved_perms,
                config.isolation.as_ref(),
                &id,
                &self.sandbox_base_dir,
            ) {
                Ok(profile) => {
                    info!(
                        id = %id,
                        trust = ?trust_level,
                        fs = ?profile.filesystem_scope,
                        net = ?profile.network_scope,
                        "Isolation profile derived"
                    );
                    Some(profile)
                }
                Err(e) => {
                    warn!(id = %id, error = %e, "Failed to derive isolation profile — proceeding without isolation");
                    None
                }
            }
        } else {
            None
        };

        info!(
            "Connecting to MCP server [{}]: {} {:?}",
            id, config.command, config.args
        );

        // Set Connecting status
        {
            let mut state = self.state.write().await;
            if let Some(handle) = state.servers.get_mut(&id) {
                handle.status = ServerStatus::Connecting;
            }
        }

        // Retry with exponential backoff (3 attempts)
        let (client, mgp_server_caps) = {
            let mut result: Option<(McpClient, Option<super::mcp_mgp::MgpServerCapabilities>)> =
                None;
            let mut last_err = None;
            for attempt in 1..=3u32 {
                match McpClient::connect(
                    &id,
                    &config.command,
                    &config.args,
                    &config.env,
                    self.notification_tx.clone(),
                    self.mcp_request_timeout_secs,
                    isolation_profile.as_ref(),
                    self.llm_proxy_port,
                    &self.sensitive_env_keys,
                )
                .await
                {
                    Ok((c, caps)) => {
                        result = Some((c, caps));
                        break;
                    }
                    Err(e) => {
                        if attempt < 3 {
                            let delay = std::time::Duration::from_secs(1 << (attempt - 1));
                            warn!(
                                "Connection attempt {}/3 failed for [MCP] {}: {}. Retrying in {:?}...",
                                attempt, id, e, delay
                            );
                            tokio::time::sleep(delay).await;
                        }
                        last_err = Some(e);
                    }
                }
            }
            match result {
                Some((c, caps)) => (c, caps),
                None => {
                    return Err(anyhow::anyhow!(
                        "Failed to connect to MCP server '{}' after 3 attempts: {}",
                        id,
                        last_err.unwrap_or_else(|| anyhow::anyhow!("unknown error"))
                    ));
                }
            }
        };

        // MGP capability negotiation (§2)
        let config_trust = config.mgp.as_ref().and_then(|m| m.trust_level.as_deref());
        let mgp_negotiated = mcp_mgp::negotiate(mgp_server_caps.as_ref(), config_trust);
        if let Some(ref mgp) = mgp_negotiated {
            info!(
                "MGP negotiated for [{}]: v{}, extensions={:?}, trust={:?}",
                id, mgp.version, mgp.active_extensions, mgp.trust_level
            );
        }

        // ──── MGP Permission Flow (§3): check server-declared permissions ────
        if let (Some(ref mgp), Some(ref server_caps)) = (&mgp_negotiated, &mgp_server_caps) {
            if mgp.active_extensions.iter().any(|e| e == "permissions")
                && !server_caps.permissions_required.is_empty()
            {
                // Merge server-declared permissions with config permissions
                let mut all_perms: Vec<String> = config.required_permissions.clone();
                for perm in &server_caps.permissions_required {
                    if !all_perms.contains(perm) {
                        all_perms.push(perm.clone());
                    }
                }

                if self.yolo_mode.load(Ordering::Relaxed) {
                    // YOLO: auto-approve all server-declared permissions
                    for perm in &all_perms {
                        let already_approved =
                            crate::db::is_permission_approved(&self.pool, &id, perm)
                                .await
                                .unwrap_or(false);
                        if !already_approved {
                            let request = crate::db::PermissionRequest {
                                request_id: format!("mgp-{}-{}", id, perm),
                                created_at: chrono::Utc::now(),
                                plugin_id: id.clone(),
                                permission_type: perm.clone(),
                                target_resource: None,
                                justification: format!(
                                    "MGP server '{}' declares '{}' (auto-approved: YOLO mode)",
                                    id, perm
                                ),
                                status: "auto-approved".to_string(),
                                approved_by: Some(YOLO_APPROVER_ID.to_string()),
                                approved_at: Some(chrono::Utc::now()),
                                expires_at: None,
                                metadata: None,
                            };
                            if let Err(e) =
                                crate::db::create_permission_request(&self.pool, request).await
                            {
                                debug!("MGP permission auto-approve note for [{}]: {}", id, e);
                            }
                        }
                    }
                    // Send mgp/permission/grant RPC (§3.6)
                    let grant_params = serde_json::json!({
                        "request_id": format!("perm-{}", id),
                        "grants": all_perms.iter().map(|p| (p.clone(), serde_json::json!({
                            "decision": "approved",
                        }))).collect::<serde_json::Map<String, serde_json::Value>>(),
                        "approved_by": YOLO_APPROVER_ID,
                    });
                    if let Err(e) = client
                        .call("mgp/permission/grant", Some(grant_params))
                        .await
                    {
                        debug!("Failed to send mgp/permission/grant to [{}]: {}", id, e);
                    }

                    crate::db::spawn_audit_log(
                        self.pool.clone(),
                        crate::db::AuditLogEntry {
                            timestamp: chrono::Utc::now(),
                            event_type: "PERMISSION_GRANTED".to_string(),
                            actor_id: Some(YOLO_APPROVER_ID.to_string()),
                            target_id: Some(id.clone()),
                            permission: Some(all_perms.join(",")),
                            result: "approved".to_string(),
                            reason: "MGP Permission Flow (YOLO auto-approve)".to_string(),
                            metadata: None,
                            trace_id: None,
                        },
                    );
                } else {
                    // Non-YOLO: check each permission
                    let mut pending_perms = Vec::new();
                    for perm in &all_perms {
                        let approved = crate::db::is_permission_approved(&self.pool, &id, perm)
                            .await
                            .unwrap_or(false);
                        if !approved {
                            pending_perms.push(perm.clone());
                            let request = crate::db::PermissionRequest {
                                request_id: format!("mgp-{}-{}", id, perm),
                                created_at: chrono::Utc::now(),
                                plugin_id: id.clone(),
                                permission_type: perm.clone(),
                                target_resource: None,
                                justification: format!(
                                    "MGP server '{}' declares '{}' permission",
                                    id, perm
                                ),
                                status: "pending".to_string(),
                                approved_by: None,
                                approved_at: None,
                                expires_at: None,
                                metadata: Some(serde_json::json!({
                                    "source": "mgp_permission_flow",
                                    "server_command": config.command,
                                })),
                            };
                            if let Err(e) =
                                crate::db::create_permission_request(&self.pool, request).await
                            {
                                debug!("MGP permission request note for [{}]: {}", id, e);
                            }
                        }
                    }

                    if !pending_perms.is_empty() {
                        // Send mgp/permission/await RPC (§3.5)
                        let await_params = serde_json::json!({
                            "request_id": format!("perm-{}", id),
                            "permissions": pending_perms,
                            "policy": "interactive",
                            "message": "Waiting for operator approval",
                        });
                        if let Err(e) = client
                            .call("mgp/permission/await", Some(await_params))
                            .await
                        {
                            debug!("Failed to send mgp/permission/await to [{}]: {}", id, e);
                        }

                        crate::db::spawn_audit_log(
                            self.pool.clone(),
                            crate::db::AuditLogEntry {
                                timestamp: chrono::Utc::now(),
                                event_type: "PERMISSION_DENIED".to_string(),
                                actor_id: None,
                                target_id: Some(id.clone()),
                                permission: Some(pending_perms.join(",")),
                                result: "pending".to_string(),
                                reason: "MGP Permission Flow (pending approval)".to_string(),
                                metadata: None,
                                trace_id: None,
                            },
                        );

                        return Err(anyhow::anyhow!(
                            "MGP server '{}' blocked: {} permission(s) pending approval: [{}]. \
                             Approve via dashboard or API, then retry.",
                            id,
                            pending_perms.len(),
                            pending_perms.join(", ")
                        ));
                    }

                    // All approved → send grant RPC (§3.6)
                    let grant_params = serde_json::json!({
                        "request_id": format!("perm-{}", id),
                        "grants": all_perms.iter().map(|p| (p.clone(), serde_json::json!({
                            "decision": "approved",
                        }))).collect::<serde_json::Map<String, serde_json::Value>>(),
                        "approved_by": "operator",
                    });
                    if let Err(e) = client
                        .call("mgp/permission/grant", Some(grant_params))
                        .await
                    {
                        debug!("Failed to send mgp/permission/grant to [{}]: {}", id, e);
                    }
                }
            }
        }

        // Send initialized notification (after Permission Flow completes)
        if let Err(e) = client.send_initialized_notification().await {
            return Err(anyhow::anyhow!(
                "Failed to send initialized notification to [{}]: {}",
                id,
                e
            ));
        }

        // Discover tools
        let tools = match client.list_tools().await {
            Ok(result) => {
                info!("Found {} tools on [MCP] {}", result.tools.len(), id);
                for tool in &result.tools {
                    info!(
                        "  - {}: {}",
                        tool.name,
                        tool.description.as_deref().unwrap_or_default()
                    );
                }
                result.tools
            }
            Err(e) => {
                error!("Failed to list tools from [MCP] {}: {}", id, e);
                Vec::new()
            }
        };

        // Attempt cloto/handshake (optional, skipped when MGP negotiation succeeded)
        let handshake = if mgp_negotiated.is_some() {
            debug!("Skipping cloto/handshake for [MCP] {} (MGP negotiated)", id);
            None
        } else {
            match client.cloto_handshake().await {
                Ok(h) => {
                    if h.is_some() {
                        info!("Cloto handshake succeeded for [MCP] {}", id);
                    }
                    h
                }
                Err(e) => {
                    debug!("Cloto handshake failed for [MCP] {}: {}", id, e);
                    None
                }
            }
        };

        let tool_names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
        let client_arc = Arc::new(client);

        let handle = McpServerHandle {
            id: id.clone(),
            config,
            client: Some(client_arc),
            tools: tools.clone(),
            handshake,
            mgp_negotiated,
            status: ServerStatus::Connected,
            source,
            audit_seq: Arc::new(AtomicU64::new(0)),
            connected_at: Some(std::time::Instant::now()),
            isolation_profile,
        };

        // Register in servers map + update tool routing index under single lock.
        // Skip mind.* servers — their tools (think, think_with_tools) are engine-internal
        // and called directly via call_server_tool(engine_id, ...), not through tool_index.
        {
            let mut state = self.state.write().await;
            state.servers.insert(id.clone(), handle);

            if !id.starts_with("mind.") {
                for tool in &tools {
                    // §1.6.3: Reject tools with reserved mgp.* namespace
                    if tool.name.starts_with("mgp.") {
                        warn!(
                            tool = %tool.name,
                            server = %id,
                            "Server tool conflicts with reserved mgp.* namespace — skipping"
                        );
                        continue;
                    }
                    if let Some(existing) = state.tool_index.get(&tool.name) {
                        warn!(
                            tool = %tool.name,
                            existing_server = %existing,
                            new_server = %id,
                            "Tool name collision — overwriting routing"
                        );
                    }
                    state.tool_index.insert(tool.name.clone(), id.clone());
                }

                // Populate rich tool index for §16 dynamic discovery.
                // Pre-compute security metadata from the just-inserted handle.
                let security_map: std::collections::HashMap<
                    String,
                    super::mcp_mgp::ToolSecurityMetadata,
                > = if let Some(h) = state.servers.get(&id) {
                    tools
                        .iter()
                        .filter_map(|t| {
                            Self::compute_tool_security(h, &t.name).map(|s| (t.name.clone(), s))
                        })
                        .collect()
                } else {
                    std::collections::HashMap::new()
                };
                self.rich_tool_index
                    .add_server_tools(&id, &tools, |tool_name| {
                        security_map.get(tool_name).cloned()
                    });
            }
        }

        // Build capability mappings for dynamic dispatch (P1 Core Minimalism)
        self.dispatcher.build_from_tools(&id, &tools).await;

        info!(
            "MCP server '{}' connected with {} tools",
            id,
            tool_names.len()
        );

        // Audit: SERVER_CONNECTED
        self.broadcast_audit_event(&crate::db::AuditLogEntry {
            timestamp: chrono::Utc::now(),
            event_type: "SERVER_CONNECTED".to_string(),
            actor_id: Some("kernel".to_string()),
            target_id: Some(id.clone()),
            permission: None,
            result: "success".to_string(),
            reason: format!("Connected with {} tool(s)", tool_names.len()),
            metadata: None,
            trace_id: None,
        })
        .await;

        super::mcp_lifecycle::emit_lifecycle_notification(
            self, &id, "Registered", "Connected", "Server connected"
        ).await;

        super::mcp_events::deliver_event(self, "lifecycle", &serde_json::json!({
            "server_id": id,
            "previous_state": "Registered",
            "new_state": "Connected",
            "timestamp": chrono::Utc::now().to_rfc3339(),
        })).await;

        Ok(tool_names)
    }

    /// Disconnect and permanently remove an MCP server (also clears stopped_configs).
    pub async fn disconnect_server(&self, id: &str) -> Result<()> {
        let mut state = self.state.write().await;
        let handle = state.servers
            .remove(id)
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not found", id))?;
        for tool in &handle.tools {
            state.tool_index.remove(&tool.name);
        }
        // Clean up rich tool index (§16)
        self.rich_tool_index.remove_server_tools(id);
        // Clean up stopped_configs too (permanent removal)
        state.stopped_configs.remove(id);
        info!("MCP server '{}' disconnected", id);
        Ok(())
    }

    /// List all registered MCP servers with status.
    pub async fn list_servers(&self) -> Vec<McpServerInfo> {
        let state = self.state.read().await;

        let mut result: Vec<McpServerInfo> = state.servers
            .values()
            .map(|h| McpServerInfo {
                id: h.id.clone(),
                command: h.config.command.clone(),
                args: h.config.args.clone(),
                status_message: match &h.status {
                    ServerStatus::Error(msg) => Some(msg.clone()),
                    _ => None,
                },
                status: h.status.clone(),
                tools: h.tools.iter().map(|t| t.name.clone()).collect(),
                is_cloto_sdk: h.handshake.is_some(),
                source: h.source,
                display_name: h.config.display_name.clone(),
                mgp_supported: h.mgp_negotiated.is_some(),
                trust_level: h
                    .mgp_negotiated
                    .as_ref()
                    .map(|m| format!("{:?}", m.trust_level).to_lowercase()),
            })
            .collect();

        // Include stopped servers as Disconnected
        for (id, (config, source)) in state.stopped_configs.iter() {
            if !state.servers.contains_key(id) {
                result.push(McpServerInfo {
                    id: id.clone(),
                    command: config.command.clone(),
                    args: config.args.clone(),
                    status_message: Some("Stopped".to_string()),
                    status: ServerStatus::Disconnected,
                    tools: Vec::new(),
                    is_cloto_sdk: false,
                    source: *source,
                    display_name: config.display_name.clone(),
                    mgp_supported: false,
                    trust_level: None,
                });
            }
        }

        result
    }

    /// Return IDs of connected mind.* servers (reasoning engines).
    pub async fn list_connected_mind_servers(&self) -> Vec<String> {
        let state = self.state.read().await;
        state.servers
            .iter()
            .filter(|(id, h)| id.starts_with("mind.") && h.status == ServerStatus::Connected)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Check if a server with the given ID is registered.
    pub async fn has_server(&self, id: &str) -> bool {
        let state = self.state.read().await;
        state.servers.contains_key(id)
    }

    /// Check if a specific server has a tool with the given name.
    pub async fn has_server_tool(&self, server_id: &str, tool_name: &str) -> bool {
        let state = self.state.read().await;
        state.servers
            .get(server_id)
            .is_some_and(|h| h.tools.iter().any(|t| t.name == tool_name))
    }

    // ============================================================
    // Tool Routing (used by PluginRegistry in Phase 1+)
    // ============================================================

    /// Compute tool security metadata for a tool on a given server handle.
    fn compute_tool_security(
        handle: &McpServerHandle,
        tool_name: &str,
    ) -> Option<ToolSecurityMetadata> {
        let mgp = handle.mgp_negotiated.as_ref()?;
        if !mgp.active_extensions.iter().any(|e| e == "tool_security") {
            return None;
        }
        let validator = handle
            .config
            .tool_validators
            .get(tool_name)
            .map(String::as_str);
        let perm_class =
            mcp_mgp::PermissionRiskClass::from_permissions(&handle.config.required_permissions);
        let effective =
            mcp_mgp::derive_effective_risk_level(mgp.trust_level, validator, perm_class);
        Some(ToolSecurityMetadata {
            effective_risk_level: effective,
            trust_level: mgp.trust_level,
            validator: validator.map(str::to_string),
            code_safety: None,
        })
    }

    /// Collect tool schemas from all MCP servers in OpenAI function format.
    /// Includes kernel-native tools when YOLO mode is enabled.
    /// Always includes §16 meta-tools (mgp.tools.discover, mgp.tools.request) for LLM context.
    pub async fn collect_tool_schemas(&self) -> Vec<Value> {
        let state = self.state.read().await;
        let mut schemas = if self.yolo_mode.load(Ordering::Relaxed) {
            super::mcp_kernel_tool::kernel_tool_schemas()
        } else {
            vec![]
        };
        // §16: Always include LLM meta-tools for dynamic discovery
        schemas.extend(super::mcp_kernel_tool::llm_meta_tool_schemas());
        for handle in state.servers.values() {
            if handle.status != ServerStatus::Connected {
                continue;
            }
            // Skip mind.* — engine-internal tools, not agent-facing
            if handle.id.starts_with("mind.") {
                continue;
            }
            for tool in &handle.tools {
                let security = Self::compute_tool_security(handle, &tool.name);
                schemas.push(mcp_tool_schema(tool, security.as_ref()));
            }
        }
        schemas
    }

    /// Collect tool schemas filtered by server IDs.
    /// Includes kernel-native tools (create_mcp_server) only when YOLO mode is enabled.
    pub async fn collect_tool_schemas_for(&self, server_ids: &[String]) -> Vec<Value> {
        let state = self.state.read().await;
        let mut schemas = if self.yolo_mode.load(Ordering::Relaxed) {
            super::mcp_kernel_tool::kernel_tool_schemas()
        } else {
            vec![]
        };
        for id in server_ids {
            if let Some(handle) = state.servers.get(id) {
                if handle.status != ServerStatus::Connected {
                    continue;
                }
                for tool in &handle.tools {
                    let security = Self::compute_tool_security(handle, &tool.name);
                    schemas.push(mcp_tool_schema(tool, security.as_ref()));
                }
            }
        }
        schemas
    }

    /// Collect tool schemas for a specific agent using `resolve_tool_access()`.
    /// Iterates all connected servers and includes only tools the agent is allowed to use.
    pub async fn collect_tool_schemas_for_agent(&self, agent_id: &str) -> Vec<Value> {
        let state = self.state.read().await;
        let mut schemas = if self.yolo_mode.load(Ordering::Relaxed) {
            super::mcp_kernel_tool::kernel_tool_schemas()
        } else {
            vec![]
        };
        // §16: Always include LLM meta-tools for dynamic discovery
        schemas.extend(super::mcp_kernel_tool::llm_meta_tool_schemas());
        for (server_id, handle) in state.servers.iter() {
            if handle.status != ServerStatus::Connected {
                continue;
            }
            // Skip mind.* — engine-internal tools, not agent-facing
            if server_id.starts_with("mind.") {
                continue;
            }
            for tool in &handle.tools {
                // speak is auto-invoked by the kernel after the agentic loop;
                // excluding it prevents the LLM from speaking different text
                // than the final displayed response.
                if server_id == "output.avatar" && tool.name == "speak" {
                    continue;
                }
                match crate::db::resolve_tool_access(&self.pool, agent_id, server_id, &tool.name)
                    .await
                {
                    Ok(ref perm) if perm == "allow" => {
                        let security = Self::compute_tool_security(handle, &tool.name);
                        schemas.push(mcp_tool_schema(tool, security.as_ref()));
                    }
                    _ => {} // deny or error → skip
                }
            }
        }
        schemas
    }

    /// Look up which server provides a given tool.
    pub async fn get_tool_server_id(&self, tool_name: &str) -> Option<String> {
        let state = self.state.read().await;
        state.tool_index.get(tool_name).cloned()
    }

    /// Check tool access for a specific agent via `resolve_tool_access()`.
    pub async fn check_tool_access(
        &self,
        agent_id: &str,
        tool_name: &str,
    ) -> anyhow::Result<String> {
        let server_id = {
            let state = self.state.read().await;
            state.tool_index
                .get(tool_name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("MCP tool '{}' not found", tool_name))?
        };
        crate::db::resolve_tool_access(&self.pool, agent_id, &server_id, tool_name).await
    }

    /// Execute a tool by name with optional caller context for audit/permission logging.
    /// Delegates to `execute_tool_internal` after recording the caller.
    pub async fn execute_tool(
        &self,
        tool_name: &str,
        args: Value,
        caller: Option<&str>,
    ) -> Result<Value> {
        if let Some(agent_id) = caller {
            debug!(tool = %tool_name, caller = %agent_id, "Tool execution requested");
        }
        self.execute_tool_internal(tool_name, args).await
    }

    /// Execute a tool by name, routing to the correct MCP server (kernel-internal).
    /// Handles kernel-native tools (create_mcp_server) internally.
    /// Applies kernel-side validation (A) before forwarding to the MCP server.
    pub(crate) async fn execute_tool_internal(&self, tool_name: &str, args: Value) -> Result<Value> {
        // Kernel-native tools
        match tool_name {
            "create_mcp_server" => {
                return super::mcp_kernel_tool::execute_create_mcp_server(self, args).await;
            }
            "mgp.access.query" => {
                return super::mcp_kernel_tool::execute_access_query(self, args).await;
            }
            "mgp.access.grant" => {
                return super::mcp_kernel_tool::execute_access_grant(self, args).await;
            }
            "mgp.access.revoke" => {
                return super::mcp_kernel_tool::execute_access_revoke(self, args).await;
            }
            "mgp.audit.replay" => {
                return super::mcp_kernel_tool::execute_audit_replay(self, args).await;
            }
            // Tier 3: Lifecycle
            "mgp.health.ping" => {
                return super::mcp_kernel_tool::execute_health_ping(self, args).await;
            }
            "mgp.health.status" => {
                return super::mcp_kernel_tool::execute_health_status(self, args).await;
            }
            "mgp.lifecycle.shutdown" => {
                return super::mcp_kernel_tool::execute_lifecycle_shutdown(self, args).await;
            }
            // Tier 3: Streaming
            "mgp.stream.cancel" => {
                return super::mcp_kernel_tool::execute_stream_cancel(self, args).await;
            }
            "mgp.stream.pace" => {
                return super::mcp_kernel_tool::execute_stream_pace(self, args).await;
            }
            // Tier 3: Events
            "mgp.events.subscribe" => {
                return super::mcp_kernel_tool::execute_events_subscribe(self, args).await;
            }
            "mgp.events.unsubscribe" => {
                return super::mcp_kernel_tool::execute_events_unsubscribe(self, args).await;
            }
            "mgp.events.replay" => {
                return super::mcp_kernel_tool::execute_events_replay(self, args).await;
            }
            "mgp.events.pending_callbacks" => {
                return super::mcp_kernel_tool::execute_events_pending_callbacks(self, args)
                    .await;
            }
            // Tier 3: Callbacks
            "mgp.callback.respond" => {
                return super::mcp_kernel_tool::execute_callback_respond(self, args).await;
            }
            // Tier 4: Discovery (§15)
            "mgp.discovery.list" => {
                return super::mcp_discovery::execute_discovery_list(self, args).await;
            }
            "mgp.discovery.register" => {
                return super::mcp_discovery::execute_discovery_register(self, args).await;
            }
            "mgp.discovery.deregister" => {
                return super::mcp_discovery::execute_discovery_deregister(self, args).await;
            }
            // Tier 4: Tool Discovery (§16)
            "mgp.tools.discover" => {
                return super::mcp_tool_discovery::execute_tools_discover(self, args).await;
            }
            "mgp.tools.request" => {
                return super::mcp_tool_discovery::execute_tools_request(self, args).await;
            }
            "mgp.tools.session" => {
                return super::mcp_tool_discovery::execute_tools_session(self, args).await;
            }
            "mgp.tools.session.evict" => {
                return super::mcp_tool_discovery::execute_tools_session_evict(self, args).await;
            }
            // Inter-agent delegation
            "ask_agent" => {
                return super::mcp_kernel_tool::execute_ask_agent(self, args).await;
            }
            // GUI documentation
            "gui.map" => {
                return super::mcp_kernel_tool::execute_gui_map(self, args).await;
            }
            "gui.read" => {
                return super::mcp_kernel_tool::execute_gui_read(self, args).await;
            }
            _ => {}
        }

        let (server_id, client, tool_validators) = {
            let state = self.state.read().await;
            let server_id = state.tool_index.get(tool_name).cloned().ok_or_else(|| {
                anyhow::Error::new(mcp_mgp::MgpError::tool_not_found(format!(
                    "MCP tool '{}' not found",
                    tool_name
                )))
            })?;
            let handle = state.servers.get(&server_id).ok_or_else(|| {
                anyhow::Error::new(mcp_mgp::MgpError::tool_not_found(format!(
                    "MCP server '{}' not found",
                    server_id
                )))
            })?;
            let client = handle.client.clone().ok_or_else(|| {
                anyhow::Error::new(mcp_mgp::MgpError::server_not_ready(format!(
                    "MCP server '{}' not connected",
                    server_id
                )))
            })?;
            (server_id, client, handle.config.tool_validators.clone())
        };

        // ──── Kernel-side Validation (A): Validate tool arguments before forwarding ────
        if let Some(validator_name) = tool_validators.get(tool_name) {
            if let Err(e) = validate_tool_arguments(validator_name, tool_name, &args) {
                self.broadcast_audit_event(&crate::db::AuditLogEntry {
                    timestamp: chrono::Utc::now(),
                    event_type: "TOOL_BLOCKED".to_string(),
                    actor_id: Some(server_id.clone()),
                    target_id: Some(tool_name.to_string()),
                    permission: None,
                    result: "blocked".to_string(),
                    reason: e.to_string(),
                    metadata: None,
                    trace_id: None,
                })
                .await;
                return Err(e);
            }
        }

        let result = client.call_tool(tool_name, args).await?;

        // Audit: TOOL_EXECUTED
        self.broadcast_audit_event(&crate::db::AuditLogEntry {
            timestamp: chrono::Utc::now(),
            event_type: "TOOL_EXECUTED".to_string(),
            actor_id: Some(server_id.clone()),
            target_id: Some(tool_name.to_string()),
            permission: None,
            result: if result.is_error == Some(true) {
                "error"
            } else {
                "success"
            }
            .to_string(),
            reason: String::new(),
            metadata: None,
            trace_id: None,
        })
        .await;

        // Update LRU timestamp in session cache (MGP §16.7)
        self.session_cache.touch("default", tool_name);

        super::mcp_events::deliver_event(self, "tools", &serde_json::json!({
            "server_id": server_id,
            "tool_name": tool_name,
            "result": if result.is_error == Some(true) { "error" } else { "success" },
            "timestamp": chrono::Utc::now().to_rfc3339(),
        })).await;

        // Convert CallToolResult to a simple JSON value
        if result.is_error == Some(true) {
            let error_text = result
                .content
                .iter()
                .filter_map(|c| match c {
                    ToolContent::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            return Err(anyhow::anyhow!("MCP tool error: {}", error_text));
        }

        // Return text content as JSON
        let text_parts: Vec<String> = result
            .content
            .iter()
            .filter_map(|c| match c {
                ToolContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect();

        if text_parts.len() == 1 {
            // Try to parse as JSON, fall back to string
            match serde_json::from_str::<Value>(&text_parts[0]) {
                Ok(val) => Ok(val),
                Err(_) => Ok(Value::String(text_parts[0].clone())),
            }
        } else {
            Ok(Value::String(text_parts.join("\n")))
        }
    }

    /// Execute a tool on a specific server by server ID and tool name.
    /// Applies kernel-side validation (A) and delegation checks (§5.6)
    /// before forwarding to the MCP server.
    pub async fn call_server_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        args: Value,
    ) -> Result<super::mcp_protocol::CallToolResult> {
        // ──── §5.6 Delegation Check ────
        // If _mgp.delegation is present, verify chain depth ≤ 3, actor validity,
        // anti-spoofing (§5.6.3), and permission intersection (§5.6.1).
        if let Some(mgp) = args.get("_mgp") {
            if let Some(delegation) = mgp.get("delegation") {
                if let Some(chain) = delegation.get("chain").and_then(|c| c.as_array()) {
                    if chain.len() > 3 {
                        return Err(mcp_mgp::MgpError::permission_denied(format!(
                            "Delegation chain depth {} exceeds maximum of 3",
                            chain.len()
                        )).into());
                    }
                }

                // §5.6.3: Verify original_actor is a known, active agent
                if let Some(original_actor) =
                    delegation.get("original_actor").and_then(|v| v.as_str())
                {
                    let agent_exists: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM agents WHERE id = ? AND enabled = 1)",
                    )
                    .bind(original_actor)
                    .fetch_one(&self.pool)
                    .await
                    .unwrap_or(false);

                    if !agent_exists {
                        warn!(
                            original_actor = %original_actor,
                            server = %server_id,
                            tool = %tool_name,
                            "Delegation rejected: unknown or inactive original_actor (§5.6.3)"
                        );
                        return Err(mcp_mgp::MgpError::permission_denied(format!(
                            "Delegation rejected: original_actor '{}' is not a known active agent (MGP §5.6.3)",
                            original_actor
                        )).into());
                    }

                    // §5.6.1: Permission intersection — verify original_actor has access to target tool
                    let permission = crate::db::resolve_tool_access(
                        &self.pool,
                        original_actor,
                        server_id,
                        tool_name,
                    )
                    .await
                    .unwrap_or_else(|_| "deny".to_string());

                    if permission == "deny" {
                        warn!(
                            original_actor = %original_actor,
                            server = %server_id,
                            tool = %tool_name,
                            "Delegation rejected: original_actor lacks access (§5.6.1)"
                        );
                        return Err(mcp_mgp::MgpError::access_denied(format!(
                            "Delegation rejected: original_actor '{}' does not have access to {}.{} (MGP §5.6.1)",
                            original_actor, server_id, tool_name
                        )).into());
                    }
                }

                // §5.6.3: Verify delegated_via is a known, operational server
                if let Some(delegated_via) =
                    delegation.get("delegated_via").and_then(|v| v.as_str())
                {
                    let valid = {
                        let state = self.state.read().await;
                        state.servers
                            .get(delegated_via)
                            .is_some_and(|h| h.status.is_operational())
                    };

                    if !valid {
                        warn!(
                            delegated_via = %delegated_via,
                            server = %server_id,
                            tool = %tool_name,
                            "Delegation rejected: unknown or non-operational delegated_via (§5.6.3)"
                        );
                        return Err(mcp_mgp::MgpError::permission_denied(format!(
                            "Delegation rejected: delegated_via '{}' is not a known operational server (MGP §5.6.3)",
                            delegated_via
                        )).into());
                    }
                }

                debug!(
                    server = %server_id,
                    tool = %tool_name,
                    delegation = %delegation,
                    "Delegated tool call"
                );
            }
        }

        let (client, tool_validators) = {
            let state = self.state.read().await;
            let handle = state.servers.get(server_id).ok_or_else(|| {
                anyhow::Error::new(mcp_mgp::MgpError::tool_not_found(format!(
                    "MCP server '{}' not found",
                    server_id
                )))
            })?;
            let client = handle.client.clone().ok_or_else(|| {
                anyhow::Error::new(mcp_mgp::MgpError::server_not_ready(format!(
                    "MCP server '{}' not connected",
                    server_id
                )))
            })?;
            (client, handle.config.tool_validators.clone())
        };

        // ──── Kernel-side Validation (A): Validate tool arguments before forwarding ────
        if let Some(validator_name) = tool_validators.get(tool_name) {
            validate_tool_arguments(validator_name, tool_name, &args)?;
        }

        client.call_tool(tool_name, args).await
    }

    /// Handle an incoming stream chunk notification: track sequence, detect gaps,
    /// request retransmission if needed, and clean up on stream completion (MGP §12).
    pub async fn handle_stream_chunk(
        &self,
        server_id: &str,
        request_id: i64,
        index: u32,
        done: bool,
    ) -> Option<Vec<u32>> {
        // Skip duplicate chunks (§12.5 retransmission dedup)
        if self.stream_assembler.is_duplicate(server_id, request_id, index) {
            tracing::debug!(
                server_id, request_id, index,
                "Skipping duplicate stream chunk"
            );
            return None;
        }
        let gaps = self.stream_assembler.record_chunk(server_id, request_id, index);
        if done {
            self.stream_assembler.remove(server_id, request_id);
        }
        if let Some(ref gap_indices) = gaps {
            if let Err(e) = super::mcp_streaming::send_gap_notification(
                self,
                server_id,
                request_id,
                gap_indices.clone(),
            )
            .await
            {
                tracing::debug!(error = %e, "Failed to send stream gap notification");
            }
        }
        gaps
    }

    // ============================================================
    // Event Forwarding (Kernel → MCP Servers)
    // ============================================================

    /// Broadcast a kernel event to all connected MCP servers as a notification.
    pub async fn broadcast_event(&self, event: &cloto_shared::ClotoEvent) {
        let state = self.state.read().await;
        for handle in state.servers.values() {
            if handle.status != ServerStatus::Connected {
                continue;
            }
            let Some(client) = &handle.client else {
                continue;
            };
            let Ok(event_json) = serde_json::to_value(event) else {
                continue;
            };
            if let Err(e) = client
                .send_notification("notifications/cloto.event", Some(event_json))
                .await
            {
                debug!(
                    server = %handle.id,
                    error = %e,
                    "Failed to forward event to MCP server"
                );
            }
        }
    }

    /// Broadcast an audit event: persist to DB and send `notifications/mgp.audit`
    /// to all connected servers that negotiated the "audit" extension.
    pub async fn broadcast_audit_event(&self, entry: &crate::db::AuditLogEntry) {
        // 1. Persist locally
        crate::db::spawn_audit_log(self.pool.clone(), entry.clone());

        // 2. Send to audit-capable servers
        let state = self.state.read().await;
        for handle in state.servers.values() {
            if handle.status != ServerStatus::Connected {
                continue;
            }
            // Only send to servers that negotiated the "audit" extension
            let has_audit = handle
                .mgp_negotiated
                .as_ref()
                .is_some_and(|m| m.active_extensions.iter().any(|e| e == "audit"));
            if !has_audit {
                continue;
            }
            let Some(client) = &handle.client else {
                continue;
            };
            let seq = handle.audit_seq.fetch_add(1, Ordering::Relaxed) + 1;
            let audit_params = serde_json::json!({
                "_mgp": { "seq": seq },
                "event_type": entry.event_type,
                "timestamp": entry.timestamp.to_rfc3339(),
                "actor": {
                    "type": if entry.actor_id.as_deref() == Some("kernel") { "kernel" } else { "agent" },
                    "id": entry.actor_id,
                },
                "target": {
                    "server_id": entry.target_id.as_ref().and_then(|t| t.split(':').next()),
                    "tool_name": entry.target_id.as_ref().and_then(|t| t.split(':').nth(1)),
                },
                "result": entry.result,
                "details": {
                    "permission": entry.permission,
                    "reason": entry.reason,
                },
            });
            if let Err(e) = client
                .send_notification("notifications/mgp.audit", Some(audit_params))
                .await
            {
                debug!(
                    server = %handle.id,
                    error = %e,
                    "Failed to send audit notification"
                );
            }
        }
    }

    /// Send a config update notification to a specific MCP server.
    pub async fn notify_config_updated(&self, server_id: &str, config: Value) {
        let state = self.state.read().await;
        if let Some(handle) = state.servers.get(server_id) {
            let Some(client) = &handle.client else {
                return;
            };
            let params = serde_json::json!({
                "server_id": server_id,
                "config": config,
            });
            if let Err(e) = client
                .send_notification("notifications/cloto.config_updated", Some(params))
                .await
            {
                debug!(
                    server = %server_id,
                    error = %e,
                    "Failed to send config update to MCP server"
                );
            }
        }
    }

    // ============================================================
    // DB persistence for dynamic servers
    // ============================================================

    /// Add a new dynamic MCP server, connect, and persist to DB.
    pub async fn add_dynamic_server(
        &self,
        id: String,
        command: String,
        args: Vec<String>,
        script_content: Option<String>,
        description: Option<String>,
        mgp: Option<super::mcp_mgp::MgpServerConfig>,
        env: HashMap<String, String>,
    ) -> Result<Vec<String>> {
        let config = McpServerConfig {
            id: id.clone(),
            command: command.clone(),
            args: args.clone(),
            env: env.clone(),
            transport: "stdio".to_string(),
            auto_restart: Some(true),
            required_permissions: Vec::new(),
            tool_validators: HashMap::new(),
            display_name: None,
            mgp,
            restart_policy: None,
            seal: None,
            isolation: None,
        };

        let tool_names = self.connect_server(config, ServerSource::Dynamic).await?;

        // Persist to DB
        let record = crate::db::McpServerRecord {
            name: id,
            command,
            args: serde_json::to_string(&args)?,
            script_content,
            description,
            created_at: chrono::Utc::now().timestamp(),
            is_active: true,
            env: serde_json::to_string(&env)?,
        };
        crate::db::save_mcp_server(&self.pool, &record).await?;

        Ok(tool_names)
    }

    /// Remove a dynamic MCP server and deactivate in DB.
    /// Config-loaded servers cannot be deleted (must be removed from mcp.toml).
    pub async fn remove_dynamic_server(&self, id: &str) -> Result<()> {
        // Reject deletion of config-loaded servers
        {
            let state = self.state.read().await;
            if let Some(handle) = state.servers.get(id) {
                if handle.source == ServerSource::Config {
                    return Err(anyhow::anyhow!(
                        "Cannot delete config-loaded server '{}'. Remove it from mcp.toml instead.",
                        id
                    ));
                }
            }
        }
        self.disconnect_server(id).await?;
        crate::db::deactivate_mcp_server(&self.pool, id).await?;
        Ok(())
    }

    // ============================================================
    // Capability-Based Dispatch (P1 Core Minimalism)
    // ============================================================

    /// Resolve the MCP server providing a given capability.
    pub async fn resolve_capability_server(
        &self,
        capability: super::capability_dispatcher::CapabilityType,
    ) -> Option<String> {
        self.dispatcher.resolve_server(capability).await
    }

    /// Call a tool by capability type, resolving the server dynamically.
    ///
    /// If `preferred_server` is provided (e.g., for engine-specific routing),
    /// it takes precedence over automatic resolution.
    pub async fn call_capability_tool(
        &self,
        capability: super::capability_dispatcher::CapabilityType,
        tool_name: &str,
        args: Value,
        preferred_server: Option<&str>,
    ) -> Result<super::mcp_protocol::CallToolResult> {
        let server_id = if let Some(pref) = preferred_server {
            pref.to_string()
        } else {
            self.dispatcher
                .resolve(capability, tool_name)
                .await
                .map(|(sid, _)| sid)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No server provides {:?} capability for tool '{}'",
                        capability,
                        tool_name
                    )
                })?
        };
        self.call_server_tool(&server_id, tool_name, args).await
    }

    // ============================================================
    // Server Lifecycle (MCP_SERVER_UI_DESIGN.md §4.3)
    // ============================================================

    /// Stop a server (disconnect but preserve config for restart).
    pub async fn stop_server(&self, id: &str) -> Result<()> {
        {
            let mut state = self.state.write().await;
            let handle = state.servers
                .remove(id)
                .ok_or_else(|| anyhow::anyhow!("Server '{}' not found or already stopped", id))?;
            state.tool_index.retain(|_, server_id| server_id != id);

            // Preserve config for restart capability (works for both config and dynamic)
            state.stopped_configs.insert(id.to_string(), (handle.config.clone(), handle.source));

            info!(server = %id, source = ?handle.source, "MCP server stopped (config preserved for restart)");
        }

        // Clean up rich tool index (§16)
        self.rich_tool_index.remove_server_tools(id);

        // Clean up capability mappings (P1 Core Minimalism)
        self.dispatcher.remove_server(id).await;

        Ok(())
    }

    /// Graceful shutdown: set Draining, notify the server, wait timeout, then stop.
    pub async fn drain_server(&self, id: &str, reason: &str, timeout_ms: u64) -> Result<()> {
        // Set status to Draining
        {
            let mut state = self.state.write().await;
            let handle = state.servers
                .get_mut(id)
                .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", id))?;
            handle.status = ServerStatus::Draining;
        }

        // Emit lifecycle notification
        super::mcp_lifecycle::emit_lifecycle_notification(
            self,
            id,
            "Connected",
            "Draining",
            reason,
        )
        .await;

        super::mcp_events::deliver_event(self, "lifecycle", &serde_json::json!({
            "server_id": id,
            "previous_state": "Connected",
            "new_state": "Draining",
            "timestamp": chrono::Utc::now().to_rfc3339(),
        })).await;

        // Notify the target server via standard lifecycle notification (§11.5)
        {
            let state = self.state.read().await;
            if let Some(handle) = state.servers.get(id) {
                if let Some(client) = &handle.client {
                    let params = serde_json::json!({
                        "server_id": id,
                        "previous_state": "connected",
                        "new_state": "draining",
                        "reason": reason,
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                    });
                    let _ = client
                        .send_notification("notifications/mgp.lifecycle", Some(params))
                        .await;
                }
            }
        }

        // Wait for drain period then stop
        tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)).await;
        let _ = self.stop_server(id).await;

        Ok(())
    }

    /// Start a server from stopped_configs or DB.
    pub async fn start_server(&self, id: &str) -> Result<Vec<String>> {
        // Check if already running + check stopped_configs under single lock
        {
            let mut state = self.state.write().await;
            if state.servers.contains_key(id) {
                return Err(anyhow::anyhow!("Server '{}' is already running", id));
            }

            // 1. Check stopped_configs first (works for both config-loaded and dynamic)
            if let Some((config, source)) = state.stopped_configs.remove(id) {
                drop(state);
                return self.connect_server(config, source).await;
            }
        }

        // 2. Fall back to DB (dynamic servers that were never stopped in this session)
        let records = crate::db::load_active_mcp_servers(&self.pool).await?;
        let record = records.into_iter().find(|r| r.name == id).ok_or_else(|| {
            anyhow::anyhow!("Server '{}' not found in stopped configs or database", id)
        })?;

        let args: Vec<String> = serde_json::from_str(&record.args).unwrap_or_default();

        let config = McpServerConfig {
            id: id.to_string(),
            command: record.command,
            args,
            env: HashMap::new(),
            transport: "stdio".to_string(),
            auto_restart: Some(true),
            required_permissions: Vec::new(),
            tool_validators: HashMap::new(),
            display_name: None,
            mgp: None,
            restart_policy: None,
            seal: None,
            isolation: None,
        };

        self.connect_server(config, ServerSource::Dynamic).await
    }

    /// Restart a server (stop + start).
    pub async fn restart_server(&self, id: &str) -> Result<Vec<String>> {
        // Set Restarting status before stop
        {
            let mut state = self.state.write().await;
            if let Some(handle) = state.servers.get_mut(id) {
                handle.status = ServerStatus::Restarting;
            }
        }

        super::mcp_lifecycle::emit_lifecycle_notification(
            self, id, "Connected", "Restarting", "Server restart initiated"
        ).await;

        super::mcp_events::deliver_event(self, "lifecycle", &serde_json::json!({
            "server_id": id,
            "previous_state": "Connected",
            "new_state": "Restarting",
            "timestamp": chrono::Utc::now().to_rfc3339(),
        })).await;

        // Stop if running (ignore error if already stopped)
        let _ = self.stop_server(id).await;
        self.start_server(id).await
    }

    /// Get a server's in-memory environment variables (from config or runtime).
    pub async fn get_server_env(&self, id: &str) -> HashMap<String, String> {
        let state = self.state.read().await;
        state.servers
            .get(id)
            .map(|h| h.config.env.clone())
            .unwrap_or_default()
    }

    /// Update a server's environment variables, persist to DB, and restart.
    pub async fn update_server_env(&self, id: &str, env: HashMap<String, String>) -> Result<()> {
        let env_json = serde_json::to_string(&env)?;
        crate::db::update_mcp_server_env(&self.pool, id, &env_json).await?;

        // Update in-memory config
        {
            let mut state = self.state.write().await;
            if let Some(handle) = state.servers.get_mut(id) {
                handle.config.env = env;
            }
        }

        // Restart to apply new env
        let _ = self.restart_server(id).await;
        Ok(())
    }

    /// Look up the kernel-side validator name for a given tool.
    /// Returns `Some("sandbox")` if the tool has a sandbox validator configured, etc.
    pub async fn get_tool_validator(&self, tool_name: &str) -> Option<String> {
        let state = self.state.read().await;
        let server_id = state.tool_index.get(tool_name)?;
        let handle = state.servers.get(server_id)?;
        handle.config.tool_validators.get(tool_name).cloned()
    }

    /// Get a reference to the database pool (for access control queries).
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Resolve a relative path against the project root.
    /// Used by `lib.rs` to find `mcp.toml` when CWD differs from the project root
    /// (e.g. `cargo tauri dev`).
    #[must_use]
    pub fn resolve_project_path(relative: &std::path::Path) -> Option<String> {
        let exe = std::env::current_exe().ok()?;
        let root = Self::detect_project_root(exe.as_path())?;
        let candidate = root.join(relative);
        if candidate.exists() {
            Some(candidate.to_string_lossy().to_string())
        } else {
            None
        }
    }

    /// Walk up from the given path to find the project root (directory
    /// containing `Cargo.toml`).  Returns `None` in production deployments
    /// where no workspace marker exists.
    pub(crate) fn detect_project_root(from: &std::path::Path) -> Option<std::path::PathBuf> {
        let start = if from.is_file() { from.parent()? } else { from };
        let canonical = std::fs::canonicalize(start).ok()?;
        // Strip Windows UNC prefix (\\?\) that canonicalize() adds — Python cannot handle it
        let mut dir = {
            let s = canonical.to_string_lossy();
            if let Some(stripped) = s.strip_prefix(r"\\?\") {
                std::path::PathBuf::from(stripped)
            } else {
                canonical
            }
        };
        for _ in 0..10 {
            if dir.join("Cargo.toml").exists() {
                return Some(dir);
            }
            if !dir.pop() {
                break;
            }
        }
        None
    }

    // ============================================================
    // Health Monitor — auto-restart dead MCP servers (bug-142)
    // ============================================================

    /// Spawn a background task that periodically checks for dead MCP servers
    /// and auto-restarts them based on their restart policy.
    pub fn spawn_health_monitor(self: Arc<Self>, shutdown: Arc<tokio::sync::Notify>) {
        super::mcp_health::spawn_health_monitor(self, shutdown);
    }
}

/// Public bridge for callback handling from lib.rs notification listener.
pub fn mcp_events_handle_callback(
    manager: &McpClientManager,
    server_id: &str,
    params: &Value,
) -> super::mcp_events::CallbackHandleResult {
    super::mcp_events::handle_callback_request(manager, server_id, params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn yolo_mode_initializes_correctly() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", "memory.cpersona").await.unwrap();

        let manager_off = McpClientManager::new(pool.clone(), false, 120);
        assert!(!manager_off.yolo_mode.load(Ordering::Relaxed));

        let manager_on = McpClientManager::new(pool, true, 120);
        assert!(manager_on.yolo_mode.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn yolo_mode_toggle_at_runtime() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", "memory.cpersona").await.unwrap();

        let manager = McpClientManager::new(pool, false, 120);
        assert!(!manager.yolo_mode.load(Ordering::Relaxed));

        manager.yolo_mode.store(true, Ordering::Relaxed);
        assert!(manager.yolo_mode.load(Ordering::Relaxed));

        manager.yolo_mode.store(false, Ordering::Relaxed);
        assert!(!manager.yolo_mode.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn yolo_mode_affects_kernel_tool_schemas() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", "memory.cpersona").await.unwrap();

        // YOLO off: no kernel tools
        let manager = McpClientManager::new(pool.clone(), false, 120);
        let schemas = manager.collect_tool_schemas().await;
        // §16: meta-tools (discover + request) are always included even with YOLO off
        assert_eq!(schemas.len(), 2, "YOLO off should only include meta-tools");

        // YOLO on: kernel tools included (create_mcp_server + access control + audit)
        let manager_on = McpClientManager::new(pool, true, 120);
        let schemas_on = manager_on.collect_tool_schemas().await;
        assert!(
            !schemas_on.is_empty(),
            "YOLO on should include kernel tools"
        );
        let name = schemas_on[0]["function"]["name"].as_str().unwrap();
        assert_eq!(name, "create_mcp_server");
    }

    #[tokio::test]
    async fn kernel_tools_include_access_control() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", "memory.cpersona").await.unwrap();

        let manager = McpClientManager::new(pool, true, 120);
        let schemas = manager.collect_tool_schemas().await;
        let names: Vec<&str> = schemas
            .iter()
            .filter_map(|s| s["function"]["name"].as_str())
            .collect();
        assert!(
            names.contains(&"mgp.access.query"),
            "Should include mgp.access.query"
        );
        assert!(
            names.contains(&"mgp.access.grant"),
            "Should include mgp.access.grant"
        );
        assert!(
            names.contains(&"mgp.access.revoke"),
            "Should include mgp.access.revoke"
        );
    }

    #[tokio::test]
    async fn kernel_tools_include_audit_replay() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", "memory.cpersona").await.unwrap();

        let manager = McpClientManager::new(pool, true, 120);
        let schemas = manager.collect_tool_schemas().await;
        let names: Vec<&str> = schemas
            .iter()
            .filter_map(|s| s["function"]["name"].as_str())
            .collect();
        assert!(
            names.contains(&"mgp.audit.replay"),
            "Should include mgp.audit.replay"
        );
    }

    #[tokio::test]
    async fn kernel_tools_include_tier3_lifecycle() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", "memory.cpersona").await.unwrap();

        let manager = McpClientManager::new(pool, true, 120);
        let schemas = manager.collect_tool_schemas().await;
        let names: Vec<&str> = schemas
            .iter()
            .filter_map(|s| s["function"]["name"].as_str())
            .collect();
        assert!(
            names.contains(&"mgp.health.ping"),
            "Should include mgp.health.ping"
        );
        assert!(
            names.contains(&"mgp.health.status"),
            "Should include mgp.health.status"
        );
        assert!(
            names.contains(&"mgp.lifecycle.shutdown"),
            "Should include mgp.lifecycle.shutdown"
        );
    }

    #[tokio::test]
    async fn kernel_tools_include_tier3_streaming() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", "memory.cpersona").await.unwrap();

        let manager = McpClientManager::new(pool, true, 120);
        let schemas = manager.collect_tool_schemas().await;
        let names: Vec<&str> = schemas
            .iter()
            .filter_map(|s| s["function"]["name"].as_str())
            .collect();
        assert!(
            names.contains(&"mgp.stream.cancel"),
            "Should include mgp.stream.cancel"
        );
    }

    #[tokio::test]
    async fn kernel_tools_include_tier3_events() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", "memory.cpersona").await.unwrap();

        let manager = McpClientManager::new(pool, true, 120);
        let schemas = manager.collect_tool_schemas().await;
        let names: Vec<&str> = schemas
            .iter()
            .filter_map(|s| s["function"]["name"].as_str())
            .collect();
        assert!(
            names.contains(&"mgp.events.subscribe"),
            "Should include mgp.events.subscribe"
        );
        assert!(
            names.contains(&"mgp.events.unsubscribe"),
            "Should include mgp.events.unsubscribe"
        );
        assert!(
            names.contains(&"mgp.events.replay"),
            "Should include mgp.events.replay"
        );
        assert!(
            names.contains(&"mgp.callback.respond"),
            "Should include mgp.callback.respond"
        );
    }

    #[tokio::test]
    async fn kernel_tools_total_count() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", "memory.cpersona").await.unwrap();

        let manager = McpClientManager::new(pool, true, 120);
        let schemas = manager.collect_tool_schemas().await;
        // 5 Tier 2 + 9 Tier 3 + 1 ask_agent + 2 gui = 17 Tier 1-3 + 3 discovery + 3 session = 23 YOLO tools + 2 meta-tools = 25
        assert_eq!(
            schemas.len(),
            25,
            "Should have 25 kernel tools total (23 YOLO + 2 meta-tools)"
        );
    }

    #[tokio::test]
    async fn server_status_serialization() {
        let statuses = vec![
            (ServerStatus::Registered, "Registered"),
            (ServerStatus::Connecting, "Connecting"),
            (ServerStatus::Connected, "Connected"),
            (ServerStatus::Draining, "Draining"),
            (ServerStatus::Disconnected, "Disconnected"),
            (ServerStatus::Error("test".into()), "Error"),
            (ServerStatus::Restarting, "Restarting"),
        ];
        for (status, expected) in statuses {
            let json = serde_json::to_value(&status).unwrap();
            assert_eq!(json.as_str().unwrap(), expected);
        }
    }

    #[test]
    fn server_status_is_operational() {
        assert!(ServerStatus::Connected.is_operational());
        assert!(!ServerStatus::Registered.is_operational());
        assert!(!ServerStatus::Connecting.is_operational());
        assert!(!ServerStatus::Draining.is_operational());
        assert!(!ServerStatus::Disconnected.is_operational());
        assert!(!ServerStatus::Error("x".into()).is_operational());
        assert!(!ServerStatus::Restarting.is_operational());
    }

    #[tokio::test]
    async fn yolo_mode_persisted_to_db() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", "memory.cpersona").await.unwrap();

        // Simulate set_yolo_mode handler persisting to DB
        sqlx::query(
            "INSERT OR REPLACE INTO plugin_configs (plugin_id, config_key, config_value) VALUES ('kernel', 'yolo_mode', 'true')"
        )
            .execute(&pool)
            .await
            .unwrap();

        // Read back
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT config_value FROM plugin_configs WHERE plugin_id = 'kernel' AND config_key = 'yolo_mode'"
        )
            .fetch_optional(&pool)
            .await
            .unwrap();
        assert_eq!(row.unwrap().0, "true");
    }
}
