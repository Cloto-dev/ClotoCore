mod agents;
pub mod mcp;
pub mod mcp_protocol;
pub mod mcp_transport;
mod plugin;
mod registry;
pub mod scheduler;

pub use agents::AgentManager;
pub use mcp::McpClientManager;
pub use plugin::PluginManager;
pub use registry::{PluginRegistry, PluginSetting, SystemMetrics};
