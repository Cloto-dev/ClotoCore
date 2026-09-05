use crate::config::AppConfig;
use crate::managers::{AgentManager, PluginManager, PluginRegistry, SystemMetrics};
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};

/// An [`McpClientManager`](crate::managers::McpClientManager) on a throwaway
/// in-memory database, for tests and benches that need a [`PluginRegistry`]
/// but never reach an MCP server.
///
/// The registry requires one — tool risk classification has no fallback — so
/// this exists to keep that requirement from turning every registry test into
/// a database fixture.
pub async fn test_mcp_manager() -> Arc<crate::managers::McpClientManager> {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    crate::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();
    Arc::new(crate::managers::McpClientManager::new(pool, false, 120, 30))
}

pub async fn create_test_app_state(admin_api_key: Option<String>) -> Arc<crate::AppState> {
    create_test_app_state_in(std::path::PathBuf::from("data"), admin_api_key).await
}

/// Like [`create_test_app_state`] but with an explicit `data_dir`, for tests
/// that write under it (marketplace installs, seal keys, tmp downloads).
pub async fn create_test_app_state_in(
    data_dir: std::path::PathBuf,
    admin_api_key: Option<String>,
) -> Arc<crate::AppState> {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    crate::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();

    let (event_tx, _event_rx) = mpsc::channel(100);
    let (tx, _rx) = broadcast::channel::<crate::events::SequencedEvent>(100);

    let mcp_manager = Arc::new(crate::managers::McpClientManager::new(
        pool.clone(),
        false, // yolo_mode disabled in tests
        120,   // mcp_request_timeout_secs
        30,    // mcp_stream_idle_timeout_secs
    ));
    let registry = Arc::new(PluginRegistry::new(5, 10, 50, mcp_manager.clone()));
    let agent_manager = AgentManager::new(pool.clone(), 90_000);
    let plugin_manager = Arc::new(PluginManager::new(pool.clone(), vec![], 30, 10, 50).unwrap());

    let metrics = Arc::new(SystemMetrics::new());
    let event_history = Arc::new(RwLock::new(VecDeque::new()));

    let mut config = AppConfig::load().unwrap();
    config.admin_api_key = admin_api_key;
    let config_admin_key = config.admin_api_key.clone();

    let rate_limiter = Arc::new(crate::middleware::RateLimiter::new(
        config.rate_limit_per_sec,
        config.rate_limit_burst,
    ));

    let shutdown = crate::shutdown::ShutdownSignal::new();

    Arc::new(crate::AppState {
        tx,
        registry,
        event_tx,
        pool,
        agent_manager,
        plugin_manager,
        mcp_manager,
        config,
        data_dir,
        event_history,
        metrics,
        rate_limiter,
        shutdown,
        revoked_keys: Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new())),
        admin_api_key: std::sync::RwLock::new(config_admin_key),
        pending_command_approvals: Arc::new(dashmap::DashMap::new()),
        session_trusted_commands: Arc::new(dashmap::DashMap::new()),
        active_cron_contexts: Arc::new(dashmap::DashMap::new()),
        max_cron_generation: Arc::new(std::sync::atomic::AtomicU8::new(2)),
        setup_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        install_task: Arc::new(tokio::sync::Mutex::new(None)),
        setup_done: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        setup_progress_tx: {
            let (tx, _) = tokio::sync::broadcast::channel(64);
            tx
        },
        marketplace_cache: Arc::new(tokio::sync::RwLock::new(
            crate::handlers::marketplace::CatalogCache::default(),
        )),
        seal_jwks_cache: Arc::new(tokio::sync::RwLock::new(
            crate::handlers::marketplace::JwksCache::default(),
        )),
        install_limiter: Arc::new(crate::middleware::RateLimiter::per_minute(5, 5)),
        last_health_report: Arc::new(tokio::sync::RwLock::new(None)),
        provider_probe_cache: crate::managers::provider_probe::ProbeCache::new(),
        last_usage: crate::managers::usage_tracker::UsageStore::new(),
        session_manager: Arc::new(crate::managers::session_manager::SessionManager::new()),
    })
}
