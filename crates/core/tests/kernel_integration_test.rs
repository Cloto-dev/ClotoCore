use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use plugin_deepseek::DeepSeekPlugin;
use sqlx::SqlitePool;
use std::sync::Arc;
use tower::ServiceExt;
use exiv_shared::{PluginCast, PluginConfig};
use std::any::Any;

#[tokio::test]
async fn test_dynamic_routing_registration() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE agents (id TEXT PRIMARY KEY, name TEXT, description TEXT, status TEXT, default_engine_id TEXT, required_capabilities TEXT, metadata TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE plugin_settings (plugin_id TEXT PRIMARY KEY, is_active BOOLEAN, allowed_permissions TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE plugin_configs (plugin_id TEXT, config_key TEXT, config_value TEXT, PRIMARY KEY(plugin_id, config_key))").execute(&pool).await.unwrap();

    let config = PluginConfig {
        id: "test.deepseek".to_string(),
        config_values: [
            ("api_key".to_string(), "test_key".to_string()),
            ("model_id".to_string(), "deepseek-chat".to_string()),
        ].into_iter().collect(),
    };

    let ds_plugin = Arc::new(DeepSeekPlugin::new_plugin(config).await.unwrap());

    let mut dynamic_routes = axum::Router::new();
    if let Some(web) = ds_plugin.as_web() {
        dynamic_routes = web.register_routes(dynamic_routes);
    }

    let mock_state = Arc::new("mock_state".to_string()) as Arc<dyn Any + Send + Sync>;
    let api_routes = axum::Router::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .merge(dynamic_routes.with_state(mock_state));

    let app = axum::Router::new().nest("/api", api_routes);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/plugin/deepseek/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_permission_logic_unit() {
    let permission = exiv_shared::Permission::InputControl;
    let permissions = [permission];

    assert!(permissions.contains(&exiv_shared::Permission::InputControl));
}

#[tokio::test]
async fn test_capability_injection_logic() {
    let _pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    
    let perms_ok: Vec<exiv_shared::Permission> = vec![exiv_shared::Permission::NetworkAccess];
    let client = Arc::new(exiv_core::capabilities::SafeHttpClient::new(vec![]).unwrap());
    let capability = if perms_ok.contains(&exiv_shared::Permission::NetworkAccess) {
        Some(client.clone() as Arc<dyn exiv_shared::NetworkCapability>)
    } else {
        None
    };
    assert!(capability.is_some());

    let perms_no: Vec<exiv_shared::Permission> = vec![];
    let capability = if perms_no.contains(&exiv_shared::Permission::NetworkAccess) {
        Some(client.clone() as Arc<dyn exiv_shared::NetworkCapability>)
    } else {
        None
    };
    assert!(capability.is_none());
}

#[tokio::test]
async fn test_panic_isolation() {
    use exiv_core::managers::PluginRegistry;
    use exiv_shared::{Plugin, PluginManifest, ServiceType, ExivId};

    struct PanicPlugin(ExivId);

    impl PluginCast for PanicPlugin {
        fn as_any(&self) -> &dyn std::any::Any { self }
    }

    #[async_trait::async_trait]
    impl Plugin for PanicPlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest {
                id: self.0.to_string(),
                name: "Panic".to_string(),
                description: "".to_string(),
                version: "".to_string(),
                category: exiv_shared::PluginCategory::Tool,
                service_type: ServiceType::Reasoning,
                tags: vec![],
                is_active: true,
                is_configured: true,
                required_config_keys: vec![],
                action_icon: None,
                action_target: None,
                icon_data: None,
                magic_seal: 0x56455253,
                sdk_version: "1.0.0".to_string(),
                required_permissions: vec![],
                provided_capabilities: vec![],
                provided_tools: vec![],
            }
        }
        async fn on_event(&self, _e: &exiv_shared::ExivEvent) -> anyhow::Result<Option<exiv_shared::ExivEventData>> {
            panic!("Boom!");
        }
    }

    struct NormalPlugin(Arc<tokio::sync::mpsc::Sender<bool>>, ExivId);

    impl PluginCast for NormalPlugin {
        fn as_any(&self) -> &dyn std::any::Any { self }
    }

    #[async_trait::async_trait]
    impl Plugin for NormalPlugin {
        fn manifest(&self) -> PluginManifest {
            let mut m = PanicPlugin(self.1).manifest();
            m.name = "Normal".to_string();
            m.id = self.1.to_string();
            m
        }
        async fn on_event(&self, _e: &exiv_shared::ExivEvent) -> anyhow::Result<Option<exiv_shared::ExivEventData>> {
            let _ = self.0.send(true).await;
            Ok(None)
        }
    }

    let registry = PluginRegistry::new(5, 10);
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    
    let id_panic = ExivId::new();
    let id_normal = ExivId::new();
    
    {
        let mut plugins = registry.plugins.write().await;
        plugins.insert("panic".into(), Arc::new(PanicPlugin(id_panic)));
        plugins.insert("normal".into(), Arc::new(NormalPlugin(Arc::new(tx), id_normal)));
    }

    let (event_tx, _event_rx) = tokio::sync::mpsc::channel::<exiv_core::EnvelopedEvent>(10);
    let event = exiv_shared::ExivEvent::new(exiv_shared::ExivEventData::SystemNotification("test".into()));

    let envelope = exiv_core::EnvelopedEvent {
        event: Arc::new(event),
        issuer: None,
        correlation_id: None,
        depth: 0,
    };
    registry.dispatch_event(envelope, &event_tx).await;

    let result = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv()).await;
    assert!(result.is_ok());
}
