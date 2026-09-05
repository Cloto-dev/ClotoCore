//! JSON-RPC 2.0 client for communicating with individual MCP servers.
//!
//! Each `McpClient` manages a single MCP server connection over stdio or
//! Streamable HTTP transport, handling connect-time negotiation, tool calls,
//! notifications, and shutdown.
//!
//! The client is **dual-era**: at connect time it decides whether the server
//! speaks the handshake era (`initialize` / `initialized`, session state) or the
//! MCP 2026-07-28 stateless core (no handshake, per-request `params._meta`).
//! See [`McpClient::negotiate`] for the decision policy. The legacy path is
//! byte-identical to the pre-dual-era client — nothing is stamped, skipped or
//! reordered for a server that turns out to be legacy.

use super::mcp_mgp::{
    MgpClientCapabilities, MgpServerCapabilities, CLIENT_EXTENSIONS, MGP_VERSION,
};
use super::mcp_protocol::{
    CallToolParams, CallToolResult, ClientCapabilities, ClientInfo, ClotoHandshakeParams,
    ClotoHandshakeResult, DiscoverResult, EraHandle, EraPreference, InitializeParams,
    JsonRpcRequest, ListToolsResult, ProtocolEra, RpcError, DISCOVER_METHOD,
    DISCOVER_PROBE_READINESS_CAP_SECS, DISCOVER_PROBE_TIMEOUT_SECS, LEGACY_PROTOCOL_VERSION,
    META_CLIENT_CAPABILITIES, META_CLIENT_INFO, META_LOG_LEVEL, META_MGP_GRANTS,
    META_PROTOCOL_VERSION, MODERN_PROTOCOL_VERSION, RESULT_TYPE_INPUT_REQUIRED,
    UNSUPPORTED_PROTOCOL_VERSION,
};
use super::mcp_transport::{ChildVoice, HttpTransport, McpTransport, StdioTransport};
use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, Mutex, Notify};
use tracing::{debug, error, info, warn};

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

/// `clientInfo.name` the kernel identifies itself with, in both eras.
pub const KERNEL_CLIENT_NAME: &str = "CLOTO-KERNEL";

/// Timeout error for a request whose response never arrived within the caller's
/// window. Typed so era negotiation can tell "the server is silent" (→ fall back
/// to the handshake) from a transport failure (→ propagate and let the caller
/// retry the whole connection). `Display` keeps the exact pre-existing text.
#[derive(Debug, Clone, Copy)]
pub struct RequestTimeout;

impl std::fmt::Display for RequestTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MCP Request timed out")
    }
}

impl std::error::Error for RequestTimeout {}

/// Outcome of connect-time protocol negotiation, returned alongside the client.
#[derive(Debug, Clone)]
pub struct NegotiatedProtocol {
    /// Era this connection settled on.
    pub era: ProtocolEra,
    /// MGP server capabilities — `initialize.capabilities.mgp` (legacy) or
    /// `DiscoverResult.capabilities.extensions["dev.cloto/mgp"]` (modern).
    pub mgp: Option<MgpServerCapabilities>,
    /// The server's own operating guidance, from whichever channel its era
    /// uses — `initialize.instructions` (legacy) or `DiscoverResult.instructions`
    /// (modern). Empty and whitespace-only are normalised to `None` at capture,
    /// so a consumer never has to tell "absent" from "blank". Stored on the
    /// handle for a future consumer; nothing acts on it yet.
    pub instructions: Option<String>,
}

/// One `server/discover` probe's outcome, mapped onto the era-decision policy.
enum ProbeOutcome {
    /// A parseable modern reply.
    Discovered(DiscoverResult),
    /// `-32022` — the server rejected the version we asked for and told us what
    /// it does support.
    VersionRejected(Vec<String>),
    /// Any other RPC-level answer (`-32601`, an unparseable result, silence
    /// until the probe timeout): this is a handshake-era server.
    FallBackToLegacy(String),
    /// The transport itself failed. Not an era signal — propagate so the
    /// caller's connect retry can act on it.
    Transport(anyhow::Error),
}

impl ProbeOutcome {
    /// One-line reason for logs.
    fn describe(&self) -> String {
        match self {
            Self::Discovered(_) => "discovered".to_string(),
            Self::VersionRejected(supported) => {
                format!("version rejected (server supports {supported:?})")
            }
            Self::FallBackToLegacy(reason) => reason.clone(),
            Self::Transport(e) => format!("transport error: {e}"),
        }
    }
}

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
    /// OS pid (== pgid, the child is its own group leader) of the stdio child,
    /// captured at spawn. Lock-free so the forced drain sweep (bug-426) can
    /// signal the group without touching the transport Mutex. None for HTTP
    /// transports.
    child_pid: Option<u32>,
    /// Negotiated era, shared with the HTTP transport (which needs it for the
    /// era headers and to stop sending `Mcp-Session-Id`). Unset until
    /// [`McpClient::negotiate`] settles it.
    era: EraHandle,
    /// Modern-era `_meta` template stamped onto every outgoing request. Written
    /// once by negotiation; while unset (legacy era, or negotiation in flight)
    /// requests go out untouched.
    modern_meta: Arc<OnceLock<Map<String, Value>>>,
    /// Approved MGP permission grants to attach to `tools/call` `_meta` in the
    /// modern era (mgp-spec 0.8.0-draft). Fed by the kernel's Permission Flow
    /// via [`McpClient::set_mgp_grants`]; ignored in the legacy era, which
    /// delivers grants through the `mgp/permission/grant` RPC instead.
    mgp_grants: Arc<RwLock<Option<Value>>>,
    /// "The spawned child has written something." `None` for HTTP transports,
    /// which have no startup of ours to wait through. Read by the era probe to
    /// tell startup silence apart from an answerless server; see
    /// [`McpClient::probe_discover`].
    child_voice: Option<ChildVoice>,
}

/// Kernel `clientInfo` for both eras.
fn client_info() -> ClientInfo {
    ClientInfo {
        name: KERNEL_CLIENT_NAME.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Kernel `clientCapabilities` for the modern era's per-request `_meta`.
/// Carries the MGP declaration that the legacy era piggybacks on `initialize`.
fn modern_client_capabilities() -> Value {
    super::mcp_mgp::client_capabilities_extension()
}

/// Apply the modern-era per-request context to outgoing `params`.
///
/// Pure so the merge rules can be tested without a server:
/// - absent `params` becomes `{"_meta": {…}}`;
/// - `protocolVersion` / `clientInfo` / `clientCapabilities` are **overwritten**
///   (kernel-owned — a caller must not be able to misdeclare the connection);
/// - `logLevel` is **setdefault** (a caller that already chose a level keeps it);
/// - `grants` (when given, i.e. on `tools/call`) is overwritten;
/// - non-object `params` (JSON-RPC permits an array) cannot carry `_meta` and is
///   returned untouched rather than silently reshaped.
pub(super) fn stamp_modern_meta(
    params: Option<Value>,
    template: &Map<String, Value>,
    grants: Option<&Value>,
) -> Value {
    let mut root = match params {
        None => Value::Object(Map::new()),
        Some(Value::Object(obj)) => Value::Object(obj),
        Some(other) => {
            debug!("Non-object MCP params cannot carry _meta — sending as-is");
            return other;
        }
    };

    if let Some(obj) = root.as_object_mut() {
        let entry = obj
            .entry("_meta".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        if let Some(meta) = entry.as_object_mut() {
            for (key, value) in template {
                if key == META_LOG_LEVEL {
                    meta.entry(key.clone()).or_insert_with(|| value.clone());
                } else {
                    meta.insert(key.clone(), value.clone());
                }
            }
            if let Some(grants) = grants {
                meta.insert(META_MGP_GRANTS.to_string(), grants.clone());
            }
        }
    }

    root
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

    /// OS pid (== pgid) of the stdio child captured at spawn; None for HTTP
    /// transports. Lock-free — safe to read while a drain holds the transport.
    #[must_use]
    pub fn child_pid(&self) -> Option<u32> {
        self.child_pid
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
        llm_proxy_token: &str,
        sensitive_env_keys: &[String],
        default_log_level: &str,
        protocol_era: Option<&str>,
    ) -> Result<(Self, NegotiatedProtocol)> {
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
            llm_proxy_token,
            sensitive_env_keys,
            Some(stderr_tx),
        )
        .await?;
        let sender = stdio.sender();
        // Captured lock-free so the forced drain sweep (bug-426) can signal the
        // process group without contending on the transport Mutex (the response
        // loop holds it across recv()).
        let child_pid = stdio.child_id();
        // Same reason: the probe reads this while the response loop owns the
        // transport across its recv().
        let child_voice = Some(stdio.voice());
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
            child_pid,
            era: EraHandle::new(),
            modern_meta: Arc::new(OnceLock::new()),
            mgp_grants: Arc::new(RwLock::new(None)),
            child_voice,
        };

        client.start_response_loop(server_id);
        let negotiated = client
            .negotiate(default_log_level, EraPreference::from_config(protocol_era))
            .await?;

        Ok((client, negotiated))
    }

    /// Connect to a remote MCP server via Streamable HTTP transport.
    #[allow(clippy::too_many_arguments)]
    pub async fn connect_http(
        server_id: &str,
        url: &str,
        auth_token: Option<&str>,
        notification_tx: mpsc::Sender<McpNotification>,
        request_timeout_secs: u64,
        stream_idle_timeout_secs: u64,
        default_log_level: &str,
        protocol_era: Option<&str>,
    ) -> Result<(Self, NegotiatedProtocol)> {
        // The transport is started before the era is known, so it gets a handle
        // to the shared era state and reads it per request (era headers,
        // Mcp-Session-Id suppression).
        let era = EraHandle::new();
        let http = HttpTransport::start(url, auth_token, era.clone()).await?;
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
            child_pid: None,
            era,
            modern_meta: Arc::new(OnceLock::new()),
            mgp_grants: Arc::new(RwLock::new(None)),
            // No child, no startup of ours to wait through: a remote peer was
            // already running when we first addressed it.
            child_voice: None,
        };

        client.start_response_loop(server_id);
        let negotiated = client
            .negotiate(default_log_level, EraPreference::from_config(protocol_era))
            .await?;

        Ok((client, negotiated))
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
                                // bug-447: widen correlation to string-typed ids
                                // that parse as the same integer, and log any id
                                // shape we still can't correlate — otherwise an
                                // id-type mismatch is indistinguishable from a
                                // hung server (bare timeout, no diagnostic).
                                let id_opt = id_val.as_i64().or_else(|| {
                                    id_val.as_str().and_then(|s| s.parse::<i64>().ok())
                                });
                                if id_opt.is_none() {
                                    warn!(
                                        id = %id_val,
                                        "Dropping JSON-RPC response with non-integer id — \
                                         cannot correlate to a pending request"
                                    );
                                }
                                if let Some(id) = id_opt {
                                    let mut map = pending.lock().await;
                                    if let Some(tx) = map.remove(&id) {
                                        if let Some(error) = response.error {
                                            // Typed (not `anyhow!`-formatted) so
                                            // era negotiation can read `code` /
                                            // `data.supported` off a -32022.
                                            // RpcError's Display renders the
                                            // identical "RPC Error {code}: {msg}".
                                            if tx
                                                .send(Err(anyhow::Error::new(RpcError {
                                                    code: error.code,
                                                    message: error.message,
                                                    data: error.data,
                                                })))
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
        self.call_with_timeout(method, params, self.request_timeout_secs)
            .await
    }

    /// `call` with an explicit response deadline. Used by the `server/discover`
    /// probe, which must not wait out a long `request_timeout_secs` before
    /// falling back to the handshake.
    async fn call_with_timeout(
        &self,
        method: &str,
        params: Option<Value>,
        timeout_secs: u64,
    ) -> Result<Value> {
        let (id, rx) = self.dispatch(method, params).await?;

        if let Ok(res) = tokio::time::timeout(Duration::from_secs(timeout_secs), rx).await {
            self.settle(method, res)
        } else {
            self.abandon(id).await;
            Err(anyhow::Error::new(RequestTimeout))
        }
    }

    /// Register a pending request and put it on the wire. Returns its id and
    /// the receiver the response loop will resolve — the wait is the caller's,
    /// and so is removing the entry if it gives up (see [`Self::abandon`]).
    async fn dispatch(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(i64, oneshot::Receiver<Result<Value>>)> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let params = self.prepare_params(method, params);
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
        Ok((id, rx))
    }

    /// Drop a pending request we are no longer waiting for. A late answer then
    /// finds no entry and the response loop discards it.
    async fn abandon(&self, id: i64) {
        self.pending_requests.lock().await.remove(&id);
    }

    /// Turn a resolved oneshot into the call's result.
    fn settle(
        &self,
        method: &str,
        res: std::result::Result<Result<Value>, oneshot::error::RecvError>,
    ) -> Result<Value> {
        let value = res.context("Response channel closed")??;
        self.check_result_type(method, value)
    }

    /// Send one request and wait for it the way the era probe must: a silence
    /// budget that starts when the child is first heard from, and a longer
    /// absolute cap for as long as it has not been heard from at all.
    ///
    /// The distinction is the whole point. A running server that does not
    /// answer `server/discover` is telling us something — it has no modern era
    /// — and `window_secs` is how long we listen for that. A child still
    /// resolving a cold venv is telling us nothing, because it has not yet run
    /// a line of its program, and reading its silence as an era is how a
    /// dual-era server ends up negotiated as legacy and a modern-only one
    /// fails the connect outright.
    ///
    /// Falls back to a plain [`Self::call_with_timeout`] when there is no child
    /// of ours to wait for (HTTP transports), which is byte-identical to the
    /// behaviour before this existed.
    async fn call_with_readiness_window(
        &self,
        method: &str,
        params: Option<Value>,
        window_secs: u64,
        cap_secs: u64,
    ) -> Result<Value> {
        let Some(voice) = self.child_voice.clone() else {
            return self.call_with_timeout(method, params, window_secs).await;
        };

        let window = Duration::from_secs(window_secs);
        // A cap below the window would make the readiness wait the shorter of
        // the two and silently tighten the ordinary path.
        let cap = Duration::from_secs(cap_secs).max(window);
        let started = Instant::now();
        let (id, mut rx) = self.dispatch(method, params).await?;

        // The budget runs from the child's first byte or from this request,
        // whichever is later, and never past the cap. A re-probe on an
        // already-talking child therefore gets the plain window, unchanged.
        let deadline = |spoke: Option<Instant>| -> Instant {
            let hard = started + cap;
            spoke.map_or(hard, |t| (t.max(started) + window).min(hard))
        };

        if voice.spoken_at().is_none() {
            tokio::select! {
                res = &mut rx => return self.settle(method, res),
                () = voice.spoken() => {}
                () = tokio::time::sleep_until(deadline(None).into()) => {
                    self.abandon(id).await;
                    warn!(
                        method = %method,
                        cap_secs = cap.as_secs(),
                        "MCP child wrote nothing at all before the readiness cap — treating \
                         it as unreachable rather than as an answer"
                    );
                    return Err(anyhow::Error::new(RequestTimeout));
                }
            }
            debug!(
                method = %method,
                startup = ?started.elapsed(),
                "MCP child spoke for the first time — the silence budget starts here"
            );
        }

        if let Ok(res) = tokio::time::timeout_at(deadline(voice.spoken_at()).into(), &mut rx).await
        {
            self.settle(method, res)
        } else {
            self.abandon(id).await;
            Err(anyhow::Error::new(RequestTimeout))
        }
    }

    /// Modern-era `_meta` / grant stamping for an outgoing message. A no-op
    /// until negotiation settles on the modern era, so legacy traffic — and the
    /// probe itself, which builds its own `_meta` — is untouched.
    fn prepare_params(&self, method: &str, params: Option<Value>) -> Option<Value> {
        let Some(template) = self.modern_meta.get() else {
            return params;
        };
        let grants = if method == "tools/call" {
            self.mgp_grants
                .read()
                .ok()
                .and_then(|g| g.as_ref().cloned())
        } else {
            None
        };
        Some(stamp_modern_meta(params, template, grants.as_ref()))
    }

    /// Reject a modern-era `resultType: "input_required"` (MRTR) result.
    ///
    /// A multi-round tool interaction asks the *client* to gather more input and
    /// call again; the kernel host has no flow for that, and treating the
    /// half-finished result as final would silently drop whatever the server was
    /// asking for. Surface it as an explicit error naming the method instead.
    fn check_result_type(&self, method: &str, value: Value) -> Result<Value> {
        if self.era.is_modern()
            && value.get("resultType").and_then(Value::as_str) == Some(RESULT_TYPE_INPUT_REQUIRED)
        {
            return Err(anyhow::anyhow!(
                "MCP server returned MRTR input_required for '{}' — multi-round tool \
                 interaction is not supported by the kernel host",
                method
            ));
        }
        Ok(value)
    }

    /// Attach the approved MGP permission grants carried on modern-era
    /// `tools/call` requests (`_meta["dev.cloto/mgp/grants"]`). Called by the
    /// kernel once its Permission Flow has approved everything the server
    /// declared. Storing them in the legacy era is harmless — nothing stamps.
    pub fn set_mgp_grants(&self, grants: Value) {
        if let Ok(mut guard) = self.mgp_grants.write() {
            *guard = Some(grants);
        }
    }

    /// Era settled by [`Self::negotiate`], or `None` before it ran.
    #[must_use]
    pub fn protocol_era(&self) -> Option<ProtocolEra> {
        self.era.era()
    }

    /// Decide which MCP era this server speaks and complete the matching
    /// connect-time exchange. Replaces the unconditional `initialize()` of the
    /// legacy-only client.
    ///
    /// Policy (mirrors the reference SDK's denylist probe, `mcp` 2.0.0
    /// `mcp/client/_probe.py`):
    /// 1. probe `server/discover` once at the newest modern version;
    /// 2. `-32022` whose `data.supported` shares a modern version → re-probe
    ///    once at the highest mutual one;
    /// 3. `-32022` offering no handshake version at all → hard failure (a
    ///    genuinely incompatible modern-only server);
    /// 4. any other RPC error (`-32601`, silence until the probe timeout, …) →
    ///    handshake fallback;
    /// 5. transport / process failures propagate — era detection must not
    ///    swallow them, the caller's connect retry owns them;
    /// 6. a discover reply we cannot parse → handshake fallback;
    /// 7. a discover reply advertising no modern `supportedVersions` → handshake
    ///    fallback (some SDKs answer `server/discover` in the handshake era);
    /// 8. if `initialize` *itself* answers `-32022`, the probe timed out on our
    ///    side while the server locked modern → one corrective re-probe.
    async fn negotiate(
        &self,
        default_log_level: &str,
        preference: EraPreference,
    ) -> Result<NegotiatedProtocol> {
        if preference == EraPreference::LegacyOnly {
            debug!("protocol_era=legacy — skipping the server/discover probe");
            return self.negotiate_legacy(default_log_level, false).await;
        }

        match self.probe_discover(MODERN_PROTOCOL_VERSION).await {
            ProbeOutcome::Discovered(discovered) => {
                if let Some(version) = discovered.mutual_modern_version() {
                    return Ok(self.settle_modern(version, discovered, default_log_level));
                }
                // (7) answered the probe but speaks no modern version.
                debug!(
                    supported = ?discovered.supported_versions,
                    "server/discover advertised no modern version — using the initialize handshake"
                );
                self.negotiate_legacy(default_log_level, true).await
            }
            ProbeOutcome::VersionRejected(supported) => {
                if let Some(version) =
                    super::mcp_protocol::highest_mutual_modern_version(&supported)
                {
                    // (2) one downgrade re-probe at the highest mutual version.
                    match self.probe_discover(version).await {
                        ProbeOutcome::Discovered(discovered) => {
                            Ok(self.settle_modern(version, discovered, default_log_level))
                        }
                        other => {
                            warn!(
                                version = %version,
                                reason = %other.describe(),
                                "server/discover re-probe at a mutually supported version failed"
                            );
                            if let ProbeOutcome::Transport(e) = other {
                                return Err(e);
                            }
                            self.negotiate_legacy(default_log_level, true).await
                        }
                    }
                } else if super::mcp_protocol::offers_handshake_version(&supported) {
                    // (4)-shaped: no modern overlap, but reachable via handshake.
                    debug!(
                        supported = ?supported,
                        "Server rejected the modern protocol version — using the initialize handshake"
                    );
                    self.negotiate_legacy(default_log_level, true).await
                } else {
                    // (3) genuinely incompatible: neither a modern version we
                    // know nor any handshake era.
                    Err(anyhow::anyhow!(
                        "MCP protocol version mismatch: server supports {:?}, this kernel speaks \
                         modern {:?} or handshake {:?}",
                        supported,
                        super::mcp_protocol::MODERN_PROTOCOL_VERSIONS,
                        super::mcp_protocol::HANDSHAKE_PROTOCOL_VERSIONS
                    ))
                }
            }
            // (4) / (6)
            ProbeOutcome::FallBackToLegacy(reason) => {
                debug!(
                    reason = %reason,
                    "server/discover unavailable — using the initialize handshake"
                );
                self.negotiate_legacy(default_log_level, true).await
            }
            // (5) never an era signal.
            ProbeOutcome::Transport(e) => Err(e),
        }
    }

    /// Send one `server/discover` probe at `version` and classify the answer.
    async fn probe_discover(&self, version: &str) -> ProbeOutcome {
        // The HTTP transport tags the probe with `mcp-protocol-version` from
        // here — the era is not settled yet, so it has no other source.
        self.era.set_wire_version(version);

        let params = serde_json::json!({
            "_meta": {
                META_PROTOCOL_VERSION: version,
                META_CLIENT_INFO: serde_json::to_value(client_info()).unwrap_or(Value::Null),
                META_CLIENT_CAPABILITIES: modern_client_capabilities(),
            }
        });
        // Bounded independently of request_timeout_secs: a silent server must
        // cost one short probe, not a full request window (reference SDK: 10s).
        // The bound is a silence budget, so it runs from the moment the child
        // proves it is running; until then the longer readiness cap applies and
        // startup is not mistaken for an era signal.
        let window_secs = self.request_timeout_secs.min(DISCOVER_PROBE_TIMEOUT_SECS);
        let cap_secs = self
            .request_timeout_secs
            .min(DISCOVER_PROBE_READINESS_CAP_SECS);

        match self
            .call_with_readiness_window(DISCOVER_METHOD, Some(params), window_secs, cap_secs)
            .await
        {
            Ok(value) => match serde_json::from_value::<DiscoverResult>(value) {
                Ok(discovered) => ProbeOutcome::Discovered(discovered),
                Err(e) => {
                    ProbeOutcome::FallBackToLegacy(format!("unparseable discover result: {e}"))
                }
            },
            Err(e) => Self::classify_probe_error(e),
        }
    }

    /// Map a failed probe onto the era-decision policy.
    ///
    /// Note: the HTTP transport reports its own failures as synthetic `-32000`
    /// JSON-RPC errors, so they land in the `FallBackToLegacy` bucket. The
    /// subsequent `initialize` then hits the same transport failure and that
    /// error is what propagates — one extra request, no misclassification.
    fn classify_probe_error(err: anyhow::Error) -> ProbeOutcome {
        if let Some(rpc) = err.downcast_ref::<RpcError>() {
            if rpc.code == UNSUPPORTED_PROTOCOL_VERSION {
                return ProbeOutcome::VersionRejected(rpc.supported_versions());
            }
            return ProbeOutcome::FallBackToLegacy(format!("{rpc}"));
        }
        if err.downcast_ref::<RequestTimeout>().is_some() {
            return ProbeOutcome::FallBackToLegacy("probe timed out".to_string());
        }
        ProbeOutcome::Transport(err)
    }

    /// Lock the connection into the modern era: build the `_meta` template every
    /// later request is stamped with, and read the MGP advertisement out of the
    /// discover capabilities. No `initialize`, no `initialized`, and no
    /// `logging/setLevel` (a method the modern era removed — the per-request
    /// `logLevel` `_meta` replaces it).
    fn settle_modern(
        &self,
        version: &'static str,
        discovered: DiscoverResult,
        default_log_level: &str,
    ) -> NegotiatedProtocol {
        let mut template = Map::new();
        template.insert(META_PROTOCOL_VERSION.to_string(), Value::from(version));
        template.insert(
            META_CLIENT_INFO.to_string(),
            serde_json::to_value(client_info()).unwrap_or(Value::Null),
        );
        template.insert(
            META_CLIENT_CAPABILITIES.to_string(),
            modern_client_capabilities(),
        );
        template.insert(
            META_LOG_LEVEL.to_string(),
            Value::from(default_log_level.to_string()),
        );
        // Template before era: a concurrent sender must never see "modern" with
        // no stamp available.
        let _ = self.modern_meta.set(template);
        self.era.set_modern(version);

        let mgp = super::mcp_mgp::server_caps_from_discover(discovered.capabilities.as_ref());
        info!(
            protocol_version = %version,
            server = %discovered.server_info_display().unwrap_or_else(|| "(no serverInfo)".to_string()),
            ttl_ms = ?discovered.ttl_ms,
            mgp = mgp.is_some(),
            "MCP modern era negotiated via server/discover"
        );

        NegotiatedProtocol {
            era: ProtocolEra::Modern,
            mgp,
            instructions: discovered.instructions,
        }
    }

    /// Complete the handshake era. `probed` records whether a `server/discover`
    /// probe preceded this, which enables the corrective re-probe of policy (8):
    /// a probe that timed out on our side may still have locked the server into
    /// the modern era, and it then answers `initialize` with `-32022`.
    async fn negotiate_legacy(
        &self,
        default_log_level: &str,
        probed: bool,
    ) -> Result<NegotiatedProtocol> {
        match self.initialize(default_log_level).await {
            Ok((mgp, instructions)) => {
                self.era.set_legacy();
                Ok(NegotiatedProtocol {
                    era: ProtocolEra::Legacy,
                    mgp,
                    instructions,
                })
            }
            Err(e) => {
                if probed {
                    if let Some(version) = e
                        .downcast_ref::<RpcError>()
                        .filter(|rpc| rpc.code == UNSUPPORTED_PROTOCOL_VERSION)
                        .and_then(|rpc| {
                            super::mcp_protocol::highest_mutual_modern_version(
                                &rpc.supported_versions(),
                            )
                        })
                    {
                        warn!(
                            version = %version,
                            "initialize was rejected as an unsupported protocol version — \
                             re-probing server/discover (the first probe likely timed out \
                             locally after the server had already locked modern)"
                        );
                        if let ProbeOutcome::Discovered(discovered) =
                            self.probe_discover(version).await
                        {
                            return Ok(self.settle_modern(version, discovered, default_log_level));
                        }
                    }
                }
                Err(e)
            }
        }
    }

    /// Run the legacy handshake, returning what it advertised: the MGP server
    /// capabilities and the server's `instructions`.
    ///
    /// `instructions` is the legacy era's carrier for the same operating
    /// guidance the modern era puts on `DiscoverResult`. Reading it here is what
    /// keeps [`NegotiatedProtocol`] era-agnostic — a legacy server that ships
    /// instructions must not lose them for having answered on the older channel.
    async fn initialize(
        &self,
        default_log_level: &str,
    ) -> Result<(Option<MgpServerCapabilities>, Option<String>)> {
        let params = InitializeParams {
            protocol_version: LEGACY_PROTOCOL_VERSION.to_string(),
            capabilities: ClientCapabilities {
                mgp: Some(MgpClientCapabilities {
                    version: MGP_VERSION.to_string(),
                    extensions: CLIENT_EXTENSIONS.iter().map(|s| (*s).to_string()).collect(),
                }),
            },
            client_info: client_info(),
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

        // Free-form operating guidance the server wants its callers to have.
        // Whitespace-only is nothing to say: normalising it to None here means
        // the injection side never has to distinguish "absent" from "blank".
        let instructions = result
            .get("instructions")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

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

        Ok((mgp_server_caps, instructions))
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

        // Same modern-era `_meta` (+ MGP grants) as the non-streaming path — a
        // streaming call is still a `tools/call`. No-op in the legacy era.
        let params_value = self.prepare_params("tools/call", Some(params_value));

        let request = JsonRpcRequest::new(id, "tools/call", params_value);
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
        // Modern-era MRTR gate for the final result (the non-streaming path gets
        // it inside `call`). Checked in the watchdog because that is where the
        // raw result Value is available.
        let era = self.era.clone();
        {
            let mut map = self.pending_requests.lock().await;
            // bug-448: enforce the same in-flight bound as call() — this path
            // shares pending_requests but previously inserted unconditionally,
            // bypassing MAX_PENDING_REQUESTS (and growing a watchdog task +
            // 256-slot channel per accepted call).
            if map.len() >= Self::MAX_PENDING_REQUESTS {
                drop(map); // avoid holding two locks at once
                self.stream_collectors.lock().await.remove(&id);
                return Err(anyhow::anyhow!(
                    "MCP pending request limit reached ({})",
                    Self::MAX_PENDING_REQUESTS
                ));
            }
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
                            Ok(Ok(val)) => {
                                if era.is_modern()
                                    && val.get("resultType").and_then(Value::as_str)
                                        == Some(RESULT_TYPE_INPUT_REQUIRED)
                                {
                                    break Err(anyhow::anyhow!(
                                        "MCP server returned MRTR input_required for \
                                         'tools/call' (streaming) — multi-round tool \
                                         interaction is not supported by the kernel host"
                                    ));
                                }
                                break serde_json::from_value::<CallToolResult>(val)
                                    .map_err(|e| anyhow::anyhow!("Failed to parse streaming result: {}", e));
                            }
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
    ///
    /// Modern-era notifications carry the same `_meta` context as requests: in a
    /// stateless protocol there is no session to infer it from, and receivers
    /// must ignore unknown `_meta` keys. Legacy notifications are unchanged.
    pub async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()> {
        let params = self.prepare_params(method, params);
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
    use super::super::mcp_test_support::{
        connect_mock, connect_slow_mock, python3_available, MOCK_READY_SENTINEL,
    };

    /// How long [`a_child_that_has_not_started_yet_is_not_read_as_an_era`] keeps
    /// its mock silent.
    ///
    /// Past [`DISCOVER_PROBE_TIMEOUT_SECS`] — checked below at compile time,
    /// since a silence inside the budget would exercise nothing and the test
    /// would still pass — with enough margin that a loaded runner cannot land
    /// the announcement early. It must also stay inside the connect's request
    /// timeout, which is what bounds the readiness cap.
    const SLOW_MOCK_QUIET_SECS: u64 = DISCOVER_PROBE_TIMEOUT_SECS + 3;
    const _: () = assert!(
        SLOW_MOCK_QUIET_SECS > DISCOVER_PROBE_TIMEOUT_SECS,
        "the slow mock's silence must outlast the probe's budget or the readiness test \
         proves nothing"
    );
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
        //
        // `server/discover` is answered with -32601 rather than ignored: a
        // handshake-era server that never answers costs the connect a full
        // probe window before it falls back, and that path has its own test
        // (`silent_probe_times_out_and_falls_back_to_the_handshake`) instead of
        // being paid for here.
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
\x20   m = req.get('method')\n\
\x20   if m == 'server/discover':\n\
\x20       sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': req.get('id'), 'error': {'code': -32601, 'message': 'Method not found'}}) + '\\n')\n\
\x20       sys.stdout.flush()\n\
\x20   elif m == 'initialize':\n\
\x20       sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': req.get('id'), 'result': {}}) + '\\n')\n\
\x20       sys.stdout.flush()\n\
\x20       break\n";

        // Skip cleanly if python3 is unavailable (keeps minimal envs green).
        if !python3_available("is_alive_flips_false_when_server_exits_idle") {
            return;
        }

        let server = connect_mock("mock-bug411", MOCK)
            .await
            .expect("mock server should complete the initialize handshake");

        // The mock exits right after responding, so the transport reaches EOF
        // and the response loop sets alive=false. Poll briefly for the flip.
        let mut became_dead = false;
        for _ in 0..50 {
            if !server.client.is_alive() {
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

    /// Source A (bug-422 sibling): a child's stderr line is
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
\x20   m = req.get('method')\n\
\x20   if m == 'server/discover':\n\
\x20       sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': req.get('id'), 'error': {'code': -32601, 'message': 'Method not found'}}) + '\\n')\n\
\x20       sys.stdout.flush()\n\
\x20   elif m == 'initialize':\n\
\x20       sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': req.get('id'), 'result': {}}) + '\\n')\n\
\x20       sys.stdout.flush()\n";

        if !python3_available("stderr_lines_are_forwarded_as_pseudo_notifications") {
            return;
        }

        let mut server = connect_mock("mock-stderr", MOCK)
            .await
            .expect("mock server should complete the initialize handshake");

        // Poll for the stderr bridge notification (ignore any others).
        let mut got = None;
        for _ in 0..50 {
            match tokio::time::timeout(
                std::time::Duration::from_millis(100),
                server.notifications.recv(),
            )
            .await
            {
                Ok(Some(n)) if n.method == CLOTO_STDERR_LOG_METHOD => {
                    // The readiness sentinel is a stderr line like any other and
                    // arrives first (mcp_test_support). It proves the bridge is
                    // live, but the mock's own line is what this test grades.
                    if stderr_line_from_params(n.params.as_ref()) == MOCK_READY_SENTINEL {
                        continue;
                    }
                    got = Some(n);
                    break;
                }
                Ok(Some(_)) => {}  // some other notification, keep looking
                Ok(None) => break, // channel closed
                Err(_) => {}       // timeout tick
            }
        }

        let n = got.expect("a stderr line must be forwarded as a pseudo-notification");
        assert_eq!(n.server_id, "mock-stderr", "must be tagged with server_id");
        assert_eq!(
            stderr_line_from_params(n.params.as_ref()),
            "hello from stderr"
        );
    }

    /// Source B (backend-B): `mcp_log_from_params` extracts
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
\x20   if m == 'server/discover':\n\
\x20       emit({'jsonrpc': '2.0', 'id': req.get('id'), 'error': {'code': -32601, 'message': 'Method not found'}})\n\
\x20   elif m == 'initialize':\n\
\x20       emit({'jsonrpc': '2.0', 'id': req.get('id'), 'result': {'capabilities': {'logging': {}}}})\n\
\x20   elif m == 'logging/setLevel':\n\
\x20       lvl = req.get('params', {}).get('level')\n\
\x20       emit({'jsonrpc': '2.0', 'id': req.get('id'), 'result': {}})\n\
\x20       emit({'jsonrpc': '2.0', 'method': 'notifications/message', 'params': {'level': 'warning', 'logger': 'test', 'data': 'setlevel=' + str(lvl)}})\n";

        if !python3_available("logging_capability_gets_setlevel_and_forwards_message") {
            return;
        }

        let mut server = connect_mock("mock-logging", MOCK)
            .await
            .expect("mock server should complete the initialize handshake");

        // Poll for the notifications/message triggered by our setLevel.
        let mut got = None;
        for _ in 0..50 {
            match tokio::time::timeout(
                std::time::Duration::from_millis(100),
                server.notifications.recv(),
            )
            .await
            {
                Ok(Some(n)) if n.method == "notifications/message" => {
                    got = Some(n);
                    break;
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => {}
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

    /// Modern-era end-to-end (dual-era): a server that answers
    /// `server/discover` with a mutual modern version is spoken to **without**
    /// `initialize` (the mock fails the connect if one arrives), every later
    /// request carries the four modern `_meta` keys, and once the kernel feeds
    /// approved MGP grants they ride `tools/call` `_meta` under
    /// `dev.cloto/mgp/grants`. The MGP advertisement is read out of
    /// `capabilities.extensions["dev.cloto/mgp"]`, and
    /// `DiscoverResult.instructions` is surfaced on the negotiation outcome.
    #[tokio::test]
    async fn modern_server_negotiates_without_initialize_and_stamps_meta() {
        const MOCK: &str = "import sys, json\n\
def emit(o):\n\
\x20   sys.stdout.write(json.dumps(o) + '\\n'); sys.stdout.flush()\n\
NEED = ['io.modelcontextprotocol/protocolVersion', 'io.modelcontextprotocol/clientInfo',\n\
\x20       'io.modelcontextprotocol/clientCapabilities', 'io.modelcontextprotocol/logLevel']\n\
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
\x20   m = req.get('method'); i = req.get('id')\n\
\x20   meta = (req.get('params') or {}).get('_meta') or {}\n\
\x20   if m == 'server/discover':\n\
\x20       if not all(k in meta for k in NEED[:3]):\n\
\x20           emit({'jsonrpc': '2.0', 'id': i, 'error': {'code': -32000, 'message': 'probe missing _meta'}})\n\
\x20       else:\n\
\x20           emit({'jsonrpc': '2.0', 'id': i, 'result': {'supportedVersions': ['2026-07-28'],\n\
\x20               'capabilities': {'extensions': {'dev.cloto/mgp': {'version': '0.6.0', 'extensions': ['permissions', 'streaming']}}},\n\
\x20               'instructions': 'probe ok', 'resultType': 'complete'}})\n\
\x20   elif m == 'initialize':\n\
\x20       emit({'jsonrpc': '2.0', 'id': i, 'error': {'code': -32600, 'message': 'initialize sent to a modern-era mock'}})\n\
\x20   elif m == 'tools/list':\n\
\x20       if all(k in meta for k in NEED):\n\
\x20           emit({'jsonrpc': '2.0', 'id': i, 'result': {'tools': [{'name': 'meta_ok', 'inputSchema': {}}]}})\n\
\x20       else:\n\
\x20           emit({'jsonrpc': '2.0', 'id': i, 'error': {'code': -32000, 'message': 'missing modern _meta on tools/list'}})\n\
\x20   elif m == 'tools/call':\n\
\x20       if 'dev.cloto/mgp/grants' in meta:\n\
\x20           emit({'jsonrpc': '2.0', 'id': i, 'result': {'content': [{'type': 'text', 'text': 'grants-ok'}], 'resultType': 'complete'}})\n\
\x20       else:\n\
\x20           emit({'jsonrpc': '2.0', 'id': i, 'error': {'code': -32000, 'message': 'missing grants _meta on tools/call'}})\n";

        if !python3_available("modern_server_negotiates_without_initialize_and_stamps_meta") {
            return;
        }

        let server = connect_mock("mock-modern", MOCK)
            .await
            .expect("modern mock must negotiate via server/discover alone");

        assert_eq!(server.negotiated.era, ProtocolEra::Modern);
        assert_eq!(server.client.protocol_era(), Some(ProtocolEra::Modern));
        assert_eq!(
            server.negotiated.instructions.as_deref(),
            Some("probe ok"),
            "DiscoverResult.instructions must be surfaced"
        );
        let mgp = server
            .negotiated
            .mgp
            .clone()
            .expect("MGP advertisement must be read from capabilities.extensions");
        assert_eq!(mgp.version, "0.6.0");
        assert!(mgp.extensions.iter().any(|e| e == "permissions"));

        // The mock rejects any tools/list whose _meta misses one of the four
        // modern keys, so a plain success pins the per-request stamping.
        let tools = server.client.list_tools().await.expect("modern tools/list");
        assert_eq!(tools.tools.len(), 1);
        assert_eq!(tools.tools[0].name, "meta_ok");

        // Approved grants ride tools/call _meta (mgp-spec 0.8.0-draft §3.8).
        server.client.set_mgp_grants(serde_json::json!({
            "network.outbound": { "decision": "approved" }
        }));
        let result = server
            .client
            .call_tool("anything", serde_json::json!({}))
            .await
            .expect("tools/call with grants attached");
        assert!(matches!(
            &result.content[0],
            super::super::mcp_protocol::ToolContent::Text { text } if text == "grants-ok"
        ));
    }

    /// Era policy (4): a handshake-era server answers the probe with a plain
    /// RPC error (`-32601`) and the client falls back to `initialize` — the
    /// pre-dual-era flow, unchanged.
    #[tokio::test]
    async fn method_not_found_probe_falls_back_to_legacy() {
        const MOCK: &str = "import sys, json\n\
def emit(o):\n\
\x20   sys.stdout.write(json.dumps(o) + '\\n'); sys.stdout.flush()\n\
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
\x20   m = req.get('method'); i = req.get('id')\n\
\x20   if m == 'server/discover':\n\
\x20       emit({'jsonrpc': '2.0', 'id': i, 'error': {'code': -32601, 'message': 'Method not found'}})\n\
\x20   elif m == 'initialize':\n\
\x20       emit({'jsonrpc': '2.0', 'id': i, 'result': {'capabilities': {}}})\n";

        if !python3_available("method_not_found_probe_falls_back_to_legacy") {
            return;
        }

        let server = connect_mock("mock-legacy-fallback", MOCK)
            .await
            .expect("a -32601 probe answer must fall back to the handshake");
        assert_eq!(server.negotiated.era, ProtocolEra::Legacy);
        assert_eq!(server.client.protocol_era(), Some(ProtocolEra::Legacy));
        assert!(server.negotiated.mgp.is_none());
        assert!(
            server.negotiated.instructions.is_none(),
            "a handshake carrying no instructions must not invent any"
        );
    }

    /// The legacy era carries `instructions` on the `initialize` result, and it
    /// has to survive the handshake the same way the modern era's does. Before
    /// this, `negotiate_legacy` hard-coded `None`, so a handshake-era server's
    /// guidance was read off the wire and dropped on the floor.
    ///
    /// Two connects: one server with padded instructions (captured, trimmed),
    /// one whose instructions are only whitespace (nothing to say -> `None`, so
    /// no consumer has to tell an empty string from an absent field).
    #[tokio::test]
    async fn legacy_instructions_are_captured_and_blank_is_nothing_to_say() {
        const SPEAKS: &str = "import sys, json\n\
def emit(o):\n\
\x20   sys.stdout.write(json.dumps(o) + '\\n'); sys.stdout.flush()\n\
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
\x20   m = req.get('method'); i = req.get('id')\n\
\x20   if m == 'server/discover':\n\
\x20       emit({'jsonrpc': '2.0', 'id': i, 'error': {'code': -32601, 'message': 'Method not found'}})\n\
\x20   elif m == 'initialize':\n\
\x20       emit({'jsonrpc': '2.0', 'id': i, 'result': {'capabilities': {}, 'instructions': '  store before you recall  '}})\n";

        const BLANK: &str = "import sys, json\n\
def emit(o):\n\
\x20   sys.stdout.write(json.dumps(o) + '\\n'); sys.stdout.flush()\n\
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
\x20   m = req.get('method'); i = req.get('id')\n\
\x20   if m == 'server/discover':\n\
\x20       emit({'jsonrpc': '2.0', 'id': i, 'error': {'code': -32601, 'message': 'Method not found'}})\n\
\x20   elif m == 'initialize':\n\
\x20       emit({'jsonrpc': '2.0', 'id': i, 'result': {'capabilities': {}, 'instructions': '   \\n  '}})\n";

        if !python3_available("legacy_instructions_are_captured_and_blank_is_nothing_to_say") {
            return;
        }

        let speaks = connect_mock("mock-legacy-instructions", SPEAKS)
            .await
            .expect("a handshake-era server must connect");
        assert_eq!(speaks.negotiated.era, ProtocolEra::Legacy);
        assert_eq!(
            speaks.negotiated.instructions.as_deref(),
            Some("store before you recall"),
            "initialize.instructions must be captured, and trimmed"
        );

        let blank = connect_mock("mock-legacy-blank-instructions", BLANK)
            .await
            .expect("a handshake-era server must connect");
        assert_eq!(blank.negotiated.era, ProtocolEra::Legacy);
        assert!(
            blank.negotiated.instructions.is_none(),
            "whitespace-only instructions are nothing to say, not an empty string"
        );
    }

    /// Era policy (4), the other way in: a server that never answers the probe
    /// at all. `classify_probe_error` maps the request timeout onto the same
    /// fallback, and the connect completes down the handshake era.
    ///
    /// Every other mock here answers `server/discover` explicitly, so without
    /// this test nothing would exercise the timeout arm — it used to be covered
    /// only by accident, by mocks that ignored the probe because it was not
    /// what they were about.
    ///
    /// The wait is the point, and it cannot race: the probe window
    /// (`min(request_timeout, DISCOVER_PROBE_TIMEOUT_SECS)` = 10 s) expires no
    /// matter how fast or slow the interpreter starts, because there is no
    /// answer to be had, and the `initialize` that follows has the full
    /// mock request timeout to itself.
    #[tokio::test]
    async fn silent_probe_times_out_and_falls_back_to_the_handshake() {
        const MOCK: &str = "import sys, json\n\
def emit(o):\n\
\x20   sys.stdout.write(json.dumps(o) + '\\n'); sys.stdout.flush()\n\
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
\x20       emit({'jsonrpc': '2.0', 'id': req.get('id'), 'result': {'capabilities': {}}})\n";

        if !python3_available("silent_probe_times_out_and_falls_back_to_the_handshake") {
            return;
        }

        let started = std::time::Instant::now();
        let server = connect_mock("mock-silent-probe", MOCK)
            .await
            .expect("an unanswered probe must fall back to the handshake, not fail the connect");
        assert_eq!(server.negotiated.era, ProtocolEra::Legacy);
        assert_eq!(server.client.protocol_era(), Some(ProtocolEra::Legacy));

        // This mock announces itself on stderr before it reads anything, so the
        // probe's *silence* budget applies and the readiness cap must not: a
        // running server that ignores `server/discover` is answering the era
        // question, and waiting the cap out would be waiting for a signal
        // already received. Bounds the short budget, not the cap, so a
        // readiness check that stops distinguishing the two fails here instead
        // of only making the suite slower.
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(DISCOVER_PROBE_READINESS_CAP_SECS),
            "the probe against a mock that had already spoken took {elapsed:?}, which is the \
             readiness cap ({DISCOVER_PROBE_READINESS_CAP_SECS}s), not the silence budget \
             ({DISCOVER_PROBE_TIMEOUT_SECS}s). The cap is for a child that has written \
             nothing at all; this one announced itself on stderr before the probe went out."
        );
    }

    /// The readiness path, and the regression that pays for it: a server whose
    /// program has not started yet.
    ///
    /// `StdioTransport::start` returns once the process *exists*. The probe is
    /// written into the stdin pipe immediately after, and until the interpreter
    /// reaches the read loop nothing on the other side has run — on a cold
    /// container resolving a venv that is seconds, not milliseconds. Charging
    /// that startup to the probe's silence budget expires it, and
    /// `classify_probe_error` then reads the timeout as era policy (4) and
    /// settles the handshake.
    ///
    /// This mock is modern-only *in what it answers* but still replies to
    /// `initialize`, so the wrong outcome is not a failure anywhere — it is a
    /// silent downgrade to an era the server never claimed, exactly what a
    /// dual-era server would suffer in production. The assertion is therefore
    /// on the negotiated era, and it is the whole test: remove the readiness
    /// distinction and this reports `Legacy` while everything else stays green.
    ///
    /// The silence is [`SLOW_MOCK_QUIET_SECS`], deliberately past the budget.
    /// That makes this one of the slower tests in the module; the alternative
    /// is not testing the path that motivated the mechanism.
    #[tokio::test]
    async fn a_child_that_has_not_started_yet_is_not_read_as_an_era() {
        const MOCK: &str = "import sys, json\n\
def emit(o):\n\
\x20   sys.stdout.write(json.dumps(o) + '\\n'); sys.stdout.flush()\n\
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
\x20   m = req.get('method'); i = req.get('id')\n\
\x20   if m == 'server/discover':\n\
\x20       emit({'jsonrpc': '2.0', 'id': i, 'result': {'supportedVersions': ['2026-07-28'],\n\
\x20           'resultType': 'complete'}})\n\
\x20   elif m == 'initialize':\n\
\x20       emit({'jsonrpc': '2.0', 'id': i, 'result': {'capabilities': {}}})\n";

        if !python3_available("a_child_that_has_not_started_yet_is_not_read_as_an_era") {
            return;
        }

        let quiet = Duration::from_secs(SLOW_MOCK_QUIET_SECS);
        let server = connect_slow_mock("mock-slow-start", MOCK, quiet)
            .await
            .expect("a child that starts late must still negotiate, not fail the connect");
        assert_eq!(
            server.negotiated.era,
            ProtocolEra::Modern,
            "the probe expired on interpreter startup and the handshake was settled from a \
             silence the server never sent — the mock answers `server/discover` in the modern \
             era, it was simply not running yet when we asked"
        );
        assert_eq!(server.client.protocol_era(), Some(ProtocolEra::Modern));
    }

    /// Era policy (3): `-32022` naming only versions this kernel knows neither
    /// as modern nor as handshake is a genuine incompatibility — the connect
    /// must fail, not silently downgrade.
    #[tokio::test]
    async fn disjoint_modern_only_server_fails_the_connect() {
        const MOCK: &str = "import sys, json\n\
def emit(o):\n\
\x20   sys.stdout.write(json.dumps(o) + '\\n'); sys.stdout.flush()\n\
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
\x20   if req.get('method') == 'server/discover':\n\
\x20       emit({'jsonrpc': '2.0', 'id': req.get('id'), 'error': {'code': -32022,\n\
\x20           'message': 'unsupported protocol version', 'data': {'supported': ['2027-01-01']}}})\n\
\x20   else:\n\
\x20       emit({'jsonrpc': '2.0', 'id': req.get('id'), 'error': {'code': -32600, 'message': 'no handshake here'}})\n";

        if !python3_available("disjoint_modern_only_server_fails_the_connect") {
            return;
        }

        let err = match connect_mock("mock-disjoint", MOCK).await {
            Ok(_) => panic!("a disjoint modern-only server must fail the connect"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("protocol version mismatch"),
            "the connect failed, but not on the disjoint-version policy: {err:#}\n\
             `-32600 no handshake here` means the mock answered `initialize`, i.e. the \
             `server/discover` probe timed out before the child was ready and negotiation \
             fell back to the handshake (policy 4). Policy (3) was never exercised — this \
             is an environment result, not a regression. The readiness gate in \
             mcp_test_support exists to keep that from happening; read its module docs."
        );
    }

    /// Modern-era MRTR: `resultType: "input_required"` asks the client to
    /// continue a multi-round interaction the kernel host has no flow for. It
    /// must surface as an explicit error, never parse as a final result.
    #[tokio::test]
    async fn modern_input_required_surfaces_as_an_error() {
        const MOCK: &str = "import sys, json\n\
def emit(o):\n\
\x20   sys.stdout.write(json.dumps(o) + '\\n'); sys.stdout.flush()\n\
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
\x20   m = req.get('method'); i = req.get('id')\n\
\x20   if m == 'server/discover':\n\
\x20       emit({'jsonrpc': '2.0', 'id': i, 'result': {'supportedVersions': ['2026-07-28'], 'resultType': 'complete'}})\n\
\x20   elif m == 'tools/call':\n\
\x20       emit({'jsonrpc': '2.0', 'id': i, 'result': {'resultType': 'input_required',\n\
\x20           'inputRequests': {'q1': {'prompt': 'which file?'}}}})\n";

        if !python3_available("modern_input_required_surfaces_as_an_error") {
            return;
        }

        let server = connect_mock("mock-mrtr", MOCK)
            .await
            .expect("modern mock must negotiate");
        assert_eq!(server.negotiated.era, ProtocolEra::Modern);

        let err = server
            .client
            .call_tool("ask", serde_json::json!({}))
            .await
            .expect_err("input_required must not parse as a final result");
        assert!(
            err.to_string().contains("input_required"),
            "unexpected error: {err:#}"
        );
    }
}
