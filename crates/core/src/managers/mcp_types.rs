//! Shared types for the MCP subsystem.
//!
//! Defines `McpServerHandle`, `ServerStatus`, and other types used across
//! the MCP client manager, health monitor, and kernel tool modules.

use super::mcp_client::McpClient;
use super::mcp_protocol::{ClotoHandshakeResult, McpServerConfig, McpTool};
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone)]
pub struct McpServerHandle {
    pub id: String,
    pub config: McpServerConfig,
    pub client: Option<Arc<McpClient>>,
    pub tools: Vec<McpTool>,
    pub handshake: Option<ClotoHandshakeResult>,
    pub status: ServerStatus,
    pub source: ServerSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerSource {
    Config,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    Connected,
    Disconnected,
    Error(String),
}

impl serde::Serialize for ServerStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Connected => serializer.serialize_str("Connected"),
            Self::Disconnected => serializer.serialize_str("Disconnected"),
            Self::Error(_) => serializer.serialize_str("Error"),
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
}

#[must_use]
pub fn mcp_tool_schema(tool: &McpTool) -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description.as_deref().unwrap_or(""),
            "parameters": tool.input_schema,
        }
    })
}
