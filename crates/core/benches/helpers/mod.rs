// Benchmark helpers module
// Reusable infrastructure for Cloto performance benchmarks
// Pattern inspired by: cloto_core/tests/handlers_http_test.rs:18-60

use cloto_core::{
    config::AppConfig,
    managers::{AgentManager, McpClientManager, PluginManager, PluginRegistry, SystemMetrics},
    AppState, DynamicRouter,
};
use cloto_shared::{ClotoEvent, ClotoEventData, ClotoMessage, MessageSource};
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Notify, RwLock};

/// Reusable helper to create test AppState for benchmarks
/// Uses larger buffer sizes (1000 vs 100) for high-throughput scenarios
#[allow(dead_code)]
pub async fn create_bench_app_state() -> Arc<AppState> {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    cloto_core::db::init_db(&pool, "sqlite::memory:")
        .await
        .unwrap();

    let (event_tx, _event_rx) = mpsc::channel(1000); // Larger buffer for benchmarks
    let (tx, _rx) = broadcast::channel(1000);

    let registry = Arc::new(PluginRegistry::new(5, 10));
    let agent_manager = AgentManager::new(pool.clone());
    let plugin_manager = Arc::new(PluginManager::new(pool.clone(), vec![], 30, 10).unwrap());

    let dynamic_router = Arc::new(DynamicRouter {
        router: RwLock::new(axum::Router::new()),
    });

    let metrics = Arc::new(SystemMetrics::new());
    let event_history = Arc::new(RwLock::new(VecDeque::new()));

    let mut config = AppConfig::load().unwrap();
    config.admin_api_key = Some("bench-key".to_string());

    let rate_limiter = Arc::new(cloto_core::middleware::RateLimiter::new(100, 200));

    let mcp_manager = Arc::new(McpClientManager::new(pool.clone(), false));

    Arc::new(AppState {
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
        shutdown: Arc::new(Notify::new()),
        evolution_engine: None,
        fitness_collector: None,
        revoked_keys: Arc::new(std::sync::RwLock::new(std::collections::HashSet::new())),
    })
}

/// Create a simple test event for benchmarking
#[allow(dead_code)]
pub fn create_test_event(message: String) -> Arc<ClotoEvent> {
    let msg = ClotoMessage::new(
        MessageSource::User {
            id: "bench_user".to_string(),
            name: "Benchmark User".to_string(),
        },
        message,
    );
    Arc::new(ClotoEvent::new(ClotoEventData::MessageReceived(msg)))
}

/// Create an enveloped event for dispatch benchmarks
#[allow(dead_code)]
pub fn create_enveloped_event(message: String) -> cloto_core::EnvelopedEvent {
    cloto_core::EnvelopedEvent {
        event: create_test_event(message),
        issuer: None,
        correlation_id: None,
        depth: 0,
    }
}
