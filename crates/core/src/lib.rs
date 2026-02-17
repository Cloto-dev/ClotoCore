pub mod config;
pub mod db;
pub mod events;
pub mod evolution;
pub mod handlers;
pub mod managers;
pub mod capabilities;
pub mod middleware;
pub mod cli;
pub mod installer;
pub mod platform;
pub mod validation;

// Re-export audit log and permission request types for external use
pub use db::{
    AuditLogEntry, write_audit_log, query_audit_logs,
    PermissionRequest, create_permission_request, get_pending_permission_requests, update_permission_request,
};

// Static Linker: Force plugin crates to be linked for inventory discovery
// Without these imports, the Rust linker will not include plugin code,
// causing inventory::submit! to never execute and plugins to be undiscoverable.
extern crate plugin_cerebras;
extern crate plugin_cursor;
extern crate plugin_deepseek;
extern crate plugin_ks22;
extern crate plugin_mcp;
extern crate plugin_moderator;
extern crate plugin_python_bridge;
extern crate plugin_vision;

use exiv_shared::ExivEvent;
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Notify, RwLock};

#[derive(Debug, Clone)]
pub struct EnvelopedEvent {
    pub event: Arc<ExivEvent>,
    pub issuer: Option<exiv_shared::ExivId>, // None = System/Kernel
    pub correlation_id: Option<exiv_shared::ExivId>, // 親イベントの trace_id
    pub depth: u8,
}

impl EnvelopedEvent {
    /// Create a system-originated event (no issuer, no correlation, depth 0)
    pub fn system(data: exiv_shared::ExivEventData) -> Self {
        Self {
            event: Arc::new(ExivEvent::new(data)),
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
    pub tx: broadcast::Sender<Arc<ExivEvent>>,
    pub registry: Arc<managers::PluginRegistry>,
    pub event_tx: mpsc::Sender<EnvelopedEvent>,
    pub pool: SqlitePool,
    pub agent_manager: managers::AgentManager,
    pub plugin_manager: Arc<managers::PluginManager>,
    pub dynamic_router: Arc<DynamicRouter>,
    pub config: config::AppConfig,
    pub event_history: Arc<RwLock<VecDeque<Arc<ExivEvent>>>>,
    pub metrics: Arc<managers::SystemMetrics>,
    pub rate_limiter: Arc<middleware::RateLimiter>,
    pub shutdown: Arc<Notify>,
    pub evolution_engine: Option<Arc<evolution::EvolutionEngine>>,
}

pub enum AppError {
    Vers(exiv_shared::ExivError),
    Internal(anyhow::Error),
    NotFound(String),
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, err_type, message) = match self {
            AppError::Vers(e) => (axum::http::StatusCode::BAD_REQUEST, format!("{:?}", e), e.to_string()),
            AppError::Internal(e) => {
                // Log full error server-side only; return generic message to client
                tracing::error!("Internal error: {}", e);
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "InternalError".to_string(), "An internal error occurred".to_string())
            },
            AppError::NotFound(m) => (axum::http::StatusCode::NOT_FOUND, "NotFound".to_string(), m),
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

impl From<exiv_shared::ExivError> for AppError {
    fn from(err: exiv_shared::ExivError) -> Self {
        AppError::Vers(err)
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::Internal(anyhow::anyhow!(err))
    }
}

pub type AppResult<T> = Result<T, AppError>;

/// Kernel 起動用のエントリポイント
pub async fn run_kernel() -> anyhow::Result<()> {
    use crate::config::AppConfig;
    use crate::db;
    use crate::events::EventProcessor;
    use crate::handlers::{self, system::SystemHandler};
    use crate::managers::{AgentManager, PluginManager};
    use axum::{routing::{get, post, any}, Router};
    use tower_http::cors::CorsLayer;
    use tracing::info;

    info!("+---------------------------------------+");
    info!("|            Exiv System Kernel         |");
    info!("|             Version {:<10}      |", env!("CARGO_PKG_VERSION"));
    info!("+---------------------------------------+");

    let config = AppConfig::load()?;
    info!(
        "📍 Loaded Config: DB_URL={}, DEFAULT_AGENT={}",
        config.database_url, config.default_agent_id
    );

    // Principle #5: Warn if admin API key is missing in release builds
    if config.admin_api_key.is_none() && !cfg!(debug_assertions) {
        tracing::warn!("⚠️  EXIV_API_KEY is not set. All admin endpoints will reject requests.");
        tracing::warn!("    Set EXIV_API_KEY in .env or environment to enable admin operations.");
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
                    let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
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
    let opts = SqliteConnectOptions::from_str(&config.database_url)?
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(opts).await?;
    db::init_db(&pool, &config.database_url).await?;

    // 2. Plugin Manager Setup
    let mut plugin_manager_obj = PluginManager::new(
        pool.clone(),
        config.allowed_hosts.clone(),
        config.plugin_event_timeout_secs,
        config.max_event_depth,
    )?;
    plugin_manager_obj.register_builtins();

    // 3. Channel Setup
    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<EnvelopedEvent>(100);
    plugin_manager_obj.set_event_tx(event_tx.clone());
    let plugin_manager = Arc::new(plugin_manager_obj);

    // 4. Initialize External Plugins
    let registry = plugin_manager.initialize_all().await?;
    let registry_arc = Arc::new(registry);

    // 5. Managers & Internal Handlers
    let agent_manager = AgentManager::new(pool.clone());
    let (tx, _rx) = tokio::sync::broadcast::channel(100);

    let mut dynamic_routes = Router::new();
    let plugins_snapshot = registry_arc.plugins.read().await;
    for (id, plugin) in plugins_snapshot.iter() {
        if let Some(web) = plugin.as_web() {
            dynamic_routes = web.register_routes(dynamic_routes);
            info!("🔌 Registered dynamic routes for web-enabled plugin: {}", id);
        }
    }
    drop(plugins_snapshot);

    let dynamic_router = Arc::new(DynamicRouter {
        router: tokio::sync::RwLock::new(dynamic_routes),
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
    ));
    
    {
        let mut plugins = registry_arc.plugins.write().await;
        plugins.insert("core.system".to_string(), system_handler);
    }

    // 5. Rate Limiter & Evolution Engine & App State
    let rate_limiter = Arc::new(middleware::RateLimiter::new(10, 20));
    let shutdown = Arc::new(Notify::new());

    // Self-Evolution Engine (E1-E5)
    let data_store: Arc<dyn exiv_shared::PluginDataStore> = Arc::new(db::SqliteDataStore::new(pool.clone()));
    let evolution_engine = Arc::new(evolution::EvolutionEngine::new(data_store, pool.clone()));

    let app_state = Arc::new(AppState {
        tx: tx.clone(),
        registry: registry_arc.clone(),
        event_tx: event_tx.clone(),
        pool: pool.clone(),
        agent_manager: agent_manager.clone(),
        plugin_manager: plugin_manager.clone(),
        dynamic_router: dynamic_router.clone(),
        config: config.clone(),
        event_history: event_history.clone(),
        metrics: metrics.clone(),
        rate_limiter: rate_limiter.clone(),
        shutdown,
        evolution_engine: Some(evolution_engine.clone()),
    });

    // 6. Event Loop
    let processor = Arc::new(EventProcessor::new(
        registry_arc.clone(),
        plugin_manager.clone(),
        agent_manager.clone(),
        tx.clone(),
        dynamic_router.clone(),
        event_history,
        metrics,
        config.event_history_size,
        config.event_retention_hours,
        Some(evolution_engine),
    ));

    // Start event history cleanup task
    processor.clone().spawn_cleanup_task();

    // 6a. Active Heartbeat task (ping all enabled agents every 30s)
    let heartbeat_interval = std::env::var("HEARTBEAT_INTERVAL_SECS")
        .unwrap_or_else(|_| "30".to_string())
        .parse::<u64>()
        .unwrap_or(30);
    EventProcessor::spawn_heartbeat_task(agent_manager.clone(), heartbeat_interval);

    let event_tx_clone = event_tx.clone();
    let processor_clone = processor.clone();
    tokio::spawn(async move {
        processor_clone.process_loop(event_rx, event_tx_clone).await;
    });

    // 6b. Rate limiter cleanup task (every 10 minutes)
    let rl = rate_limiter.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
            rl.cleanup();
        }
    });

    // 7. Web Server

    // Admin endpoints: rate-limited (10 req/s, burst 20)
    let admin_routes = Router::new()
        .route("/system/shutdown", post(handlers::shutdown_handler))
        .route("/system/update/apply", post(handlers::update::apply_handler))
        .route("/plugins/apply", post(handlers::apply_plugin_settings))
        .route("/plugins/:id/config", post(handlers::update_plugin_config))
        .route("/plugins/:id/permissions/grant", post(handlers::grant_permission_handler))
        .route("/agents", post(handlers::create_agent))
        .route("/agents/:id", post(handlers::update_agent))
        .route("/agents/:id/power", post(handlers::power_toggle))
        .route("/events/publish", post(handlers::post_event_handler))
        .route("/permissions/:id/approve", post(handlers::approve_permission))
        .route("/permissions/:id/deny", post(handlers::deny_permission))
        // M-08: chat_handler moved here to apply rate limiting
        .route("/chat", post(handlers::chat_handler))
        // Chat persistence endpoints
        .route("/chat/:agent_id/messages", get(handlers::chat::get_messages).post(handlers::chat::post_message).delete(handlers::chat::delete_messages))
        .route("/chat/attachments/:attachment_id", get(handlers::chat::get_attachment))
        // Evolution Engine endpoints (auth required for write)
        .route("/evolution/params", get(handlers::evolution::get_evolution_params).put(handlers::evolution::update_evolution_params))
        .layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            middleware::rate_limit_middleware,
        ));

    // Public/read endpoints (no rate limiting)
    let api_routes = Router::new()
        .route("/system/version", get(handlers::update::version_handler))
        .route("/system/update/check", get(handlers::update::check_handler))
        .route("/events", get(handlers::sse_handler))
        .route("/history", get(handlers::get_history))
        .route("/metrics", get(handlers::get_metrics))
        .route("/memories", get(handlers::get_memories))
        .route("/plugins", get(handlers::get_plugins))
        .route("/plugins/:id/config", get(handlers::get_plugin_config))
        .route("/agents", get(handlers::get_agents))
        .route("/permissions/pending", get(handlers::get_pending_permissions))
        // Evolution Engine public endpoints (read-only)
        .route("/evolution/status", get(handlers::evolution::get_evolution_status))
        .route("/evolution/generations", get(handlers::evolution::get_generation_history))
        .route("/evolution/generations/:n", get(handlers::evolution::get_generation))
        .route("/evolution/fitness", get(handlers::evolution::get_fitness_timeline))
        .route("/evolution/rollbacks", get(handlers::evolution::get_rollback_history))
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
                .allow_methods([axum::http::Method::GET, axum::http::Method::POST, axum::http::Method::DELETE])
                .allow_headers([axum::http::header::CONTENT_TYPE]),
        );

    let listener = tokio::net::TcpListener::bind(format!("{}:{}", config.bind_address, config.port)).await?;
    info!(
        "🚀 Exiv System Kernel is listening on http://{}:{}",
        config.bind_address, config.port
    );

    let shutdown_signal = app_state.shutdown.clone();
    axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>())
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
