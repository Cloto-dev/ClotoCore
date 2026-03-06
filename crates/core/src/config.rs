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
    /// Enable cron job scheduler (Layer 2: Autonomous Trigger).
    pub cron_enabled: bool,
    /// How often (seconds) the scheduler checks for due jobs.
    pub cron_check_interval_secs: u64,
    /// Port for internal LLM proxy (MGP §13.4).
    pub llm_proxy_port: u16,
    /// Database operation timeout in seconds.
    pub db_timeout_secs: u64,
    /// Memory retrieval timeout in seconds.
    pub memory_timeout_secs: u64,
    /// Agent heartbeat threshold in milliseconds.
    pub heartbeat_threshold_ms: i64,
    /// MCP request timeout in seconds.
    pub mcp_request_timeout_secs: u64,
    /// LLM proxy HTTP client timeout in seconds.
    pub llm_proxy_timeout_secs: u64,
    /// Rate limiter: requests per second.
    pub rate_limit_per_sec: u32,
    /// Rate limiter: burst size.
    pub rate_limit_burst: u32,
    /// Maximum event history ring buffer size.
    pub max_event_history: usize,
    /// Event processing concurrency limit.
    pub event_concurrency_limit: usize,
    /// Maximum chat query limit per request.
    pub max_chat_query_limit: i64,
    /// Attachment inline threshold in bytes.
    pub attachment_inline_threshold: usize,
    /// Default max iterations for cron jobs.
    pub cron_default_max_iterations: u8,
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
            .unwrap_or_else(|_| "120".to_string())
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

        let cron_enabled = env::var("CLOTO_CRON_ENABLED")
            .unwrap_or_else(|_| "true".to_string())
            .parse::<bool>()
            .unwrap_or(true);
        let cron_check_interval_secs = env::var("CLOTO_CRON_INTERVAL")
            .unwrap_or_else(|_| "60".to_string())
            .parse::<u64>()
            .unwrap_or(60)
            .max(10); // minimum 10 seconds

        let llm_proxy_port = env::var("CLOTO_LLM_PROXY_PORT")
            .unwrap_or_else(|_| "8082".to_string())
            .parse::<u16>()
            .unwrap_or(8082);

        let db_timeout_secs = env::var("CLOTO_DB_TIMEOUT_SECS")
            .unwrap_or_else(|_| "10".to_string())
            .parse::<u64>()
            .context("Failed to parse CLOTO_DB_TIMEOUT_SECS")?;
        if db_timeout_secs == 0 || db_timeout_secs > 120 {
            anyhow::bail!(
                "CLOTO_DB_TIMEOUT_SECS must be between 1 and 120 (got {})",
                db_timeout_secs
            );
        }

        let memory_timeout_secs = env::var("CLOTO_MEMORY_TIMEOUT_SECS")
            .unwrap_or_else(|_| "5".to_string())
            .parse::<u64>()
            .context("Failed to parse CLOTO_MEMORY_TIMEOUT_SECS")?;
        if memory_timeout_secs == 0 || memory_timeout_secs > 60 {
            anyhow::bail!(
                "CLOTO_MEMORY_TIMEOUT_SECS must be between 1 and 60 (got {})",
                memory_timeout_secs
            );
        }

        let heartbeat_threshold_ms = env::var("CLOTO_HEARTBEAT_THRESHOLD_MS")
            .unwrap_or_else(|_| "90000".to_string())
            .parse::<i64>()
            .context("Failed to parse CLOTO_HEARTBEAT_THRESHOLD_MS")?;
        if !(10_000..=600_000).contains(&heartbeat_threshold_ms) {
            anyhow::bail!(
                "CLOTO_HEARTBEAT_THRESHOLD_MS must be between 10000 and 600000 (got {})",
                heartbeat_threshold_ms
            );
        }

        let mcp_request_timeout_secs = env::var("CLOTO_MCP_REQUEST_TIMEOUT_SECS")
            .unwrap_or_else(|_| "120".to_string())
            .parse::<u64>()
            .context("Failed to parse CLOTO_MCP_REQUEST_TIMEOUT_SECS")?;
        if !(10..=600).contains(&mcp_request_timeout_secs) {
            anyhow::bail!(
                "CLOTO_MCP_REQUEST_TIMEOUT_SECS must be between 10 and 600 (got {})",
                mcp_request_timeout_secs
            );
        }

        let llm_proxy_timeout_secs = env::var("CLOTO_LLM_PROXY_TIMEOUT_SECS")
            .unwrap_or_else(|_| "180".to_string())
            .parse::<u64>()
            .context("Failed to parse CLOTO_LLM_PROXY_TIMEOUT_SECS")?;
        if !(30..=600).contains(&llm_proxy_timeout_secs) {
            anyhow::bail!(
                "CLOTO_LLM_PROXY_TIMEOUT_SECS must be between 30 and 600 (got {})",
                llm_proxy_timeout_secs
            );
        }

        let rate_limit_per_sec = env::var("CLOTO_RATE_LIMIT_PER_SEC")
            .unwrap_or_else(|_| "10".to_string())
            .parse::<u32>()
            .context("Failed to parse CLOTO_RATE_LIMIT_PER_SEC")?;
        if rate_limit_per_sec == 0 || rate_limit_per_sec > 1000 {
            anyhow::bail!(
                "CLOTO_RATE_LIMIT_PER_SEC must be between 1 and 1000 (got {})",
                rate_limit_per_sec
            );
        }

        let rate_limit_burst = env::var("CLOTO_RATE_LIMIT_BURST")
            .unwrap_or_else(|_| "20".to_string())
            .parse::<u32>()
            .context("Failed to parse CLOTO_RATE_LIMIT_BURST")?;
        if rate_limit_burst == 0 || rate_limit_burst > 10_000 {
            anyhow::bail!(
                "CLOTO_RATE_LIMIT_BURST must be between 1 and 10000 (got {})",
                rate_limit_burst
            );
        }

        let max_event_history = env::var("CLOTO_MAX_EVENT_HISTORY")
            .unwrap_or_else(|_| "10000".to_string())
            .parse::<usize>()
            .context("Failed to parse CLOTO_MAX_EVENT_HISTORY")?;
        if !(100..=1_000_000).contains(&max_event_history) {
            anyhow::bail!(
                "CLOTO_MAX_EVENT_HISTORY must be between 100 and 1000000 (got {})",
                max_event_history
            );
        }

        let event_concurrency_limit = env::var("CLOTO_EVENT_CONCURRENCY")
            .unwrap_or_else(|_| "50".to_string())
            .parse::<usize>()
            .context("Failed to parse CLOTO_EVENT_CONCURRENCY")?;
        if event_concurrency_limit == 0 || event_concurrency_limit > 500 {
            anyhow::bail!(
                "CLOTO_EVENT_CONCURRENCY must be between 1 and 500 (got {})",
                event_concurrency_limit
            );
        }

        let max_chat_query_limit = env::var("CLOTO_MAX_CHAT_QUERY_LIMIT")
            .unwrap_or_else(|_| "200".to_string())
            .parse::<i64>()
            .context("Failed to parse CLOTO_MAX_CHAT_QUERY_LIMIT")?;
        if !(10..=10_000).contains(&max_chat_query_limit) {
            anyhow::bail!(
                "CLOTO_MAX_CHAT_QUERY_LIMIT must be between 10 and 10000 (got {})",
                max_chat_query_limit
            );
        }

        let attachment_inline_threshold = env::var("CLOTO_ATTACHMENT_INLINE_THRESHOLD")
            .unwrap_or_else(|_| "65536".to_string())
            .parse::<usize>()
            .context("Failed to parse CLOTO_ATTACHMENT_INLINE_THRESHOLD")?;
        if attachment_inline_threshold > 10_485_760 {
            anyhow::bail!(
                "CLOTO_ATTACHMENT_INLINE_THRESHOLD must be between 0 and 10485760 (got {})",
                attachment_inline_threshold
            );
        }

        let cron_default_max_iterations = env::var("CLOTO_CRON_DEFAULT_MAX_ITERATIONS")
            .unwrap_or_else(|_| "8".to_string())
            .parse::<u8>()
            .context("Failed to parse CLOTO_CRON_DEFAULT_MAX_ITERATIONS")?;
        if cron_default_max_iterations == 0 || cron_default_max_iterations > 64 {
            anyhow::bail!(
                "CLOTO_CRON_DEFAULT_MAX_ITERATIONS must be between 1 and 64 (got {})",
                cron_default_max_iterations
            );
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
            cron_enabled,
            cron_check_interval_secs,
            llm_proxy_port,
            db_timeout_secs,
            memory_timeout_secs,
            heartbeat_threshold_ms,
            mcp_request_timeout_secs,
            llm_proxy_timeout_secs,
            rate_limit_per_sec,
            rate_limit_burst,
            max_event_history,
            event_concurrency_limit,
            max_chat_query_limit,
            attachment_inline_threshold,
            cron_default_max_iterations,
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
