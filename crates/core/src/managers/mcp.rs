//! MCP Client Manager — orchestrates all MCP server lifecycles.
//!
//! Responsible for spawning, stopping, and restarting MCP server processes,
//! routing tool calls to the correct server, managing manifests, and
//! forwarding kernel events as MCP notifications.

pub use super::mcp_client::{McpClient, McpNotification};
pub use super::mcp_types::*;

use super::mcp_protocol::{McpConfigFile, McpServerConfig, ToolContent};
use super::mcp_tool_validator::validate_tool_arguments;
use super::mcp_transport;
use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, error, info, warn};

/// Approver identity used for YOLO-mode auto-approved permissions.
const YOLO_APPROVER_ID: &str = "YOLO";

// ============================================================
// McpClientManager — kernel-level MCP server orchestrator
// ============================================================

pub struct McpClientManager {
    pub(crate) servers: RwLock<HashMap<String, McpServerHandle>>,
    pool: SqlitePool,
    /// Tool name → server ID index for fast routing
    tool_index: RwLock<HashMap<String, String>>,
    /// YOLO mode: auto-approve all MCP server permissions (ARCHITECTURE.md §5.7).
    /// Arc<AtomicBool> allows runtime toggle via API without restart.
    pub yolo_mode: Arc<AtomicBool>,
    /// Preserved configs from stopped servers, enabling restart for config-loaded servers
    stopped_configs: RwLock<HashMap<String, (McpServerConfig, ServerSource)>>,
    /// Shared notification channel — all MCP servers' notifications are collected here
    notification_tx: mpsc::Sender<McpNotification>,
    notification_rx: Mutex<Option<mpsc::Receiver<McpNotification>>>,
    mcp_request_timeout_secs: u64,
}

impl McpClientManager {
    #[must_use]
    pub fn new(pool: SqlitePool, yolo_mode: bool, mcp_request_timeout_secs: u64) -> Self {
        let (notification_tx, notification_rx) = mpsc::channel(256);
        Self {
            servers: RwLock::new(HashMap::new()),
            pool,
            tool_index: RwLock::new(HashMap::new()),
            yolo_mode: Arc::new(AtomicBool::new(yolo_mode)),
            stopped_configs: RwLock::new(HashMap::new()),
            notification_tx,
            notification_rx: Mutex::new(Some(notification_rx)),
            mcp_request_timeout_secs,
        }
    }

    /// Take the notification receiver (can only be called once).
    /// The Kernel event loop uses this to forward MCP notifications to the event bus.
    pub async fn take_notification_receiver(&self) -> Option<mpsc::Receiver<McpNotification>> {
        self.notification_rx.lock().await.take()
    }

    /// Load server configs from mcp.toml file (if exists) and connect.
    ///
    /// Relative paths in `args` are resolved against the project root directory
    /// (detected by walking up from the config file to find `Cargo.toml`) or,
    /// in production, against the config file's parent directory.
    /// This allows `mcp.toml` to use portable paths like
    /// `"mcp-servers/terminal/server.py"` instead of absolute ones.
    pub async fn load_config_file(&self, config_path: &str) -> Result<()> {
        let path = std::path::Path::new(config_path);
        if !path.exists() {
            info!("No MCP config file at {}, skipping", config_path);
            return Ok(());
        }

        let content = std::fs::read_to_string(path).context("Failed to read MCP config file")?;
        let config: McpConfigFile =
            toml::from_str(&content).context("Failed to parse MCP config file")?;

        // Determine the base directory for resolving relative paths.
        // In development: walk up from the config file to find the workspace root
        //   (directory containing `Cargo.toml`).
        // In production: fall back to the config file's parent directory.
        let base_dir = Self::detect_project_root(path).unwrap_or_else(|| {
            path.parent().map_or_else(
                || std::path::PathBuf::from("."),
                std::path::Path::to_path_buf,
            )
        });

        let total = config.servers.len();
        info!(
            "Loading {} MCP server(s) from {} (base_dir={})",
            total,
            config_path,
            base_dir.display()
        );

        let mut failed = 0usize;
        for mut server_config in config.servers {
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
                let mut servers = self.servers.write().await;
                servers
                    .entry(server_config.id.clone())
                    .or_insert_with(|| McpServerHandle {
                        id: server_config.id.clone(),
                        config: server_config,
                        client: None,
                        tools: Vec::new(),
                        handshake: None,
                        status: ServerStatus::Error(e.to_string()),
                        source: ServerSource::Config,
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

        Ok(())
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
            if self.servers.read().await.contains_key(&record.name) {
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
                auto_restart: true,
                required_permissions: Vec::new(),
                tool_validators: HashMap::new(),
                display_name: None,
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
                let mut servers = self.servers.write().await;
                servers
                    .entry(config.id.clone())
                    .or_insert_with(|| McpServerHandle {
                        id: config.id.clone(),
                        config,
                        client: None,
                        tools: Vec::new(),
                        handshake: None,
                        status: ServerStatus::Error(e.to_string()),
                        source: ServerSource::Dynamic,
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
            let servers = self.servers.read().await;
            if let Some(existing) = servers.get(&id) {
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
                            status: "approved".to_string(),
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

        info!(
            "Connecting to MCP server [{}]: {} {:?}",
            id, config.command, config.args
        );

        // Retry with exponential backoff (3 attempts)
        let client = {
            let mut result: Option<McpClient> = None;
            let mut last_err = None;
            for attempt in 1..=3u32 {
                match McpClient::connect(
                    &id,
                    &config.command,
                    &config.args,
                    &config.env,
                    self.notification_tx.clone(),
                    self.mcp_request_timeout_secs,
                )
                .await
                {
                    Ok(c) => {
                        result = Some(c);
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
                Some(c) => c,
                None => {
                    return Err(anyhow::anyhow!(
                        "Failed to connect to MCP server '{}' after 3 attempts: {}",
                        id,
                        last_err.unwrap_or_else(|| anyhow::anyhow!("unknown error"))
                    ));
                }
            }
        };

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

        // Attempt cloto/handshake (optional)
        let handshake = match client.cloto_handshake().await {
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
        };

        let tool_names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
        let client_arc = Arc::new(client);

        let handle = McpServerHandle {
            id: id.clone(),
            config,
            client: Some(client_arc),
            tools: tools.clone(),
            handshake,
            status: ServerStatus::Connected,
            source,
        };

        // Register in servers map
        {
            let mut servers = self.servers.write().await;
            servers.insert(id.clone(), handle);
        }

        // Update tool routing index.
        // Skip mind.* servers — their tools (think, think_with_tools) are engine-internal
        // and called directly via call_server_tool(engine_id, ...), not through tool_index.
        if !id.starts_with("mind.") {
            let mut index = self.tool_index.write().await;
            for tool in &tools {
                if let Some(existing) = index.get(&tool.name) {
                    warn!(
                        tool = %tool.name,
                        existing_server = %existing,
                        new_server = %id,
                        "Tool name collision — overwriting routing"
                    );
                }
                index.insert(tool.name.clone(), id.clone());
            }
        }

        info!(
            "MCP server '{}' connected with {} tools",
            id,
            tool_names.len()
        );
        Ok(tool_names)
    }

    /// Disconnect and permanently remove an MCP server (also clears stopped_configs).
    pub async fn disconnect_server(&self, id: &str) -> Result<()> {
        let mut servers = self.servers.write().await;
        let handle = servers
            .remove(id)
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not found", id))?;
        let mut index = self.tool_index.write().await;
        for tool in &handle.tools {
            index.remove(&tool.name);
        }
        // Clean up stopped_configs too (permanent removal)
        let mut stopped = self.stopped_configs.write().await;
        stopped.remove(id);
        info!("MCP server '{}' disconnected", id);
        Ok(())
    }

    /// List all registered MCP servers with status.
    pub async fn list_servers(&self) -> Vec<McpServerInfo> {
        let servers = self.servers.read().await;
        let stopped = self.stopped_configs.read().await;

        let mut result: Vec<McpServerInfo> = servers
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
            })
            .collect();

        // Include stopped servers as Disconnected
        for (id, (config, source)) in stopped.iter() {
            if !servers.contains_key(id) {
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
                });
            }
        }

        result
    }

    /// Return IDs of connected mind.* servers (reasoning engines).
    pub async fn list_connected_mind_servers(&self) -> Vec<String> {
        let servers = self.servers.read().await;
        servers
            .iter()
            .filter(|(id, h)| id.starts_with("mind.") && h.status == ServerStatus::Connected)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Check if a server with the given ID is registered.
    pub async fn has_server(&self, id: &str) -> bool {
        let servers = self.servers.read().await;
        servers.contains_key(id)
    }

    /// Check if a specific server has a tool with the given name.
    pub async fn has_server_tool(&self, server_id: &str, tool_name: &str) -> bool {
        let servers = self.servers.read().await;
        servers
            .get(server_id)
            .is_some_and(|h| h.tools.iter().any(|t| t.name == tool_name))
    }

    // ============================================================
    // Tool Routing (used by PluginRegistry in Phase 1+)
    // ============================================================

    /// Collect tool schemas from all MCP servers in OpenAI function format.
    /// Includes kernel-native tools (create_mcp_server) only when YOLO mode is enabled.
    pub async fn collect_tool_schemas(&self) -> Vec<Value> {
        let servers = self.servers.read().await;
        let mut schemas = if self.yolo_mode.load(Ordering::Relaxed) {
            vec![super::mcp_kernel_tool::kernel_tool_schema()]
        } else {
            vec![]
        };
        for handle in servers.values() {
            if handle.status != ServerStatus::Connected {
                continue;
            }
            // Skip mind.* — engine-internal tools, not agent-facing
            if handle.id.starts_with("mind.") {
                continue;
            }
            for tool in &handle.tools {
                schemas.push(mcp_tool_schema(tool));
            }
        }
        schemas
    }

    /// Collect tool schemas filtered by server IDs.
    /// Includes kernel-native tools (create_mcp_server) only when YOLO mode is enabled.
    pub async fn collect_tool_schemas_for(&self, server_ids: &[String]) -> Vec<Value> {
        let servers = self.servers.read().await;
        let mut schemas = if self.yolo_mode.load(Ordering::Relaxed) {
            vec![super::mcp_kernel_tool::kernel_tool_schema()]
        } else {
            vec![]
        };
        for id in server_ids {
            if let Some(handle) = servers.get(id) {
                if handle.status != ServerStatus::Connected {
                    continue;
                }
                for tool in &handle.tools {
                    schemas.push(mcp_tool_schema(tool));
                }
            }
        }
        schemas
    }

    /// Collect tool schemas for a specific agent using `resolve_tool_access()`.
    /// Iterates all connected servers and includes only tools the agent is allowed to use.
    pub async fn collect_tool_schemas_for_agent(&self, agent_id: &str) -> Vec<Value> {
        let servers = self.servers.read().await;
        let mut schemas = if self.yolo_mode.load(Ordering::Relaxed) {
            vec![super::mcp_kernel_tool::kernel_tool_schema()]
        } else {
            vec![]
        };
        for (server_id, handle) in servers.iter() {
            if handle.status != ServerStatus::Connected {
                continue;
            }
            // Skip mind.* — engine-internal tools, not agent-facing
            if server_id.starts_with("mind.") {
                continue;
            }
            for tool in &handle.tools {
                match crate::db::resolve_tool_access(&self.pool, agent_id, server_id, &tool.name)
                    .await
                {
                    Ok(ref perm) if perm == "allow" => {
                        schemas.push(mcp_tool_schema(tool));
                    }
                    _ => {} // deny or error → skip
                }
            }
        }
        schemas
    }

    /// Look up which server provides a given tool.
    pub async fn get_tool_server_id(&self, tool_name: &str) -> Option<String> {
        let index = self.tool_index.read().await;
        index.get(tool_name).cloned()
    }

    /// Check tool access for a specific agent via `resolve_tool_access()`.
    pub async fn check_tool_access(
        &self,
        agent_id: &str,
        tool_name: &str,
    ) -> anyhow::Result<String> {
        let server_id = {
            let index = self.tool_index.read().await;
            index
                .get(tool_name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("MCP tool '{}' not found", tool_name))?
        };
        crate::db::resolve_tool_access(&self.pool, agent_id, &server_id, tool_name).await
    }

    /// Execute a tool by name, routing to the correct MCP server.
    /// Handles kernel-native tools (create_mcp_server) internally.
    /// Applies kernel-side validation (A) before forwarding to the MCP server.
    pub async fn execute_tool(&self, tool_name: &str, args: Value) -> Result<Value> {
        // Kernel-native tool: create_mcp_server
        if tool_name == "create_mcp_server" {
            return super::mcp_kernel_tool::execute_create_mcp_server(self, args).await;
        }

        let server_id = {
            let index = self.tool_index.read().await;
            index
                .get(tool_name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("MCP tool '{}' not found", tool_name))?
        };

        let (client, tool_validators) = {
            let servers = self.servers.read().await;
            let handle = servers
                .get(&server_id)
                .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not found", server_id))?;
            let client = handle
                .client
                .clone()
                .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not connected", server_id))?;
            (client, handle.config.tool_validators.clone())
        };

        // ──── Kernel-side Validation (A): Validate tool arguments before forwarding ────
        if let Some(validator_name) = tool_validators.get(tool_name) {
            validate_tool_arguments(validator_name, tool_name, &args)?;
        }

        let result = client.call_tool(tool_name, args).await?;

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
    /// Applies kernel-side validation (A) before forwarding to the MCP server.
    pub async fn call_server_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        args: Value,
    ) -> Result<super::mcp_protocol::CallToolResult> {
        let (client, tool_validators) = {
            let servers = self.servers.read().await;
            let handle = servers
                .get(server_id)
                .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not found", server_id))?;
            let client = handle
                .client
                .clone()
                .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not connected", server_id))?;
            (client, handle.config.tool_validators.clone())
        };

        // ──── Kernel-side Validation (A): Validate tool arguments before forwarding ────
        if let Some(validator_name) = tool_validators.get(tool_name) {
            validate_tool_arguments(validator_name, tool_name, &args)?;
        }

        client.call_tool(tool_name, args).await
    }

    // ============================================================
    // Event Forwarding (Kernel → MCP Servers)
    // ============================================================

    /// Broadcast a kernel event to all connected MCP servers as a notification.
    pub async fn broadcast_event(&self, event: &cloto_shared::ClotoEvent) {
        let servers = self.servers.read().await;
        for handle in servers.values() {
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

    /// Send a config update notification to a specific MCP server.
    pub async fn notify_config_updated(&self, server_id: &str, config: Value) {
        let servers = self.servers.read().await;
        if let Some(handle) = servers.get(server_id) {
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
    ) -> Result<Vec<String>> {
        let config = McpServerConfig {
            id: id.clone(),
            command: command.clone(),
            args: args.clone(),
            env: HashMap::new(),
            transport: "stdio".to_string(),
            auto_restart: true,
            required_permissions: Vec::new(),
            tool_validators: HashMap::new(),
            display_name: None,
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
            env: "{}".to_string(),
        };
        crate::db::save_mcp_server(&self.pool, &record).await?;

        Ok(tool_names)
    }

    /// Remove a dynamic MCP server and deactivate in DB.
    /// Config-loaded servers cannot be deleted (must be removed from mcp.toml).
    pub async fn remove_dynamic_server(&self, id: &str) -> Result<()> {
        // Reject deletion of config-loaded servers
        {
            let servers = self.servers.read().await;
            if let Some(handle) = servers.get(id) {
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
    // Memory Provider Discovery
    // ============================================================

    /// Find an MCP server that provides memory capabilities (has both `store` and `recall` tools).
    /// Returns the server ID if found.
    pub async fn find_memory_server(&self) -> Option<String> {
        let index = self.tool_index.read().await;
        let store_server = index.get("store").cloned();
        let recall_server = index.get("recall").cloned();
        match (store_server, recall_server) {
            (Some(s1), Some(s2)) if s1 == s2 => Some(s1),
            _ => None,
        }
    }

    // ============================================================
    // Server Lifecycle (MCP_SERVER_UI_DESIGN.md §4.3)
    // ============================================================

    /// Stop a server (disconnect but preserve config for restart).
    pub async fn stop_server(&self, id: &str) -> Result<()> {
        let mut servers = self.servers.write().await;
        let handle = servers
            .remove(id)
            .ok_or_else(|| anyhow::anyhow!("Server '{}' not found or already stopped", id))?;
        let mut index = self.tool_index.write().await;
        index.retain(|_, server_id| server_id != id);

        // Preserve config for restart capability (works for both config and dynamic)
        let mut stopped = self.stopped_configs.write().await;
        stopped.insert(id.to_string(), (handle.config.clone(), handle.source));

        info!(server = %id, source = ?handle.source, "MCP server stopped (config preserved for restart)");
        Ok(())
    }

    /// Start a server from stopped_configs or DB.
    pub async fn start_server(&self, id: &str) -> Result<Vec<String>> {
        // Check if already running
        {
            let servers = self.servers.read().await;
            if servers.contains_key(id) {
                return Err(anyhow::anyhow!("Server '{}' is already running", id));
            }
        }

        // 1. Check stopped_configs first (works for both config-loaded and dynamic)
        {
            let mut stopped = self.stopped_configs.write().await;
            if let Some((config, source)) = stopped.remove(id) {
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
            auto_restart: true,
            required_permissions: Vec::new(),
            tool_validators: HashMap::new(),
            display_name: None,
        };

        self.connect_server(config, ServerSource::Dynamic).await
    }

    /// Restart a server (stop + start).
    pub async fn restart_server(&self, id: &str) -> Result<Vec<String>> {
        // Stop if running (ignore error if already stopped)
        let _ = self.stop_server(id).await;
        self.start_server(id).await
    }

    /// Get a server's in-memory environment variables (from config or runtime).
    pub async fn get_server_env(&self, id: &str) -> HashMap<String, String> {
        let servers = self.servers.read().await;
        servers
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
            let mut servers = self.servers.write().await;
            if let Some(handle) = servers.get_mut(id) {
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
        let server_id = {
            let index = self.tool_index.read().await;
            index.get(tool_name).cloned()
        }?;
        let servers = self.servers.read().await;
        let handle = servers.get(&server_id)?;
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
    /// and auto-restarts them if `auto_restart` is enabled in their config.
    pub fn spawn_health_monitor(self: Arc<Self>, shutdown: Arc<tokio::sync::Notify>) {
        super::mcp_health::spawn_health_monitor(self, shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn yolo_mode_initializes_correctly() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:").await.unwrap();

        let manager_off = McpClientManager::new(pool.clone(), false, 120);
        assert!(!manager_off.yolo_mode.load(Ordering::Relaxed));

        let manager_on = McpClientManager::new(pool, true, 120);
        assert!(manager_on.yolo_mode.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn yolo_mode_toggle_at_runtime() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:").await.unwrap();

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
        crate::db::init_db(&pool, "sqlite::memory:").await.unwrap();

        // YOLO off: no kernel tools
        let manager = McpClientManager::new(pool.clone(), false, 120);
        let schemas = manager.collect_tool_schemas().await;
        assert!(schemas.is_empty(), "YOLO off should not include kernel tools");

        // YOLO on: kernel tool (create_mcp_server) included
        let manager_on = McpClientManager::new(pool, true, 120);
        let schemas_on = manager_on.collect_tool_schemas().await;
        assert!(!schemas_on.is_empty(), "YOLO on should include kernel tools");
        let name = schemas_on[0]["function"]["name"].as_str().unwrap();
        assert_eq!(name, "create_mcp_server");
    }

    #[tokio::test]
    async fn yolo_mode_persisted_to_db() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:").await.unwrap();

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
