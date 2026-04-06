//! ClotoCore kernel — an AI agent orchestration platform.
//!
//! Provides the Axum HTTP server, event-driven plugin system, SQLite persistence,
//! MCP server management, and the agentic loop that ties agents to reasoning engines.

pub mod capabilities;
pub mod cli;
pub mod config;
pub mod consensus;
pub mod db;
pub mod events;
pub mod handlers;
pub mod installer;
pub mod managers;
pub mod middleware;
pub mod platform;
pub mod test_utils;
pub mod viseme;

// Re-export audit log and permission request types for external use
pub use db::{
    create_permission_request, get_pending_permission_requests, is_permission_approved,
    query_audit_logs, update_permission_request, write_audit_log, AuditLogEntry, PermissionRequest,
};

/// Rate limiter stale-entry cleanup interval in seconds.
const RATE_LIMITER_CLEANUP_SECS: u64 = 600;

/// Revoked API keys TTL cleanup interval in seconds (6 hours).
const REVOKED_KEYS_CLEANUP_SECS: u64 = 21_600;

use cloto_shared::ClotoEvent;
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Notify, RwLock};

/// Context for a currently-executing CRON job (tracks generation for recursion depth).
#[derive(Debug, Clone)]
pub struct CronExecContext {
    pub job_id: String,
    pub generation: i32,
}

/// Per-agent active CRON execution contexts (agent_id → context).
pub type ActiveCronContexts = Arc<dashmap::DashMap<String, CronExecContext>>;

#[derive(Debug, Clone)]
pub struct EnvelopedEvent {
    pub event: Arc<ClotoEvent>,
    pub issuer: Option<cloto_shared::ClotoId>, // None = System/Kernel
    pub correlation_id: Option<cloto_shared::ClotoId>, // 親イベントの trace_id
    pub depth: u8,
}

impl EnvelopedEvent {
    /// Create a system-originated event (no issuer, no correlation, depth 0)
    #[must_use]
    pub fn system(data: cloto_shared::ClotoEventData) -> Self {
        Self {
            event: Arc::new(ClotoEvent::new(data)),
            issuer: None,
            correlation_id: None,
            depth: 0,
        }
    }
}

pub struct DynamicRouter {
    pub router: RwLock<axum::Router<Arc<dyn std::any::Any + Send + Sync>>>,
}

pub struct AppState {
    pub tx: broadcast::Sender<events::SequencedEvent>,
    pub registry: Arc<managers::PluginRegistry>,
    pub event_tx: mpsc::Sender<EnvelopedEvent>,
    pub pool: SqlitePool,
    pub agent_manager: managers::AgentManager,
    pub plugin_manager: Arc<managers::PluginManager>,
    pub mcp_manager: Arc<managers::McpClientManager>,
    pub dynamic_router: Arc<DynamicRouter>,
    pub config: config::AppConfig,
    pub data_dir: std::path::PathBuf,
    pub event_history: Arc<RwLock<VecDeque<events::SequencedEvent>>>,
    pub metrics: Arc<managers::SystemMetrics>,
    pub rate_limiter: Arc<middleware::RateLimiter>,
    pub shutdown: Arc<Notify>,
    /// In-memory cache of revoked API key hashes (SHA-256 fingerprints).
    /// Loaded from DB at startup; updated on POST /api/system/invalidate-key.
    pub revoked_keys: Arc<tokio::sync::RwLock<std::collections::HashSet<String>>>,
    /// Pending command approval requests (kernel ↔ API handler bridge).
    pub pending_command_approvals: handlers::command_approval::PendingApprovals,
    /// Session-scoped trusted command names (cleared on restart).
    pub session_trusted_commands: handlers::command_approval::SessionTrustedCommands,
    /// Per-agent active CRON execution contexts (for recursion depth tracking).
    pub active_cron_contexts: ActiveCronContexts,
    /// Maximum allowed CRON recursion depth (0-6, default 2).
    pub max_cron_generation: Arc<AtomicU8>,
    /// Whether a bootstrap setup is currently running.
    pub setup_in_progress: Arc<AtomicBool>,
    /// Broadcast channel for setup progress events (SSE).
    pub setup_progress_tx: broadcast::Sender<handlers::setup::SetupProgressEvent>,
    /// In-memory cache for marketplace catalog (registry.json).
    pub marketplace_cache: Arc<tokio::sync::RwLock<handlers::marketplace::CatalogCache>>,
    /// Stricter rate limiter for heavy operations (install, setup).
    /// 5 req/min per IP to prevent GitHub API abuse and disk exhaustion.
    pub install_limiter: Arc<middleware::RateLimiter>,
    /// Cached result from the last health scan (populated at startup and on-demand).
    pub last_health_report: Arc<tokio::sync::RwLock<Option<db::health::HealthReport>>>,
}

pub enum AppError {
    Cloto(cloto_shared::ClotoError),
    Internal(anyhow::Error),
    NotFound(String),
    Validation(String),
    Mgp(Box<managers::mcp_mgp::MgpError>),
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, err_type, message) = match self {
            AppError::Cloto(e) => {
                let status = match &e {
                    cloto_shared::ClotoError::PermissionDenied(_) => {
                        axum::http::StatusCode::FORBIDDEN
                    }
                    cloto_shared::ClotoError::PluginNotFound(_)
                    | cloto_shared::ClotoError::AgentNotFound(_) => {
                        axum::http::StatusCode::NOT_FOUND
                    }
                    _ => axum::http::StatusCode::BAD_REQUEST,
                };
                (status, format!("{:?}", e), e.to_string())
            }
            AppError::Internal(e) => {
                // Log full error server-side only; return generic message to client
                tracing::error!("Internal error: {}", e);
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "InternalError".to_string(),
                    "An internal error occurred".to_string(),
                )
            }
            AppError::NotFound(m) => (axum::http::StatusCode::NOT_FOUND, "NotFound".to_string(), m),
            AppError::Validation(m) => (
                axum::http::StatusCode::BAD_REQUEST,
                "ValidationError".to_string(),
                m,
            ),
            AppError::Mgp(ref e) => {
                let status = match e.code {
                    1000 | 1001 | 1010 | 1011 => axum::http::StatusCode::FORBIDDEN,
                    1002 | 1003 => axum::http::StatusCode::UNAUTHORIZED,
                    2000..=2002 => axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    3000 | 3002 => axum::http::StatusCode::TOO_MANY_REQUESTS,
                    3003 | 5001 => axum::http::StatusCode::GATEWAY_TIMEOUT,
                    4000 => axum::http::StatusCode::BAD_REQUEST,
                    4001..=4003 | 4100..=4102 => axum::http::StatusCode::NOT_FOUND,
                    _ => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                };
                let body = axum::Json(serde_json::json!({
                    "error": e.to_json_rpc_error()
                }));
                return (status, body).into_response();
            }
        };

        let body = axum::Json(serde_json::json!({
            "error": {
                "type": err_type,
                "message": message
            }
        }));

        (status, body).into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Internal(err)
    }
}

impl From<managers::mcp_mgp::MgpError> for AppError {
    fn from(err: managers::mcp_mgp::MgpError) -> Self {
        AppError::Mgp(Box::new(err))
    }
}

impl From<cloto_shared::ClotoError> for AppError {
    fn from(err: cloto_shared::ClotoError) -> Self {
        AppError::Cloto(err)
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::Internal(anyhow::anyhow!(err))
    }
}

pub type AppResult<T> = Result<T, AppError>;

/// Handle returned by [`start_kernel`] — keeps the kernel alive.
///
/// Dropping this handle does **not** trigger shutdown; use [`KernelHandle::shutdown`]
/// or the `/api/system/shutdown` endpoint instead.
pub struct KernelHandle {
    /// Notify to trigger graceful shutdown of the HTTP server and background tasks.
    pub shutdown: Arc<Notify>,
    /// Join handle for the HTTP server task.
    _server_task: tokio::task::JoinHandle<()>,
}

/// Initialize the kernel (DB, plugins, MCP, LLM proxy, event loop) and spawn the
/// HTTP server in the background.  Returns a [`KernelHandle`] on success.
///
/// Use this from Tauri (or other embedders) when you need to detect startup failures
/// **before** showing the UI.  For standalone CLI usage, prefer [`run_kernel`] which
/// blocks until shutdown.
#[allow(clippy::too_many_lines)]
pub async fn start_kernel() -> anyhow::Result<KernelHandle> {
    use crate::config::AppConfig;
    use crate::db;
    use crate::events::EventProcessor;
    use crate::handlers::{self, system::SystemHandler};
    use crate::managers::{AgentManager, PluginManager};
    use axum::{
        routing::{any, delete, get, post},
        Router,
    };
    use tower_http::cors::CorsLayer;
    use tracing::info;

    let kernel_start = std::time::Instant::now();

    info!("+---------------------------------------+");
    info!("|            Cloto System Kernel         |");
    info!(
        "|             Version {:<10}      |",
        env!("CARGO_PKG_VERSION")
    );
    info!("+---------------------------------------+");

    let config = AppConfig::load()?;
    // H-06: Mask DB path in logs (show filename only, not full path)
    let db_display = config
        .database_url
        .rsplit_once('/')
        .or_else(|| config.database_url.rsplit_once('\\'))
        .map_or("***", |(_, name)| name);
    info!(
        "📍 Loaded Config: DB={}, DEFAULT_AGENT={}",
        db_display, config.default_agent_id
    );
    // Full DB path at debug level for troubleshooting persistence issues
    tracing::debug!("📍 DB full path: {}", config.database_url);
    tracing::debug!("📍 exe_dir resolved to: {}", config::exe_dir().display());

    // Principle #5: Warn if admin API key is missing in release builds
    if config.admin_api_key.is_none() && !cfg!(debug_assertions) {
        tracing::warn!("⚠️  CLOTO_API_KEY is not set. All admin endpoints will reject requests.");
        tracing::warn!("    Set CLOTO_API_KEY in .env or environment to enable admin operations.");
    }

    // 0. Ensure parent directory of DB file exists (for deployed layout)
    if let Some(path_str) = config.database_url.strip_prefix("sqlite:") {
        let db_path = std::path::Path::new(path_str);
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() && parent != std::path::Path::new(".") {
                std::fs::create_dir_all(parent)?;
                // Restrict data directory permissions (contains SQLite DB)
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ =
                        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
                }
                info!("📁 Data directory: {}", parent.display());
            }
        }
    }

    // 0b. Ensure storage directories exist (relative to exe_dir for Tauri compatibility)
    let data_dir = config::exe_dir().join("data");
    if let Err(e) = std::fs::create_dir_all(data_dir.join("attachments")) {
        tracing::warn!("Failed to create data/attachments directory: {}", e);
    }
    if let Err(e) = std::fs::create_dir_all(data_dir.join("avatars")) {
        tracing::warn!("Failed to create data/avatars directory: {}", e);
    }
    if let Err(e) = std::fs::create_dir_all(data_dir.join("vrm")) {
        tracing::warn!("Failed to create data/vrm directory: {}", e);
    }
    if let Err(e) = std::fs::create_dir_all(data_dir.join("speech")) {
        tracing::warn!("Failed to create data/speech directory: {}", e);
    }
    tracing::info!("📁 Data directory: {}", data_dir.display());

    // 0c. Ensure Python MCP venv exists (auto-setup on first run)
    // Skip in production if bootstrap setup has not been completed yet.
    let setup_json = data_dir.join("setup-complete.json");
    let is_dev = {
        let exe = std::env::current_exe().unwrap_or_default();
        managers::McpClientManager::detect_project_root(&exe)
            .is_some_and(|r| r.join("Cargo.toml").exists())
    };
    if setup_json.exists() || is_dev {
        // Run venv dependency sync in background — not on the critical startup path.
        // Servers can start immediately since the venv python and existing packages
        // are already available; pip install only adds/updates packages.
        let data_dir_bg = data_dir.clone();
        tokio::spawn(async move {
            managers::mcp_venv::ensure_mcp_venv(Some(&data_dir_bg)).await;
        });
    } else {
        tracing::info!("Setup not complete — skipping MCP venv sync");
    }

    // 0d. Set database timeout from config
    db::set_db_timeout(config.db_timeout_secs);

    // 1. データベースの初期化
    use sqlx::sqlite::SqliteConnectOptions;
    use std::str::FromStr;
    let opts = SqliteConnectOptions::from_str(&config.database_url)?
        .create_if_missing(true)
        .pragma("foreign_keys", "ON");
    let pool = sqlx::SqlitePool::connect_with(opts).await?;
    db::init_db(&pool, &config.database_url, &config.memory_plugin_id).await?;

    // 1b. Sync API keys from environment variables into llm_providers table
    db::sync_env_api_keys(&pool, &config.llm_provider_env_mappings).await;

    // 2. Plugin Manager Setup
    let shutdown = Arc::new(Notify::new());
    // P1: Merge network whitelist + API host whitelist for SafeHttpClient
    let mut all_allowed_hosts = config.allowed_hosts.clone();
    all_allowed_hosts.extend(config.default_allowed_api_hosts.clone());
    let mut plugin_manager_obj = PluginManager::new(
        pool.clone(),
        all_allowed_hosts,
        config.plugin_event_timeout_secs,
        config.max_event_depth,
        config.event_concurrency_limit,
    )?;
    plugin_manager_obj.shutdown = shutdown.clone();

    // 3. Channel Setup
    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<EnvelopedEvent>(100);
    plugin_manager_obj.set_event_tx(event_tx.clone());
    let plugin_manager = Arc::new(plugin_manager_obj);

    // 3b. MCP Client Manager (created early so PluginRegistry can reference it)
    // Resolve YOLO mode: DB-persisted value takes precedence over env var
    let yolo_mode = {
        let db_yolo: Option<(String,)> = sqlx::query_as(
            "SELECT config_value FROM plugin_configs WHERE plugin_id = 'kernel' AND config_key = 'yolo_mode'"
        )
            .fetch_optional(&pool)
            .await
            .unwrap_or(None);
        match db_yolo {
            Some((val,)) => val == "true",
            None => config.yolo_mode, // fall back to env var
        }
    };
    let mut mcp_manager =
        managers::McpClientManager::new(pool.clone(), yolo_mode, config.mcp_request_timeout_secs);
    mcp_manager.configure_isolation(&config);
    let mcp_manager = Arc::new(mcp_manager);

    // 4. Initialize External Plugins
    let mut registry = plugin_manager.initialize_all().await?;
    registry.set_mcp_manager(mcp_manager.clone());
    let registry_arc = Arc::new(registry);

    // 5. Managers & Internal Handlers
    let agent_manager = AgentManager::new(pool.clone(), config.heartbeat_threshold_ms);
    let (tx, _rx) = tokio::sync::broadcast::channel::<events::SequencedEvent>(100);

    let dynamic_router = Arc::new(DynamicRouter {
        router: tokio::sync::RwLock::new(Router::new()),
    });

    let metrics = Arc::new(managers::SystemMetrics::new());
    let event_history = Arc::new(tokio::sync::RwLock::new(VecDeque::new()));

    // 🔌 System Handler の登録
    let pending_command_approvals: handlers::command_approval::PendingApprovals =
        Arc::new(dashmap::DashMap::new());
    let session_trusted_commands: handlers::command_approval::SessionTrustedCommands =
        Arc::new(dashmap::DashMap::new());
    let active_cron_contexts: ActiveCronContexts = Arc::new(dashmap::DashMap::new());
    let max_cron_generation = Arc::new(AtomicU8::new(
        std::env::var("CLOTO_MAX_CRON_GENERATION")
            .ok()
            .and_then(|v| v.parse::<u8>().ok())
            .map_or(2, |v| v.min(6)),
    ));

    let system_handler = Arc::new(SystemHandler::new(
        registry_arc.clone(),
        agent_manager.clone(),
        config.default_agent_id.clone(),
        event_tx.clone(),
        config.memory_context_limit,
        metrics.clone(),
        config.consensus_engines.clone(),
        config.max_agentic_iterations,
        config.tool_execution_timeout_secs,
        pending_command_approvals.clone(),
        session_trusted_commands.clone(),
        pool.clone(),
        active_cron_contexts.clone(),
        config.memory_timeout_secs,
    ));

    // SystemHandler is NOT registered as a plugin — it runs outside the dispatch
    // pipeline to avoid blocking the event loop during agentic loops.
    // It is passed directly to EventProcessor instead.

    // Load MCP servers from config file (mcp.toml)
    // Priority boot: connect default agent's granted servers first, defer the rest.
    let deferred_mcp_configs = {
        let config_path = config.mcp_config_path.clone().unwrap_or_else(|| {
            config::exe_dir()
                .join("data")
                .join("mcp.toml")
                .to_string_lossy()
                .to_string()
        });
        // Resolve config path against the project root (handles
        // cargo tauri dev where CWD differs from project root).
        let config_path = {
            let p = std::path::Path::new(&config_path);
            if p.exists() {
                config_path
            } else {
                // Walk up from exe_dir to find the workspace root (Cargo.toml)
                // and resolve mcp.toml relative to it.
                let fallback = std::path::Path::new("mcp.toml");
                managers::McpClientManager::resolve_project_path(fallback).unwrap_or(config_path)
            }
        };
        // Production fallback: Tauri NSIS bundles mcp.toml as a resource
        // alongside the executable (not in data/).
        let config_path = if std::path::Path::new(&config_path).exists() {
            config_path
        } else {
            let alongside_exe = config::exe_dir().join("mcp.toml");
            if alongside_exe.exists() {
                alongside_exe.to_string_lossy().to_string()
            } else {
                config_path
            }
        };

        match mcp_manager.parse_config_file(&config_path) {
            Ok(all_configs) => {
                // Get default agent's granted server IDs for priority boot
                let granted_ids = agent_manager
                    .get_granted_server_ids(&config.default_agent_id)
                    .await
                    .unwrap_or_default();

                let (priority, deferred): (Vec<_>, Vec<_>) = all_configs
                    .into_iter()
                    .partition(|c| granted_ids.contains(&c.id));

                if !priority.is_empty() {
                    info!(
                        count = priority.len(),
                        agent = %config.default_agent_id,
                        "⚡ Priority boot: connecting granted MCP servers"
                    );
                    mcp_manager.connect_server_configs(&priority).await;
                }

                if !deferred.is_empty() {
                    info!(
                        count = deferred.len(),
                        "⏳ Deferring {} non-priority MCP server(s) to background",
                        deferred.len()
                    );
                }

                deferred
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    path = %config_path,
                    "CRITICAL: Failed to parse MCP config file — all MCP servers disabled"
                );
                Vec::new()
            }
        }
    };

    // Restore persisted dynamic MCP servers from database
    if let Err(e) = mcp_manager.restore_from_db().await {
        tracing::warn!(error = %e, "Failed to restore MCP servers from database");
    }

    // 5. Rate Limiter & App State
    let rate_limiter = Arc::new(middleware::RateLimiter::new(
        config.rate_limit_per_sec,
        config.rate_limit_burst,
    ));
    // Stricter limiter for heavy operations (marketplace install, batch setup)
    let install_limiter = Arc::new(middleware::RateLimiter::per_minute(5, 5));

    // Load revoked key hashes into memory
    let revoked_keys = {
        let mut set = std::collections::HashSet::new();
        match db::load_revoked_key_hashes(&pool).await {
            Ok(hashes) => {
                let count = hashes.len();
                set.extend(hashes);
                if count > 0 {
                    info!(count = count, "🔑 Loaded revoked API key hashes");
                }
            }
            Err(e) => tracing::warn!(error = %e, "Failed to load revoked key hashes"),
        }
        Arc::new(tokio::sync::RwLock::new(set))
    };

    let app_state = Arc::new(AppState {
        tx: tx.clone(),
        registry: registry_arc.clone(),
        event_tx: event_tx.clone(),
        pool: pool.clone(),
        agent_manager: agent_manager.clone(),
        plugin_manager: plugin_manager.clone(),
        mcp_manager: mcp_manager.clone(),
        dynamic_router: dynamic_router.clone(),
        config: config.clone(),
        data_dir: data_dir.clone(),
        event_history: event_history.clone(),
        metrics: metrics.clone(),
        rate_limiter: rate_limiter.clone(),
        shutdown,
        revoked_keys,
        pending_command_approvals,
        session_trusted_commands,
        active_cron_contexts,
        max_cron_generation,
        setup_in_progress: Arc::new(AtomicBool::new(false)),
        setup_progress_tx: {
            let (tx, _) = broadcast::channel(64);
            tx
        },
        marketplace_cache: Arc::new(tokio::sync::RwLock::new(
            handlers::marketplace::CatalogCache::default(),
        )),
        install_limiter: install_limiter.clone(),
        last_health_report: Arc::new(tokio::sync::RwLock::new(None)),
    });

    // Wire up kernel event bus to MCP manager (for PermissionRequested emission)
    mcp_manager.set_kernel_event_tx(event_tx.clone()).await;

    // 6. Consensus Orchestrator (kernel-level, replaces core.moderator plugin)
    let consensus_config = consensus::ConsensusConfig {
        synthesizer_engine: std::env::var("CONSENSUS_SYNTHESIZER").unwrap_or_default(),
        min_proposals: std::env::var("CONSENSUS_MIN_PROPOSALS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2)
            .max(2),
        session_timeout_secs: std::env::var("CONSENSUS_SESSION_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60)
            .max(10),
    };
    let consensus_orchestrator = consensus::ConsensusOrchestrator::new(consensus_config);

    // 6a. Event Loop
    let processor = Arc::new(EventProcessor::new(
        registry_arc.clone(),
        plugin_manager.clone(),
        agent_manager.clone(),
        tx.clone(),
        event_history,
        metrics,
        config.event_history_size,
        config.event_retention_hours,
        Some(consensus_orchestrator),
        system_handler,
        config.max_event_history,
    ));

    // Start event history cleanup task
    processor
        .clone()
        .spawn_cleanup_task(app_state.shutdown.clone());

    // 6a. Active Heartbeat task (ping all enabled agents every 30s)
    let heartbeat_interval = std::env::var("HEARTBEAT_INTERVAL_SECS")
        .unwrap_or_else(|_| "30".to_string())
        .parse::<u64>()
        .unwrap_or(30);
    EventProcessor::spawn_heartbeat_task(
        agent_manager.clone(),
        heartbeat_interval,
        app_state.shutdown.clone(),
    );

    // 6b. MCP deferred boot — connect non-priority servers in background
    //     Wait for HTTP server to be ready before connecting (MGP callbacks need it).
    let http_ready = Arc::new(Notify::new());
    if !deferred_mcp_configs.is_empty() {
        let deferred_mcp = mcp_manager.clone();
        let deferred_shutdown = app_state.shutdown.clone();
        let deferred_http_ready = http_ready.clone();
        tokio::spawn(async move {
            // Wait for HTTP server to bind before connecting deferred MCP servers,
            // because they may send MGP callbacks that hit kernel HTTP endpoints.
            if tokio::time::timeout(
                std::time::Duration::from_secs(30),
                deferred_http_ready.notified(),
            )
            .await
            .is_err()
            {
                tracing::warn!(
                    "HTTP server readiness timed out (30s), proceeding with deferred MCP boot"
                );
            }
            info!(
                count = deferred_mcp_configs.len(),
                "🔌 Background: connecting deferred MCP servers"
            );
            deferred_mcp
                .connect_server_configs(&deferred_mcp_configs)
                .await;
            let _ = &deferred_shutdown; // hold reference to prevent premature shutdown
            info!("✅ Background MCP server boot complete");
        });
    }

    // 6b2. MCP health monitor — auto-restart dead servers (bug-142)
    Arc::clone(&mcp_manager).spawn_health_monitor(app_state.shutdown.clone());

    // 6b2. MCP notification listener — forward Server→Kernel notifications to event bus
    if let Some(mut notif_rx) = mcp_manager.take_notification_receiver().await {
        let notif_event_tx = event_tx.clone();
        let notif_mcp_manager = mcp_manager.clone();
        let shutdown_clone = app_state.shutdown.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = shutdown_clone.notified() => {
                        tracing::info!("MCP notification listener shutting down");
                        break;
                    }
                    notif_opt = notif_rx.recv() => {
                        let Some(notif) = notif_opt else { break };

                        // Intercept callback requests (MGP §13)
                        if notif.method == "notifications/mgp.callback.request" {
                            if let Some(ref params) = notif.params {
                                match managers::mcp::mcp_events_handle_callback(
                                    &notif_mcp_manager, &notif.server_id, params,
                                ) {
                                    managers::mcp::CallbackHandleResult::NewCallback(event_data) => {
                                        let envelope = EnvelopedEvent::system(*event_data);
                                        if let Err(e) = notif_event_tx.send(envelope).await {
                                            tracing::warn!("Failed to forward callback event: {}", e);
                                        }
                                    }
                                    managers::mcp::CallbackHandleResult::DuplicateWithResponse {
                                        server_id,
                                        callback_id,
                                        response,
                                    } => {
                                        let mgr = notif_mcp_manager.clone();
                                        tokio::spawn(async move {
                                            let state = mgr.state.read().await;
                                            if let Some(handle) = state.servers.get(&server_id) {
                                                if let Some(client) = &handle.client {
                                                    let params = serde_json::json!({
                                                        "callback_id": callback_id,
                                                        "response": response,
                                                    });
                                                    let _ = client.call("mgp/callback/respond", Some(params)).await;
                                                }
                                            }
                                        });
                                    }
                                    managers::mcp::CallbackHandleResult::DuplicateNoResponse => {}
                                }
                            }
                            continue;
                        }

                        // Intercept stream chunks for gap detection (MGP §12)
                        if notif.method == "notifications/mgp.stream.chunk" {
                            if let Some(ref params) = notif.params {
                                let request_id = params
                                    .get("request_id")
                                    .and_then(serde_json::Value::as_i64)
                                    .unwrap_or(-1);
                                let index = params
                                    .get("index")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0) as u32;
                                let done = params
                                    .get("done")
                                    .and_then(serde_json::Value::as_bool)
                                    .unwrap_or(false);
                                let mgr = notif_mcp_manager.clone();
                                let sid = notif.server_id.clone();
                                tokio::spawn(async move {
                                    mgr.handle_stream_chunk(&sid, request_id, index, done)
                                        .await;
                                });
                            }
                            // Fall through to normal notification forwarding
                        }

                        // Method-based filtering: MGP notifications → event bus, others → log only
                        if notif.method.starts_with("notifications/mgp.")
                            || notif.method.starts_with("notifications/cloto.")
                        {
                            info!(
                                server = %notif.server_id,
                                method = %notif.method,
                                "📨 MCP server notification received"
                            );
                            let event_data = cloto_shared::ClotoEventData::McpNotification {
                                server_id: notif.server_id,
                                method: notif.method,
                                params: notif.params.unwrap_or(serde_json::Value::Null),
                            };
                            let envelope = EnvelopedEvent::system(event_data);
                            if let Err(e) = notif_event_tx.send(envelope).await {
                                tracing::warn!("Failed to forward MCP notification: {}", e);
                            }
                        } else {
                            tracing::debug!(
                                server = %notif.server_id,
                                method = %notif.method,
                                "MCP notification received (not forwarded)"
                            );
                        }
                    }
                }
            }
        });
    }

    // 6c. Cron job scheduler (Layer 2: Autonomous Trigger)
    if config.cron_enabled {
        managers::scheduler::spawn_cron_task(
            pool.clone(),
            event_tx.clone(),
            config.cron_check_interval_secs,
            app_state.shutdown.clone(),
        );
    }

    // 6d. Startup health scan (optional, default: on)
    if config.health_scan_on_startup {
        let scan_pool = pool.clone();
        let scan_report = app_state.last_health_report.clone();
        tokio::spawn(async move {
            match db::health::run_quick_scan(&scan_pool).await {
                Ok(report) => {
                    let issue_count = report
                        .checks
                        .iter()
                        .filter(|c| c.status != db::health::HealthStatus::Healthy)
                        .count();
                    if issue_count > 0 {
                        tracing::warn!("⚠️ Startup health scan: {issue_count} issue(s) detected");
                    } else {
                        tracing::info!("✓ Startup health scan: all clear");
                    }
                    let mut cached = scan_report.write().await;
                    *cached = Some(report);
                }
                Err(e) => tracing::warn!("Startup health scan failed: {e}"),
            }
        });
    }

    // 6e. Internal LLM Proxy (MGP §13.4 — centralized API key management)
    //     Check result in background to avoid blocking HTTP server startup.
    let llm_proxy_rx = managers::llm_proxy::spawn_llm_proxy(
        pool.clone(),
        config.llm_proxy_port,
        config.llm_proxy_timeout_secs,
        app_state.shutdown.clone(),
    );
    {
        let proxy_port = config.llm_proxy_port;
        tokio::spawn(async move {
            match tokio::time::timeout(std::time::Duration::from_secs(15), llm_proxy_rx).await {
                Ok(Ok(Ok(()))) => {
                    info!("LLM Proxy ready on port {}", proxy_port);
                }
                Ok(Ok(Err(msg))) => {
                    tracing::warn!(
                        "⚠️  LLM Proxy failed to start: {}. Mind servers will not have LLM access.",
                        msg
                    );
                }
                Ok(Err(_)) => {
                    tracing::warn!("⚠️  LLM Proxy startup channel dropped unexpectedly");
                }
                Err(_) => {
                    tracing::warn!(
                        "⚠️  LLM Proxy startup timed out (15s). Mind servers may not have LLM access."
                    );
                }
            }
        });
    }

    let event_tx_clone = event_tx.clone();
    let processor_clone = processor.clone();
    let shutdown_clone = app_state.shutdown.clone();
    tokio::spawn(async move {
        tokio::select! {
            () = shutdown_clone.notified() => {
                tracing::info!("Event processor shutting down");
            }
            () = processor_clone.process_loop(event_rx, event_tx_clone) => {}
        }
    });

    // 6b. Rate limiter cleanup task (every 10 minutes)
    let rl = rate_limiter.clone();
    let il = install_limiter.clone();
    let shutdown_clone = app_state.shutdown.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(RATE_LIMITER_CLEANUP_SECS));
        loop {
            tokio::select! {
                () = shutdown_clone.notified() => {
                    tracing::info!("Rate limiter cleanup shutting down");
                    break;
                }
                _ = interval.tick() => {
                    rl.cleanup();
                    il.cleanup();
                }
            }
        }
    });

    // 6e. Revoked keys TTL cleanup task (every 6 hours, bug-158)
    {
        let pool_clone = pool.clone();
        let revoked_keys_clone = app_state.revoked_keys.clone();
        let shutdown_clone = app_state.shutdown.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(REVOKED_KEYS_CLEANUP_SECS));
            loop {
                tokio::select! {
                    () = shutdown_clone.notified() => {
                        tracing::info!("Revoked keys cleanup shutting down");
                        break;
                    }
                    _ = interval.tick() => {
                        match db::cleanup_revoked_keys(&pool_clone, 90).await {
                            Ok(remaining) => {
                                {
                                    let mut cache = revoked_keys_clone.write().await;
                                    cache.clear();
                                    cache.extend(remaining);
                                }
                                tracing::debug!("Revoked keys cleanup completed");
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "Revoked keys cleanup failed");
                            }
                        }
                    }
                }
            }
        });
    }

    // 7. Web Server

    // Admin endpoints: rate-limited (10 req/s, burst 20)
    let admin_routes = Router::new()
        .route("/health/scan", get(handlers::health::scan_handler))
        .route("/health/repair", post(handlers::health::repair_handler))
        .route("/system/shutdown", post(handlers::shutdown_handler))
        .route("/plugins/apply", post(handlers::apply_plugin_settings))
        .route("/plugins/:id/config", post(handlers::update_plugin_config))
        .route(
            "/plugins/:id/permissions",
            get(handlers::get_plugin_permissions).delete(handlers::revoke_permission_handler),
        )
        .route(
            "/plugins/:id/permissions/grant",
            post(handlers::grant_permission_handler),
        )
        .route("/agents", post(handlers::create_agent))
        .route(
            "/agents/:id",
            post(handlers::update_agent).delete(handlers::delete_agent),
        )
        .route("/agents/:id/power", post(handlers::power_toggle))
        .route(
            "/agents/:id/avatar",
            get(handlers::get_avatar)
                .post(handlers::upload_avatar)
                .delete(handlers::delete_avatar),
        )
        .route(
            "/agents/:id/vrm",
            get(handlers::get_vrm)
                .post(handlers::upload_vrm)
                .delete(handlers::delete_vrm),
        )
        .route("/agents/:id/visemes", post(handlers::generate_visemes))
        .route("/speech/:filename", get(handlers::serve_speech_file))
        .route("/events/publish", post(handlers::post_event_handler))
        // Cron job management (Layer 2: Autonomous Trigger)
        .route(
            "/cron/jobs",
            get(handlers::list_cron_jobs).post(handlers::create_cron_job),
        )
        .route("/cron/jobs/:id", delete(handlers::delete_cron_job))
        .route("/cron/jobs/:id/toggle", post(handlers::toggle_cron_job))
        .route("/cron/jobs/:id/run", post(handlers::run_cron_job_now))
        // LLM Provider management (MGP §13.4 — centralized key management)
        .route("/llm/providers", get(handlers::list_llm_providers))
        .route(
            "/llm/providers/:id/key",
            post(handlers::set_llm_provider_key).delete(handlers::delete_llm_provider_key),
        )
        .route(
            "/permissions/:id/approve",
            post(handlers::approve_permission),
        )
        .route("/permissions/:id/deny", post(handlers::deny_permission))
        // Command approval endpoints
        .route("/commands/:id/approve", post(handlers::approve_command))
        .route("/commands/:id/trust", post(handlers::trust_command))
        .route("/commands/:id/deny", post(handlers::deny_command))
        // M-08: chat_handler moved here to apply rate limiting
        .route("/chat", post(handlers::chat_handler))
        // Chat persistence endpoints
        .route(
            "/chat/:agent_id/messages",
            get(handlers::chat::get_messages)
                .post(handlers::chat::post_message)
                .delete(handlers::chat::delete_messages),
        )
        .route(
            "/chat/:agent_id/messages/:message_id/retry",
            post(handlers::chat::retry_response),
        )
        .route(
            "/chat/attachments/:attachment_id",
            get(handlers::chat::get_attachment),
        )
        // MCP dynamic server management
        .route(
            "/mcp/servers",
            get(handlers::list_mcp_servers).post(handlers::create_mcp_server),
        )
        .route(
            "/mcp/servers/:name",
            axum::routing::delete(handlers::delete_mcp_server),
        )
        // MCP server settings & access control (MCP_SERVER_UI_DESIGN.md §4)
        .route(
            "/mcp/servers/:name/settings",
            get(handlers::get_mcp_server_settings).put(handlers::update_mcp_server_settings),
        )
        .route(
            "/mcp/servers/:name/access",
            get(handlers::get_mcp_server_access).put(handlers::put_mcp_server_access),
        )
        // MCP server lifecycle
        .route(
            "/mcp/servers/:name/restart",
            post(handlers::restart_mcp_server),
        )
        .route("/mcp/servers/:name/start", post(handlers::start_mcp_server))
        .route("/mcp/servers/:name/stop", post(handlers::stop_mcp_server))
        // Direct tool call for coordinator-pattern servers (MGP §5.6, §19.1)
        .route("/mcp/call", post(handlers::call_mcp_tool))
        // Settings
        .route(
            "/settings/yolo",
            get(handlers::get_yolo_mode).put(handlers::set_yolo_mode),
        )
        .route(
            "/settings/max-cron-generation",
            get(handlers::get_max_cron_generation).put(handlers::set_max_cron_generation),
        )
        // API key invalidation
        .route("/system/invalidate-key", post(handlers::invalidate_api_key))
        // Bootstrap setup (auth required to start)
        .route("/setup/start", post(handlers::setup::start_handler))
        // Marketplace (auth required)
        .route("/marketplace/catalog", get(handlers::catalog_handler))
        .route("/marketplace/install", post(handlers::install_handler))
        .route(
            "/marketplace/batch-install",
            post(handlers::batch_install_handler),
        )
        .route(
            "/marketplace/servers/{id}",
            delete(handlers::uninstall_handler),
        );

    // Read endpoints (authenticated, rate-limited — bug-157)
    let api_routes = Router::new()
        .route("/system/version", get(handlers::version_handler))
        .route("/system/health", get(handlers::health_handler))
        // Bootstrap setup (no auth — like health_handler)
        .route("/setup/status", get(handlers::setup::status_handler))
        .route("/setup/progress", get(handlers::setup::progress_handler))
        .route(
            "/setup/check-python",
            post(handlers::setup::check_python_handler),
        )
        // Marketplace progress (no auth — SSE stream)
        .route(
            "/marketplace/progress",
            get(handlers::marketplace_progress_handler),
        )
        .route("/events", get(handlers::sse_handler))
        .route("/history", get(handlers::get_history))
        .route("/metrics", get(handlers::get_metrics))
        .route("/memories", get(handlers::get_memories))
        .route(
            "/memories/:id",
            delete(handlers::delete_memory).put(handlers::update_memory),
        )
        .route("/memories/:id/lock", post(handlers::lock_memory))
        .route("/memories/:id/unlock", post(handlers::unlock_memory))
        .route("/episodes", get(handlers::get_episodes))
        .route("/episodes/:id", delete(handlers::delete_episode))
        .route("/memories/import", post(handlers::import_memories))
        .route("/plugins", get(handlers::get_plugins))
        .route("/plugins/:id/config", get(handlers::get_plugin_config))
        .route("/agents", get(handlers::get_agents))
        .route(
            "/permissions/pending",
            get(handlers::get_pending_permissions),
        )
        // MCP access control (public/read)
        .route(
            "/mcp/access/by-agent/:agent_id",
            get(handlers::get_agent_access),
        )
        .merge(admin_routes)
        .layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            middleware::rate_limit_middleware,
        ))
        .layer(axum::extract::DefaultBodyLimit::max(50 * 1024 * 1024)); // 50MB for VRM uploads

    let app = Router::new()
        .nest("/api", api_routes.with_state(app_state.clone()))
        .route("/api/plugin/*path", any(dynamic_proxy_handler))
        .with_state(app_state.clone())
        .fallback(handlers::assets::static_handler)
        .layer(
            CorsLayer::new()
                .allow_origin(config.cors_origins)
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::DELETE,
                    axum::http::Method::PUT,
                ])
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderName::from_static("x-api-key"),
                ]),
        );

    let bind_addr: std::net::SocketAddr =
        format!("{}:{}", config.bind_address, config.port).parse()?;
    let listener = bind_with_retry(bind_addr, 5, std::time::Duration::from_secs(2)).await?;
    // Signal deferred MCP boot that the HTTP server is now ready for callbacks.
    http_ready.notify_waiters();
    info!(
        "🚀 Cloto System Kernel is listening on http://{}:{} (startup: {:.1}s)",
        config.bind_address,
        config.port,
        kernel_start.elapsed().as_secs_f64()
    );

    let shutdown_handle = app_state.shutdown.clone();
    let shutdown_signal = app_state.shutdown.clone();
    let server_task = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            shutdown_signal.notified().await;
            info!("🛑 Graceful shutdown signal received. Stopping server...");
        })
        .await
        .ok();
    });

    Ok(KernelHandle {
        shutdown: shutdown_handle,
        _server_task: server_task,
    })
}

/// Initialize the kernel and block until shutdown.
///
/// Convenience wrapper around [`start_kernel`] for standalone CLI usage.
pub async fn run_kernel() -> anyhow::Result<()> {
    let handle = start_kernel().await?;
    // Block until the shutdown signal is received.
    handle.shutdown.notified().await;
    Ok(())
}

/// Bind a TCP listener with retry logic for port conflicts (e.g., previous process
/// still holding the port in CLOSE_WAIT/TIME_WAIT state during `tauri dev` restarts).
async fn bind_with_retry(
    addr: std::net::SocketAddr,
    max_retries: u32,
    delay: std::time::Duration,
) -> anyhow::Result<tokio::net::TcpListener> {
    for attempt in 0..=max_retries {
        let socket = tokio::net::TcpSocket::new_v4()?;
        socket.set_reuseaddr(true)?;
        match socket.bind(addr) {
            Ok(()) => match socket.listen(1024) {
                Ok(listener) => return Ok(listener),
                Err(e) if attempt < max_retries => {
                    tracing::warn!(
                        "Port {} listen failed (attempt {}/{}): {}. Retrying in {:?}...",
                        addr.port(),
                        attempt + 1,
                        max_retries,
                        e,
                        delay
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(e.into()),
            },
            Err(e) if attempt < max_retries => {
                tracing::warn!(
                    "Port {} bind failed (attempt {}/{}): {}. Retrying in {:?}...",
                    addr.port(),
                    attempt + 1,
                    max_retries,
                    e,
                    delay
                );
                tokio::time::sleep(delay).await;
            }
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!()
}

use axum::extract::State;
use axum::http::Request;
use axum::response::IntoResponse;
use tower::ServiceExt;

async fn dynamic_proxy_handler(
    State(state): State<Arc<AppState>>,
    request: Request<axum::body::Body>,
) -> impl IntoResponse {
    let router = {
        let router_lock = state.dynamic_router.router.read().await;
        router_lock.clone()
    };

    let any_state = state.clone() as Arc<dyn std::any::Any + Send + Sync>;
    router
        .with_state(any_state)
        .oneshot(request)
        .await
        .into_response()
}
