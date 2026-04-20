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

#[allow(clippy::struct_excessive_bools)]
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
    pub mcp_sdk_secret: Option<String>,
    /// YOLO mode: auto-approve all permission requests (ARCHITECTURE.md §5.7).
    /// SafetyGate remains active even in YOLO mode.
    pub yolo_mode: bool,
    /// Permissions that still require approval even in YOLO mode.
    /// Default: `["filesystem.write", "network.outbound"]`.
    pub yolo_exceptions: Vec<String>,
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
    /// MCP streaming per-chunk idle timeout in seconds (MGP §12). Aborts a
    /// streaming tool call when no chunk arrives within this window, bounded
    /// above by `mcp_request_timeout_secs`. bug-351.
    pub mcp_stream_idle_timeout_secs: u64,
    /// Opt-in gate for routing `mind.*` engine calls through
    /// `call_tool_streaming`. When enabled, the agentic loop emits
    /// `ClotoEventData::AgentTokenStream` for each chunk. Default off to
    /// preserve the non-streaming path; flip to true with
    /// `CLOTO_MCP_STREAMING_ENABLED=true`.
    pub mcp_streaming_enabled: bool,
    /// MCP health check interval in seconds.
    pub mcp_health_interval_secs: u64,
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
    /// Default memory plugin ID for DB config initialization.
    /// Overridable via CLOTO_MEMORY_PLUGIN_ID env var.
    pub memory_plugin_id: String,
    /// Default API host whitelist for SafeHttpClient.
    /// Overridable via CLOTO_ALLOWED_API_HOSTS env var (comma-separated).
    pub default_allowed_api_hosts: Vec<String>,
    /// LLM provider-to-env-var mappings for API key sync.
    /// Overridable via CLOTO_LLM_ENV_MAPPINGS env var (format: "provider:ENV_VAR,...").
    pub llm_provider_env_mappings: Vec<(String, String)>,
    /// Allow unsigned MCP servers (no Magic Seal check). Default: true (development mode).
    pub allow_unsigned: bool,
    /// Master switch for OS-level isolation. Default: true.
    pub isolation_enabled: bool,
    /// Base directory for MCP server sandboxes. Default: "data/mcp-sandbox".
    pub sandbox_base_dir: PathBuf,
    /// Run a quick health scan on startup. Default: true.
    pub health_scan_on_startup: bool,
}

impl AppConfig {
    #[allow(clippy::too_many_lines)]
    pub fn load() -> anyhow::Result<Self> {
        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
            let db_path = exe_dir().join("data").join("cloto_memories.db");
            format!("sqlite:{}", db_path.display())
        });

        // Trim surrounding whitespace to survive CRLF in `.env` on Windows/macOS.
        // A stray `\r` in the value breaks HTTP header parsing (400 with empty body).
        let admin_api_key = env::var("CLOTO_API_KEY")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

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

        if max_event_depth == 0 || max_event_depth > 25 {
            anyhow::bail!(
                "MAX_EVENT_DEPTH must be between 1 and 25 (got {})",
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

        // P1: Default empty — consensus engines are configured per-deployment, not hard-coded
        let consensus_engines_str = env::var("CONSENSUS_ENGINES").unwrap_or_default();
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

        let mcp_sdk_secret = env::var("CLOTO_SDK_SECRET").ok();
        let yolo_mode = env::var("CLOTO_YOLO")
            .unwrap_or_else(|_| "false".to_string())
            .parse::<bool>()
            .unwrap_or(false);

        let yolo_exceptions: Vec<String> = env::var("CLOTO_YOLO_EXCEPTIONS")
            .unwrap_or_else(|_| "filesystem.write,network.outbound".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if yolo_mode {
            tracing::warn!("YOLO mode enabled: MCP server permissions will be auto-approved");
            if !yolo_exceptions.is_empty() {
                tracing::info!(
                    exceptions = ?yolo_exceptions,
                    "YOLO exceptions: these permissions still require approval"
                );
            }
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

        let mcp_stream_idle_timeout_secs = env::var("CLOTO_MCP_STREAM_IDLE_TIMEOUT_SECS")
            .unwrap_or_else(|_| "30".to_string())
            .parse::<u64>()
            .context("Failed to parse CLOTO_MCP_STREAM_IDLE_TIMEOUT_SECS")?;
        if !(5..=300).contains(&mcp_stream_idle_timeout_secs) {
            anyhow::bail!(
                "CLOTO_MCP_STREAM_IDLE_TIMEOUT_SECS must be between 5 and 300 (got {})",
                mcp_stream_idle_timeout_secs
            );
        }

        let mcp_streaming_enabled = env::var("CLOTO_MCP_STREAMING_ENABLED")
            .map(|v| matches!(v.as_str(), "true" | "1" | "yes" | "on"))
            .unwrap_or(false);

        let mcp_health_interval_secs = env::var("CLOTO_MCP_HEALTH_INTERVAL_SECS")
            .unwrap_or_else(|_| "10".to_string())
            .parse::<u64>()
            .context("Failed to parse CLOTO_MCP_HEALTH_INTERVAL_SECS")?;
        if !(5..=60).contains(&mcp_health_interval_secs) {
            anyhow::bail!(
                "CLOTO_MCP_HEALTH_INTERVAL_SECS must be between 5 and 60 (got {})",
                mcp_health_interval_secs
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
            .unwrap_or_else(|_| "50".to_string())
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

        let memory_plugin_id =
            env::var("CLOTO_MEMORY_PLUGIN_ID").unwrap_or_else(|_| "memory.cpersona".to_string());

        // P1: Configurable API host whitelist (default providers included for compatibility)
        let default_allowed_api_hosts = env::var("CLOTO_ALLOWED_API_HOSTS").map_or_else(
            |_| {
                vec![
                    "api.deepseek.com".to_string(),
                    "api.cerebras.ai".to_string(),
                    "api.openai.com".to_string(),
                    "api.anthropic.com".to_string(),
                ]
            },
            |s| s.split(',').map(|h| h.trim().to_string()).collect(),
        );

        // P1: Configurable LLM provider-to-env-var mappings
        let llm_provider_env_mappings = env::var("CLOTO_LLM_ENV_MAPPINGS").map_or_else(
            |_| {
                vec![
                    ("deepseek".to_string(), "DEEPSEEK_API_KEY".to_string()),
                    ("cerebras".to_string(), "CEREBRAS_API_KEY".to_string()),
                    ("claude".to_string(), "CLAUDE_API_KEY".to_string()),
                    ("ollama".to_string(), "OLLAMA_API_KEY".to_string()),
                ]
            },
            |s| {
                s.split(',')
                    .filter_map(|pair| {
                        let parts: Vec<&str> = pair.trim().splitn(2, ':').collect();
                        if parts.len() == 2 {
                            Some((parts[0].to_string(), parts[1].to_string()))
                        } else {
                            None
                        }
                    })
                    .collect()
            },
        );

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
            mcp_sdk_secret,
            yolo_mode,
            yolo_exceptions,
            cron_enabled,
            cron_check_interval_secs,
            llm_proxy_port,
            db_timeout_secs,
            memory_timeout_secs,
            heartbeat_threshold_ms,
            mcp_request_timeout_secs,
            mcp_stream_idle_timeout_secs,
            mcp_streaming_enabled,
            mcp_health_interval_secs,
            llm_proxy_timeout_secs,
            rate_limit_per_sec,
            rate_limit_burst,
            max_event_history,
            event_concurrency_limit,
            max_chat_query_limit,
            attachment_inline_threshold,
            cron_default_max_iterations,
            memory_plugin_id,
            default_allowed_api_hosts,
            llm_provider_env_mappings,
            // Magic Seal: block unsigned MCP servers at Untrusted trust level.
            // Core/Standard/Experimental servers are always allowed without a seal.
            // Only affects Untrusted servers — set to true if you register Untrusted servers in dev.
            allow_unsigned: env::var("CLOTO_ALLOW_UNSIGNED")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
            isolation_enabled: env::var("CLOTO_ISOLATION_ENABLED")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true), // Default: true
            sandbox_base_dir: env::var("CLOTO_SANDBOX_DIR")
                .map_or_else(|_| PathBuf::from("data/mcp-sandbox"), PathBuf::from),
            health_scan_on_startup: env::var("CLOTO_HEALTH_SCAN_ON_STARTUP")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true),
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
        // P1: Default is empty (no hard-coded engines)
        assert!(config.consensus_engines.is_empty());
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

    #[test]
    fn test_admin_api_key_strips_crlf() {
        let _lock = ENV_LOCK.lock().unwrap();
        // A CR at the end is the symptom when .env is saved as CRLF on macOS.
        std::env::set_var(
            "CLOTO_API_KEY",
            "d6b705613200449d6c9e08ecf218b0571742937c9575c26982c5be29b10443f3\r",
        );
        let _guard = EnvGuard("CLOTO_API_KEY");

        let config = AppConfig::load().unwrap();
        let key = config.admin_api_key.expect("CLOTO_API_KEY must be loaded");
        assert!(
            !key.contains('\r'),
            "CR must be stripped from admin_api_key"
        );
        assert!(
            !key.contains('\n'),
            "LF must be stripped from admin_api_key"
        );
        assert_eq!(key.len(), 64, "64-hex key must survive trim");
    }

    #[test]
    fn test_admin_api_key_blank_is_none() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::set_var("CLOTO_API_KEY", "   \t\r\n  ");
        let _guard = EnvGuard("CLOTO_API_KEY");

        let config = AppConfig::load().unwrap();
        assert!(
            config.admin_api_key.is_none(),
            "whitespace-only CLOTO_API_KEY must collapse to None"
        );
    }
}
