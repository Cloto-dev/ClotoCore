//! Database migration and initialization tests.
//! Tests that DB init is idempotent and creates required tables.

use sqlx::SqlitePool;

async fn fresh_pool() -> SqlitePool {
    SqlitePool::connect("sqlite::memory:").await.unwrap()
}

#[tokio::test]
async fn test_db_init_is_idempotent() {
    let pool = fresh_pool().await;

    // Running init_db twice should not fail (idempotent migrations)
    exiv_core::db::init_db(&pool, "sqlite::memory:").await.unwrap();
    exiv_core::db::init_db(&pool, "sqlite::memory:").await.unwrap();
}

#[tokio::test]
async fn test_migration_creates_required_tables() {
    let pool = fresh_pool().await;
    exiv_core::db::init_db(&pool, "sqlite::memory:").await.unwrap();

    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    let table_names: Vec<String> = tables.into_iter().map(|(n,)| n).collect();

    // Core tables required for operation
    for required in &["agents", "plugin_settings", "plugin_configs", "plugin_data", "audit_logs"] {
        assert!(
            table_names.contains(&(*required).to_string()),
            "Required table '{}' not found; existing tables: {:?}",
            required,
            table_names
        );
    }
}

#[tokio::test]
async fn test_plugin_data_store_basic_roundtrip() {
    use std::sync::Arc;
    use exiv_shared::PluginDataStore;

    let pool = fresh_pool().await;
    exiv_core::db::init_db(&pool, "sqlite::memory:").await.unwrap();

    let store = Arc::new(exiv_core::db::SqliteDataStore::new(pool));
    let value = serde_json::json!({"hello": "world", "n": 42});

    store.set_json("test.plugin", "my_key", value.clone()).await.unwrap();
    let retrieved = store.get_json("test.plugin", "my_key").await.unwrap();
    assert_eq!(retrieved, Some(value));
}

#[tokio::test]
async fn test_plugin_data_store_missing_key_returns_none() {
    use std::sync::Arc;
    use exiv_shared::PluginDataStore;

    let pool = fresh_pool().await;
    exiv_core::db::init_db(&pool, "sqlite::memory:").await.unwrap();

    let store = Arc::new(exiv_core::db::SqliteDataStore::new(pool));
    let result = store.get_json("test.plugin", "nonexistent_key").await.unwrap();
    assert!(result.is_none());
}
