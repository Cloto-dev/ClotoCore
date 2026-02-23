mod common;

use sqlx::SqlitePool;
use std::sync::Arc;

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
    use common::{create_mock_plugin, create_panicking_plugin};
    use exiv_core::managers::PluginRegistry;
    use exiv_shared::ExivId;

    let registry = PluginRegistry::new(5, 10);
    let id_panic = ExivId::new();
    let id_normal = ExivId::new();
    let (normal_plugin, received_events) = create_mock_plugin(id_normal);

    {
        let mut plugins = registry.plugins.write().await;
        plugins.insert(
            "panic".into(),
            create_panicking_plugin(id_panic) as Arc<dyn exiv_shared::Plugin>,
        );
        plugins.insert(
            "normal".into(),
            normal_plugin as Arc<dyn exiv_shared::Plugin>,
        );
    }

    let (event_tx, _event_rx) = tokio::sync::mpsc::channel::<exiv_core::EnvelopedEvent>(10);
    let event = exiv_shared::ExivEvent::new(exiv_shared::ExivEventData::SystemNotification(
        "test".into(),
    ));

    let envelope = exiv_core::EnvelopedEvent {
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
