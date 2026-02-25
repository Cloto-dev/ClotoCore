//! Concurrent event dispatch tests.
//! Verifies no deadlocks or crashes under high event load.

mod common;

use cloto_core::managers::PluginRegistry;
use cloto_core::EnvelopedEvent;
use cloto_shared::ClotoId;
use std::sync::Arc;

#[tokio::test]
async fn test_concurrent_event_dispatch_100() {
    use common::create_mock_plugin;

    let registry = Arc::new(PluginRegistry::new(5, 10));
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel::<EnvelopedEvent>(256);

    // Register 5 mock plugins
    for i in 0..5 {
        let id = ClotoId::new();
        let (plugin, _) = create_mock_plugin(id);
        let mut plugins = registry.plugins.write().await;
        plugins.insert(
            format!("mock_{}", i),
            plugin as Arc<dyn cloto_shared::Plugin>,
        );
    }

    // Dispatch 100 concurrent events
    let mut handles = Vec::new();
    for _ in 0..100 {
        let registry = registry.clone();
        let event_tx = event_tx.clone();
        let handle = tokio::spawn(async move {
            let event = cloto_shared::ClotoEvent::new(
                cloto_shared::ClotoEventData::SystemNotification("concurrent".into()),
            );
            let envelope = EnvelopedEvent {
                event: Arc::new(event),
                issuer: None,
                correlation_id: None,
                depth: 0,
            };
            registry.dispatch_event(envelope, &event_tx).await;
        });
        handles.push(handle);
    }

    // All 100 dispatches should complete without deadlock
    let results = futures::future::join_all(handles).await;
    for r in results {
        r.expect("Concurrent event dispatch panicked");
    }
}

#[tokio::test]
async fn test_event_depth_limit_prevents_infinite_loop() {
    use common::create_mock_plugin;

    let registry = PluginRegistry::new(2, 2); // depth limit = 2
    let id = ClotoId::new();
    let (plugin, received) = create_mock_plugin(id);

    {
        let mut plugins = registry.plugins.write().await;
        plugins.insert("mock".into(), plugin as Arc<dyn cloto_shared::Plugin>);
    }

    let (event_tx, _event_rx) = tokio::sync::mpsc::channel::<EnvelopedEvent>(10);
    let event = cloto_shared::ClotoEvent::new(cloto_shared::ClotoEventData::SystemNotification(
        "depth_test".into(),
    ));

    // depth = 0: allowed
    let envelope = EnvelopedEvent {
        event: Arc::new(event.clone()),
        issuer: None,
        correlation_id: None,
        depth: 0,
    };
    registry.dispatch_event(envelope, &event_tx).await;

    let count_at_depth_0 = received.lock().await.len();
    assert_eq!(count_at_depth_0, 1, "Event at depth 0 should be dispatched");

    // depth = 2: equals max_event_depth, should be dropped
    let envelope_deep = EnvelopedEvent {
        event: Arc::new(event),
        issuer: None,
        correlation_id: None,
        depth: 2,
    };
    registry.dispatch_event(envelope_deep, &event_tx).await;

    let count_after = received.lock().await.len();
    assert_eq!(
        count_after, 1,
        "Event at max depth must be dropped (depth limit enforced)"
    );
}

#[tokio::test]
async fn test_dispatch_with_no_plugins_is_safe() {
    let registry = PluginRegistry::new(5, 10);
    let (event_tx, _) = tokio::sync::mpsc::channel::<EnvelopedEvent>(10);

    let event = cloto_shared::ClotoEvent::new(cloto_shared::ClotoEventData::SystemNotification(
        "empty_registry".into(),
    ));
    let envelope = EnvelopedEvent {
        event: Arc::new(event),
        issuer: None,
        correlation_id: None,
        depth: 0,
    };

    // Should complete immediately with no plugins registered
    registry.dispatch_event(envelope, &event_tx).await;
}
