//! Kernel-native `create_mcp_server` tool implementation.
//!
//! Allows agents to dynamically create MCP servers at runtime by generating
//! Python code, validating it against security rules, and spawning a new process.

use super::mcp::McpClientManager;
use super::mcp_tool_validator::{
    validate_mcp_code, BLOCKED_IMPORTS, BLOCKED_PATTERNS, MAX_CODE_SIZE,
};
use anyhow::Result;
use serde_json::Value;
use std::sync::atomic::Ordering;
use tracing::info;

/// Kernel-native tool schema: create_mcp_server
pub(super) fn kernel_tool_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "create_mcp_server",
            "description": concat!(
                "Create a new MCP server from Python code. ",
                "The code is safety-validated before execution. ",
                "Returns the list of discovered tools from the new server.\n\n",
                "Auto-provided (do NOT include): imports (asyncio, json, mcp.server.Server, ",
                "mcp.types.TextContent/Tool), `app = Server(name)`, main() entrypoint, stdio transport.\n\n",
                "Blocked imports: subprocess, shutil, socket, ctypes, multiprocessing, signal, ",
                "pty, fcntl, resource, importlib, code, codeop, compileall, py_compile.\n",
                "Blocked patterns: eval(), exec(), __import__(), compile(), open(), globals(), locals(), ",
                "os.system, os.popen, os.spawn, os.exec, os.remove, os.unlink, os.rmdir, os.makedirs, ",
                "subprocess., __builtins__, getattr(), setattr(), delattr().\n",
                "Max code size: 10KB. Allowed imports: json, asyncio, httpx, os, datetime, time, ",
                "math, re, hashlib, base64, urllib.request, typing.",
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Server name (1-64 chars, alphanumeric + underscore/hyphen)"
                    },
                    "description": {
                        "type": "string",
                        "description": "Short description of the server's purpose"
                    },
                    "code": {
                        "type": "string",
                        "description": concat!(
                            "Python code body defining MCP tool handlers. You MUST define exactly two decorated functions:\n\n",
                            "1. @app.list_tools()\\nasync def list_tools() -> list[Tool]:\\n",
                            "    return [Tool(name=\"tool_name\", description=\"...\", ",
                            "inputSchema={\"type\": \"object\", \"properties\": {...}, \"required\": [...]})]\n\n",
                            "2. @app.call_tool()\\nasync def call_tool(name: str, arguments: dict) -> list[TextContent]:\\n",
                            "    if name == \"tool_name\":\\n",
                            "        result = ...  # your logic\\n",
                            "        return [TextContent(type=\"text\", text=json.dumps(result))]\\n",
                            "    raise ValueError(f\"Unknown tool: {name}\")\n\n",
                            "You may add helper functions and use httpx for HTTP requests. ",
                            "Do not include imports already provided (asyncio, json, mcp.server, mcp.types).",
                        )
                    }
                },
                "required": ["name", "description", "code"]
            }
        }
    })
}

/// Execute the kernel-native create_mcp_server tool.
/// Requires YOLO mode to be enabled — autonomous server creation is a privileged operation.
#[allow(clippy::too_many_lines)]
pub(super) async fn execute_create_mcp_server(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    if !manager.yolo_mode.load(Ordering::Relaxed) {
        return Ok(serde_json::json!({
            "status": "rejected",
            "reason": "Autonomous MCP server creation requires YOLO mode to be enabled. Ask the operator to enable it in Settings.",
        }));
    }

    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: name"))?;
    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("Agent-generated MCP server");
    let code = args
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: code"))?;

    // Validate name (same rules as handlers.rs)
    if name.is_empty() || name.len() > 64 {
        return Err(anyhow::anyhow!("Server name must be 1-64 characters"));
    }
    let valid_name = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !valid_name {
        return Err(anyhow::anyhow!(
            "Server name must contain only alphanumeric, underscore, or hyphen"
        ));
    }

    // Code safety validation (Layer 5)
    if let Err(violations) = validate_mcp_code(code) {
        return Ok(serde_json::json!({
            "status": "rejected",
            "reason": "Code validation failed — review violations and use hints to fix",
            "violations": violations,
            "hints": {
                "blocked_imports": BLOCKED_IMPORTS,
                "blocked_patterns": BLOCKED_PATTERNS,
                "max_code_size_bytes": MAX_CODE_SIZE,
                "auto_provided_imports": [
                    "asyncio", "json", "mcp.server.Server",
                    "mcp.server.stdio.stdio_server",
                    "mcp.types.TextContent", "mcp.types.Tool"
                ],
                "allowed_additional_imports": [
                    "httpx", "os", "datetime", "time", "math",
                    "re", "hashlib", "base64", "urllib.request", "typing"
                ],
            }
        }));
    }

    // Generate script from template
    let script = format!(
        r#""""MCP Server: {name} — {desc}"""
import asyncio
import json

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

app = Server("{name}")

{code}

async def main():
    async with stdio_server() as (read, write):
        await app.run(read, write, app.create_initialization_options())

if __name__ == "__main__":
    asyncio.run(main())
"#,
        name = name,
        desc = description.replace('"', r#"\""#),
        code = code,
    );

    // Write script file
    let scripts_dir = std::path::Path::new("scripts");
    if !scripts_dir.exists() {
        std::fs::create_dir_all(scripts_dir)
            .map_err(|e| anyhow::anyhow!("Failed to create scripts directory: {}", e))?;
    }
    let script_path = scripts_dir.join(format!("mcp_{name}.py"));
    std::fs::write(&script_path, &script)
        .map_err(|e| anyhow::anyhow!("Failed to write script: {}", e))?;

    // Register and connect the server
    let server_id = format!("agent.{name}");
    let tool_names = manager
        .add_dynamic_server(
            server_id.clone(),
            "python".to_string(),
            vec![script_path.to_string_lossy().to_string()],
            Some(script),
            Some(description.to_string()),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to start server: {}", e))?;

    info!(
        "Agent created MCP server '{}' with {} tool(s): {:?}",
        server_id,
        tool_names.len(),
        tool_names
    );

    Ok(serde_json::json!({
        "status": "created",
        "server_id": server_id,
        "tools": tool_names,
        "script_path": script_path.to_string_lossy(),
    }))
}
