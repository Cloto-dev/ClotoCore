//! Shared types for the MCP subsystem.
//!
//! Defines `McpServerHandle`, `ServerStatus`, and other types used across
//! the MCP client manager, health monitor, and kernel tool modules.

use super::mcp_client::McpClient;
use super::mcp_mgp::{NegotiatedMgp, ToolSecurityMetadata};
use super::mcp_protocol::{ClotoHandshakeResult, McpServerConfig, McpTool};
use serde_json::Value;
use std::path::{Path, PathBuf};
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
    /// The server's own usage guidance, captured from `initialize.instructions`
    /// (legacy era) or `DiscoverResult.instructions` (modern era). Stored only;
    /// no consumer wires it into prompts yet.
    pub instructions: Option<String>,
}

/// Tool names that mark a server as a reasoning engine. These are
/// engine-internal (invoked directly via `call_server_tool(engine_id, …)`),
/// not agent-facing, so classifiers use this tool surface — not an id prefix —
/// to recognise an engine. This is the id-prefix-agnostic replacement for the
/// retired `mind.` prefix (engine ids are bare, e.g. `local`,
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
    /// `id.starts_with("mind.")` classifier so bare-id engines
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

/// Where the kernel keeps the Python scripts it generates for dynamic MCP
/// servers, relative to the process working directory.
pub const MCP_SCRIPTS_DIR: &str = "data/mcp_scripts";

/// File name of the generated script for `name`.
#[must_use]
pub fn mcp_script_filename(name: &str) -> String {
    format!("mcp_{name}.py")
}

/// Path of that script under `base`.
///
/// Every caller that writes, regenerates or removes one of these files goes
/// through here. They used to derive the path independently, and when the
/// directory moved the removal path was left behind, so deleting a dynamic
/// server left its script -- user-supplied Python -- on disk (bug-505).
#[must_use]
pub fn mcp_script_path_in(base: &Path, name: &str) -> PathBuf {
    base.join(mcp_script_filename(name))
}

/// [`mcp_script_path_in`] against the kernel's own script directory.
#[must_use]
pub fn mcp_script_path(name: &str) -> PathBuf {
    mcp_script_path_in(Path::new(MCP_SCRIPTS_DIR), name)
}

/// Remove the generated script for `name` under `base`.
///
/// Returns whether a file was there to remove; a server that never had a
/// generated script is not an error, but a file that refuses to go is.
pub fn remove_mcp_script_in(base: &Path, name: &str) -> std::io::Result<bool> {
    match std::fs::remove_file(mcp_script_path_in(base, name)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// [`remove_mcp_script_in`] against the kernel's own script directory.
pub fn remove_mcp_script(name: &str) -> std::io::Result<bool> {
    remove_mcp_script_in(Path::new(MCP_SCRIPTS_DIR), name)
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
        // replacing the retired `mind.` prefix classifier.
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

    #[test]
    fn script_path_pins_the_generated_location() {
        assert_eq!(
            mcp_script_path("weather")
                .to_string_lossy()
                .replace('\\', "/"),
            "data/mcp_scripts/mcp_weather.py"
        );
    }

    #[test]
    fn the_stored_command_argument_stays_a_forward_slash_relative_path() {
        // This exact string is written into the server row's args and handed
        // to python, so it must not pick up platform separators. That the
        // delete path targets the same file is enforced by construction --
        // both sides go through the helpers above -- not by this assertion.
        let stored = format!("{MCP_SCRIPTS_DIR}/{}", mcp_script_filename("weather"));
        assert_eq!(stored, "data/mcp_scripts/mcp_weather.py");
        assert_eq!(
            Path::new(&stored).file_name(),
            mcp_script_path("weather").file_name()
        );
    }

    #[test]
    fn removing_a_script_takes_the_file_and_reports_whether_one_was_there() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = mcp_script_path_in(dir.path(), "weather");
        std::fs::write(&path, "print('hi')").expect("seed");

        assert!(path.exists());
        assert!(remove_mcp_script_in(dir.path(), "weather").expect("remove"));
        assert!(!path.exists());
        // A second removal is a no-op, not an error: a server may never have
        // had a generated script.
        assert!(!remove_mcp_script_in(dir.path(), "weather").expect("remove again"));
    }

    #[test]
    fn removing_a_script_leaves_its_neighbours_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let keep = mcp_script_path_in(dir.path(), "weather_v2");
        std::fs::write(&keep, "print('keep')").expect("seed");
        std::fs::write(mcp_script_path_in(dir.path(), "weather"), "print('go')").expect("seed");

        assert!(remove_mcp_script_in(dir.path(), "weather").expect("remove"));
        assert!(keep.exists(), "a prefix-sharing sibling must survive");
    }
}
