use anyhow::Result;
use async_trait::async_trait;
use exiv_shared::{
    ExivEvent, ExivEventData, ExivId, Plugin, PluginCast, PluginManifest, ServiceType,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub struct MockPlugin {
    pub manifest: PluginManifest,
    pub received_events: Arc<Mutex<Vec<ExivEvent>>>,
    pub should_panic: bool,
    pub response_delay: Duration,
    pub response: Option<ExivEventData>,
}

impl PluginCast for MockPlugin {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait]
impl Plugin for MockPlugin {
    fn manifest(&self) -> PluginManifest {
        self.manifest.clone()
    }
    async fn on_event(&self, event: &ExivEvent) -> Result<Option<ExivEventData>> {
        self.received_events.lock().await.push(event.clone());
        assert!(!self.should_panic, "Intentional test panic");
        tokio::time::sleep(self.response_delay).await;
        Ok(self.response.clone())
    }
}

fn base_manifest(id: ExivId, name: &str) -> PluginManifest {
    PluginManifest {
        id: id.to_string(),
        name: name.to_string(),
        description: String::new(),
        version: "0.0.0".to_string(),
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

/// Standard mock plugin: records events, returns None.
pub fn create_mock_plugin(id: ExivId) -> (Arc<MockPlugin>, Arc<Mutex<Vec<ExivEvent>>>) {
    let received_events = Arc::new(Mutex::new(Vec::new()));
    let plugin = Arc::new(MockPlugin {
        manifest: base_manifest(id, "MockPlugin"),
        received_events: received_events.clone(),
        should_panic: false,
        response_delay: Duration::ZERO,
        response: None,
    });
    (plugin, received_events)
}

/// Slow mock plugin: introduces a configurable delay before returning.
pub fn create_slow_plugin(id: ExivId, delay: Duration) -> Arc<MockPlugin> {
    Arc::new(MockPlugin {
        manifest: base_manifest(id, "SlowPlugin"),
        received_events: Arc::new(Mutex::new(Vec::new())),
        should_panic: false,
        response_delay: delay,
        response: None,
    })
}

/// Panicking mock plugin: panics on every on_event call.
pub fn create_panicking_plugin(id: ExivId) -> Arc<MockPlugin> {
    Arc::new(MockPlugin {
        manifest: base_manifest(id, "PanickingPlugin"),
        received_events: Arc::new(Mutex::new(Vec::new())),
        should_panic: true,
        response_delay: Duration::ZERO,
        response: None,
    })
}
