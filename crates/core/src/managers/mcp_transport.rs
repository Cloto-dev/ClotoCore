use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::mcp_isolation::{FilesystemScope, IsolationProfile, NetworkScope};

// ── McpTransport Enum ──

/// Unified transport abstraction for MCP client connections.
pub enum McpTransport {
    Stdio(Box<StdioTransport>),
    Http(Box<HttpTransport>),
}

impl McpTransport {
    #[must_use]
    pub fn sender(&self) -> mpsc::Sender<String> {
        match self {
            Self::Stdio(t) => t.sender(),
            Self::Http(t) => t.sender(),
        }
    }

    /// OS pid of the spawned child (None for HTTP transports or once the child
    /// has been reaped). The child is its own process-group leader
    /// (`process_group(0)` in `start`), so this doubles as the pgid — the
    /// forced drain sweep signals `-pid` to reap the whole subtree (bug-426).
    #[must_use]
    pub fn child_id(&self) -> Option<u32> {
        match self {
            Self::Stdio(t) => t.child_id(),
            Self::Http(_) => None,
        }
    }

    /// Kill the child process and wait for it to exit (up to 5 seconds).
    /// Prevents race conditions where a new server starts before the old one releases resources.
    pub async fn kill_and_wait(&mut self) {
        match self {
            Self::Stdio(t) => t.kill_and_wait().await,
            Self::Http(_) => {} // No child process to kill
        }
    }

    pub async fn recv(&mut self) -> Option<String> {
        match self {
            Self::Stdio(t) => t.recv().await,
            Self::Http(t) => t.recv().await,
        }
    }

    pub fn is_alive(&mut self) -> bool {
        match self {
            Self::Stdio(t) => t.is_alive(),
            Self::Http(t) => t.is_alive(),
        }
    }
}

/// Allowed commands for MCP server execution (security whitelist)
const ALLOWED_COMMANDS: &[&str] = &["npx", "node", "python", "python3", "deno", "bun", "docker"];

/// Buffer size for MCP stdio channel (request and response).
const MCP_CHANNEL_BUFFER_SIZE: usize = 100;

/// bug-355: timeout for a single write to the child's stdin pipe. Guards
/// against a child that stops reading stdin (full pipe buffer) wedging the
/// writer task — and, transitively, the bounded request channel upstream.
const STDIO_WRITE_TIMEOUT_SECS: u64 = 10;

/// bug-356: per-request send timeout for the HTTP transport. Bounds how long a
/// single queued message can hold the request loop before the next one runs,
/// on top of the reqwest client's global timeout.
const HTTP_PER_MESSAGE_TIMEOUT_SECS: u64 = 30;

/// bug-358: grace period to wait for a graceful (SIGTERM) exit before
/// escalating to SIGKILL in `kill_and_wait`.
const SHUTDOWN_GRACE_SECS: u64 = 3;

/// If command is python/python3, resolve to venv Python if available.
/// Returns (resolved_command, is_venv) tuple.
fn resolve_python_command(command: &str) -> (String, bool) {
    if command != "python" && command != "python3" {
        return (command.to_string(), false);
    }

    if let Some(venv_python) = super::mcp_venv::resolve_venv_python() {
        let path_str = venv_python.to_string_lossy().to_string();
        info!("Resolved {} → {}", command, path_str);
        return (path_str, true);
    }

    (command.to_string(), false)
}

/// Check if command is a workspace-internal Rust binary (e.g. target/debug/mgp-avatar).
/// Handles both relative paths ("target/debug/mgp-avatar") and absolute paths
/// resolved by the config loader.
fn is_workspace_binary(command: &str) -> bool {
    let normalized = command.replace('\\', "/");
    if normalized.contains("..") {
        return false;
    }
    // Relative path form
    if normalized.starts_with("target/debug/mgp-") || normalized.starts_with("target/release/mgp-")
    {
        return true;
    }
    // Absolute path form (resolved by config loader)
    if let Some(idx) = normalized.find("/target/debug/mgp-") {
        return idx > 0;
    }
    if let Some(idx) = normalized.find("/target/release/mgp-") {
        return idx > 0;
    }
    false
}

/// Validate command against whitelist (bare command names only, no paths)
pub fn validate_command(command: &str) -> Result<String> {
    // Allow workspace-internal Rust binaries (target/{debug,release}/mgp-*)
    if is_workspace_binary(command) {
        return Ok(command.to_string());
    }

    if command.contains('/') || command.contains('\\') {
        bail!(
            "Command must not contain path separators: '{}'. Use bare command names only.",
            command
        );
    }

    if !ALLOWED_COMMANDS.contains(&command) {
        bail!(
            "Command '{}' not in whitelist. Allowed commands: {:?}",
            command,
            ALLOWED_COMMANDS
        );
    }

    Ok(command.to_string())
}

/// Pre-spawn validation: reject a stale or incomplete install whose entry-point
/// file no longer exists on disk, so the server fails with a clear, actionable
/// error instead of an opaque interpreter / spawn failure (e.g. python printing
/// "can't open file '…server.py'" then exiting, surfaced only as "error, 0
/// tools"). This guards against a recorded entry-point that has gone stale —
/// e.g. a past marketplace install-path bug, or a removed install directory.
///
/// Only **absolute** paths are checked: a module invocation (`python -m
/// module`) resolves to a bare module name, and a relative script depends on the
/// child's working directory — neither is a filesystem path we can verify here,
/// so both are skipped to avoid false negatives.
fn validate_entry_point_exists(command: &str, args: &[String]) -> Result<()> {
    let entry = crate::managers::mcp::resolve_sealable_entry_point(command, args);
    if entry.is_absolute() && !entry.exists() {
        bail!(
            "MCP server entry-point not found: {} — the install is stale or \
             incomplete. Reinstall this server from the marketplace.",
            entry.display()
        );
    }
    Ok(())
}

pub struct StdioTransport {
    child: Child,
    request_tx: mpsc::Sender<String>,
    response_rx: mpsc::Receiver<String>,
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // P9 + orphan-leak fix: the child is spawned as its own process-group
        // leader (`process_group(0)` in `start`), so kill the whole group — not
        // just the direct PID — otherwise a dropped transport strands any helper
        // processes the server forked (grandchildren). Best-effort and
        // synchronous: Drop can't await, and `kill_on_drop(true)` still reaps the
        // direct child. Prefer explicit `kill_and_wait()` wherever an async
        // context is available.
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            // SAFETY: forceful kill of a process group we created and own.
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
        debug!("StdioTransport dropped — child process group kill signal sent");
    }
}

impl StdioTransport {
    /// OS pid of the spawned child == its pgid (`process_group(0)` in `start`).
    /// None once the child has been reaped.
    #[must_use]
    pub fn child_id(&self) -> Option<u32> {
        self.child.id()
    }

    /// Kill the child process and wait for it to actually exit.
    /// Ensures file locks (DB, ports) are released before returning.
    pub async fn kill_and_wait(&mut self) {
        // bug-358 + orphan-leak fix: the child is spawned as its own
        // process-group leader (`process_group(0)` in `start`), so signalling the
        // negative pid (== pgid) delivers to the server AND every helper process
        // it forked, reaping the whole subtree instead of leaking grandchildren.
        // Try a graceful SIGTERM first so the server can flush state and release
        // locks, then escalate to SIGKILL. Unix-only; other platforms go straight
        // to start_kill (single-PID — subtree containment there needs a Job
        // Object, tracked for the cross-platform harness).
        #[cfg(unix)]
        {
            if let Some(pid) = self.child.id() {
                let pgid = pid as libc::pid_t;
                // SAFETY: signalling a process group we created and own (the child
                // is its own group leader). A stray pgid at worst no-ops (ESRCH),
                // and the start_kill fallback below still reaps the direct child.
                unsafe {
                    libc::kill(-pgid, libc::SIGTERM);
                }
                if let Ok(Ok(status)) = tokio::time::timeout(
                    std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS),
                    self.child.wait(),
                )
                .await
                {
                    debug!("Child process group exited gracefully after SIGTERM: {status}");
                    return;
                }
                tracing::warn!(
                    "Child did not exit within {SHUTDOWN_GRACE_SECS}s of SIGTERM — escalating to SIGKILL (process group)"
                );
                // SAFETY: same owned process group, forceful kill of the subtree.
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
            }
        }

        // SIGKILL fallback for the direct child (also the primary path on non-Unix
        // targets) + reap so we never leave a zombie of our own child.
        let _ = self.child.start_kill();
        match tokio::time::timeout(std::time::Duration::from_secs(5), self.child.wait()).await {
            Ok(Ok(status)) => {
                debug!("Child process exited: {status}");
            }
            Ok(Err(e)) => {
                debug!("Child process wait error: {e}");
            }
            Err(_) => {
                tracing::warn!("Child process did not exit within 5s after SIGKILL");
            }
        }
    }

    /// Get a clone of the request sender for lock-free sending.
    #[must_use]
    pub fn sender(&self) -> mpsc::Sender<String> {
        self.request_tx.clone()
    }

    /// Start a new MCP server process with environment variable injection
    /// and optional OS-level isolation.
    ///
    /// When `isolation` is provided:
    /// - Working directory is set to the sandbox dir (created if needed).
    /// - `CLOTO_SANDBOX_DIR`, `HOME`/`TMPDIR`/`TMP`/`TEMP` point into the sandbox.
    /// - For `NetworkScope::ProxyOnly`: `CLOTO_LLM_PROXY`, `HTTP_PROXY`, `HTTPS_PROXY`
    ///   are set to the kernel LLM proxy.
    /// - Sensitive env vars (LLM API keys) are stripped from the child environment.
    #[allow(clippy::too_many_lines)]
    pub async fn start(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        isolation: Option<&IsolationProfile>,
        llm_proxy_port: u16,
        sensitive_env_keys: &[String],
        stderr_tx: Option<mpsc::Sender<String>>,
    ) -> Result<Self> {
        info!("Starting MCP Server: {} {:?}", command, args);

        // Resolve python/python3 to venv Python if available
        let (resolved, is_venv) = resolve_python_command(command);
        let final_command = if is_venv {
            resolved // Venv path is internally generated — skip whitelist validation
        } else {
            validate_command(command).context("Command validation failed")?
        };

        // Fail fast with a clear message if the entry-point script/binary is
        // missing, instead of letting the interpreter exit opaquely.
        validate_entry_point_exists(&final_command, args)?;

        let mut cmd = Command::new(&final_command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Orphan-leak fix: put each MCP server in its own process group so the
        // child becomes the group leader (pgid == child pid). On restart/shutdown
        // we signal the whole group (see `kill_and_wait` / `Drop`), reaping any
        // helper processes the server forks (a launcher → real server, a bridge →
        // workers) as a unit instead of orphaning them with a bare single-PID
        // kill. Unix-only; Windows subtree containment needs a Job Object (TODO,
        // tracked for the cross-platform harness) — the CREATE_NO_WINDOW path
        // below is unchanged there.
        #[cfg(unix)]
        cmd.process_group(0);

        // Windows: prevent console windows from appearing for child processes
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        // Inject environment variables (with shell variable expansion)
        for (key, value) in env {
            let resolved = resolve_env_value(value);
            cmd.env(key, resolved);
        }

        // Force unbuffered stdout for Python MCP servers.
        // Without this, piped stdout uses block buffering (4-8KB) on Windows,
        // causing JSON-RPC responses to sit in the buffer and never reach
        // the Rust reader — resulting in MCP Request timeouts.
        cmd.env("PYTHONUNBUFFERED", "1");

        // Always inject the actual LLM proxy port so MCP servers can
        // discover it at runtime (supplements config-level env vars).
        cmd.env("CLOTO_LLM_PROXY_PORT", llm_proxy_port.to_string());

        // Never hand a child the Magic Seal key (an earlier decision). There is no
        // `env_clear` here — a child inherits the kernel's environment — so
        // when the kernel was started with `CLOTO_SEAL_KEY` set, every
        // connector was being given the key that authenticates its own
        // integrity seal. Removed unconditionally, outside the isolation
        // block: the strip list there is built from the LLM provider env
        // mappings and only applies when a profile is attached, and this must
        // hold for every spawn regardless of trust level.
        cmd.env_remove("CLOTO_SEAL_KEY");

        // Apply OS-level isolation (Phase 1: environment-based soft isolation).
        if let Some(profile) = isolation {
            // Create sandbox directory if it doesn't exist.
            if profile.filesystem_scope != FilesystemScope::Unrestricted {
                std::fs::create_dir_all(&profile.sandbox_dir).with_context(|| {
                    format!(
                        "Failed to create sandbox directory: {}",
                        profile.sandbox_dir.display()
                    )
                })?;
                cmd.current_dir(&profile.sandbox_dir);

                // Redirect HOME/TMPDIR/TMP/TEMP into the sandbox.
                let sandbox_str = profile.sandbox_dir.to_string_lossy().to_string();
                cmd.env("CLOTO_SANDBOX_DIR", &sandbox_str);
                cmd.env("HOME", &sandbox_str);
                cmd.env("TMPDIR", &sandbox_str);
                cmd.env("TMP", &sandbox_str);
                cmd.env("TEMP", &sandbox_str);
            }

            // Network isolation: inject proxy env vars for ProxyOnly.
            if profile.network_scope == NetworkScope::ProxyOnly {
                let proxy_url = format!("http://127.0.0.1:{llm_proxy_port}");
                cmd.env("CLOTO_LLM_PROXY", &proxy_url);
                cmd.env("HTTP_PROXY", &proxy_url);
                cmd.env("HTTPS_PROXY", &proxy_url);
                // Allow localhost communication (e.g. VOICEVOX Engine, inter-server HTTP).
                cmd.env("NO_PROXY", "localhost,127.0.0.1,::1");
            }

            // Strip sensitive environment variables (LLM API keys).
            for key in sensitive_env_keys {
                cmd.env_remove(key);
            }

            debug!(
                sandbox = ?profile.sandbox_dir,
                fs = ?profile.filesystem_scope,
                net = ?profile.network_scope,
                "Isolation profile applied"
            );
        }

        let mut child = cmd
            .spawn()
            .context(format!("Failed to spawn MCP server: {}", command))?;

        let stdin = child.stdin.take().context("Failed to open stdin")?;
        let stdout = child.stdout.take().context("Failed to open stdout")?;
        let stderr = child.stderr.take().context("Failed to open stderr")?;

        let (req_tx, mut req_rx) = mpsc::channel::<String>(MCP_CHANNEL_BUFFER_SIZE);
        let (res_tx, res_rx) = mpsc::channel::<String>(MCP_CHANNEL_BUFFER_SIZE);

        // Writer Task
        tokio::spawn(async move {
            let mut writer = stdin;
            // bug-355: bound each write/flush so a child that stops reading its
            // stdin (full pipe buffer) cannot wedge this task forever. On stall
            // we break — closing req_rx, which surfaces as is_alive()==false and
            // lets the health monitor restart the server.
            let write_timeout = std::time::Duration::from_secs(STDIO_WRITE_TIMEOUT_SECS);
            while let Some(msg) = req_rx.recv().await {
                let line = format!("{}\n", msg);
                match tokio::time::timeout(write_timeout, writer.write_all(line.as_bytes())).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        error!("Failed to write to MCP server stdin: {}", e);
                        break;
                    }
                    Err(_) => {
                        error!(
                            "Timed out writing to MCP server stdin ({STDIO_WRITE_TIMEOUT_SECS}s) — child stdin pipe likely full"
                        );
                        break;
                    }
                }
                match tokio::time::timeout(write_timeout, writer.flush()).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        error!("Failed to flush MCP server stdin: {}", e);
                        break;
                    }
                    Err(_) => {
                        error!("Timed out flushing MCP server stdin ({STDIO_WRITE_TIMEOUT_SECS}s)");
                        break;
                    }
                }
            }
        });

        // Reader Task (Stdout) — runs until EOF or read error.
        // No silence timeout: MCP servers are idle between tool calls and may
        // produce no output for extended periods. The health check system
        // (mcp_health.rs) handles frozen server detection separately.
        tokio::spawn(async move {
            // bug-449: bounded line reader. `lines()`/`next_line()` buffer an
            // unterminated line without any size cap; read in chunks and enforce
            // MAX_STDIO_LINE_BYTES on the accumulator instead.
            let mut reader = BufReader::new(stdout);
            let mut line_buf: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 8192];
            'outer: loop {
                let n = match reader.read(&mut chunk).await {
                    Ok(0) => break, // EOF
                    Ok(n) => n,
                    Err(e) => {
                        warn!("MCP stdout read error: {}", e);
                        break;
                    }
                };
                let mut start = 0;
                for i in 0..n {
                    if chunk[i] == b'\n' {
                        line_buf.extend_from_slice(&chunk[start..i]);
                        let line = String::from_utf8_lossy(&line_buf).trim().to_string();
                        line_buf.clear();
                        start = i + 1;
                        if !line.is_empty() && res_tx.send(line).await.is_err() {
                            break 'outer;
                        }
                    }
                }
                line_buf.extend_from_slice(&chunk[start..n]);
                if line_buf.len() > MAX_STDIO_LINE_BYTES {
                    warn!(
                        "MCP stdout line exceeded {} bytes without a newline — aborting reader",
                        MAX_STDIO_LINE_BYTES
                    );
                    break;
                }
            }
            warn!("MCP Server stdout closed.");
        });

        // Logger Task (Stderr) — dual sink: kernel tracing (warn! so it is
        // visible in release builds and post-mortem logs, independent of any
        // dashboard) AND, if a stderr_tx is provided, the raw line is forwarded
        // to the owning client which tags it with server_id and turns it into a
        // McpServerLog event for the dashboard Log tab. See
        // docs/MCP_SERVER_LOGS_DESIGN.md §6.
        let cmd_display = command.to_string();
        tokio::spawn(async move {
            // bug-449: same bounded reader as stdout — an unterminated stderr
            // blob must not grow the buffer without bound.
            let mut reader = BufReader::new(stderr);
            let mut line_buf: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 8192];
            'outer: loop {
                let n = match reader.read(&mut chunk).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                let mut start = 0;
                for i in 0..n {
                    if chunk[i] == b'\n' {
                        line_buf.extend_from_slice(&chunk[start..i]);
                        let line = String::from_utf8_lossy(&line_buf).trim().to_string();
                        line_buf.clear();
                        start = i + 1;
                        if line.is_empty() {
                            continue;
                        }
                        warn!("[MCP:{}] {}", cmd_display, line);
                        if let Some(ref tx) = stderr_tx {
                            // Best-effort: drop on a gone/full consumer (tracing
                            // already captured it). Never block the reader.
                            let _ = tx.try_send(line);
                        }
                    }
                }
                line_buf.extend_from_slice(&chunk[start..n]);
                if line_buf.len() > MAX_STDIO_LINE_BYTES {
                    warn!(
                        "MCP stderr line exceeded {MAX_STDIO_LINE_BYTES} bytes — aborting logger"
                    );
                    break 'outer;
                }
            }
        });

        Ok(Self {
            child,
            request_tx: req_tx,
            response_rx: res_rx,
        })
    }

    pub async fn send(&self, msg: String) -> Result<()> {
        self.request_tx
            .send(msg)
            .await
            .context("Failed to send message to transport task")
    }

    pub async fn recv(&mut self) -> Option<String> {
        self.response_rx.recv().await
    }

    /// Check if the child process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

// ── HttpTransport ──

/// HTTP-based MCP transport using Streamable HTTP (MCP 2025-03-26 spec).
///
/// Connects to a remote MCP server via HTTP POST requests and SSE responses.
/// Presents the same channel-based interface as StdioTransport.
pub struct HttpTransport {
    #[allow(dead_code)]
    url: String,
    request_tx: mpsc::Sender<String>,
    response_rx: mpsc::Receiver<String>,
    alive: Arc<AtomicBool>,
    request_task: tokio::task::JoinHandle<()>,
}

impl Drop for HttpTransport {
    fn drop(&mut self) {
        self.request_task.abort();
        self.alive.store(false, Ordering::Relaxed);
        debug!("HttpTransport dropped — request task aborted");
    }
}

impl HttpTransport {
    /// Get a clone of the request sender for lock-free sending.
    #[must_use]
    pub fn sender(&self) -> mpsc::Sender<String> {
        self.request_tx.clone()
    }

    /// Receive the next response or notification from the remote server.
    pub async fn recv(&mut self) -> Option<String> {
        self.response_rx.recv().await
    }

    /// Check if the HTTP connection is alive.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Start an HTTP transport connection to a remote MCP server.
    ///
    /// `era` is the connection's shared protocol-era state, still unset at this
    /// point: the client negotiates *through* this transport. The request loop
    /// therefore reads it per message, which is what lets the modern era add its
    /// routing headers and stop sending `Mcp-Session-Id` (a header MCP
    /// 2026-07-28 removed) the moment negotiation settles.
    #[allow(clippy::too_many_lines)]
    pub async fn start(
        url: &str,
        auth_token: Option<&str>,
        era: super::mcp_protocol::EraHandle,
    ) -> Result<Self> {
        // URL validation: HTTPS required for non-localhost
        validate_http_url(url)?;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_mins(1))
            .build()
            .context("Failed to create HTTP client")?;

        let (req_tx, mut req_rx) = mpsc::channel::<String>(MCP_CHANNEL_BUFFER_SIZE);
        let (res_tx, res_rx) = mpsc::channel::<String>(MCP_CHANNEL_BUFFER_SIZE);

        let alive = Arc::new(AtomicBool::new(true));
        let alive_clone = alive.clone();
        let url_owned = url.to_string();
        let auth = auth_token.map(std::string::ToString::to_string);
        let session_id: Arc<std::sync::Mutex<Option<String>>> =
            Arc::new(std::sync::Mutex::new(None));

        let request_task = tokio::spawn(async move {
            info!(url = %url_owned, "HttpTransport request handler started");

            while let Some(msg) = req_rx.recv().await {
                let mut request = client
                    .post(&url_owned)
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json, text/event-stream");

                if let Some(ref token) = auth {
                    request = request.header("Authorization", format!("Bearer {token}"));
                }

                // bug-450: capture the request id before `msg` is moved into the
                // body — error envelopes must echo it so start_response_loop can
                // route the failure to the exact pending request (a null id
                // never matches, leaving the caller to a generic timeout).
                let parsed = serde_json::from_str::<serde_json::Value>(&msg).ok();
                let req_id = parsed
                    .as_ref()
                    .and_then(|v| v.get("id").cloned())
                    .unwrap_or(serde_json::Value::Null);
                let method = parsed
                    .as_ref()
                    .and_then(|v| v.get("method"))
                    .and_then(|m| m.as_str())
                    .map(str::to_string);

                // Modern era (and the `server/discover` probe that decides it,
                // which necessarily runs before the era is known) declares the
                // protocol version and method out-of-band so the server's era
                // router can dispatch without parsing the body.
                let is_modern = era.is_modern();
                let is_probe = method.as_deref() == Some(super::mcp_protocol::DISCOVER_METHOD);
                if is_modern || is_probe {
                    let version = era.wire_version().unwrap_or_else(|| {
                        super::mcp_protocol::MODERN_PROTOCOL_VERSION.to_string()
                    });
                    request = request.header(super::mcp_protocol::HEADER_PROTOCOL_VERSION, version);
                    if let Some(ref m) = method {
                        request = request.header(super::mcp_protocol::HEADER_METHOD, m.clone());
                    }
                }

                // `Mcp-Session-Id` is legacy-only: MCP 2026-07-28 removed
                // session identity from the protocol, so a modern connection
                // neither sends nor tracks it.
                if !is_modern {
                    if let Some(ref sid) = *session_id
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                    {
                        request = request.header("Mcp-Session-Id", sid.clone());
                    }
                }

                // bug-356: bound each request so one slow message can't stall the
                // serial request loop (and the queue behind it). This is in
                // addition to the reqwest client's global timeout above.
                let send_fut = request.body(msg).send();
                let response = match tokio::time::timeout(
                    std::time::Duration::from_secs(HTTP_PER_MESSAGE_TIMEOUT_SECS),
                    send_fut,
                )
                .await
                {
                    Ok(Ok(r)) => r,
                    Ok(Err(e)) => {
                        error!(url = %url_owned, error = %e, "HTTP request failed");
                        // Send JSON-RPC error so pending requests don't hang
                        let err_msg = serde_json::json!({
                            "jsonrpc": "2.0",
                            "error": {"code": -32000, "message": format!("HTTP transport error: {e}")},
                            "id": req_id.clone()
                        });
                        let _ = res_tx.send(err_msg.to_string()).await;
                        continue;
                    }
                    Err(_) => {
                        error!(url = %url_owned, "HTTP request timed out ({HTTP_PER_MESSAGE_TIMEOUT_SECS}s)");
                        let err_msg = serde_json::json!({
                            "jsonrpc": "2.0",
                            "error": {"code": -32000, "message": format!("HTTP per-message timeout ({HTTP_PER_MESSAGE_TIMEOUT_SECS}s)")},
                            "id": req_id.clone()
                        });
                        let _ = res_tx.send(err_msg.to_string()).await;
                        continue;
                    }
                };

                let status = response.status();
                if !status.is_success() {
                    let body = response.text().await.unwrap_or_default();
                    error!(url = %url_owned, status = %status, "HTTP error response: {body}");
                    let err_msg = serde_json::json!({
                        "jsonrpc": "2.0",
                        "error": {"code": -32000, "message": format!("HTTP {status}: {body}")},
                        "id": req_id.clone()
                    });
                    let _ = res_tx.send(err_msg.to_string()).await;
                    continue;
                }

                // Track session ID from response header (legacy era only — see
                // the send-side guard above).
                if !is_modern {
                    if let Some(sid) = response.headers().get("mcp-session-id") {
                        if let Ok(sid_str) = sid.to_str() {
                            let mut guard = session_id
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            *guard = Some(sid_str.to_string());
                        }
                    }
                }

                let content_type = response
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();

                if content_type.contains("text/event-stream") {
                    // SSE response: parse event stream
                    if let Err(e) = parse_sse_stream(response, &res_tx).await {
                        warn!(url = %url_owned, error = %e, "SSE stream error");
                    }
                } else {
                    // JSON response: send body directly
                    match response.text().await {
                        Ok(body) if !body.trim().is_empty() => {
                            let _ = res_tx.send(body).await;
                        }
                        Ok(_) => {} // Empty response (e.g., notification acknowledgement)
                        Err(e) => {
                            warn!(url = %url_owned, error = %e, "Failed to read response body");
                        }
                    }
                }
            }

            alive_clone.store(false, Ordering::Relaxed);
            info!("HttpTransport request handler stopped");
        });

        Ok(Self {
            url: url.to_string(),
            request_tx: req_tx,
            response_rx: res_rx,
            alive,
            request_task,
        })
    }
}

/// Maximum bytes buffered for a single SSE line before a terminating newline.
///
/// bug-410: a server that streams a large `text/event-stream` body without any
/// newline would otherwise grow the assembly buffer without bound → memory
/// exhaustion. An unbroken run past this limit is treated as a protocol
/// violation and aborts the stream.
const MAX_SSE_LINE_BYTES: usize = 1024 * 1024; // 1 MiB

/// bug-449: cap for a single stdio line from a spawned MCP server. Mirrors
/// `MAX_SSE_LINE_BYTES` — a child that writes a huge blob without a newline
/// would otherwise grow the line buffer without bound and OOM the whole kernel
/// (not just the connection). A line past this limit aborts the reader task.
const MAX_STDIO_LINE_BYTES: usize = 1024 * 1024; // 1 MiB

/// Parse an SSE (Server-Sent Events) stream, forwarding `data:` lines to the channel.
async fn parse_sse_stream(
    response: reqwest::Response,
    res_tx: &mpsc::Sender<String>,
) -> Result<()> {
    use futures::StreamExt;
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::<u8>::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("SSE stream chunk error")?;
        buffer.extend_from_slice(&chunk);

        // Process complete lines from the buffer. Track how many bytes are
        // consumed and drain them once per chunk instead of reallocating the
        // whole buffer per line (the old `buffer = buffer[pos+1..].to_vec()` was
        // O(n^2)).
        let mut consumed = 0;
        while let Some(rel) = buffer[consumed..].iter().position(|&b| b == b'\n') {
            let line_end = consumed + rel;
            // Scope the borrow of `buffer` so it is released before the await.
            let to_send = {
                let line = String::from_utf8_lossy(&buffer[consumed..line_end]);
                let line = line.trim_end_matches('\r');
                line.strip_prefix("data: ")
                    .filter(|d| !d.is_empty())
                    .map(str::to_string)
            };
            consumed = line_end + 1;
            if let Some(data) = to_send {
                let _ = res_tx.send(data).await;
            }
            // Skip event:, id:, retry:, and comment (:) lines
        }
        if consumed > 0 {
            buffer.drain(..consumed);
        }

        // bug-410: bound the unterminated remainder so a newline-less stream
        // cannot exhaust memory.
        if buffer.len() > MAX_SSE_LINE_BYTES {
            anyhow::bail!(
                "SSE line exceeded {} bytes without a newline — aborting stream",
                MAX_SSE_LINE_BYTES
            );
        }
    }

    // bug-451: a stream that ends without a trailing newline (connection close
    // right after `data: {...}`) leaves its final line in the buffer — flush it
    // instead of silently dropping the only response for the request.
    if !buffer.is_empty() {
        let line = String::from_utf8_lossy(&buffer);
        let line = line.trim_end_matches('\r');
        if let Some(data) = line.strip_prefix("data: ").filter(|d| !d.is_empty()) {
            let _ = res_tx.send(data.to_string()).await;
        }
    }

    Ok(())
}

/// Validate that an HTTP URL meets security requirements.
/// Non-localhost URLs must use HTTPS.
fn validate_http_url(url: &str) -> Result<()> {
    let is_https = url.starts_with("https://");
    let is_http = url.starts_with("http://");

    if !is_https && !is_http {
        bail!("MCP server URL must start with http:// or https://: {url}");
    }

    if is_http {
        // HTTP only allowed for localhost
        let after_scheme = &url[7..]; // skip "http://"
        let host = after_scheme.split('/').next().unwrap_or("");
        let host = host.split(':').next().unwrap_or(host); // strip port
        let is_localhost =
            host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "[::1]";
        if !is_localhost {
            bail!(
                "Remote MCP server URL must use HTTPS. \
                 HTTP is only allowed for localhost. Got: {url}"
            );
        }
    }

    Ok(())
}

/// Resolve `${ENV_VAR}` references in a value string to actual environment variables.
fn resolve_env_value(value: &str) -> String {
    if let Some(var_name) = value.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
        std::env::var(var_name).unwrap_or_default()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_command_allowed() {
        assert!(validate_command("npx").is_ok());
        assert!(validate_command("node").is_ok());
        assert!(validate_command("python3").is_ok());
        assert!(validate_command("deno").is_ok());
        assert!(validate_command("bun").is_ok());
    }

    #[test]
    fn test_validate_command_blocked() {
        assert!(validate_command("bash").is_err());
        assert!(validate_command("sh").is_err());
        assert!(validate_command("cmd").is_err());
        assert!(validate_command("powershell").is_err());
        assert!(validate_command("/bin/sh").is_err());
        assert!(validate_command("../../../bin/sh").is_err());
    }

    #[test]
    fn test_validate_command_rejects_paths() {
        assert!(validate_command("/usr/bin/node").is_err());
        assert!(validate_command("../../../bin/node").is_err());
        assert!(validate_command("C:\\Windows\\node").is_err());
    }

    #[test]
    fn test_validate_command_allows_workspace_binaries() {
        // Relative paths
        assert!(validate_command("target/debug/mgp-avatar").is_ok());
        assert!(validate_command("target/release/mgp-avatar").is_ok());
        // Absolute paths (resolved by config loader)
        assert!(validate_command("C:\\Users\\Dev\\project\\target\\debug\\mgp-avatar").is_ok());
        assert!(validate_command("/home/user/project/target/debug/mgp-avatar").is_ok());
        // Reject traversal attempts
        assert!(validate_command("target/debug/../../../bin/sh").is_err());
        // Reject non-mgp binaries
        assert!(validate_command("target/debug/malicious").is_err());
    }

    #[test]
    fn test_resolve_env_value_passthrough() {
        assert_eq!(resolve_env_value("hello"), "hello");
        assert_eq!(resolve_env_value(""), "");
    }

    #[test]
    fn test_resolve_env_value_expansion() {
        std::env::set_var("TEST_CLOTO_VAR", "resolved_value");
        assert_eq!(resolve_env_value("${TEST_CLOTO_VAR}"), "resolved_value");
        std::env::remove_var("TEST_CLOTO_VAR");
    }

    #[test]
    fn test_resolve_env_value_missing() {
        assert_eq!(resolve_env_value("${NONEXISTENT_CLOTO_VAR_12345}"), "");
    }

    // ── validate_entry_point_exists (bug-398 / an earlier decision robustness) ──

    #[test]
    fn entry_point_missing_absolute_script_is_rejected() {
        // The doubled-path install bug left rows like
        // …/mcp-servers/servers/cscheduler/servers/cscheduler/server.py that no
        // longer exist on disk; this must be caught before spawn. Build the path
        // from the crate dir so it is platform-absolute (a Unix-style `/…` path
        // is not absolute on Windows and would be skipped).
        let missing = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("definitely_not_here")
            .join("servers")
            .join("x")
            .join("server.py");
        let args = vec![missing.to_string_lossy().to_string()];
        let err = validate_entry_point_exists("python", &args)
            .expect_err("a missing absolute entry-point must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("entry-point not found"), "got: {msg}");
        assert!(
            msg.contains("Reinstall"),
            "message should be actionable: {msg}"
        );
    }

    #[test]
    fn entry_point_existing_absolute_script_is_accepted() {
        // A real file (this crate's Cargo.toml) standing in for a present
        // entry-point: an interpreter invocation pointing at it must pass.
        let existing = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml").to_string();
        assert!(validate_entry_point_exists("python", &[existing]).is_ok());
    }

    #[test]
    fn entry_point_module_invocation_is_skipped() {
        // `python -m cpersona` (PyPI install) resolves to the bare module name
        // "cpersona", which is not a filesystem path — must not be rejected.
        let args = vec!["-m".to_string(), "cpersona".to_string()];
        assert!(validate_entry_point_exists("python", &args).is_ok());
    }

    #[test]
    fn entry_point_relative_script_is_skipped() {
        // Relative scripts depend on the child cwd; not checkable here.
        let args = vec!["server.py".to_string()];
        assert!(validate_entry_point_exists("python", &args).is_ok());
    }

    // ── process-group reaping (orphan-leak fix, Step 3) ──

    /// Poll `kill(pid, 0)` until the process is gone (reaped by init) or the
    /// deadline elapses. Returns true if it terminated.
    #[cfg(unix)]
    async fn wait_gone(pid: libc::pid_t, within: std::time::Duration) -> bool {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            // SAFETY: signal 0 only probes existence/permission, sends nothing.
            if unsafe { libc::kill(pid, 0) } != 0 {
                return true; // ESRCH — process no longer exists
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// The core RC3 guarantee: a helper process the server forks (a grandchild)
    /// must be reaped when the server is killed. Because each server is spawned as
    /// its own process-group leader and `kill_and_wait` signals the whole group,
    /// the grandchild dies too — a bare single-PID kill would leave it orphaned.
    #[cfg(unix)]
    #[tokio::test]
    async fn kill_and_wait_reaps_the_whole_process_group() {
        // python launcher that forks a long-lived `sleep 600` grandchild, prints
        // its PID to stderr, then idles. Killing only the direct (python) PID
        // would strand the sleep; the process-group kill must take it out.
        let script = "import sys,subprocess,time;\
                      p=subprocess.Popen(['sleep','600']);\
                      sys.stderr.write(str(p.pid)+chr(10));sys.stderr.flush();\
                      time.sleep(600)";
        let (tx, mut rx) = mpsc::channel::<String>(16);
        let started = StdioTransport::start(
            "python3",
            &["-c".to_string(), script.to_string()],
            &HashMap::new(),
            None,
            0,
            &[],
            Some(tx),
        )
        .await;
        // Skip gracefully where python3 is unavailable rather than fail spuriously.
        let Ok(mut transport) = started else {
            eprintln!("skipping: python3 unavailable for process-group test");
            return;
        };

        // First stderr line is the grandchild PID.
        let line = match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
            Ok(Some(l)) => l,
            _ => {
                eprintln!("skipping: no grandchild PID line from python3");
                return;
            }
        };
        let grandchild: libc::pid_t = line.trim().parse().expect("grandchild pid");

        // Precondition: the grandchild is alive before we kill.
        assert_eq!(
            unsafe { libc::kill(grandchild, 0) },
            0,
            "grandchild {grandchild} should be alive before kill_and_wait"
        );

        transport.kill_and_wait().await;

        assert!(
            wait_gone(grandchild, std::time::Duration::from_secs(5)).await,
            "grandchild {grandchild} survived kill_and_wait — process-group reap failed (orphan leak)"
        );
    }
}
