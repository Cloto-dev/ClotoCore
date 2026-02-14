use anyhow::{Context, Result, bail};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

/// Allowed commands for MCP server execution (security whitelist)
const ALLOWED_COMMANDS: &[&str] = &[
    "npx",
    "node",
    "python",
    "python3",
    "deno",
    "bun",
];

/// Validate command against whitelist (bare command names only, no paths)
fn validate_command(command: &str) -> Result<String> {
    // Reject any command containing path separators to prevent path-based bypass
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
    _child: Child,
    request_tx: mpsc::Sender<String>,
    response_rx: mpsc::Receiver<String>,
}

impl StdioTransport {
    pub async fn start(command: &str, args: &[String]) -> Result<Self> {
        info!("🔌 Starting MCP Server: {} {:?}", command, args);

        // Security: Validate command against whitelist
        let validated_command = validate_command(command)
            .context("Command validation failed")?;

        let mut child = Command::new(validated_command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true) // Guardrail: Clean up process on drop
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
                // MCP requires messages to be line-delimited
                let line = format!("{}\n", msg);
                if let Err(e) = writer.write_all(line.as_bytes()).await {
                    error!("❌ Failed to write to MCP server stdin: {}", e);
                    break;
                }
                if let Err(e) = writer.flush().await {
                    error!("❌ Failed to flush MCP server stdin: {}", e);
                    break;
                }
            }
        });

        // Reader Task (Stdout)
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if !line.trim().is_empty() && res_tx.send(line).await.is_err() {
                    break; // Channel closed
                }
            }
            warn!("🔌 MCP Server stdout closed.");
        });

        // Logger Task (Stderr)
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                warn!("[MCP Stderr] {}", line);
            }
        });

        Ok(Self {
            _child: child,
            request_tx: req_tx,
            response_rx: res_rx,
        })
    }

    pub async fn send(&self, msg: String) -> Result<()> {
        self.request_tx.send(msg).await.context("Failed to send message to transport task")
    }

    pub async fn recv(&mut self) -> Option<String> {
        self.response_rx.recv().await
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
        // Malicious commands should be blocked
        assert!(validate_command("bash").is_err());
        assert!(validate_command("sh").is_err());
        assert!(validate_command("cmd").is_err());
        assert!(validate_command("powershell").is_err());
        assert!(validate_command("/bin/sh").is_err());
        assert!(validate_command("../../../bin/sh").is_err());
    }

    #[test]
    fn test_validate_command_rejects_paths() {
        // Paths should be rejected to prevent whitelist bypass
        assert!(validate_command("/usr/bin/node").is_err());
        assert!(validate_command("../../../bin/node").is_err());
        assert!(validate_command("C:\\Windows\\node").is_err());
    }
}
