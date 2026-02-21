mod sandbox;

use async_trait::async_trait;
use exiv_shared::{
    exiv_plugin, ExivEvent, ExivEventData, Permission, Plugin, PluginConfig, PluginRuntimeContext,
};
use std::sync::Arc;
use tokio::sync::RwLock;

#[exiv_plugin(
    name = "tool.terminal",
    kind = "Skill",
    description = "Executes shell commands in a sandboxed environment.",
    version = "0.1.0",
    category = "Tool",
    config_keys = ["working_dir", "max_output_bytes"],
    permissions = ["ProcessExecution"],
    capabilities = ["Tool"],
    tags = ["#TOOL", "#TERMINAL"]
)]
pub struct TerminalPlugin {
    state: Arc<RwLock<TerminalState>>,
}

struct TerminalState {
    working_dir: String,
    max_output_bytes: usize,
    command_allowlist: Option<Vec<String>>,
    initialized: bool,
}

/// Safely truncate a UTF-8 string at a byte boundary.
/// Returns a slice no longer than `max_bytes` that ends at a valid char boundary.
fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if max_bytes >= s.len() {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

impl TerminalPlugin {
    pub async fn new_plugin(config: PluginConfig) -> anyhow::Result<Self> {
        let working_dir = config
            .config_values
            .get("working_dir")
            .cloned()
            .unwrap_or_else(|| "/tmp/exiv-sandbox".to_string());
        let max_output_bytes = config
            .config_values
            .get("max_output_bytes")
            .and_then(|v| v.parse().ok())
            .unwrap_or(65536); // 64KB default
        let command_allowlist = config
            .config_values
            .get("allowed_commands")
            .map(|v| v.split(',').map(|s| s.trim().to_string()).collect());

        Ok(Self {
            state: Arc::new(RwLock::new(TerminalState {
                working_dir,
                max_output_bytes,
                command_allowlist,
                initialized: false,
            })),
        })
    }
}

#[async_trait]
impl Plugin for TerminalPlugin {
    fn manifest(&self) -> exiv_shared::PluginManifest {
        self.auto_manifest()
    }

    async fn on_plugin_init(
        &self,
        context: PluginRuntimeContext,
        _network: Option<Arc<dyn exiv_shared::NetworkCapability>>,
    ) -> anyhow::Result<()> {
        if !context
            .effective_permissions
            .contains(&Permission::ProcessExecution)
        {
            tracing::error!(
                "🚫 tool.terminal requires ProcessExecution permission. Plugin will not function."
            );
            return Ok(());
        }

        let mut state = self.state.write().await;
        // Ensure working directory exists
        if let Err(e) = std::fs::create_dir_all(&state.working_dir) {
            tracing::warn!(
                "⚠️ Could not create working dir '{}': {}",
                state.working_dir,
                e
            );
        }
        // C-03: Canonicalize working_dir to prevent path traversal
        match std::fs::canonicalize(&state.working_dir) {
            Ok(canonical) => {
                state.working_dir = canonical.to_string_lossy().to_string();
            }
            Err(e) => {
                tracing::warn!("⚠️ Could not canonicalize working dir: {}", e);
            }
        }
        state.initialized = true;
        tracing::info!(
            "🖥️ Terminal plugin initialized. Working dir: {}",
            state.working_dir
        );
        Ok(())
    }

    async fn on_event(&self, _event: &ExivEvent) -> anyhow::Result<Option<ExivEventData>> {
        Ok(None)
    }
}

#[async_trait]
impl exiv_shared::Tool for TerminalPlugin {
    fn name(&self) -> &str {
        "terminal"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return stdout, stderr, and exit code. \
         Use this to run scripts, check file contents, inspect system state, \
         compile code, run tests, or perform any command-line operation."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30, max: 120)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let state = self.state.read().await;
        if !state.initialized {
            return Err(anyhow::anyhow!(
                "Terminal plugin not initialized (missing ProcessExecution permission)"
            ));
        }

        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'command' argument"))?;

        // C-03: Always use the configured working_dir (no user override)
        let working_dir = &state.working_dir;

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(30)
            .min(120);

        // Validate command against sandbox rules
        sandbox::validate_command(command, &state.command_allowlist)?;

        tracing::info!("🖥️ Executing: {}", command);

        // M-02: Use spawn + kill_on_drop to ensure child process is killed on timeout
        let child = tokio::process::Command::new("sh")
            .args(["-c", command])
            .current_dir(working_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn();

        let output = match child {
            Ok(child) => {
                tokio::time::timeout(
                    std::time::Duration::from_secs(timeout_secs),
                    child.wait_with_output(),
                )
                .await
            }
            Err(e) => Ok(Err(e)),
        };

        match output {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let max = state.max_output_bytes;

                // C-01: Safe UTF-8 truncation (never panic on multi-byte boundary)
                let stdout_str = if stdout.len() > max {
                    format!(
                        "{}...[truncated, {} bytes total]",
                        safe_truncate(&stdout, max),
                        stdout.len()
                    )
                } else {
                    stdout.to_string()
                };
                let stderr_str = if stderr.len() > max {
                    format!(
                        "{}...[truncated, {} bytes total]",
                        safe_truncate(&stderr, max),
                        stderr.len()
                    )
                } else {
                    stderr.to_string()
                };

                Ok(serde_json::json!({
                    "exit_code": output.status.code().unwrap_or(-1),
                    "stdout": stdout_str,
                    "stderr": stderr_str
                }))
            }
            Ok(Err(e)) => Ok(serde_json::json!({
                "exit_code": -1,
                "stdout": "",
                "stderr": format!("Failed to execute command: {}", e)
            })),
            Err(_) => Ok(serde_json::json!({
                "exit_code": -1,
                "stdout": "",
                "stderr": format!("Command timed out after {} seconds", timeout_secs)
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_truncate_ascii() {
        assert_eq!(safe_truncate("hello", 3), "hel");
        assert_eq!(safe_truncate("hello", 10), "hello");
        assert_eq!(safe_truncate("hello", 5), "hello");
    }

    #[test]
    fn test_safe_truncate_multibyte() {
        // "あいう" = 9 bytes (3 bytes per char)
        let s = "あいう";
        assert_eq!(safe_truncate(s, 9), "あいう");
        assert_eq!(safe_truncate(s, 6), "あい");
        assert_eq!(safe_truncate(s, 5), "あ"); // can't split い (bytes 3-5)
        assert_eq!(safe_truncate(s, 4), "あ");
        assert_eq!(safe_truncate(s, 3), "あ");
        assert_eq!(safe_truncate(s, 2), ""); // can't fit even one char
        assert_eq!(safe_truncate(s, 0), "");
    }

    #[test]
    fn test_safe_truncate_mixed() {
        // "a€b" = 1 + 3 + 1 = 5 bytes
        let s = "a€b";
        assert_eq!(safe_truncate(s, 5), "a€b");
        assert_eq!(safe_truncate(s, 4), "a€");
        assert_eq!(safe_truncate(s, 3), "a"); // byte 1-3 is inside €
        assert_eq!(safe_truncate(s, 2), "a");
        assert_eq!(safe_truncate(s, 1), "a");
    }
}
