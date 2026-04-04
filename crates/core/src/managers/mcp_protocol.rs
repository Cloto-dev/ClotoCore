use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================
// JSON-RPC 2.0 Types
// ============================================================

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Server→Client notification (JSON-RPC 2.0 notification: no `id`, has `method`)
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcNotification {
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// Unified Server→Client message parser.
/// Tries Notification first (requires `method` field), then Response (all-Optional fields).
/// Order matters: `#[serde(untagged)]` tries variants in order, and Response's all-Optional
/// fields would greedily match notification JSON if tried first (silently swallowing notifications).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Notification(JsonRpcNotification),
    Response(JsonRpcResponse),
}

impl JsonRpcRequest {
    #[must_use]
    pub fn new(id: i64, method: &str, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::Number(id.into())),
            method: method.to_string(),
            params,
        }
    }

    #[must_use]
    pub fn notification(method: &str, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: method.to_string(),
            params,
        }
    }
}

// ============================================================
// MCP Standard Types
// ============================================================

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: ClientInfo,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mgp: Option<super::mcp_mgp::MgpClientCapabilities>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
    /// MCP tool annotations (destructiveHint, readOnlyHint, etc.)
    #[serde(default)]
    pub annotations: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListToolsResult {
    pub tools: Vec<McpTool>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolParams {
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    pub content: Vec<ToolContent>,
    pub is_error: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { data: String, mime_type: String },
    #[serde(rename = "resource")]
    Resource { resource: Value },
}

// ============================================================
// Streaming Types (MGP §12)
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    pub request_id: i64,
    pub index: u32,
    pub content: ToolContent,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamProgress {
    pub request_id: i64,
    pub progress: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_remaining_ms: Option<u64>,
}

// ============================================================
// Cloto Custom MCP Extensions
// ============================================================

/// Request params for cloto/handshake custom method
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClotoHandshakeParams {
    pub kernel_version: String,
}

/// Response from cloto/handshake
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ClotoHandshakeResult {
    pub server_id: String,
    pub version: Option<String>,
    pub capabilities: Vec<String>,
    pub tools: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seal: Option<String>,
}

// ============================================================
// Restart Policy (MGP §11)
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartStrategy {
    Never,
    OnFailure,
    Always,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartPolicy {
    pub strategy: RestartStrategy,
    #[serde(default = "default_max_restarts")]
    pub max_restarts: u32,
    #[serde(default = "default_restart_window_secs")]
    pub restart_window_secs: u64,
    #[serde(default = "default_backoff_base_ms")]
    pub backoff_base_ms: u64,
    #[serde(default = "default_backoff_max_ms")]
    pub backoff_max_ms: u64,
}

fn default_max_restarts() -> u32 {
    5
}
fn default_restart_window_secs() -> u64 {
    300
}
fn default_backoff_base_ms() -> u64 {
    1000
}
fn default_backoff_max_ms() -> u64 {
    30000
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            strategy: RestartStrategy::OnFailure,
            max_restarts: default_max_restarts(),
            restart_window_secs: default_restart_window_secs(),
            backoff_base_ms: default_backoff_base_ms(),
            backoff_max_ms: default_backoff_max_ms(),
        }
    }
}

/// MCP Server configuration (from mcp.toml or database)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default = "default_transport")]
    pub transport: String,
    /// URL for HTTP-based transports (required when transport = "streamable-http").
    #[serde(default)]
    pub url: Option<String>,
    /// Authentication token for HTTP transport (Bearer token).
    #[serde(default)]
    pub auth_token: Option<String>,
    /// Legacy field — prefer `restart_policy`. When restart_policy is None,
    /// auto_restart controls fallback: Some(true) → OnFailure, Some(false)/None → Never.
    #[serde(default)]
    pub auto_restart: Option<bool>,
    /// Required permissions for this MCP server (Permission gate: D).
    /// In non-YOLO mode, all permissions must be approved before the server starts.
    #[serde(default)]
    pub required_permissions: Vec<String>,
    /// Tool-level validation rules applied by the kernel before forwarding calls.
    /// Maps tool name → validator name (e.g., "execute_command" → "sandbox").
    #[serde(default)]
    pub tool_validators: std::collections::HashMap<String, String>,
    /// Human-readable display name for the UI (e.g., "DeepSeek", "Cerebras").
    #[serde(default)]
    pub display_name: Option<String>,
    /// MGP configuration for this server (optional, from mcp.toml `[servers.mgp]`).
    #[serde(default)]
    pub mgp: Option<super::mcp_mgp::MgpServerConfig>,
    /// Restart policy for this server (MGP §11).
    #[serde(default)]
    pub restart_policy: Option<RestartPolicy>,
    /// HMAC-SHA256 seal of the server entry point (MGP §8 L0: Magic Seal).
    #[serde(default)]
    pub seal: Option<String>,
    /// Per-server isolation config overrides (MGP §8-10).
    #[serde(default)]
    pub isolation: Option<super::mcp_isolation::IsolationConfig>,
}

fn default_transport() -> String {
    "stdio".to_string()
}

impl McpServerConfig {
    /// Returns the effective restart policy, respecting legacy auto_restart fallback.
    #[must_use]
    pub fn effective_restart_policy(&self) -> RestartPolicy {
        self.restart_policy.clone().unwrap_or_else(|| {
            if self.auto_restart.unwrap_or(false) {
                RestartPolicy::default() // OnFailure
            } else {
                RestartPolicy {
                    strategy: RestartStrategy::Never,
                    ..Default::default()
                }
            }
        })
    }
}

/// Top-level config structure for mcp.toml
#[derive(Debug, Deserialize)]
pub struct McpConfigFile {
    /// Path variables for resolving `${var}` in server args/command.
    /// Example: `[paths] servers = "C:/path/to/cloto-mcp-servers/servers"`
    #[serde(default)]
    pub paths: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}
