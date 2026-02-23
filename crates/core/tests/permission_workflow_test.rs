//! Permission workflow tests.
//! Tests permission grant, effective permission tracking, and validation.

use exiv_core::managers::PluginRegistry;
use exiv_shared::{ExivId, Permission};
use std::sync::Arc;

#[tokio::test]
async fn test_update_effective_permissions_adds_permission() {
    let registry = PluginRegistry::new(5, 10);
    let plugin_id = ExivId::from_name("test.plugin");

    registry
        .update_effective_permissions(plugin_id, Permission::NetworkAccess)
        .await;

    let perms = registry.effective_permissions.read().await;
    let granted = perms.get(&plugin_id).unwrap();
    assert!(granted.contains(&Permission::NetworkAccess));
}

#[tokio::test]
async fn test_update_effective_permissions_no_duplicates() {
    let registry = PluginRegistry::new(5, 10);
    let plugin_id = ExivId::from_name("test.plugin");

    // Grant the same permission twice
    registry
        .update_effective_permissions(plugin_id, Permission::NetworkAccess)
        .await;
    registry
        .update_effective_permissions(plugin_id, Permission::NetworkAccess)
        .await;

    let perms = registry.effective_permissions.read().await;
    let granted = perms.get(&plugin_id).unwrap();
    assert_eq!(granted.len(), 1, "Duplicate permissions must not be stored");
}

#[tokio::test]
async fn test_update_effective_permissions_multiple_types() {
    let registry = PluginRegistry::new(5, 10);
    let plugin_id = ExivId::from_name("test.plugin");

    registry
        .update_effective_permissions(plugin_id, Permission::NetworkAccess)
        .await;
    registry
        .update_effective_permissions(plugin_id, Permission::InputControl)
        .await;

    let perms = registry.effective_permissions.read().await;
    let granted = perms.get(&plugin_id).unwrap();
    assert_eq!(granted.len(), 2);
    assert!(granted.contains(&Permission::NetworkAccess));
    assert!(granted.contains(&Permission::InputControl));
}

#[tokio::test]
async fn test_permissions_are_isolated_between_plugins() {
    let registry = PluginRegistry::new(5, 10);
    let plugin_a = ExivId::from_name("plugin.a");
    let plugin_b = ExivId::from_name("plugin.b");

    registry
        .update_effective_permissions(plugin_a, Permission::NetworkAccess)
        .await;

    let perms = registry.effective_permissions.read().await;
    // plugin_b should have no permissions
    assert!(
        perms.get(&plugin_b).is_none_or(std::vec::Vec::is_empty),
        "plugin.b must not inherit plugin.a's permissions"
    );
}

#[tokio::test]
async fn test_plugin_registry_list_plugins_empty() {
    let registry = PluginRegistry::new(5, 10);
    let manifests = registry.list_plugins().await;
    assert!(
        manifests.is_empty(),
        "Empty registry must return empty manifest list"
    );
}

#[tokio::test]
async fn test_plugin_registry_get_engine_missing_returns_none() {
    let registry = PluginRegistry::new(5, 10);
    let result = registry.get_engine("nonexistent.plugin").await;
    assert!(
        result.is_none(),
        "Getting nonexistent plugin must return None"
    );
}

#[tokio::test]
async fn test_plugin_registry_find_memory_empty_returns_none() {
    let registry = PluginRegistry::new(5, 10);
    let result = registry.find_memory().await;
    assert!(
        result.is_none(),
        "find_memory on empty registry must return None"
    );
}

#[tokio::test]
async fn test_db_permission_grant_roundtrip() {
    use sqlx::SqlitePool;

    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    exiv_core::db::init_db(&pool, "sqlite::memory:")
        .await
        .unwrap();

    // Insert a plugin setting
    sqlx::query(
        "INSERT INTO plugin_settings (plugin_id, is_active, allowed_permissions) VALUES (?, 1, '[]')"
    )
    .bind("test.plugin")
    .execute(&pool)
    .await
    .unwrap();

    // Use AgentManager pool is separate; test the SQL directly
    let manager = exiv_core::managers::PluginManager::new(pool.clone(), vec![], 5, 10).unwrap();

    // Grant permission via PluginManager
    manager
        .grant_permission(
            "test.plugin",
            Arc::new(Permission::NetworkAccess).as_ref().clone(),
        )
        .await
        .unwrap();

    // Read back and verify
    let row: (String,) =
        sqlx::query_as("SELECT allowed_permissions FROM plugin_settings WHERE plugin_id = ?")
            .bind("test.plugin")
            .fetch_one(&pool)
            .await
            .unwrap();

    let perms: Vec<Permission> = serde_json::from_str(&row.0).unwrap();
    assert!(
        perms.contains(&Permission::NetworkAccess),
        "Granted permission must be persisted"
    );
}
