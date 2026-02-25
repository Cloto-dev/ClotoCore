use anyhow::Context;
use axum::http::HeaderValue;
use std::env;
use std::path::PathBuf;

/// Returns the directory containing the running executable.
/// Falls back to CWD if the exe path cannot be determined.
#[must_use]
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[derive(Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub port: u16,
    pub bind_address: String,
    pub cors_origins: Vec<HeaderValue>,
    pub default_agent_id: String,
    pub allowed_hosts: Vec<String>,
    pub plugin_event_timeout_secs: u64,
    pub max_event_depth: u8,
    pub memory_context_limit: usize,
    pub admin_api_key: Option<String>,
    pub consensus_engines: Vec<String>,
    pub event_history_size: usize,
    pub event_retention_hours: u64,
    pub max_agentic_iterations: u8,
    pub tool_execution_timeout_secs: u64,
    pub mcp_config_path: Option<String>,
    pub mcp_sdk_secret: Option<String>,
    /// YOLO mode: auto-approve all permission requests (ARCHITECTURE.md §5.7).
    /// SafetyGate remains active even in YOLO mode.
    pub yolo_mode: bool,
}

impl AppConfig {
    #[allow(clippy::too_many_lines)]
    pub fn load() -> anyhow::Result<Self> {
        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
            let db_path = exe_dir().join("data").join("cloto_memories.db");
            format!("sqlite:{}", db_path.display())
        });

        let admin_api_key = env::var("CLOTO_API_KEY").ok();

        if let Some(ref key) = admin_api_key {
            if key.len() < 32 {
                tracing::warn!("CLOTO_API_KEY is shorter than recommended minimum (32 chars)");
            }
        }

        let default_agent_id =
            env::var("DEFAULT_AGENT_ID").unwrap_or_else(|_| "agent.cloto_default".to_string());

        let plugin_event_timeout_secs = env::var("PLUGIN_EVENT_TIMEOUT_SECS")
            .unwrap_or_else(|_| "30".to_string())
            .parse::<u64>()
            .context("Failed to parse PLUGIN_EVENT_TIMEOUT_SECS")?;

        // M-01: Value range validation
        if plugin_event_timeout_secs == 0 || plugin_event_timeout_secs > 300 {
            anyhow::bail!(
                "PLUGIN_EVENT_TIMEOUT_SECS must be between 1 and 300 (got {})",
                plugin_event_timeout_secs
            );
        }

        let max_event_depth = env::var("MAX_EVENT_DEPTH")
            .unwrap_or_else(|_| "10".to_string())
            .parse::<u8>()
            .context("Failed to parse MAX_EVENT_DEPTH")?;

        if max_event_depth == 0 || max_event_depth > 50 {
            anyhow::bail!(
                "MAX_EVENT_DEPTH must be between 1 and 50 (got {})",
                max_event_depth
            );
        }

        let memory_context_limit = env::var("MEMORY_CONTEXT_LIMIT")
            .unwrap_or_else(|_| "10".to_string())
            .parse::<usize>()
            .context("Failed to parse MEMORY_CONTEXT_LIMIT")?;

        let port_str = env::var("PORT").unwrap_or_else(|_| "8081".to_string());
        let port = port_str.parse::<u16>().map_err(|_| {
            anyhow::anyhow!(
                "Invalid PORT value '{}': must be an integer between 1 and 65535",
                port_str
            )
        })?;

        if port == 0 {
            anyhow::bail!("Invalid PORT value '0': must be between 1 and 65535");
        }

        // BIND_ADDRESS: defaults to 127.0.0.1 (loopback only) for safety.
        // Set to 0.0.0.0 explicitly in .env if network access from other hosts is required.
        let bind_address = match env::var("BIND_ADDRESS") {
            Ok(addr) => {
                addr.parse::<std::net::IpAddr>()
                    .with_context(|| format!(
                        "Invalid BIND_ADDRESS '{}': must be a valid IP address (e.g., '127.0.0.1' or '::1')",
                        addr
                    ))?;
                addr
            }
            Err(_) => "127.0.0.1".to_string(),
        };

        let cors_origins_str = env::var("CORS_ORIGINS")
            .unwrap_or_else(|_| "http://localhost:5173,http://127.0.0.1:5173".to_string());

        // M-02: Skip invalid CORS origins with warning instead of failing entirely
        let cors_origins: Vec<HeaderValue> = cors_origins_str
            .split(',')
            .filter_map(|s| {
                let trimmed = s.trim();
                // Reject non-HTTP(S) schemes (prevent file://, javascript://, data://)
                if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
                    tracing::warn!("Skipping CORS origin with invalid scheme '{}': must be http:// or https://", trimmed);
                    return None;
                }
                match trimmed.parse::<HeaderValue>() {
                    Ok(v) => Some(v),
                    Err(e) => {
                        tracing::warn!("Skipping invalid CORS origin '{}': {}", trimmed, e);
                        None
                    }
                }
            })
            .collect();

        let allowed_hosts_str = env::var("ALLOWED_HOSTS").unwrap_or_default();
        let allowed_hosts = if allowed_hosts_str.is_empty() {
            vec![]
        } else {
            allowed_hosts_str
                .split(',')
                .map(std::string::ToString::to_string)
                .collect()
        };

        let consensus_engines_str = env::var("CONSENSUS_ENGINES")
            .unwrap_or_else(|_| "mind.deepseek,mind.cerebras".to_string());
        let consensus_engines = consensus_engines_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let event_history_size = env::var("EVENT_HISTORY_SIZE")
            .unwrap_or_else(|_| "1000".to_string())
            .parse::<usize>()
            .context("Failed to parse EVENT_HISTORY_SIZE")?;

        // M-10: Configurable event retention period (default 24 hours)
        let event_retention_hours = env::var("EVENT_RETENTION_HOURS")
            .unwrap_or_else(|_| "24".to_string())
            .parse::<u64>()
            .context("Failed to parse EVENT_RETENTION_HOURS")?;

        if event_retention_hours == 0 || event_retention_hours > 720 {
            anyhow::bail!(
                "EVENT_RETENTION_HOURS must be between 1 and 720 (got {})",
                event_retention_hours
            );
        }

        let max_agentic_iterations = env::var("CLOTO_MAX_AGENTIC_ITERATIONS")
            .unwrap_or_else(|_| "16".to_string())
            .parse::<u8>()
            .context("Failed to parse CLOTO_MAX_AGENTIC_ITERATIONS")?;

        if max_agentic_iterations == 0 || max_agentic_iterations > 64 {
            anyhow::bail!(
                "CLOTO_MAX_AGENTIC_ITERATIONS must be between 1 and 64 (got {})",
                max_agentic_iterations
            );
        }

        let tool_execution_timeout_secs = env::var("CLOTO_TOOL_TIMEOUT_SECS")
            .unwrap_or_else(|_| "30".to_string())
            .parse::<u64>()
            .context("Failed to parse CLOTO_TOOL_TIMEOUT_SECS")?;

        if tool_execution_timeout_secs == 0 || tool_execution_timeout_secs > 300 {
            anyhow::bail!(
                "CLOTO_TOOL_TIMEOUT_SECS must be between 1 and 300 (got {})",
                tool_execution_timeout_secs
            );
        }

        let mcp_config_path = env::var("CLOTO_MCP_CONFIG").ok();
        let mcp_sdk_secret = env::var("CLOTO_SDK_SECRET").ok();
        let yolo_mode = env::var("CLOTO_YOLO")
            .unwrap_or_else(|_| "false".to_string())
            .parse::<bool>()
            .unwrap_or(false);

        if yolo_mode {
            tracing::warn!("YOLO mode enabled: MCP server permissions will be auto-approved");
        }

        Ok(Self {
            database_url,
            port,
            bind_address,
            cors_origins,
            default_agent_id,
            allowed_hosts,
            plugin_event_timeout_secs,
            max_event_depth,
            memory_context_limit,
            admin_api_key,
            consensus_engines,
            event_history_size,
            event_retention_hours,
            max_agentic_iterations,
            tool_execution_timeout_secs,
            mcp_config_path,
            mcp_sdk_secret,
            yolo_mode,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Mutex to ensure env var tests run serially (prevents parallel test interference)
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // Guard to ensure env var cleanup even on panic
    struct EnvGuard(&'static str);

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var(self.0);
        }
    }

    #[test]
    fn test_consensus_engines_parsing() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::set_var("CONSENSUS_ENGINES", "mind.deepseek,mind.anthropic");
        let _guard = EnvGuard("CONSENSUS_ENGINES");

        let config = AppConfig::load().unwrap();
        assert_eq!(config.consensus_engines.len(), 2);
        assert_eq!(config.consensus_engines[0], "mind.deepseek");
        assert_eq!(config.consensus_engines[1], "mind.anthropic");
    }

    #[test]
    fn test_consensus_engines_default() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard("CONSENSUS_ENGINES");

        let config = AppConfig::load().unwrap();
        assert_eq!(
            config.consensus_engines,
            vec!["mind.deepseek", "mind.cerebras"]
        );
    }

    #[test]
    fn test_consensus_engines_whitespace_handling() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::set_var(
            "CONSENSUS_ENGINES",
            " mind.deepseek , mind.anthropic , mind.openai ",
        );
        let _guard = EnvGuard("CONSENSUS_ENGINES");

        let config = AppConfig::load().unwrap();
        assert_eq!(config.consensus_engines.len(), 3);
        assert_eq!(config.consensus_engines[0], "mind.deepseek");
        assert_eq!(config.consensus_engines[1], "mind.anthropic");
        assert_eq!(config.consensus_engines[2], "mind.openai");
    }
}
