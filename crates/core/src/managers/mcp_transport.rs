use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::mcp_isolation::{FilesystemScope, IsolationProfile, NetworkScope};

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
    if normalized.starts_with("target/debug/mgp-") || normalized.starts_with("target/release/mgp-") {
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

        // Reader Task (Stdout)
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if !line.trim().is_empty() && res_tx.send(line).await.is_err() {
                    break;
                }
            }
            warn!("MCP Server stdout closed.");
        });

        // Logger Task (Stderr)
        let cmd_display = command.to_string();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                debug!("[MCP:{}] {}", cmd_display, line);
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
