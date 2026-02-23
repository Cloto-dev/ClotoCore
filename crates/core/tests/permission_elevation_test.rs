use exiv_core::events::EventProcessor;
use exiv_core::managers::{AgentManager, PluginManager, PluginRegistry};
use exiv_shared::{
    ExivEvent, ExivId, Permission, Plugin, PluginCapability, PluginCast, PluginManifest,
    ServiceType,
};
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

struct MockPlugin {
    id: String,
    injected: Arc<RwLock<bool>>,
}

impl MockPlugin {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            injected: Arc::new(RwLock::new(false)),
        }
    }
}

impl PluginCast for MockPlugin {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait::async_trait]
impl Plugin for MockPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: self.id.clone(),
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
            magic_seal: 0x56455253,
            sdk_version: "1.0.0".to_string(),
            required_permissions: vec![],
            provided_capabilities: vec![],
            provided_tools: vec![],
        }
    }

    async fn on_event(
        &self,
        _event: &ExivEvent,
    ) -> anyhow::Result<Option<exiv_shared::ExivEventData>> {
        Ok(None)
    }

    async fn on_capability_injected(&self, capability: PluginCapability) -> anyhow::Result<()> {
        if let PluginCapability::Network(_) = capability {
            let mut lock = self.injected.write().await;
            *lock = true;
        }
        Ok(())
    }
}

#[tokio::test]
async fn test_dynamic_permission_elevation_flow() {
    // 1. Setup Kernel Components
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    exiv_core::db::init_db(&pool, "sqlite::memory:")
        .await
        .unwrap();
    let registry_raw = PluginRegistry::new(5, 10);
    let plugin_manager = Arc::new(PluginManager::new(pool.clone(), vec![], 5, 10).unwrap());
    let agent_manager = AgentManager::new(pool.clone());
    let (tx_internal, _rx_internal) = tokio::sync::broadcast::channel(10);

    // 2. Register Mock Plugin
    let plugin_id = "test.mock";
    let mock_plugin = Arc::new(MockPlugin::new(plugin_id));
    let injected_flag = mock_plugin.injected.clone();

    {
        let mut plugins = registry_raw.plugins.write().await;
        plugins.insert(plugin_id.to_string(), mock_plugin.clone());
    }

    let registry = Arc::new(registry_raw);

    let metrics = Arc::new(exiv_core::managers::SystemMetrics::new());
    let event_history = Arc::new(tokio::sync::RwLock::new(VecDeque::new()));

    let processor = EventProcessor::new(
        registry.clone(),
        plugin_manager.clone(),
        agent_manager,
        tx_internal,
        event_history,
        metrics,
        1000, // max_history_size
        24,   // event_retention_hours
        None, // evolution_engine
        None, // fitness_collector
        None, // consensus
    );
    let (event_tx, event_rx) = mpsc::channel(10);

    // 3. Verify initial state (no permission)
    let exiv_id = ExivId::from_name(plugin_id);
    {
        let perms = registry.effective_permissions.read().await;
        assert!(!perms.contains_key(&exiv_id));
    }

    // 4. Simulate PermissionGranted Event
    let grant_event_data = exiv_shared::ExivEventData::PermissionGranted {
        plugin_id: plugin_id.to_string(),
        permission: Permission::NetworkAccess,
    };

    // Start processor in background
    let event_tx_clone = event_tx.clone();
    tokio::spawn(async move {
        processor.process_loop(event_rx, event_tx_clone).await;
    });

    // Send event
    event_tx
        .send(exiv_core::EnvelopedEvent {
            event: Arc::new(exiv_shared::ExivEvent::new(grant_event_data)),
            issuer: None,
            correlation_id: None,
            depth: 0,
        })
        .await
        .unwrap();

    // 5. Assert: Registry is updated
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    {
        let perms = registry.effective_permissions.read().await;
        assert!(perms.contains_key(&exiv_id));
        assert!(perms
            .get(&exiv_id)
            .unwrap()
            .contains(&Permission::NetworkAccess));
    }

    // 6. Assert: Plugin received the capability
    {
        let is_injected = injected_flag.read().await;
        assert!(
            *is_injected,
            "Capability should have been injected into the plugin"
        );
    }
}
