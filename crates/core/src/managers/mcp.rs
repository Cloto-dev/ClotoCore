use super::mcp_protocol::{
    CallToolParams, CallToolResult, ClientCapabilities, ClientInfo, ClotoHandshakeParams,
    ClotoHandshakeResult, InitializeParams, JsonRpcRequest, JsonRpcResponse, ListToolsResult,
    McpConfigFile, McpServerConfig, McpTool, ToolContent,
};
use super::mcp_transport::{self, StdioTransport};
use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tracing::{debug, error, info, warn};

// ============================================================
// McpClient — JSON-RPC client for a single MCP server
// ============================================================

pub struct McpClient {
    transport: Arc<Mutex<StdioTransport>>,
    /// Cloned sender for lock-free request dispatch.
    /// The response loop holds `transport` Mutex during recv(); sending through
    /// this channel avoids the deadlock where call() would block on the same Mutex.
    sender: mpsc::Sender<String>,
    pending_requests: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>>,
    next_id: Arc<AtomicI64>,
    response_task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for McpClient {
    fn drop(&mut self) {
        if let Some(handle) = self.response_task.take() {
            handle.abort();
        }
    }
}

impl McpClient {
    const MAX_PENDING_REQUESTS: usize = 100;
    const REQUEST_TIMEOUT_SECS: u64 = 120;

    pub async fn connect(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self> {
        let transport = StdioTransport::start(command, args, env).await?;
        let sender = transport.sender();
        let mut client = Self {
            transport: Arc::new(Mutex::new(transport)),
            sender,
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicI64::new(1)),
            response_task: None,
        };

        client.start_response_loop();
        client.initialize().await?;

        Ok(client)
    }

    fn start_response_loop(&mut self) {
        let transport = self.transport.clone();
        let pending = self.pending_requests.clone();

        let handle = tokio::spawn(async move {
            loop {
                let msg_opt = {
                    let mut tp = transport.lock().await;
                    tp.recv().await
                };

                if let Some(line) = msg_opt {
                    if let Ok(response) = serde_json::from_str::<JsonRpcResponse>(&line) {
                        if let Some(id_val) = response.id {
                            if let Some(id) = id_val.as_i64() {
                                let mut map = pending.lock().await;
                                if let Some(tx) = map.remove(&id) {
                                    if let Some(error) = response.error {
                                        if tx
                                            .send(Err(anyhow::anyhow!(
                                                "RPC Error {}: {}",
                                                error.code,
                                                error.message
                                            )))
                                            .is_err()
                                        {
                                            debug!("Response receiver dropped for request {}", id);
                                        }
                                    } else if tx
                                        .send(Ok(response.result.unwrap_or(Value::Null)))
                                        .is_err()
                                    {
                                        debug!("Response receiver dropped for request {}", id);
                                    }
                                }
                            }
                        }
                    } else {
                        debug!("Received non-response message: {}", line);
                    }
                } else {
                    error!("MCP Connection closed.");
                    let mut map = pending.lock().await;
                    let count = map.len();
                    for (id, tx) in map.drain() {
                        if tx
                            .send(Err(anyhow::anyhow!("MCP server process terminated")))
                            .is_err()
                        {
                            debug!("Response receiver dropped for request {}", id);
                        }
                    }
                    if count > 0 {
                        error!(
                            "Failed {} pending MCP requests due to process termination",
                            count
                        );
                    }
                    break;
                }
            }
        });
        self.response_task = Some(handle);
    }

    async fn call(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let request = JsonRpcRequest::new(id, method, params);
        let req_str = serde_json::to_string(&request)?;

        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.pending_requests.lock().await;
            if map.len() >= Self::MAX_PENDING_REQUESTS {
                return Err(anyhow::anyhow!(
                    "MCP pending request limit reached ({})",
                    Self::MAX_PENDING_REQUESTS
                ));
            }
            map.insert(id, tx);
        }

        self.sender
            .send(req_str)
            .await
            .context("Failed to send request to MCP transport")?;

        if let Ok(res) = tokio::time::timeout(
            std::time::Duration::from_secs(Self::REQUEST_TIMEOUT_SECS),
            rx,
        )
        .await
        {
            res.context("Response channel closed")?
        } else {
            let mut map = self.pending_requests.lock().await;
            map.remove(&id);
            Err(anyhow::anyhow!("MCP Request timed out"))
        }
    }

    async fn initialize(&self) -> Result<()> {
        let params = InitializeParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: ClientCapabilities {},
            client_info: ClientInfo {
                name: "CLOTO-KERNEL".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let result = self
            .call("initialize", Some(serde_json::to_value(params)?))
            .await?;
        info!("MCP Initialized: {:?}", result);

        // Send initialized notification
        let notify = JsonRpcRequest::notification("notifications/initialized", None);
        let notify_str = serde_json::to_string(&notify)?;
        self.sender
            .send(notify_str)
            .await
            .context("Failed to send initialized notification")?;

        Ok(())
    }

    pub async fn list_tools(&self) -> Result<ListToolsResult> {
        let val = self.call("tools/list", None).await?;
        let result: ListToolsResult = serde_json::from_value(val)?;
        Ok(result)
    }

    pub async fn call_tool(&self, name: &str, args: Value) -> Result<CallToolResult> {
        let params = CallToolParams {
            name: name.to_string(),
            arguments: args,
        };
        let val = self
            .call("tools/call", Some(serde_json::to_value(params)?))
            .await?;
        let result: CallToolResult = serde_json::from_value(val)?;
        Ok(result)
    }

    /// Send a JSON-RPC notification (fire-and-forget, no response expected).
    pub async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()> {
        let request = JsonRpcRequest::notification(method, params);
        let req_str = serde_json::to_string(&request)?;
        self.sender
            .send(req_str)
            .await
            .context("Failed to send notification to MCP transport")
    }

    /// Perform cloto/handshake custom method.
    pub async fn cloto_handshake(&self) -> Result<Option<ClotoHandshakeResult>> {
        let params = ClotoHandshakeParams {
            kernel_version: env!("CARGO_PKG_VERSION").to_string(),
        };

        match self
            .call("cloto/handshake", Some(serde_json::to_value(params)?))
            .await
        {
            Ok(val) => {
                let result: ClotoHandshakeResult = serde_json::from_value(val)?;
                Ok(Some(result))
            }
            Err(e) => {
                // cloto/handshake is optional — non-Cloto MCP servers won't support it
                debug!("cloto/handshake not supported: {}", e);
                Ok(None)
            }
        }
    }

    /// Check if the underlying transport process is still alive.
    /// Uses sender channel state to avoid contending with the response loop's Mutex.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        !self.sender.is_closed()
    }
}

// ============================================================
// McpServerHandle — per-server state
// ============================================================

#[derive(Clone)]
pub struct McpServerHandle {
    pub id: String,
    pub config: McpServerConfig,
    pub client: Option<Arc<McpClient>>,
    pub tools: Vec<McpTool>,
    pub handshake: Option<ClotoHandshakeResult>,
    pub status: ServerStatus,
    pub source: ServerSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerSource {
    Config,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    Connected,
    Disconnected,
    Error(String),
}

impl serde::Serialize for ServerStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Connected => serializer.serialize_str("Connected"),
            Self::Disconnected => serializer.serialize_str("Disconnected"),
            Self::Error(_) => serializer.serialize_str("Error"),
        }
    }
}

/// Public info about a connected MCP server.
#[derive(Debug, Clone, serde::Serialize)]
pub struct McpServerInfo {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pub status: ServerStatus,
    pub status_message: Option<String>,
    pub tools: Vec<String>,
    pub is_cloto_sdk: bool,
    pub source: ServerSource,
}

// ============================================================
// McpClientManager — kernel-level MCP server orchestrator
// ============================================================

pub struct McpClientManager {
    servers: RwLock<HashMap<String, McpServerHandle>>,
    pool: SqlitePool,
    /// Tool name → server ID index for fast routing
    tool_index: RwLock<HashMap<String, String>>,
    /// YOLO mode: auto-approve all MCP server permissions (ARCHITECTURE.md §5.7).
    /// Arc<AtomicBool> allows runtime toggle via API without restart.
    pub yolo_mode: Arc<AtomicBool>,
    /// Preserved configs from stopped servers, enabling restart for config-loaded servers
    stopped_configs: RwLock<HashMap<String, (McpServerConfig, ServerSource)>>,
}

impl McpClientManager {
    #[must_use]
    pub fn new(pool: SqlitePool, yolo_mode: bool) -> Self {
        Self {
            servers: RwLock::new(HashMap::new()),
            pool,
            tool_index: RwLock::new(HashMap::new()),
            yolo_mode: Arc::new(AtomicBool::new(yolo_mode)),
            stopped_configs: RwLock::new(HashMap::new()),
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
                            approved_by: Some("YOLO".to_string()),
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
                match McpClient::connect(&config.command, &config.args, &config.env).await {
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

        // Update tool routing index
        {
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

    /// Kernel-native tool schema: create_mcp_server
    fn kernel_tool_schema() -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "create_mcp_server",
                "description": concat!(
                    "Create a new MCP server from Python code. ",
                    "The code is safety-validated before execution. ",
                    "Returns the list of discovered tools from the new server.\n\n",
                    "Auto-provided (do NOT include): imports (asyncio, json, mcp.server.Server, ",
                    "mcp.types.TextContent/Tool), `app = Server(name)`, main() entrypoint, stdio transport.\n\n",
                    "Blocked imports: subprocess, shutil, socket, ctypes, multiprocessing, signal, ",
                    "pty, fcntl, resource, importlib, code, codeop, compileall, py_compile.\n",
                    "Blocked patterns: eval(), exec(), __import__(), compile(), open(), globals(), locals(), ",
                    "os.system, os.popen, os.spawn, os.exec, os.remove, os.unlink, os.rmdir, os.makedirs, ",
                    "subprocess., __builtins__, getattr(), setattr(), delattr().\n",
                    "Max code size: 10KB. Allowed imports: json, asyncio, httpx, os, datetime, time, ",
                    "math, re, hashlib, base64, urllib.request, typing.",
                ),
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Server name (1-64 chars, alphanumeric + underscore/hyphen)"
                        },
                        "description": {
                            "type": "string",
                            "description": "Short description of the server's purpose"
                        },
                        "code": {
                            "type": "string",
                            "description": concat!(
                                "Python code body defining MCP tool handlers. You MUST define exactly two decorated functions:\n\n",
                                "1. @app.list_tools()\\nasync def list_tools() -> list[Tool]:\\n",
                                "    return [Tool(name=\"tool_name\", description=\"...\", ",
                                "inputSchema={\"type\": \"object\", \"properties\": {...}, \"required\": [...]})]\n\n",
                                "2. @app.call_tool()\\nasync def call_tool(name: str, arguments: dict) -> list[TextContent]:\\n",
                                "    if name == \"tool_name\":\\n",
                                "        result = ...  # your logic\\n",
                                "        return [TextContent(type=\"text\", text=json.dumps(result))]\\n",
                                "    raise ValueError(f\"Unknown tool: {name}\")\n\n",
                                "You may add helper functions and use httpx for HTTP requests. ",
                                "Do not include imports already provided (asyncio, json, mcp.server, mcp.types).",
                            )
                        }
                    },
                    "required": ["name", "description", "code"]
                }
            }
        })
    }

    /// Collect tool schemas from all MCP servers in OpenAI function format.
    /// Includes kernel-native tools (create_mcp_server) only when YOLO mode is enabled.
    pub async fn collect_tool_schemas(&self) -> Vec<Value> {
        let servers = self.servers.read().await;
        let mut schemas = if self.yolo_mode.load(Ordering::Relaxed) {
            vec![Self::kernel_tool_schema()]
        } else {
            vec![]
        };
        for handle in servers.values() {
            if handle.status != ServerStatus::Connected {
                continue;
            }
            for tool in &handle.tools {
                schemas.push(serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description.as_deref().unwrap_or(""),
                        "parameters": tool.input_schema,
                    }
                }));
            }
        }
        schemas
    }

    /// Collect tool schemas filtered by server IDs.
    /// Includes kernel-native tools (create_mcp_server) only when YOLO mode is enabled.
    pub async fn collect_tool_schemas_for(&self, server_ids: &[String]) -> Vec<Value> {
        let servers = self.servers.read().await;
        let mut schemas = if self.yolo_mode.load(Ordering::Relaxed) {
            vec![Self::kernel_tool_schema()]
        } else {
            vec![]
        };
        for id in server_ids {
            if let Some(handle) = servers.get(id) {
                if handle.status != ServerStatus::Connected {
                    continue;
                }
                for tool in &handle.tools {
                    schemas.push(serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description.as_deref().unwrap_or(""),
                            "parameters": tool.input_schema,
                        }
                    }));
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
            vec![Self::kernel_tool_schema()]
        } else {
            vec![]
        };
        for (server_id, handle) in servers.iter() {
            if handle.status != ServerStatus::Connected {
                continue;
            }
            for tool in &handle.tools {
                match crate::db::resolve_tool_access(
                    &self.pool,
                    agent_id,
                    server_id,
                    &tool.name,
                )
                .await
                {
                    Ok(ref perm) if perm == "allow" => {
                        schemas.push(serde_json::json!({
                            "type": "function",
                            "function": {
                                "name": tool.name,
                                "description": tool.description.as_deref().unwrap_or(""),
                                "parameters": tool.input_schema,
                            }
                        }));
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
            return self.execute_create_mcp_server(args).await;
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
    pub async fn call_server_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        args: Value,
    ) -> Result<CallToolResult> {
        let client = {
            let servers = self.servers.read().await;
            let handle = servers
                .get(server_id)
                .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not found", server_id))?;
            handle
                .client
                .clone()
                .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not connected", server_id))?
        };

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
        let record = records
            .into_iter()
            .find(|r| r.name == id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Server '{}' not found in stopped configs or database",
                    id
                )
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
    pub async fn update_server_env(
        &self,
        id: &str,
        env: HashMap<String, String>,
    ) -> Result<()> {
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
    fn detect_project_root(from: &std::path::Path) -> Option<std::path::PathBuf> {
        let start = if from.is_file() { from.parent()? } else { from };
        let canonical = std::fs::canonicalize(start).ok()?;
        // Strip Windows UNC prefix (\\?\) that canonicalize() adds — Python cannot handle it
        let mut dir = {
            let s = canonical.to_string_lossy();
            if s.starts_with(r"\\?\") {
                std::path::PathBuf::from(&s[4..])
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
    /// Follows the `tokio::select!` + `Arc<Notify>` shutdown pattern from events.rs.
    pub fn spawn_health_monitor(self: Arc<Self>, shutdown: Arc<tokio::sync::Notify>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                tokio::select! {
                    () = shutdown.notified() => {
                        info!("MCP health monitor shutting down");
                        break;
                    }
                    _ = interval.tick() => {
                        self.check_and_restart_dead_servers().await;
                    }
                }
            }
        });
    }

    /// Scan all registered MCP servers and restart any that have died
    /// (process exited / channel closed) if their config has `auto_restart: true`.
    async fn check_and_restart_dead_servers(&self) {
        let dead_servers: Vec<String> = {
            let servers = self.servers.read().await;
            servers
                .iter()
                .filter_map(|(id, handle)| {
                    if !handle.config.auto_restart {
                        return None;
                    }
                    let is_dead = match &handle.client {
                        Some(client) => !client.is_alive(),
                        None => matches!(handle.status, ServerStatus::Error(_)),
                    };
                    if is_dead {
                        Some(id.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };

        for server_id in dead_servers {
            warn!(server_id = %server_id, "MCP server died, attempting auto-restart");
            match self.restart_server(&server_id).await {
                Ok(tools) => {
                    info!(
                        server_id = %server_id,
                        tools = tools.len(),
                        "MCP server auto-restarted successfully"
                    );
                }
                Err(e) => {
                    error!(
                        server_id = %server_id,
                        error = %e,
                        "MCP server auto-restart failed"
                    );
                    // Update status to Error so the UI reflects the failure
                    let mut servers = self.servers.write().await;
                    if let Some(handle) = servers.get_mut(&server_id) {
                        handle.status =
                            ServerStatus::Error(format!("Auto-restart failed: {}", e));
                    }
                }
            }
        }
    }
}

// ============================================================
// Kernel-side Tool Validation (Security Feature A)
// ============================================================

/// Blocked shell patterns for the "sandbox" validator.
/// Ported from plugins/terminal/src/sandbox.rs for kernel-level defense-in-depth.
const SANDBOX_BLOCKED_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -fr /",
    "mkfs",
    "dd if=/dev",
    ":(){ :|:& };:",
    "> /dev/sda",
    "shutdown",
    "reboot",
    "init 0",
    "init 6",
    "chmod -r 777 /",
    "chown -r",
    "sudo ",
    "su ",
    "su\t",
    "doas ",
    "/bin/rm -rf",
    "/usr/bin/rm -rf",
];

/// Blocked shell metacharacters for the "sandbox" validator.
const SANDBOX_BLOCKED_METACHAR: &[&str] = &["$(", "`", "|", ";", "&&", "||"];

/// Validate tool arguments at the kernel level before forwarding to an MCP server.
/// This provides defense-in-depth: even if the MCP server's own validation is
/// bypassed (e.g., compromised server), the kernel still catches dangerous inputs.
fn validate_tool_arguments(validator_name: &str, tool_name: &str, args: &Value) -> Result<()> {
    match validator_name {
        "sandbox" => validate_sandbox_args(tool_name, args),
        other => {
            warn!(
                "Unknown tool validator '{}' for tool '{}', skipping",
                other, tool_name
            );
            Ok(())
        }
    }
}

/// "sandbox" validator: checks command arguments against blocked patterns.
/// Applied to tools like `execute_command` that run shell commands.
fn validate_sandbox_args(_tool_name: &str, args: &Value) -> Result<()> {
    let Some(command) = args.get("command").and_then(|v| v.as_str()) else {
        return Ok(()); // No command argument, nothing to validate
    };

    if command.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "Kernel validation: empty command is not allowed"
        ));
    }

    // Note: NFKC normalization is handled by the MCP server itself (Python side).
    // Kernel validation operates on the raw string for defense-in-depth.
    let lower = command.to_lowercase();

    // Block embedded newlines/carriage returns (injection vectors)
    if lower.contains('\n')
        || lower.contains('\r')
        || lower.contains('\u{2028}')
        || lower.contains('\u{2029}')
    {
        return Err(anyhow::anyhow!(
            "Kernel validation: command contains embedded newline or line separator"
        ));
    }

    // Block shell metacharacters
    for meta in SANDBOX_BLOCKED_METACHAR {
        if lower.contains(meta) {
            return Err(anyhow::anyhow!(
                "Kernel validation: command contains blocked shell metacharacter: '{}'",
                meta
            ));
        }
    }

    // Check for blocked patterns
    for pattern in SANDBOX_BLOCKED_PATTERNS {
        if lower.contains(pattern) {
            return Err(anyhow::anyhow!(
                "Kernel validation: command contains blocked pattern: '{}'",
                pattern
            ));
        }
    }

    // Block rm with both -r and -f flags
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    if let Some(first) = tokens.first() {
        if *first == "rm" || first.ends_with("/rm") {
            let has_recursive = tokens.iter().any(|t| {
                t.starts_with('-') && !t.starts_with("--") && (t.contains('r') || t.contains('R'))
            });
            let has_force = tokens
                .iter()
                .any(|t| t.starts_with('-') && !t.starts_with("--") && t.contains('f'));
            if has_recursive && has_force {
                return Err(anyhow::anyhow!(
                    "Kernel validation: command contains dangerous rm flags (-r and -f)"
                ));
            }
        }
    }

    Ok(())
}

// ============================================================
// Code Validator — safety checks for agent-generated MCP code
// ============================================================

/// Blocked imports that could enable system access or code execution.
const BLOCKED_IMPORTS: &[&str] = &[
    "subprocess", "shutil", "socket", "ctypes", "multiprocessing",
    "signal", "pty", "fcntl", "resource", "importlib", "code",
    "codeop", "compileall", "py_compile",
];

/// Blocked function/attribute patterns.
const BLOCKED_PATTERNS: &[&str] = &[
    "eval(", "exec(", "__import__(", "compile(",
    "open(", "globals(", "locals(",
    "os.system", "os.popen", "os.spawn", "os.exec",
    "os.remove", "os.unlink", "os.rmdir", "os.makedirs",
    "subprocess.", "__builtins__",
    "getattr(", "setattr(", "delattr(",
];

/// Maximum allowed code size in bytes.
const MAX_CODE_SIZE: usize = 10_000;

/// Validate agent-generated Python code for safety.
/// Returns Ok(()) if code is safe, Err with list of violations otherwise.
fn validate_mcp_code(code: &str) -> std::result::Result<(), Vec<String>> {
    let mut errors = Vec::new();

    // L1: Size limit
    if code.len() > MAX_CODE_SIZE {
        errors.push(format!(
            "Code too large: {} bytes (max {})",
            code.len(),
            MAX_CODE_SIZE
        ));
    }

    // Normalize for pattern matching (lowercase for import checks)
    let code_lower = code.to_lowercase();

    // L2: Blocked imports
    for &blocked in BLOCKED_IMPORTS {
        // Match "import subprocess", "from subprocess", "import subprocess,"
        let import_pattern = format!("import {blocked}");
        let from_pattern = format!("from {blocked}");
        if code_lower.contains(&import_pattern) || code_lower.contains(&from_pattern) {
            errors.push(format!("Blocked import: '{blocked}'"));
        }
    }

    // L3: Blocked function/attribute patterns
    for &pattern in BLOCKED_PATTERNS {
        if code.contains(pattern) {
            errors.push(format!("Blocked pattern: '{pattern}'"));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// ============================================================
// Kernel Tool: create_mcp_server
// ============================================================

impl McpClientManager {
    /// Execute the kernel-native create_mcp_server tool.
    /// Requires YOLO mode to be enabled — autonomous server creation is a privileged operation.
    async fn execute_create_mcp_server(&self, args: Value) -> Result<Value> {
        if !self.yolo_mode.load(Ordering::Relaxed) {
            return Ok(serde_json::json!({
                "status": "rejected",
                "reason": "Autonomous MCP server creation requires YOLO mode to be enabled. Ask the operator to enable it in Settings.",
            }));
        }

        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: name"))?;
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("Agent-generated MCP server");
        let code = args
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: code"))?;

        // Validate name (same rules as handlers.rs)
        if name.is_empty() || name.len() > 64 {
            return Err(anyhow::anyhow!(
                "Server name must be 1-64 characters"
            ));
        }
        let valid_name = name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !valid_name {
            return Err(anyhow::anyhow!(
                "Server name must contain only alphanumeric, underscore, or hyphen"
            ));
        }

        // Code safety validation (Layer 5)
        if let Err(violations) = validate_mcp_code(code) {
            return Ok(serde_json::json!({
                "status": "rejected",
                "reason": "Code validation failed — review violations and use hints to fix",
                "violations": violations,
                "hints": {
                    "blocked_imports": BLOCKED_IMPORTS,
                    "blocked_patterns": BLOCKED_PATTERNS,
                    "max_code_size_bytes": MAX_CODE_SIZE,
                    "auto_provided_imports": [
                        "asyncio", "json", "mcp.server.Server",
                        "mcp.server.stdio.stdio_server",
                        "mcp.types.TextContent", "mcp.types.Tool"
                    ],
                    "allowed_additional_imports": [
                        "httpx", "os", "datetime", "time", "math",
                        "re", "hashlib", "base64", "urllib.request", "typing"
                    ],
                }
            }));
        }

        // Generate script from template
        let script = format!(
            r#""""MCP Server: {name} — {desc}"""
import asyncio
import json

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

app = Server("{name}")

{code}

async def main():
    async with stdio_server() as (read, write):
        await app.run(read, write, app.create_initialization_options())

if __name__ == "__main__":
    asyncio.run(main())
"#,
            name = name,
            desc = description.replace('"', r#"\""#),
            code = code,
        );

        // Write script file
        let scripts_dir = std::path::Path::new("scripts");
        if !scripts_dir.exists() {
            std::fs::create_dir_all(scripts_dir)
                .map_err(|e| anyhow::anyhow!("Failed to create scripts directory: {}", e))?;
        }
        let script_path = scripts_dir.join(format!("mcp_{name}.py"));
        std::fs::write(&script_path, &script)
            .map_err(|e| anyhow::anyhow!("Failed to write script: {}", e))?;

        // Register and connect the server
        let server_id = format!("agent.{name}");
        let tool_names = self
            .add_dynamic_server(
                server_id.clone(),
                "python".to_string(),
                vec![script_path.to_string_lossy().to_string()],
                Some(script),
                Some(description.to_string()),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to start server: {}", e))?;

        info!(
            "Agent created MCP server '{}' with {} tool(s): {:?}",
            server_id,
            tool_names.len(),
            tool_names
        );

        Ok(serde_json::json!({
            "status": "created",
            "server_id": server_id,
            "tools": tool_names,
            "script_path": script_path.to_string_lossy(),
        }))
    }
}
