mod agents;
mod plugin;
mod registry;

pub use agents::AgentManager;
pub use plugin::PluginManager;
pub use registry::{PluginRegistry, PluginSetting, SystemMetrics};
