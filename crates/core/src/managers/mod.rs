//! Manager subsystem for ClotoCore kernel.
//!
//! Contains the plugin manager, agent manager, MCP client manager, LLM proxy,
//! scheduler, and supporting modules for MCP transport, protocol, and health monitoring.

mod agents;
pub mod capability_dispatcher;
pub mod llm_proxy;
pub mod mcp;
pub mod mcp_client;
mod mcp_discovery;
mod mcp_events;
mod mcp_health;
mod mcp_kernel_tool;
mod mcp_lifecycle;
pub mod mcp_mgp;
pub mod mcp_protocol;
mod mcp_streaming;
mod mcp_tool_discovery;
pub mod mcp_tool_validator;
pub mod mcp_transport;
pub mod mcp_types;
pub mod mcp_venv;
mod plugin;
mod registry;
pub mod scheduler;

pub use agents::AgentManager;
pub use capability_dispatcher::{CapabilityDispatcher, CapabilityType};
pub use mcp::McpClientManager;
pub use plugin::PluginManager;
pub use registry::{PluginRegistry, PluginSetting, SystemMetrics};
