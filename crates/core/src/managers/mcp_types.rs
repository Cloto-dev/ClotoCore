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
    /// Per-server audit sequence counter (in-memory, resets on reconnect).
    pub audit_seq: Arc<AtomicU64>,
    /// Timestamp when the server was connected (for uptime calculation).
    pub connected_at: Option<std::time::Instant>,
    /// OS-level isolation profile applied at spawn time (immutable after spawn).
    pub isolation_profile: Option<super::mcp_isolation::IsolationProfile>,
    /// MCP era this connection negotiated (`None` until a connect succeeds, and
    /// for handles registered as placeholders after a failure).
    pub protocol_era: Option<super::mcp_protocol::ProtocolEra>,
    /// `DiscoverResult.instructions` from the modern-era probe — the server's
    /// own usage guidance. Stored only; no consumer wires it into prompts yet.
    pub instructions: Option<String>,
}

/// Tool names that mark a server as a reasoning engine. These are
/// engine-internal (invoked directly via `call_server_tool(engine_id, …)`),
/// not agent-facing, so classifiers use this tool surface — not an id prefix —
/// to recognise an engine. This is the id-prefix-agnostic replacement for the
/// retired `mind.` prefix (an earlier decision: engine ids are bare, e.g. `local`,
/// `ollama`, `deepseek`).
pub const ENGINE_TOOL_NAMES: [&str; 2] = ["think", "think_with_tools"];

/// True when a tool list exposes the reasoning-engine tool surface
/// (`think` / `think_with_tools`).
#[must_use]
pub fn tools_expose_reasoning(tools: &[McpTool]) -> bool {
    tools
        .iter()
        .any(|t| ENGINE_TOOL_NAMES.contains(&t.name.as_str()))
}

impl McpServerHandle {
    /// True when this server is a reasoning engine (exposes the
    /// `think` / `think_with_tools` tool surface). Replaces the legacy
    /// `id.starts_with("mind.")` classifier (an earlier decision) so bare-id engines
    /// (`local`, `ollama`, `deepseek`, …) are recognised uniformly.
    #[must_use]
    pub fn is_reasoning_engine(&self) -> bool {
        tools_expose_reasoning(&self.tools)
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub mgp_supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// True when the server config contains env vars that reference unset variables.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub has_unresolved_env: bool,
    /// ClotoHub catalog connector id when the server was installed via marketplace.
    /// NULL ⇒ manually registered. The dashboard uses this together with
    /// `mgp_supported` to render the MGP purple card (catalog-origin OR
    /// protocol-negotiated), separate from the seal-based Verified badge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marketplace_id: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> McpTool {
        McpTool {
            name: name.to_string(),
            description: None,
            input_schema: serde_json::json!({}),
            annotations: None,
        }
    }

    #[test]
    fn tools_expose_reasoning_keys_on_think_surface_not_id() {
        // A reasoning engine is recognised by its think / think_with_tools tool
        // surface regardless of id — bare `local`/`ollama`/`deepseek` all qualify
        // (an earlier decision), replacing the retired `mind.` prefix classifier.
        assert!(tools_expose_reasoning(&[tool("think")]));
        assert!(tools_expose_reasoning(&[
            tool("switch_model"),
            tool("think_with_tools"),
        ]));
        // Ordinary MCP servers (memory, tools) are not engines.
        assert!(!tools_expose_reasoning(&[tool("store"), tool("recall")]));
        assert!(!tools_expose_reasoning(&[]));
        // A tool merely containing "think" as a substring is not the surface.
        assert!(!tools_expose_reasoning(&[tool("rethink_plan")]));
    }
}
