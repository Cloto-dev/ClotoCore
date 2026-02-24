mod common;

use sqlx::SqlitePool;
use std::sync::Arc;

#[tokio::test]
async fn test_permission_logic_unit() {
    let permission = cloto_shared::Permission::InputControl;
    let permissions = [permission];

    assert!(permissions.contains(&cloto_shared::Permission::InputControl));
}

#[tokio::test]
async fn test_capability_injection_logic() {
    let _pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

    let perms_ok: Vec<cloto_shared::Permission> = vec![cloto_shared::Permission::NetworkAccess];
    let client = Arc::new(cloto_core::capabilities::SafeHttpClient::new(vec![]).unwrap());
    let capability = if perms_ok.contains(&cloto_shared::Permission::NetworkAccess) {
        Some(client.clone() as Arc<dyn cloto_shared::NetworkCapability>)
    } else {
        None
    };
    assert!(capability.is_some());

    let perms_no: Vec<cloto_shared::Permission> = vec![];
    let capability = if perms_no.contains(&cloto_shared::Permission::NetworkAccess) {
        Some(client.clone() as Arc<dyn cloto_shared::NetworkCapability>)
    } else {
        None
    };
    assert!(capability.is_none());
}

#[tokio::test]
async fn test_panic_isolation() {
    use common::{create_mock_plugin, create_panicking_plugin};
    use cloto_core::managers::PluginRegistry;
    use cloto_shared::ClotoId;

    let registry = PluginRegistry::new(5, 10);
    let id_panic = ClotoId::new();
    let id_normal = ClotoId::new();
    let (normal_plugin, received_events) = create_mock_plugin(id_normal);

    {
        let mut plugins = registry.plugins.write().await;
        plugins.insert(
            "panic".into(),
            create_panicking_plugin(id_panic) as Arc<dyn cloto_shared::Plugin>,
        );
        plugins.insert(
            "normal".into(),
            normal_plugin as Arc<dyn cloto_shared::Plugin>,
        );
    }

    let (event_tx, _event_rx) = tokio::sync::mpsc::channel::<cloto_core::EnvelopedEvent>(10);
    let event = cloto_shared::ClotoEvent::new(cloto_shared::ClotoEventData::SystemNotification(
        "test".into(),
    ));

    let envelope = cloto_core::EnvelopedEvent {
        event: Arc::new(event),
        issuer: None,
        correlation_id: None,
        depth: 0,
    };
    registry.dispatch_event(envelope, &event_tx).await;

    // Normal plugin should have received the event despite panic plugin
    let events = received_events.lock().await;
    assert_eq!(events.len(), 1);
}
