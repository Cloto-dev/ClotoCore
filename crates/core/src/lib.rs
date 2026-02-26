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
pub mod validation;

// Re-export audit log and permission request types for external use
pub use db::{
    create_permission_request, get_pending_permission_requests, is_permission_approved,
    query_audit_logs, update_permission_request, write_audit_log, AuditLogEntry, PermissionRequest,
};

use cloto_shared::ClotoEvent;
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Notify, RwLock};

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
    pub tx: broadcast::Sender<Arc<ClotoEvent>>,
    pub registry: Arc<managers::PluginRegistry>,
    pub event_tx: mpsc::Sender<EnvelopedEvent>,
    pub pool: SqlitePool,
    pub agent_manager: managers::AgentManager,
    pub plugin_manager: Arc<managers::PluginManager>,
    pub mcp_manager: Arc<managers::McpClientManager>,
    pub dynamic_router: Arc<DynamicRouter>,
    pub config: config::AppConfig,
    pub event_history: Arc<RwLock<VecDeque<Arc<ClotoEvent>>>>,
    pub metrics: Arc<managers::SystemMetrics>,
    pub rate_limiter: Arc<middleware::RateLimiter>,
    pub shutdown: Arc<Notify>,
    /// In-memory cache of revoked API key hashes (SHA-256 fingerprints).
    /// Loaded from DB at startup; updated on POST /api/system/invalidate-key.
    pub revoked_keys: Arc<std::sync::RwLock<std::collections::HashSet<String>>>,
}

pub enum AppError {
    Cloto(cloto_shared::ClotoError),
    Internal(anyhow::Error),
    NotFound(String),
    Validation(String),
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
                    | cloto_shared::ClotoError::AgentNotFound(_) => axum::http::StatusCode::NOT_FOUND,
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
        };

        let body = axum::Json(serde_json::json!({
            "status": "error",
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

/// Kernel 起動用のエントリポイント
#[allow(clippy::too_many_lines)]
pub async fn run_kernel() -> anyhow::Result<()> {
    use crate::config::AppConfig;
    use crate::db;
    use crate::events::EventProcessor;
    use crate::handlers::{self, system::SystemHandler};
    use crate::managers::{AgentManager, PluginManager};
    use axum::{
        routing::{any, get, post},
        Router,
    };
    use tower_http::cors::CorsLayer;
    use tracing::info;

    info!("+---------------------------------------+");
    info!("|            Cloto System Kernel         |");
    info!(
        "|             Version {:<10}      |",
        env!("CARGO_PKG_VERSION")
    );
    info!("+---------------------------------------+");

    let config = AppConfig::load()?;
    info!(
        "📍 Loaded Config: DB_URL={}, DEFAULT_AGENT={}",
        config.database_url, config.default_agent_id
    );

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

    // 0b. Ensure attachment storage directory exists
    if let Err(e) = std::fs::create_dir_all("data/attachments") {
        tracing::warn!("Failed to create data/attachments directory: {}", e);
    }

    // 1. データベースの初期化
    use sqlx::sqlite::SqliteConnectOptions;
    use std::str::FromStr;
    let opts = SqliteConnectOptions::from_str(&config.database_url)?.create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(opts).await?;
    db::init_db(&pool, &config.database_url).await?;

    // 2. Plugin Manager Setup
    let shutdown = Arc::new(Notify::new());
    let mut plugin_manager_obj = PluginManager::new(
        pool.clone(),
        config.allowed_hosts.clone(),
        config.plugin_event_timeout_secs,
        config.max_event_depth,
    )?;
    plugin_manager_obj.shutdown = shutdown.clone();

    // 3. Channel Setup
    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<EnvelopedEvent>(100);
    plugin_manager_obj.set_event_tx(event_tx.clone());
    let plugin_manager = Arc::new(plugin_manager_obj);

    // 3b. MCP Client Manager (created early so PluginRegistry can reference it)
    let mcp_manager = Arc::new(managers::McpClientManager::new(
        pool.clone(),
        config.yolo_mode,
    ));

    // 4. Initialize External Plugins
    let mut registry = plugin_manager.initialize_all().await?;
    registry.set_mcp_manager(mcp_manager.clone());
    let registry_arc = Arc::new(registry);

    // 5. Managers & Internal Handlers
    let agent_manager = AgentManager::new(pool.clone());
    let (tx, _rx) = tokio::sync::broadcast::channel(100);

    let dynamic_router = Arc::new(DynamicRouter {
        router: tokio::sync::RwLock::new(Router::new()),
    });

    let metrics = Arc::new(managers::SystemMetrics::new());
    let event_history = Arc::new(tokio::sync::RwLock::new(VecDeque::new()));

    // 🔌 System Handler の登録
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
    ));

    {
        let mut plugins = registry_arc.plugins.write().await;
        plugins.insert("core.system".to_string(), system_handler);
    }

    // Load MCP servers from config file (mcp.toml)
    {
        let config_path = config.mcp_config_path.clone().unwrap_or_else(|| {
            config::exe_dir()
                .join("data")
                .join("mcp.toml")
                .to_string_lossy()
                .to_string()
        });
        // Resolve relative config paths against the project root (handles
        // cargo tauri dev where CWD differs from project root).
        let config_path = {
            let p = std::path::Path::new(&config_path);
            if p.is_relative() && !p.exists() {
                // Walk up from exe_dir to find the workspace root (Cargo.toml)
                managers::McpClientManager::resolve_project_path(p).unwrap_or(config_path)
            } else {
                config_path
            }
        };
        if let Err(e) = mcp_manager.load_config_file(&config_path).await {
            tracing::warn!(error = %e, "Failed to load MCP config file");
        }
    }

    // Restore persisted dynamic MCP servers from database
    if let Err(e) = mcp_manager.restore_from_db().await {
        tracing::warn!(error = %e, "Failed to restore MCP servers from database");
    }

    // 5. Rate Limiter & App State
    let rate_limiter = Arc::new(middleware::RateLimiter::new(10, 20));

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
        Arc::new(std::sync::RwLock::new(set))
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
        event_history: event_history.clone(),
        metrics: metrics.clone(),
        rate_limiter: rate_limiter.clone(),
        shutdown,
        revoked_keys,
    });

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
    let shutdown_clone = app_state.shutdown.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(600));
        loop {
            tokio::select! {
                () = shutdown_clone.notified() => {
                    tracing::info!("Rate limiter cleanup shutting down");
                    break;
                }
                _ = interval.tick() => {
                    rl.cleanup();
                }
            }
        }
    });

    // 7. Web Server

    // Admin endpoints: rate-limited (10 req/s, burst 20)
    let admin_routes = Router::new()
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
        .route(
            "/agents/:id/plugins",
            get(handlers::get_agent_plugins).put(handlers::set_agent_plugins),
        )
        .route("/agents/:id/power", post(handlers::power_toggle))
        .route("/events/publish", post(handlers::post_event_handler))
        .route(
            "/permissions/:id/approve",
            post(handlers::approve_permission),
        )
        .route("/permissions/:id/deny", post(handlers::deny_permission))
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
        // Settings
        .route(
            "/settings/yolo",
            get(handlers::get_yolo_mode).put(handlers::set_yolo_mode),
        )
        // API key invalidation
        .route("/system/invalidate-key", post(handlers::invalidate_api_key))
        .layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            middleware::rate_limit_middleware,
        ));

    // Public/read endpoints (no rate limiting)
    let api_routes = Router::new()
        .route("/system/version", get(handlers::version_handler))
        .route("/events", get(handlers::sse_handler))
        .route("/history", get(handlers::get_history))
        .route("/metrics", get(handlers::get_metrics))
        .route("/memories", get(handlers::get_memories))
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
        .layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024)); // 10MB for chat attachments

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

    let listener =
        tokio::net::TcpListener::bind(format!("{}:{}", config.bind_address, config.port)).await?;
    info!(
        "🚀 Cloto System Kernel is listening on http://{}:{}",
        config.bind_address, config.port
    );

    let shutdown_signal = app_state.shutdown.clone();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal.notified().await;
        info!("🛑 Graceful shutdown signal received. Stopping server...");
    })
    .await?;
    Ok(())
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
