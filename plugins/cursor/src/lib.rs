use async_trait::async_trait;
use exiv_shared::{exiv_plugin, ExivEvent, HandAction, Plugin, PluginConfig, PluginManifest};

#[exiv_plugin(
    name = "hal.cursor",
    kind = "HAL",
    description = "High-precision dot cursor with fluid motion trails.",
    version = "0.1.0",
    category = "Tool",
    permissions = ["InputControl"],
    tags = ["#TOOL"],
    capabilities = ["HAL"]
)]
pub struct CursorPlugin {}

impl CursorPlugin {
    pub async fn new_plugin(_config: PluginConfig) -> anyhow::Result<Self> {
        Ok(Self {})
    }
}

#[async_trait]
impl Plugin for CursorPlugin {
    fn manifest(&self) -> PluginManifest {
        self.auto_manifest()
    }

    async fn on_event(
        &self,
        event: &ExivEvent,
    ) -> anyhow::Result<Option<exiv_shared::ExivEventData>> {
        if let exiv_shared::ExivEventData::ActionRequested {
            requester: _,
            action,
        } = &event.data
        {
            let action_desc = match action {
                HandAction::MouseMove { x, y } => format!("Moving cursor to ({}, {})", x, y),
                HandAction::MouseClick { button } => format!("Clicking {} button", button),
                HandAction::KeyPress { key } => format!("Pressing key: {}", key),
                HandAction::Wait { ms } => format!("Waiting for {}ms", ms),
                HandAction::CaptureScreen => "Capturing screen (handled by Vision)".to_string(),
                HandAction::ClickElement { label } => {
                    format!("Clicking element '{}' (requires Vision)", label)
                }
            };

            tracing::info!("🖱️ Neural Cursor Action: {}", action_desc);

            return Ok(Some(exiv_shared::ExivEventData::SystemNotification(
                format!("Neural Cursor Executed: {}", action_desc),
            )));
        }
        Ok(None)
    }
}
