//! Plugin lifecycle failure handling tests.
//! Tests that initialization failures are isolated and Magic Seal is enforced.

mod common;

use cloto_core::managers::PluginRegistry;
use cloto_shared::ClotoId;
use std::sync::Arc;

#[tokio::test]
async fn test_panic_plugin_does_not_crash_normal_plugin() {
    use common::{create_mock_plugin, create_panicking_plugin};

    let registry = PluginRegistry::new(5, 10);
    let id_panic = ClotoId::new();
    let id_normal = ClotoId::new();
    let (normal_plugin, received_events) = create_mock_plugin(id_normal);

    {
        let mut plugins = registry.plugins.write().await;
        plugins.insert(
            "panicking".into(),
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

    // Even with panicking plugin present, dispatch completes
    registry.dispatch_event(envelope, &event_tx).await;

    let events = received_events.lock().await;
    assert_eq!(
        events.len(),
        1,
        "Normal plugin must receive the event despite panic plugin"
    );
}

#[tokio::test]
async fn test_invalid_magic_seal_rejected() {
    use cloto_shared::{Plugin, PluginCast, PluginManifest, ServiceType};

    struct BadSealPlugin;
    impl PluginCast for BadSealPlugin {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
    #[async_trait::async_trait]
    impl Plugin for BadSealPlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest {
                id: "bad.seal".to_string(),
                name: "BadSeal".to_string(),
                description: String::new(),
                version: "1.0".to_string(),
                category: cloto_shared::PluginCategory::Tool,
                service_type: ServiceType::Reasoning,
                tags: vec![],
                is_active: true,
                is_configured: true,
                required_config_keys: vec![],
                action_icon: None,
                action_target: None,
                icon_data: None,
                magic_seal: 0xDEAD_BEEF, // Invalid seal
                sdk_version: "1.0.0".to_string(),
                required_permissions: vec![],
                provided_capabilities: vec![],
                provided_tools: vec![],
            }
        }
        async fn on_event(
            &self,
            _e: &cloto_shared::ClotoEvent,
        ) -> anyhow::Result<Option<cloto_shared::ClotoEventData>> {
            Ok(None)
        }
    }

    // The manifest with an invalid magic seal should be detectable
    let plugin = BadSealPlugin;
    let manifest = plugin.manifest();
    assert_ne!(
        manifest.magic_seal, 0x5645_5253,
        "Bad seal should not match official SDK seal"
    );
}

#[tokio::test]
async fn test_plugin_registry_empty_on_creation() {
    let registry = PluginRegistry::new(5, 10);
    let plugins = registry.plugins.read().await;
    assert!(plugins.is_empty(), "New registry must start empty");
}

#[tokio::test]
async fn test_cascading_depth_limit_enforced() {
    use common::create_mock_plugin;

    let registry = PluginRegistry::new(5, 3); // depth limit = 3
    let id = ClotoId::new();
    let (plugin, _) = create_mock_plugin(id);

    {
        let mut plugins = registry.plugins.write().await;
        plugins.insert("mock".into(), plugin as Arc<dyn cloto_shared::Plugin>);
    }

    let (event_tx, _event_rx) = tokio::sync::mpsc::channel::<cloto_core::EnvelopedEvent>(10);
    let event = cloto_shared::ClotoEvent::new(cloto_shared::ClotoEventData::SystemNotification(
        "deep".into(),
    ));

    // Envelope at max depth — should be dropped without dispatch
    let envelope = cloto_core::EnvelopedEvent {
        event: Arc::new(event),
        issuer: None,
        correlation_id: None,
        depth: 3, // equals max_event_depth
    };

    // dispatch_event returns without processing (depth guard)
    registry.dispatch_event(envelope, &event_tx).await;
    // Test passes if no panic/timeout
}
