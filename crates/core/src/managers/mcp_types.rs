//! Shared types for the MCP subsystem.
//!
//! Defines `McpServerHandle`, `ServerStatus`, and other types used across
//! the MCP client manager, health monitor, and kernel tool modules.

use super::mcp_client::McpClient;
use super::mcp_mgp::{NegotiatedMgp, ToolSecurityMetadata};
use super::mcp_protocol::{ClotoHandshakeResult, McpServerConfig, McpTool};
use serde_json::Value;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

#[derive(Clone)]
pub struct McpServerHandle {
    pub id: String,
    pub config: McpServerConfig,
    pub client: Option<Arc<McpClient>>,
    pub tools: Vec<McpTool>,
    pub handshake: Option<ClotoHandshakeResult>,
    pub mgp_negotiated: Option<NegotiatedMgp>,
    pub status: ServerStatus,
    pub source: ServerSource,
    /// Per-server audit sequence counter (in-memory, resets on reconnect).
    pub audit_seq: Arc<AtomicU64>,
    /// Timestamp when the server was connected (for uptime calculation).
    pub connected_at: Option<std::time::Instant>,
    /// OS-level isolation profile applied at spawn time (immutable after spawn).
    pub isolation_profile: Option<super::mcp_isolation::IsolationProfile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerSource {
    Config,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    Registered,    // config loaded, not started
    Connecting,    // handshake in progress
    Connected,     // operational
    Draining,      // graceful shutdown in progress
    Disconnected,  // cleanly stopped
    Error(String), // failed
    Restarting,    // restart in progress
}

impl ServerStatus {
    /// Returns true only when the server is fully operational.
    #[must_use]
    pub fn is_operational(&self) -> bool {
        matches!(self, Self::Connected)
    }
}

impl serde::Serialize for ServerStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Registered => serializer.serialize_str("Registered"),
            Self::Connecting => serializer.serialize_str("Connecting"),
            Self::Connected => serializer.serialize_str("Connected"),
            Self::Draining => serializer.serialize_str("Draining"),
            Self::Disconnected => serializer.serialize_str("Disconnected"),
            Self::Error(_) => serializer.serialize_str("Error"),
            Self::Restarting => serializer.serialize_str("Restarting"),
        }
    }
}

/// Public info about a connected MCP server.
#[derive(Debug, Clone, serde::Serialize)]
pub struct McpServerInfo {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pub status: ServerStatus,
    pub status_message: Option<String>,
    pub tools: Vec<String>,
    pub is_cloto_sdk: bool,
    pub source: ServerSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub mgp_supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[must_use]
pub fn mcp_tool_schema(tool: &McpTool, security: Option<&ToolSecurityMetadata>) -> Value {
    let mut schema = serde_json::json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description.as_deref().unwrap_or(""),
            "parameters": tool.input_schema,
        }
    });
    if let Some(sec) = security {
        schema["security"] = serde_json::to_value(sec).unwrap_or_default();
    }
    schema
}
