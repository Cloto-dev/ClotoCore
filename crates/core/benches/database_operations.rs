// Database Operations Benchmarks
// Critical paths:
// - cloto_core/src/db.rs:21-30 (SqliteDataStore::set_json)
// - cloto_core/src/db.rs:32-45 (SqliteDataStore::get_json)
// - cloto_core/src/managers.rs:274-291 (PluginManager::fetch_plugin_configs)

use cloto_core::db::SqliteDataStore;
use cloto_shared::PluginDataStore;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use sqlx::SqlitePool;

mod helpers;

fn json_serialization_benchmark(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("db_json_operations");

    for size in &[10, 100, 1000] {
        let json_data = serde_json::json!({
            "data": vec!["test".to_string(); *size],
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });

        group.bench_with_input(BenchmarkId::new("set_json", size), &json_data, |b, data| {
            b.to_async(&runtime).iter(|| async {
                let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
                cloto_core::db::init_db(&pool, "sqlite::memory:")
                    .await
                    .unwrap();

                let store = SqliteDataStore::new(pool);

                // Critical path: JSON serialization + SQLite insert
                // From cloto_core/src/db.rs:21-30
                store
                    .set_json("test_plugin", "bench_key", data.clone())
                    .await
                    .unwrap();
            });
        });
    }
    group.finish();
}

fn get_json_benchmark(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("db_get_json", |b| {
        b.to_async(&runtime).iter(|| async {
            let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
            cloto_core::db::init_db(&pool, "sqlite::memory:")
                .await
                .unwrap();
            let store = SqliteDataStore::new(pool);

            // Setup: insert test data
            let data = serde_json::json!({"value": "test", "count": 42});
            store.set_json("test_plugin", "key", data).await.unwrap();

            // Benchmark: get_json with deserialization
            // From cloto_core/src/db.rs:32-45
            let result = store.get_json("test_plugin", "key").await.unwrap();
            black_box(result);
        });
    });
}

fn get_all_json_benchmark(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("db_get_all_json", |b| {
        b.to_async(&runtime).iter(|| async {
            let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
            cloto_core::db::init_db(&pool, "sqlite::memory:")
                .await
                .unwrap();
            let store = SqliteDataStore::new(pool);

            // Setup: insert multiple test entries
            for i in 0..100 {
                let data = serde_json::json!({"index": i, "value": format!("test_{}", i)});
                store
                    .set_json("test_plugin", &format!("mem:agent:key_{}", i), data)
                    .await
                    .unwrap();
            }

            // Benchmark: get_all_json with LIKE query
            // From cloto_core/src/db.rs:47-62
            let results = store.get_all_json("test_plugin", "mem:").await.unwrap();
            black_box(results);
        });
    });
}

fn plugin_config_operations(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("plugin_config_update", |b| {
        b.to_async(&runtime).iter(|| async {
            let state = helpers::create_bench_app_state().await;

            // Benchmark: update plugin config
            // Involves SQLite INSERT OR REPLACE
            state
                .plugin_manager
                .update_config("test.plugin", "api_key", "test_value")
                .await
                .unwrap();
        });
    });
}

criterion_group!(
    benches,
    json_serialization_benchmark,
    get_json_benchmark,
    get_all_json_benchmark,
    plugin_config_operations
);
criterion_main!(benches);
