use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use exiv_shared::{
    Plugin, PluginConfig, PluginRuntimeContext, exiv_plugin, ExivEvent, ExivEventData,
    HandAction, ColorVisionData, DetectedElement, Permission, NetworkCapability,
};
use chrono::Utc;

#[exiv_plugin(
    name = "vision.screen",
    kind = "Vision",
    description = "Screen capture and analysis module.",
    version = "0.1.0",
    category = "Tool",
    permissions = ["VisionRead"],
    tags = ["#TOOL", "#VISION"],
    capabilities = ["Vision"]
)]
pub struct VisionPlugin {
    state: Arc<RwLock<VisionState>>,
}

struct VisionState {
    vision_read_granted: bool,
}

impl VisionPlugin {
    pub async fn new_plugin(_config: PluginConfig) -> anyhow::Result<Self> {
        Ok(Self {
            state: Arc::new(RwLock::new(VisionState { vision_read_granted: false })),
        })
    }
}

#[async_trait]
impl Plugin for VisionPlugin {
    fn manifest(&self) -> exiv_shared::PluginManifest {
        self.auto_manifest()
    }

    async fn on_plugin_init(
        &self,
        context: PluginRuntimeContext,
        _network: Option<Arc<dyn NetworkCapability>>,
    ) -> anyhow::Result<()> {
        let granted = context.effective_permissions.contains(&Permission::VisionRead);
        let mut state = self.state.write().await;
        state.vision_read_granted = granted;
        if !granted {
            tracing::warn!(
                "📷 vision.screen: VisionRead permission not granted — screen capture will be blocked"
            );
        }
        Ok(())
    }

    async fn on_event(&self, event: &ExivEvent) -> anyhow::Result<Option<ExivEventData>> {
        if let ExivEventData::ActionRequested { requester: _, action: HandAction::CaptureScreen } = &event.data {
            // 🔐 Enforce VisionRead: block capture if permission not granted
            let state = self.state.read().await;
            if !state.vision_read_granted {
                tracing::error!(
                    "🚫 SECURITY: vision.screen attempted screen capture without VisionRead permission"
                );
                return Err(anyhow::anyhow!(
                    "VisionRead permission required for screen capture"
                ));
            }

            // L-07: This is mock data - real implementation requires platform-specific screen capture
            tracing::warn!("📷 Vision Plugin: Returning MOCK screen capture data (not yet implemented)");

            let vision_data = ColorVisionData {
                captured_at: Utc::now(),
                detected_elements: vec![
                    DetectedElement {
                        label: "Submit Button".to_string(),
                        bounds: (100, 200, 50, 20),
                        confidence: 0.99,
                        attributes: std::collections::HashMap::new(),
                    }
                ],
                image_ref: Some("memory://mock-image-id".to_string()),
            };

            return Ok(Some(ExivEvent::with_trace(
                event.trace_id,
                ExivEventData::VisionUpdated(vision_data)
            ).data));
        }

        Ok(None)
    }
}
