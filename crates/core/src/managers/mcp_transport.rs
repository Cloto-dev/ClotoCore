use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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
const ALLOWED_COMMANDS: &[&str] = &["npx", "node", "python", "python3", "deno", "bun"];

/// Buffer size for MCP stdio channel (request and response).
const MCP_CHANNEL_BUFFER_SIZE: usize = 100;

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

pub struct StdioTransport {
    child: Child,
    request_tx: mpsc::Sender<String>,
    response_rx: mpsc::Receiver<String>,
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // P9: Ensure child process is killed on transport drop
        let _ = self.child.start_kill();
        debug!("StdioTransport dropped — child process kill signal sent");
    }
}

impl StdioTransport {
    /// Kill the child process and wait for it to actually exit.
    /// Ensures file locks (DB, ports) are released before returning.
    pub async fn kill_and_wait(&mut self) {
        let _ = self.child.start_kill();
        match tokio::time::timeout(std::time::Duration::from_secs(5), self.child.wait()).await {
            Ok(Ok(status)) => {
                debug!("Child process exited: {status}");
            }
            Ok(Err(e)) => {
                debug!("Child process wait error: {e}");
            }
            Err(_) => {
                tracing::warn!("Child process did not exit within 5s after kill signal");
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
    ) -> Result<Self> {
        info!("Starting MCP Server: {} {:?}", command, args);

        // Resolve python/python3 to venv Python if available
        let (resolved, is_venv) = resolve_python_command(command);
        let final_command = if is_venv {
            resolved // Venv path is internally generated — skip whitelist validation
        } else {
            validate_command(command).context("Command validation failed")?
        };

        let mut cmd = Command::new(&final_command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

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
            while let Some(msg) = req_rx.recv().await {
                let line = format!("{}\n", msg);
                if let Err(e) = writer.write_all(line.as_bytes()).await {
                    error!("Failed to write to MCP server stdin: {}", e);
                    break;
                }
                if let Err(e) = writer.flush().await {
                    error!("Failed to flush MCP server stdin: {}", e);
                    break;
                }
            }
        });

        // Reader Task (Stdout) — runs until EOF or read error.
        // No silence timeout: MCP servers are idle between tool calls and may
        // produce no output for extended periods. The health check system
        // (mcp_health.rs) handles frozen server detection separately.
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            loop {
                match reader.next_line().await {
                    Ok(Some(line)) => {
                        if !line.trim().is_empty() && res_tx.send(line).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => break, // EOF
                    Err(e) => {
                        warn!("MCP stdout read error: {}", e);
                        break;
                    }
                }
            }
            warn!("MCP Server stdout closed.");
        });

        // Logger Task (Stderr) — warn level so it's visible in release builds
        let cmd_display = command.to_string();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                warn!("[MCP:{}] {}", cmd_display, line);
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
    pub async fn start(url: &str, auth_token: Option<&str>) -> Result<Self> {
        // URL validation: HTTPS required for non-localhost
        validate_http_url(url)?;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
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
                if let Some(ref sid) = *session_id
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                {
                    request = request.header("Mcp-Session-Id", sid.clone());
                }

                let response = match request.body(msg).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        error!(url = %url_owned, error = %e, "HTTP request failed");
                        // Send JSON-RPC error so pending requests don't hang
                        let err_msg = serde_json::json!({
                            "jsonrpc": "2.0",
                            "error": {"code": -32000, "message": format!("HTTP transport error: {e}")},
                            "id": null
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
                        "id": null
                    });
                    let _ = res_tx.send(err_msg.to_string()).await;
                    continue;
                }

                // Track session ID from response header
                if let Some(sid) = response.headers().get("mcp-session-id") {
                    if let Ok(sid_str) = sid.to_str() {
                        let mut guard = session_id
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        *guard = Some(sid_str.to_string());
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

        // Process complete lines from the buffer
        while let Some(newline_pos) = buffer.iter().position(|&b| b == b'\n') {
            let line_bytes = buffer[..newline_pos].to_vec();
            buffer = buffer[newline_pos + 1..].to_vec();

            let line = String::from_utf8_lossy(&line_bytes);
            let line = line.trim_end_matches('\r');

            if let Some(data) = line.strip_prefix("data: ") {
                if !data.is_empty() {
                    let _ = res_tx.send(data.to_string()).await;
                }
            }
            // Skip event:, id:, retry:, and comment (:) lines
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
}
