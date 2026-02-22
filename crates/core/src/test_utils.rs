use crate::config::AppConfig;
use crate::managers::{AgentManager, PluginManager, PluginRegistry, SystemMetrics};
use crate::DynamicRouter;
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Notify, RwLock};

pub async fn create_test_app_state(admin_api_key: Option<String>) -> Arc<crate::AppState> {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    crate::db::init_db(&pool, "sqlite::memory:").await.unwrap();

    let (event_tx, _event_rx) = mpsc::channel(100);
    let (tx, _rx) = broadcast::channel(100);

    let registry = Arc::new(PluginRegistry::new(5, 10));
    let agent_manager = AgentManager::new(pool.clone());
    let plugin_manager = Arc::new(PluginManager::new(pool.clone(), vec![], 30, 10).unwrap());

    let dynamic_router = Arc::new(DynamicRouter {
        router: RwLock::new(axum::Router::new()),
    });

    let metrics = Arc::new(SystemMetrics::new());
    let event_history = Arc::new(RwLock::new(VecDeque::new()));

    let mut config = AppConfig::load().unwrap();
    config.admin_api_key = admin_api_key;

    let rate_limiter = Arc::new(crate::middleware::RateLimiter::new(10, 20));

    let shutdown = Arc::new(Notify::new());
    let mcp_manager = Arc::new(crate::managers::McpClientManager::new(
        pool.clone(),
        shutdown.clone(),
        false, // yolo_mode disabled in tests
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
        event_history,
        metrics,
        rate_limiter,
        shutdown,
        evolution_engine: None,
        fitness_collector: None,
        revoked_keys: Arc::new(std::sync::RwLock::new(std::collections::HashSet::new())),
    })
}
