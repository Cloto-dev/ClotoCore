// Benchmark helpers module
// Reusable infrastructure for Cloto performance benchmarks
// Pattern inspired by: cloto_core/tests/handlers_http_test.rs:18-60

use cloto_core::{
    config::AppConfig,
    managers::{AgentManager, McpClientManager, PluginManager, PluginRegistry, SystemMetrics},
    AppState,
};
use cloto_shared::{ClotoEvent, ClotoEventData, ClotoMessage, MessageSource};
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};

#[allow(dead_code)]
pub async fn create_bench_app_state() -> Arc<AppState> {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();

    let (event_tx, _event_rx) = mpsc::channel(1000); // Larger buffer for benchmarks
    let (tx, _rx) = broadcast::channel(1000);

    let mcp_manager = Arc::new(McpClientManager::new(pool.clone(), false, 30, 30));
    let registry = Arc::new(PluginRegistry::new(5, 10, 50, mcp_manager.clone()));
    let agent_manager = AgentManager::new(pool.clone(), 30_000);
    let plugin_manager = Arc::new(PluginManager::new(pool.clone(), vec![], 30, 10, 50).unwrap());

    let metrics = Arc::new(SystemMetrics::new());
    let event_history = Arc::new(RwLock::new(VecDeque::new()));

    let mut config = AppConfig::load().unwrap();
    config.admin_api_key = Some("bench-key".to_string());
    let admin_api_key = config.admin_api_key.clone();

    let rate_limiter = Arc::new(cloto_core::middleware::RateLimiter::new(100, 200));

    Arc::new(AppState {
        tx,
        registry,
        event_tx,
        pool,
        agent_manager,
        plugin_manager,
        mcp_manager,
        config,
        // Seeded from the config the same way the real boot path does
        // (`lib.rs`), so a bench authenticates with the key it just set.
        admin_api_key: std::sync::RwLock::new(admin_api_key),
        install_task: Arc::new(tokio::sync::Mutex::new(None)),
        data_dir: std::path::PathBuf::from("target/debug/data"),
        event_history,
        metrics,
        rate_limiter,
        shutdown: cloto_core::shutdown::ShutdownSignal::new(),
        revoked_keys: Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new())),
        pending_command_approvals: Arc::new(dashmap::DashMap::new()),
        session_trusted_commands: Arc::new(dashmap::DashMap::new()),
        active_cron_contexts: Arc::new(dashmap::DashMap::new()),
        max_cron_generation: Arc::new(AtomicU8::new(2)),
        setup_in_progress: Arc::new(AtomicBool::new(false)),
        setup_done: Arc::new(AtomicBool::new(true)),
        setup_progress_tx: broadcast::channel(100).0,
        marketplace_cache: Arc::new(tokio::sync::RwLock::new(
            cloto_core::handlers::marketplace::CatalogCache::default(),
        )),
        seal_jwks_cache: Arc::new(tokio::sync::RwLock::new(
            cloto_core::handlers::marketplace::JwksCache::default(),
        )),
        install_limiter: Arc::new(cloto_core::middleware::RateLimiter::new(5, 60)),
        last_health_report: Arc::new(tokio::sync::RwLock::new(None)),
        provider_probe_cache: cloto_core::managers::provider_probe::ProbeCache::new(),
        last_usage: cloto_core::managers::usage_tracker::UsageStore::new(),
        session_manager: Arc::new(cloto_core::managers::session_manager::SessionManager::new()),
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
