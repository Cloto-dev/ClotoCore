// Benchmark helpers module
// Reusable infrastructure for Exiv performance benchmarks
// Pattern inspired by: exiv_core/tests/handlers_http_test.rs:18-60

use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock, Notify};
use exiv_core::{
    managers::{PluginRegistry, PluginManager, AgentManager, SystemMetrics},
    DynamicRouter, AppState,
    config::AppConfig,
};
use std::collections::VecDeque;
use exiv_shared::{ExivEvent, ExivEventData, ExivMessage, MessageSource};

/// Reusable helper to create test AppState for benchmarks
/// Uses larger buffer sizes (1000 vs 100) for high-throughput scenarios
#[allow(dead_code)]
pub async fn create_bench_app_state() -> Arc<AppState> {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    exiv_core::db::init_db(&pool, "sqlite::memory:").await.unwrap();

    let (event_tx, _event_rx) = mpsc::channel(1000); // Larger buffer for benchmarks
    let (tx, _rx) = broadcast::channel(1000);

    let registry = Arc::new(PluginRegistry::new(5, 10));
    let agent_manager = AgentManager::new(pool.clone());
    let plugin_manager = Arc::new(PluginManager::new(
        pool.clone(),
        vec![],
        30,
        10,
    ).unwrap());

    let dynamic_router = Arc::new(DynamicRouter {
        router: RwLock::new(axum::Router::new()),
    });

    let metrics = Arc::new(SystemMetrics::new());
    let event_history = Arc::new(RwLock::new(VecDeque::new()));

    let mut config = AppConfig::load().unwrap();
    config.admin_api_key = Some("bench-key".to_string());

    let rate_limiter = Arc::new(exiv_core::middleware::RateLimiter::new(100, 200));

    Arc::new(AppState {
        tx,
        registry,
        event_tx,
        pool,
        agent_manager,
        plugin_manager,
        dynamic_router,
        config,
        event_history,
        metrics,
        rate_limiter,
        shutdown: Arc::new(Notify::new()),
    })
}

/// Create a simple test event for benchmarking
#[allow(dead_code)]
pub fn create_test_event(message: String) -> Arc<ExivEvent> {
    let msg = ExivMessage::new(
        MessageSource::User {
            id: "bench_user".to_string(),
            name: "Benchmark User".to_string(),
        },
        message,
    );
    Arc::new(ExivEvent::new(ExivEventData::MessageReceived(msg)))
}

/// Create an enveloped event for dispatch benchmarks
#[allow(dead_code)]
pub fn create_enveloped_event(message: String) -> exiv_core::EnvelopedEvent {
    exiv_core::EnvelopedEvent {
        event: create_test_event(message),
        issuer: None,
        correlation_id: None,
        depth: 0,
    }
}
