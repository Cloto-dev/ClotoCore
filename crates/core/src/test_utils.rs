use crate::config::AppConfig;
use crate::managers::{AgentManager, PluginManager, PluginRegistry, SystemMetrics};
use crate::DynamicRouter;
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Notify, RwLock};

pub async fn create_test_app_state(admin_api_key: Option<String>) -> Arc<crate::AppState> {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    crate::db::init_db(&pool, "sqlite::memory:", "memory.cpersona")
        .await
        .unwrap();

    let (event_tx, _event_rx) = mpsc::channel(100);
    let (tx, _rx) = broadcast::channel::<crate::events::SequencedEvent>(100);

    let registry = Arc::new(PluginRegistry::new(5, 10, 50));
    let agent_manager = AgentManager::new(pool.clone(), 90_000);
    let plugin_manager = Arc::new(PluginManager::new(pool.clone(), vec![], 30, 10, 50).unwrap());

    let dynamic_router = Arc::new(DynamicRouter {
        router: RwLock::new(axum::Router::new()),
    });

    let metrics = Arc::new(SystemMetrics::new());
    let event_history = Arc::new(RwLock::new(VecDeque::new()));

    let mut config = AppConfig::load().unwrap();
    config.admin_api_key = admin_api_key;

    let rate_limiter = Arc::new(crate::middleware::RateLimiter::new(
        config.rate_limit_per_sec,
        config.rate_limit_burst,
    ));

    let shutdown = Arc::new(Notify::new());
    let mcp_manager = Arc::new(crate::managers::McpClientManager::new(
        pool.clone(),
        false, // yolo_mode disabled in tests
        120,   // mcp_request_timeout_secs
        30,    // mcp_stream_idle_timeout_secs
    ));

    Arc::new(crate::AppState {
        tx,
        registry,
        event_tx,
        pool,
        agent_manager,
        plugin_manager,
        mcp_manager,
        dynamic_router,
        config,
        data_dir: std::path::PathBuf::from("data"),
        event_history,
        metrics,
        rate_limiter,
        shutdown,
        revoked_keys: Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new())),
        pending_command_approvals: Arc::new(dashmap::DashMap::new()),
        session_trusted_commands: Arc::new(dashmap::DashMap::new()),
        active_cron_contexts: Arc::new(dashmap::DashMap::new()),
        max_cron_generation: Arc::new(std::sync::atomic::AtomicU8::new(2)),
        setup_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        setup_done: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        setup_progress_tx: {
            let (tx, _) = tokio::sync::broadcast::channel(64);
            tx
        },
        marketplace_cache: Arc::new(tokio::sync::RwLock::new(
            crate::handlers::marketplace::CatalogCache::default(),
        )),
        install_limiter: Arc::new(crate::middleware::RateLimiter::per_minute(5, 5)),
        last_health_report: Arc::new(tokio::sync::RwLock::new(None)),
        provider_probe_cache: crate::managers::provider_probe::ProbeCache::new(),
        last_usage: crate::managers::usage_tracker::UsageStore::new(),
    })
}
