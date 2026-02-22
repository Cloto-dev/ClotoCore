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

impl JsonRpcRequest {
    pub fn new(id: i64, method: &str, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::Number(id.into())),
            method: method.to_string(),
            params,
        }
    }

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
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: ClientInfo,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListToolsResult {
    pub tools: Vec<McpTool>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CallToolParams {
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CallToolResult {
    pub content: Vec<ToolContent>,
    pub is_error: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { data: String, mime_type: String },
    #[serde(rename = "resource")]
    Resource { resource: Value },
}

// ============================================================
// Exiv Custom MCP Extensions
// ============================================================

/// Request params for exiv/handshake custom method
#[derive(Debug, Serialize, Deserialize)]
pub struct ExivHandshakeParams {
    pub kernel_version: String,
}

/// Response from exiv/handshake
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExivHandshakeResult {
    pub server_id: String,
    pub version: Option<String>,
    pub capabilities: Vec<String>,
    pub tools: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seal: Option<String>,
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
    #[serde(default)]
    pub auto_restart: bool,
    /// Required permissions for this MCP server (Permission gate: D).
    /// In non-YOLO mode, all permissions must be approved before the server starts.
    #[serde(default)]
    pub required_permissions: Vec<String>,
    /// Tool-level validation rules applied by the kernel before forwarding calls.
    /// Maps tool name → validator name (e.g., "execute_command" → "sandbox").
    #[serde(default)]
    pub tool_validators: std::collections::HashMap<String, String>,
}

fn default_transport() -> String {
    "stdio".to_string()
}

/// Top-level config structure for mcp.toml
#[derive(Debug, Deserialize)]
pub struct McpConfigFile {
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}
