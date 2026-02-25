use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

/// Allowed commands for MCP server execution (security whitelist)
const ALLOWED_COMMANDS: &[&str] = &["npx", "node", "python", "python3", "deno", "bun"];

/// Validate command against whitelist (bare command names only, no paths)
pub fn validate_command(command: &str) -> Result<String> {
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

impl StdioTransport {
    /// Get a clone of the request sender for lock-free sending.
    #[must_use]
    pub fn sender(&self) -> mpsc::Sender<String> {
        self.request_tx.clone()
    }

    /// Start a new MCP server process with environment variable injection.
    pub async fn start(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self> {
        info!("Starting MCP Server: {} {:?}", command, args);

        let validated_command = validate_command(command).context("Command validation failed")?;

        let mut cmd = Command::new(validated_command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Inject environment variables (with shell variable expansion)
        for (key, value) in env {
            let resolved = resolve_env_value(value);
            cmd.env(key, resolved);
        }

        let mut child = cmd
            .spawn()
            .context(format!("Failed to spawn MCP server: {}", command))?;

        let stdin = child.stdin.take().context("Failed to open stdin")?;
        let stdout = child.stdout.take().context("Failed to open stdout")?;
        let stderr = child.stderr.take().context("Failed to open stderr")?;

        let (req_tx, mut req_rx) = mpsc::channel::<String>(100);
        let (res_tx, res_rx) = mpsc::channel::<String>(100);

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
