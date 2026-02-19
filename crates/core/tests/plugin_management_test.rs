use sqlx::SqlitePool;
use std::sync::Arc;
use exiv_shared::{PluginConfig, PluginFactory, ServiceType, Plugin, PluginManifest};
use exiv_core::managers::PluginManager;

struct MockFactory;

#[async_trait::async_trait]
impl PluginFactory for MockFactory {
    fn name(&self) -> &'static str { "test.mock" }
    fn service_type(&self) -> ServiceType { ServiceType::Skill }
    async fn create(&self, _config: PluginConfig) -> anyhow::Result<Arc<dyn Plugin>> {
        Ok(Arc::new(MockPlugin))
    }
}

struct MockPlugin;

impl exiv_shared::PluginCast for MockPlugin {
    fn as_any(&self) -> &dyn std::any::Any { self }
}

#[async_trait::async_trait]
impl Plugin for MockPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "test.mock".to_string(),
            name: "Mock".to_string(),
            description: String::new(),
            version: "1.0.0".to_string(),
            category: exiv_shared::PluginCategory::Tool,
            service_type: ServiceType::Skill,
            tags: vec![],
            is_active: true,
            is_configured: true,
            required_config_keys: vec![],
            action_icon: None,
            action_target: None,
            icon_data: None,
            magic_seal: 0x56455253, // Valid Seal
            sdk_version: "0.1.0".to_string(),
            required_permissions: vec![],
            provided_capabilities: vec![],
            provided_tools: vec![],
        }
    }
}

#[tokio::test]
async fn test_plugin_bootstrap_with_valid_seal() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE plugin_settings (plugin_id TEXT PRIMARY KEY, is_active BOOLEAN, allowed_permissions TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE plugin_configs (plugin_id TEXT, config_key TEXT, config_value TEXT, PRIMARY KEY(plugin_id, config_key))").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE plugin_data (plugin_id TEXT, key TEXT, value TEXT, PRIMARY KEY(plugin_id, key))").execute(&pool).await.unwrap();

    let mut manager = PluginManager::new(pool.clone(), vec![], 5, 10).unwrap();
    manager.register_factory(Arc::new(MockFactory));

    // Simulate enabled plugin in DB
    sqlx::query("INSERT INTO plugin_settings (plugin_id, is_active, allowed_permissions) VALUES ('test.mock', 1, '[]')")
        .execute(&pool).await.unwrap();

    let registry = manager.initialize_all().await.unwrap();
    let plugins = registry.plugins.read().await;
    assert!(plugins.contains_key("test.mock"));
}

struct InvalidSealPlugin;

impl exiv_shared::PluginCast for InvalidSealPlugin {
    fn as_any(&self) -> &dyn std::any::Any { self }
}

#[async_trait::async_trait]
impl Plugin for InvalidSealPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut m = MockPlugin.manifest();
        m.magic_seal = 0xBAADF00D;
        m
    }
}

struct InvalidFactory;
#[async_trait::async_trait]
impl PluginFactory for InvalidFactory {
    fn name(&self) -> &'static str { "test.invalid" }
    fn service_type(&self) -> ServiceType { ServiceType::Skill }
    async fn create(&self, _config: PluginConfig) -> anyhow::Result<Arc<dyn Plugin>> {
        Ok(Arc::new(InvalidSealPlugin))
    }
}

#[tokio::test]
async fn test_plugin_bootstrap_fails_with_invalid_seal() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE plugin_settings (plugin_id TEXT PRIMARY KEY, is_active BOOLEAN, allowed_permissions TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE plugin_configs (plugin_id TEXT, config_key TEXT, config_value TEXT, PRIMARY KEY(plugin_id, config_key))").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE plugin_data (plugin_id TEXT, key TEXT, value TEXT, PRIMARY KEY(plugin_id, key))").execute(&pool).await.unwrap();

    let mut manager = PluginManager::new(pool.clone(), vec![], 5, 10).unwrap();
    manager.register_factory(Arc::new(InvalidFactory));

    sqlx::query("INSERT INTO plugin_settings (plugin_id, is_active, allowed_permissions) VALUES ('test.invalid', 1, '[]')")
        .execute(&pool).await.unwrap();

    let registry = manager.initialize_all().await.unwrap();
    let plugins = registry.plugins.read().await;
    // Should NOT contain the plugin because bootstrap should fail and log error
    assert!(!plugins.contains_key("test.invalid"));
}
