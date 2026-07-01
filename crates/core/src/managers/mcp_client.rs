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
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
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

/// Kernel-internal pseudo-notification method used to carry a child-process
/// stderr line through the existing notification channel. The notification
/// consumer converts it to a `ClotoEventData::McpServerLog { source: Stderr }`
/// (it is not a real wire method — it lives in the `notifications/cloto.*`
/// kernel namespace). See `docs/MCP_SERVER_LOGS_DESIGN.md` §6.
pub const CLOTO_STDERR_LOG_METHOD: &str = "notifications/cloto.stderr";

/// Bounded buffer for the per-server stderr→log forwarding channel. Logs are
/// best-effort (dropped on overflow — tracing still has them), so a modest
/// buffer is enough to smooth bursts without holding memory.
const MCP_STDERR_CHANNEL_BUFFER: usize = 128;

/// Extract the log line carried by a [`CLOTO_STDERR_LOG_METHOD`] pseudo-notification
/// (`params.line`). Empty string if the shape is unexpected. The notification
/// consumer uses this to build a `McpServerLog{source:Stderr}`.
pub fn stderr_line_from_params(params: Option<&Value>) -> String {
    params
        .and_then(|p| p.get("line"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Default minimum severity sent via `logging/setLevel` when a server advertises
/// the MCP `logging` capability but the kernel config supplies no override.
/// See `docs/MCP_SERVER_LOGS_DESIGN.md` §7.
pub const DEFAULT_MCP_LOG_LEVEL: &str = "info";

/// Extract `(level, logger, message)` from an MCP `notifications/message` params
/// object (`{ level, logger?, data }`). The notification consumer uses this to
/// build a `McpServerLog{source:McpLogging}`. An unknown/absent `level`
/// deserializes to `None`; non-string `data` is rendered as compact JSON.
/// See `docs/MCP_SERVER_LOGS_DESIGN.md` §7.
pub fn mcp_log_from_params(
    params: Option<&Value>,
) -> (Option<cloto_shared::McpLogLevel>, Option<String>, String) {
    let Some(p) = params else {
        return (None, None, String::new());
    };
    let level = p.get("level").and_then(Value::as_str).and_then(|s| {
        serde_json::from_value::<cloto_shared::McpLogLevel>(serde_json::json!(s)).ok()
    });
    let logger = p.get("logger").and_then(Value::as_str).map(str::to_string);
    let message = match p.get("data") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    (level, logger, message)
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
    /// bug-411: true while the response loop is running. The loop only exits
    /// when the transport read side reaches EOF (the child process is gone), so
    /// this flips to false the moment the server dies — even while idle, before
    /// any tool call observes the failure. `is_alive()` reads this directly so
    /// the health monitor can restart a dead-but-idle server instead of waiting
    /// for the next request to hang/fail. `sender.is_closed()` alone misses this:
    /// it only tracks the writer task's receiver, not the read path.
    alive: Arc<AtomicBool>,
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

    /// bug-357: upper bound for enqueueing a request into the transport channel.
    /// The send should be near-instant; a stall means the writer/HTTP task is
    /// wedged (bug-355/bug-356), so we fail fast instead of blocking the caller
    /// — the response-side timeout only protects the receive path.
    const SEND_TIMEOUT_SECS: u64 = 10;

    /// Send a payload into the transport request channel with a timeout. Without
    /// this, a wedged transport (full child stdin pipe / stalled HTTP loop)
    /// could block the caller indefinitely, since the bounded request channel
    /// applies back-pressure and the response timeout does not cover the send
    /// path (bug-357).
    async fn send_with_timeout(&self, payload: String, what: &str) -> Result<()> {
        match tokio::time::timeout(
            std::time::Duration::from_secs(Self::SEND_TIMEOUT_SECS),
            self.sender.send(payload),
        )
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                Err(anyhow::Error::new(e)
                    .context(format!("Failed to send {what} to MCP transport")))
            }
            Err(_) => Err(anyhow::anyhow!(
                "Timed out sending {what} to MCP transport ({}s)",
                Self::SEND_TIMEOUT_SECS
            )),
        }
    }

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
        default_log_level: &str,
    ) -> Result<(Self, Option<MgpServerCapabilities>)> {
        // stderr → dashboard: the transport forwards raw stderr lines here; the
        // task below tags them with server_id and pushes them through the same
        // notification channel as a kernel-internal pseudo-notification, which
        // the consumer turns into a McpServerLog{source:Stderr} event.
        // docs/MCP_SERVER_LOGS_DESIGN.md §6.
        let (stderr_tx, mut stderr_rx) = mpsc::channel::<String>(MCP_STDERR_CHANNEL_BUFFER);
        {
            let notif_tx = notification_tx.clone();
            let sid = server_id.to_string();
            tokio::spawn(async move {
                while let Some(line) = stderr_rx.recv().await {
                    if notif_tx
                        .try_send(McpNotification {
                            server_id: sid.clone(),
                            method: CLOTO_STDERR_LOG_METHOD.to_string(),
                            params: Some(serde_json::json!({ "line": line })),
                        })
                        .is_err()
                    {
                        debug!("stderr log channel full/closed, dropping line");
                    }
                }
            });
        }

        let stdio = StdioTransport::start(
            command,
            args,
            env,
            isolation,
            llm_proxy_port,
            sensitive_env_keys,
            Some(stderr_tx),
        )
        .await?;
        let sender = stdio.sender();
        let transport = McpTransport::Stdio(Box::new(stdio));
        let mut client = Self {
            transport: Arc::new(Mutex::new(transport)),
            sender,
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicI64::new(1)),
            alive: Arc::new(AtomicBool::new(true)),
            response_task: None,
            notification_tx,
            request_timeout_secs,
            stream_idle_timeout_secs,
            stream_collectors: Arc::new(Mutex::new(HashMap::new())),
        };

        client.start_response_loop(server_id);
        let mgp_caps = client.initialize(default_log_level).await?;

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
        default_log_level: &str,
    ) -> Result<(Self, Option<MgpServerCapabilities>)> {
        let http = HttpTransport::start(url, auth_token).await?;
        let sender = http.sender();
        let transport = McpTransport::Http(Box::new(http));
        let mut client = Self {
            transport: Arc::new(Mutex::new(transport)),
            sender,
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicI64::new(1)),
            alive: Arc::new(AtomicBool::new(true)),
            response_task: None,
            notification_tx,
            request_timeout_secs,
            stream_idle_timeout_secs,
            stream_collectors: Arc::new(Mutex::new(HashMap::new())),
        };

        client.start_response_loop(server_id);
        let mgp_caps = client.initialize(default_log_level).await?;

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
        let alive = self.alive.clone();

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
                                // char-safe truncation: byte-slicing `&line[..200]`
                                // panics when a multibyte UTF-8 codepoint straddles
                                // byte 200 (e.g. a long non-JSON diagnostic line),
                                // which would abort this response loop and wedge the
                                // server.
                                "Received unparseable message: {}",
                                line.chars().take(200).collect::<String>()
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

            // bug-411: the loop only exits on transport EOF (child process gone).
            // Mark the client dead so is_alive()/the health monitor can restart it
            // immediately, rather than waiting for the next tool call to fail.
            alive.store(false, Ordering::SeqCst);
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

        if let Err(e) = self.send_with_timeout(req_str, "request").await {
            self.pending_requests.lock().await.remove(&id);
            return Err(e);
        }

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

    async fn initialize(&self, default_log_level: &str) -> Result<Option<MgpServerCapabilities>> {
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

        // MCP logging capability (design §7): a server that advertises
        // `capabilities.logging` only emits `notifications/message` after the
        // client sets a minimum severity. Send `logging/setLevel` once, right
        // after initialize, with the config-driven default. Best-effort — a
        // failure here must never abort the connection.
        let advertises_logging = result
            .get("capabilities")
            .and_then(|caps| caps.get("logging"))
            .is_some();
        if advertises_logging {
            let params = serde_json::json!({ "level": default_log_level });
            match self.call("logging/setLevel", Some(params)).await {
                Ok(_) => info!("logging/setLevel={} sent to MCP server", default_log_level),
                Err(e) => debug!("logging/setLevel failed (non-fatal): {}", e),
            }
        }

        Ok(mgp_server_caps)
    }

    /// Send `notifications/initialized` to the server.
    /// Split from `initialize()` to allow Permission Flow insertion between
    /// initialize response and initialized notification (MGP §3).
    pub async fn send_initialized_notification(&self) -> Result<()> {
        let notify = JsonRpcRequest::notification("notifications/initialized", None);
        let notify_str = serde_json::to_string(&notify)?;
        self.send_with_timeout(notify_str, "initialized notification")
            .await
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
            //
            // Subtle: the per-chunk idle deadline is only armed AFTER the first
            // chunk arrives. Before that, the upstream may legitimately be
            // busy with prompt processing (a 9B model digesting a multi-k
            // token system prompt can easily exceed the idle window). During
            // that phase we rely on the total cap alone. Once streaming has
            // actually started, idle silence is a real stall.
            tokio::spawn(async move {
                let total_deadline = tokio::time::Instant::now()
                    + std::time::Duration::from_secs(total_timeout_secs);
                let idle_duration = std::time::Duration::from_secs(idle_timeout_secs);
                let mut idle_deadline: Option<tokio::time::Instant> = None;
                let mut inner_rx = inner_rx;

                let result: Result<CallToolResult> = loop {
                    // Compose the idle branch dynamically — a pending future
                    // (never resolves) until the first chunk arms the deadline.
                    let idle_sleep: std::pin::Pin<
                        Box<dyn std::future::Future<Output = ()> + Send>,
                    > = match idle_deadline {
                        Some(d) => Box::pin(tokio::time::sleep_until(d)),
                        None => Box::pin(std::future::pending::<()>()),
                    };

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
                        // Idle window elapsed after streaming had started.
                        () = idle_sleep => {
                            break Err(anyhow::anyhow!(
                                "Streaming request timed out (idle {}s, no chunk received)",
                                idle_timeout_secs
                            ));
                        }
                        // Chunk delivered — arm (on first notify) or reset the idle deadline.
                        () = activity_notify.notified() => {
                            idle_deadline = Some(tokio::time::Instant::now() + idle_duration);
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

        if let Err(e) = self.send_with_timeout(req_str, "streaming request").await {
            // Dropping the pending entry closes the watchdog's inner_rx, which
            // self-cleans the stream collector registered for this id.
            self.pending_requests.lock().await.remove(&id);
            return Err(e);
        }

        Ok((chunk_rx, result_rx))
    }

    /// Send a JSON-RPC notification (fire-and-forget, no response expected).
    pub async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()> {
        let request = JsonRpcRequest::notification(method, params);
        let req_str = serde_json::to_string(&request)?;
        self.send_with_timeout(req_str, "notification").await
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
    ///
    /// Reads two lock-free signals (never contends with the response loop's
    /// transport Mutex): the bug-411 `alive` flag (cleared when the response
    /// loop exits on transport EOF — catches an idle server dying), and
    /// `sender.is_closed()` (the writer task's receiver dropped). Either being
    /// dead means the client is dead.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst) && !self.sender.is_closed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// bug-411: when the server process dies (transport EOF) while idle, the
    /// response loop exits and `is_alive()` must flip to false promptly —
    /// without waiting for a tool call to fail. A tiny mock MCP server answers
    /// the `initialize` handshake and then exits, closing its stdout (EOF). The
    /// pre-fix `is_alive()` (only `!sender.is_closed()`) stayed true here.
    #[tokio::test]
    async fn is_alive_flips_false_when_server_exits_idle() {
        // Mock MCP server: answer `initialize`, then exit (EOF on stdout).
        // readline() (not `for line in sys.stdin`) avoids stdin read-ahead
        // buffering so the response is emitted immediately.
        const MOCK: &str = "import sys, json\n\
while True:\n\
\x20   line = sys.stdin.readline()\n\
\x20   if not line:\n\
\x20       break\n\
\x20   line = line.strip()\n\
\x20   if not line:\n\
\x20       continue\n\
\x20   try:\n\
\x20       req = json.loads(line)\n\
\x20   except Exception:\n\
\x20       continue\n\
\x20   if req.get('method') == 'initialize':\n\
\x20       sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': req.get('id'), 'result': {}}) + '\\n')\n\
\x20       sys.stdout.flush()\n\
\x20       break\n";

        // Skip cleanly if python3 is unavailable (keeps minimal envs green).
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("skipping is_alive_flips_false_when_server_exits_idle: python3 not found");
            return;
        }

        let (notif_tx, _notif_rx) = mpsc::channel(8);
        let (client, _caps) = McpClient::connect(
            "mock-bug411",
            "python3",
            &["-c".to_string(), MOCK.to_string()],
            &HashMap::new(),
            notif_tx,
            5,
            5,
            None,
            0,
            &[],
            DEFAULT_MCP_LOG_LEVEL,
        )
        .await
        .expect("mock server should complete the initialize handshake");

        // The mock exits right after responding, so the transport reaches EOF
        // and the response loop sets alive=false. Poll briefly for the flip.
        let mut became_dead = false;
        for _ in 0..50 {
            if !client.is_alive() {
                became_dead = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(
            became_dead,
            "is_alive() must become false after the server process exits (EOF)"
        );
    }

    #[test]
    fn stderr_line_from_params_extracts_line() {
        assert_eq!(
            stderr_line_from_params(Some(&serde_json::json!({ "line": "boot ok" }))),
            "boot ok"
        );
        // Unexpected shapes degrade to empty, never panic.
        assert_eq!(stderr_line_from_params(None), "");
        assert_eq!(stderr_line_from_params(Some(&serde_json::json!({}))), "");
        assert_eq!(
            stderr_line_from_params(Some(&serde_json::json!({ "line": 42 }))),
            ""
        );
    }

    /// Source A (bug-422 sibling / Goal #141): a child's stderr line is
    /// forwarded — tagged with the server_id — through the notification channel
    /// as the kernel-internal CLOTO_STDERR_LOG_METHOD pseudo-notification, which
    /// the consumer turns into McpServerLog{source:Stderr}. This pins the
    /// transport→client half (server_id tagging + method + params.line).
    #[tokio::test]
    async fn stderr_lines_are_forwarded_as_pseudo_notifications() {
        // Mock MCP server: emit one stderr line, answer initialize, stay alive
        // (keep reading stdin) so the notification can be observed.
        const MOCK: &str = "import sys, json\n\
sys.stderr.write('hello from stderr\\n')\n\
sys.stderr.flush()\n\
while True:\n\
\x20   line = sys.stdin.readline()\n\
\x20   if not line:\n\
\x20       break\n\
\x20   line = line.strip()\n\
\x20   if not line:\n\
\x20       continue\n\
\x20   try:\n\
\x20       req = json.loads(line)\n\
\x20   except Exception:\n\
\x20       continue\n\
\x20   if req.get('method') == 'initialize':\n\
\x20       sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': req.get('id'), 'result': {}}) + '\\n')\n\
\x20       sys.stdout.flush()\n";

        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!(
                "skipping stderr_lines_are_forwarded_as_pseudo_notifications: python3 not found"
            );
            return;
        }

        let (notif_tx, mut notif_rx) = mpsc::channel(8);
        let (_client, _caps) = McpClient::connect(
            "mock-stderr",
            "python3",
            &["-c".to_string(), MOCK.to_string()],
            &HashMap::new(),
            notif_tx,
            5,
            5,
            None,
            0,
            &[],
            DEFAULT_MCP_LOG_LEVEL,
        )
        .await
        .expect("mock server should complete the initialize handshake");

        // Poll for the stderr bridge notification (ignore any others).
        let mut got = None;
        for _ in 0..50 {
            match tokio::time::timeout(std::time::Duration::from_millis(100), notif_rx.recv()).await
            {
                Ok(Some(n)) if n.method == CLOTO_STDERR_LOG_METHOD => {
                    got = Some(n);
                    break;
                }
                Ok(Some(_)) => continue, // some other notification, keep looking
                Ok(None) => break,       // channel closed
                Err(_) => continue,      // timeout tick
            }
        }

        let n = got.expect("a stderr line must be forwarded as a pseudo-notification");
        assert_eq!(n.server_id, "mock-stderr", "must be tagged with server_id");
        assert_eq!(
            stderr_line_from_params(n.params.as_ref()),
            "hello from stderr"
        );
    }

    /// Source B (Goal #141 backend-B): `mcp_log_from_params` extracts
    /// `(level, logger, message)` from an MCP `notifications/message` params
    /// object, tolerating missing/unknown fields and non-string `data`.
    #[test]
    fn mcp_log_from_params_extracts_fields() {
        let params = serde_json::json!({
            "level": "warning", "logger": "db", "data": "connection lost"
        });
        let (level, logger, message) = mcp_log_from_params(Some(&params));
        assert_eq!(level, Some(cloto_shared::McpLogLevel::Warning));
        assert_eq!(logger.as_deref(), Some("db"));
        assert_eq!(message, "connection lost");

        // Non-string `data` → compact JSON; missing level/logger → None.
        let structured = serde_json::json!({ "data": {"k": 1} });
        let (level2, logger2, message2) = mcp_log_from_params(Some(&structured));
        assert_eq!(level2, None);
        assert_eq!(logger2, None);
        assert_eq!(message2, "{\"k\":1}");

        // Unknown level string → None (tolerated, not an error).
        let unknown = serde_json::json!({ "level": "verbose", "data": "x" });
        assert_eq!(mcp_log_from_params(Some(&unknown)).0, None);

        // Absent params.
        assert_eq!(mcp_log_from_params(None), (None, None, String::new()));
    }

    /// Source B end-to-end (client half): a server advertising
    /// `capabilities.logging` receives `logging/setLevel` with the default
    /// `info` right after initialize, then its `notifications/message` reaches
    /// the notification channel intact (level/logger/data). The mock echoes the
    /// received level back inside the notification's `data`, so one assertion
    /// pins both the setLevel send and the message forwarding.
    #[tokio::test]
    async fn logging_capability_gets_setlevel_and_forwards_message() {
        const MOCK: &str = "import sys, json\n\
def emit(obj):\n\
\x20   sys.stdout.write(json.dumps(obj) + '\\n'); sys.stdout.flush()\n\
while True:\n\
\x20   line = sys.stdin.readline()\n\
\x20   if not line:\n\
\x20       break\n\
\x20   line = line.strip()\n\
\x20   if not line:\n\
\x20       continue\n\
\x20   try:\n\
\x20       req = json.loads(line)\n\
\x20   except Exception:\n\
\x20       continue\n\
\x20   m = req.get('method')\n\
\x20   if m == 'initialize':\n\
\x20       emit({'jsonrpc': '2.0', 'id': req.get('id'), 'result': {'capabilities': {'logging': {}}}})\n\
\x20   elif m == 'logging/setLevel':\n\
\x20       lvl = req.get('params', {}).get('level')\n\
\x20       emit({'jsonrpc': '2.0', 'id': req.get('id'), 'result': {}})\n\
\x20       emit({'jsonrpc': '2.0', 'method': 'notifications/message', 'params': {'level': 'warning', 'logger': 'test', 'data': 'setlevel=' + str(lvl)}})\n";

        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!(
                "skipping logging_capability_gets_setlevel_and_forwards_message: python3 not found"
            );
            return;
        }

        let (notif_tx, mut notif_rx) = mpsc::channel(8);
        let (_client, _caps) = McpClient::connect(
            "mock-logging",
            "python3",
            &["-c".to_string(), MOCK.to_string()],
            &HashMap::new(),
            notif_tx,
            5,
            5,
            None,
            0,
            &[],
            DEFAULT_MCP_LOG_LEVEL,
        )
        .await
        .expect("mock server should complete the initialize handshake");

        // Poll for the notifications/message triggered by our setLevel.
        let mut got = None;
        for _ in 0..50 {
            match tokio::time::timeout(std::time::Duration::from_millis(100), notif_rx.recv()).await
            {
                Ok(Some(n)) if n.method == "notifications/message" => {
                    got = Some(n);
                    break;
                }
                Ok(Some(_)) => continue,
                Ok(None) => break,
                Err(_) => continue,
            }
        }

        let n = got.expect("a logging notification must be forwarded");
        assert_eq!(n.server_id, "mock-logging", "must be tagged with server_id");
        let (level, logger, message) = mcp_log_from_params(n.params.as_ref());
        assert_eq!(level, Some(cloto_shared::McpLogLevel::Warning));
        assert_eq!(logger.as_deref(), Some("test"));
        // Proves setLevel was sent with the default `info`.
        assert_eq!(
            message, "setlevel=info",
            "kernel must send logging/setLevel with the default level"
        );
    }
}
