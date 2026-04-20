//! JSON-RPC 2.0 client for communicating with individual MCP servers.
//!
//! Each `McpClient` manages a single MCP server connection over stdio transport,
//! handling initialization, tool calls, notifications, and shutdown.

use super::mcp_mgp::{
    MgpClientCapabilities, MgpServerCapabilities, CLIENT_EXTENSIONS, MGP_VERSION,
};
use super::mcp_protocol::{
    CallToolParams, CallToolResult, ClientCapabilities, ClientInfo, ClotoHandshakeParams,
    ClotoHandshakeResult, InitializeParams, JsonRpcRequest, ListToolsResult,
};
use super::mcp_transport::{HttpTransport, McpTransport, StdioTransport};
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex, Notify};
use tracing::{debug, error, info};

/// MCP server-initiated notification (Server→Kernel).
#[derive(Debug, Clone)]
pub struct McpNotification {
    pub server_id: String,
    pub method: String,
    pub params: Option<Value>,
}

/// A single streaming request's dispatch state. `sender` forwards chunks to
/// the caller's `mpsc::Receiver`; `activity` is pulsed on each chunk so that
/// the per-request watchdog in `call_tool_streaming` can reset its idle
/// deadline (bug-351).
pub(super) type StreamCollector = (mpsc::Sender<Value>, Arc<Notify>);

pub struct McpClient {
    transport: Arc<Mutex<McpTransport>>,
    /// Cloned sender for lock-free request dispatch.
    /// The response loop holds `transport` Mutex during recv(); sending through
    /// this channel avoids the deadlock where call() would block on the same Mutex.
    sender: mpsc::Sender<String>,
    pending_requests: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>>,
    next_id: Arc<AtomicI64>,
    response_task: Option<tokio::task::JoinHandle<()>>,
    notification_tx: mpsc::Sender<McpNotification>,
    request_timeout_secs: u64,
    /// Per-request idle timeout for streaming calls (MGP §12). When no chunk
    /// arrives within this window, `call_tool_streaming` aborts with a
    /// "Streaming request timed out" error. bug-351.
    stream_idle_timeout_secs: u64,
    /// Stream chunk collectors: request_id → (chunk sender, activity notifier).
    stream_collectors: Arc<Mutex<HashMap<i64, StreamCollector>>>,
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

    /// Kill the underlying child process and wait for it to exit.
    /// Must be called before dropping the handle to avoid race conditions
    /// where the old process still holds file locks (Issue #65).
    pub async fn shutdown(&self) {
        let mut transport = self.transport.lock().await;
        transport.kill_and_wait().await;
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn connect(
        server_id: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        notification_tx: mpsc::Sender<McpNotification>,
        request_timeout_secs: u64,
        stream_idle_timeout_secs: u64,
        isolation: Option<&super::mcp_isolation::IsolationProfile>,
        llm_proxy_port: u16,
        sensitive_env_keys: &[String],
    ) -> Result<(Self, Option<MgpServerCapabilities>)> {
        let stdio = StdioTransport::start(
            command,
            args,
            env,
            isolation,
            llm_proxy_port,
            sensitive_env_keys,
        )
        .await?;
        let sender = stdio.sender();
        let transport = McpTransport::Stdio(Box::new(stdio));
        let mut client = Self {
            transport: Arc::new(Mutex::new(transport)),
            sender,
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicI64::new(1)),
            response_task: None,
            notification_tx,
            request_timeout_secs,
            stream_idle_timeout_secs,
            stream_collectors: Arc::new(Mutex::new(HashMap::new())),
        };

        client.start_response_loop(server_id);
        let mgp_caps = client.initialize().await?;

        Ok((client, mgp_caps))
    }

    /// Connect to a remote MCP server via Streamable HTTP transport.
    pub async fn connect_http(
        server_id: &str,
        url: &str,
        auth_token: Option<&str>,
        notification_tx: mpsc::Sender<McpNotification>,
        request_timeout_secs: u64,
        stream_idle_timeout_secs: u64,
    ) -> Result<(Self, Option<MgpServerCapabilities>)> {
        let http = HttpTransport::start(url, auth_token).await?;
        let sender = http.sender();
        let transport = McpTransport::Http(Box::new(http));
        let mut client = Self {
            transport: Arc::new(Mutex::new(transport)),
            sender,
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicI64::new(1)),
            response_task: None,
            notification_tx,
            request_timeout_secs,
            stream_idle_timeout_secs,
            stream_collectors: Arc::new(Mutex::new(HashMap::new())),
        };

        client.start_response_loop(server_id);
        let mgp_caps = client.initialize().await?;

        Ok((client, mgp_caps))
    }

    #[allow(clippy::too_many_lines)]
    fn start_response_loop(&mut self, server_id: &str) {
        use super::mcp_protocol::JsonRpcMessage;

        let transport = self.transport.clone();
        let pending = self.pending_requests.clone();
        let notif_tx = self.notification_tx.clone();
        let stream_collectors = self.stream_collectors.clone();
        let server_id_owned = server_id.to_string();

        let handle = tokio::spawn(async move {
            loop {
                let msg_opt = {
                    let mut tp = transport.lock().await;
                    // Release Mutex after 5s to prevent deadlock when reader hangs
                    match tokio::time::timeout(std::time::Duration::from_secs(5), tp.recv()).await {
                        Ok(msg) => msg,
                        Err(_) => continue, // Timeout — release lock, retry
                    }
                };

                if let Some(line) = msg_opt {
                    match serde_json::from_str::<JsonRpcMessage>(&line) {
                        Ok(JsonRpcMessage::Response(response)) => {
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
                                                debug!(
                                                    "Response receiver dropped for request {}",
                                                    id
                                                );
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
                        }
                        Ok(JsonRpcMessage::Notification(notif)) => {
                            // Route streaming notifications to collectors (MGP §12)
                            let is_stream = notif.method == "notifications/mgp.stream.chunk"
                                || notif.method == "notifications/mgp.stream.progress";
                            if is_stream {
                                if let Some(ref params) = notif.params {
                                    if let Some(req_id) =
                                        params.get("request_id").and_then(serde_json::Value::as_i64)
                                    {
                                        let collectors = stream_collectors.lock().await;
                                        if let Some((tx, notify)) = collectors.get(&req_id) {
                                            let _ = tx.try_send(params.clone());
                                            // Pulse the per-stream watchdog so its idle
                                            // deadline resets. Buffered — safe even if the
                                            // watchdog hasn't entered `notified()` yet.
                                            notify.notify_one();
                                            continue; // routed to collector, skip normal path
                                        }
                                    }
                                }
                            }
                            if notif_tx
                                .try_send(McpNotification {
                                    server_id: server_id_owned.clone(),
                                    method: notif.method,
                                    params: notif.params,
                                })
                                .is_err()
                            {
                                debug!("Notification channel full, dropping");
                            }
                        }
                        Err(e) => {
                            debug!(
                                error = %e,
                                "Received unparseable message: {}",
                                &line[..line.len().min(200)]
                            );
                        }
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

    pub async fn call(&self, method: &str, params: Option<Value>) -> Result<Value> {
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
            std::time::Duration::from_secs(self.request_timeout_secs),
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

    async fn initialize(&self) -> Result<Option<MgpServerCapabilities>> {
        let params = InitializeParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: ClientCapabilities {
                mgp: Some(MgpClientCapabilities {
                    version: MGP_VERSION.to_string(),
                    extensions: CLIENT_EXTENSIONS.iter().map(|s| (*s).to_string()).collect(),
                }),
            },
            client_info: ClientInfo {
                name: "CLOTO-KERNEL".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let result = self
            .call("initialize", Some(serde_json::to_value(params)?))
            .await?;
        info!("MCP Initialized: {:?}", result);

        // Extract MGP server capabilities from response (if present)
        // Primary: capabilities.mgp (direct). Fallback: capabilities.experimental.mgp (Python SDK compatible)
        let mgp_server_caps = result
            .get("capabilities")
            .and_then(|caps| {
                caps.get("mgp")
                    .or_else(|| caps.get("experimental").and_then(|exp| exp.get("mgp")))
            })
            .and_then(|mgp| serde_json::from_value::<MgpServerCapabilities>(mgp.clone()).ok());

        Ok(mgp_server_caps)
    }

    /// Send `notifications/initialized` to the server.
    /// Split from `initialize()` to allow Permission Flow insertion between
    /// initialize response and initialized notification (MGP §3).
    pub async fn send_initialized_notification(&self) -> Result<()> {
        let notify = JsonRpcRequest::notification("notifications/initialized", None);
        let notify_str = serde_json::to_string(&notify)?;
        self.sender
            .send(notify_str)
            .await
            .context("Failed to send initialized notification")
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

    /// Call a tool with streaming enabled (MGP §12).
    /// Returns a receiver for stream chunks and a receiver for the final result.
    pub async fn call_tool_streaming(
        &self,
        name: &str,
        args: Value,
    ) -> Result<(
        mpsc::Receiver<Value>,
        oneshot::Receiver<Result<CallToolResult>>,
    )> {
        use super::mcp_protocol::CallToolParams;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let params = CallToolParams {
            name: name.to_string(),
            arguments: args,
        };
        let mut params_value = serde_json::to_value(params)?;
        // Inject _mgp stream hint
        params_value["_mgp"] = serde_json::json!({ "stream": true });

        let request = JsonRpcRequest::new(id, "tools/call", Some(params_value));
        let req_str = serde_json::to_string(&request)?;

        // Create stream chunk channel + per-request activity notifier (bug-351).
        // The notifier is pulsed by response_loop on every chunk arrival so the
        // watchdog task below can reset its idle deadline.
        let (chunk_tx, chunk_rx) = mpsc::channel(256);
        let activity_notify = Arc::new(Notify::new());
        {
            let mut collectors = self.stream_collectors.lock().await;
            collectors.insert(id, (chunk_tx, activity_notify.clone()));
        }

        // Create final result channel
        let (result_tx, result_rx) = oneshot::channel();
        let stream_collectors = self.stream_collectors.clone();
        let final_id = id;
        let total_timeout_secs = self.request_timeout_secs;
        let idle_timeout_secs = self.stream_idle_timeout_secs;
        {
            let mut map = self.pending_requests.lock().await;
            let (inner_tx, inner_rx) = oneshot::channel();
            map.insert(id, inner_tx);

            // Spawn a watchdog task that enforces both the total request cap
            // and a per-chunk idle timeout (MGP §12, bug-351). All three error
            // paths emit a message containing "Streaming request timed out" so
            // that qa/issue-registry.json's bug-351 pattern still matches.
            tokio::spawn(async move {
                let total_deadline = tokio::time::Instant::now()
                    + std::time::Duration::from_secs(total_timeout_secs);
                let idle_duration = std::time::Duration::from_secs(idle_timeout_secs);
                let mut idle_deadline = tokio::time::Instant::now() + idle_duration;
                let mut inner_rx = inner_rx;

                let result: Result<CallToolResult> = loop {
                    tokio::select! {
                        // Final response arrived (or the oneshot was dropped).
                        res = &mut inner_rx => match res {
                            Ok(Ok(val)) => break serde_json::from_value::<CallToolResult>(val)
                                .map_err(|e| anyhow::anyhow!("Failed to parse streaming result: {}", e)),
                            Ok(Err(e)) => break Err(e),
                            Err(_) => break Err(anyhow::anyhow!("Response channel closed")),
                        },
                        // Request-total cap reached (existing behavior, preserved).
                        () = tokio::time::sleep_until(total_deadline) => {
                            break Err(anyhow::anyhow!(
                                "Streaming request timed out (total {}s)",
                                total_timeout_secs
                            ));
                        }
                        // No chunk arrived within the per-chunk idle window.
                        () = tokio::time::sleep_until(idle_deadline) => {
                            break Err(anyhow::anyhow!(
                                "Streaming request timed out (idle {}s, no chunk received)",
                                idle_timeout_secs
                            ));
                        }
                        // Chunk delivered — bump the idle deadline and keep waiting.
                        () = activity_notify.notified() => {
                            idle_deadline = tokio::time::Instant::now() + idle_duration;
                        }
                    }
                };

                // Clean up stream collector regardless of how we exited.
                {
                    let mut collectors = stream_collectors.lock().await;
                    collectors.remove(&final_id);
                }
                let _ = result_tx.send(result);
            });
        }

        self.sender
            .send(req_str)
            .await
            .context("Failed to send streaming request to MCP transport")?;

        Ok((chunk_rx, result_rx))
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
