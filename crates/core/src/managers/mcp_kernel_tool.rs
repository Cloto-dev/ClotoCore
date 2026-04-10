//! Kernel-native tool implementations.
//!
//! Includes `create_mcp_server` (dynamic MCP server creation) and `ask_agent`
//! (inter-agent delegation for specialist consultation).

use super::mcp::McpClientManager;
use super::mcp_tool_validator::{
    validate_mcp_code, BLOCKED_IMPORTS, BLOCKED_PATTERNS, MAX_CODE_SIZE,
};
use anyhow::Result;
use serde_json::Value;
use std::sync::atomic::Ordering;
use tracing::info;

/// Return Tier 1-4 kernel tool schemas (create_mcp_server + access control + audit replay + Tier 3 + Tier 4).
/// Only exposed when YOLO mode is enabled.
pub(super) fn kernel_tool_schemas() -> Vec<Value> {
    let mut schemas = vec![
        kernel_tool_schema(),
        access_query_schema(),
        access_grant_schema(),
        access_revoke_schema(),
        audit_replay_schema(),
        // Tier 3: Lifecycle
        health_ping_schema(),
        health_status_schema(),
        lifecycle_shutdown_schema(),
        // Tier 3: Streaming
        stream_cancel_schema(),
        stream_pace_schema(),
        // Tier 3: Events
        events_subscribe_schema(),
        events_unsubscribe_schema(),
        events_replay_schema(),
        events_pending_callbacks_schema(),
        // Tier 3: Callbacks
        callback_respond_schema(),
        // Inter-agent delegation
        ask_agent_schema(),
        // GUI documentation
        gui_map_schema(),
        gui_read_schema(),
    ];
    // Tier 4: Discovery (§15)
    schemas.extend(super::mcp_discovery::discovery_tool_schemas());
    // Tier 4: Tool Discovery session tools (§16)
    schemas.push(super::mcp_tool_discovery::tools_session_schema());
    schemas.push(super::mcp_tool_discovery::tools_session_evict_schema());
    schemas
}

/// Return LLM-visible meta-tool schemas (§1.6.3).
/// Only `mgp.tools.discover` and `mgp.tools.request` are injected into LLM context.
pub(super) fn llm_meta_tool_schemas() -> Vec<Value> {
    vec![
        super::mcp_tool_discovery::tools_discover_schema(),
        super::mcp_tool_discovery::tools_request_schema(),
    ]
}

fn access_query_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.access.query",
            "description": "Query access control entries for an agent or resolve access for a specific tool.",
            "parameters": {
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent ID to query access entries for"
                    },
                    "server_id": {
                        "type": "string",
                        "description": "Server ID (required when tool_name is provided)"
                    },
                    "tool_name": {
                        "type": "string",
                        "description": "Specific tool name to resolve access for"
                    }
                },
                "required": ["agent_id", "server_id"]
            }
        }
    })
}

fn access_grant_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.access.grant",
            "description": "Grant access control entry for an agent to a server or tool.",
            "parameters": {
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent ID to grant access to"
                    },
                    "server_id": {
                        "type": "string",
                        "description": "Server ID to grant access for"
                    },
                    "entry_type": {
                        "type": "string",
                        "enum": ["server_grant", "tool_grant"],
                        "description": "Type of access entry"
                    },
                    "tool_name": {
                        "type": "string",
                        "description": "Tool name (required for tool_grant)"
                    },
                    "permission": {
                        "type": "string",
                        "enum": ["allow", "deny"],
                        "description": "Permission to grant"
                    },
                    "justification": {
                        "type": "string",
                        "description": "Reason for granting access"
                    }
                },
                "required": ["agent_id", "server_id", "entry_type", "permission"]
            }
        }
    })
}

fn access_revoke_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.access.revoke",
            "description": "Revoke an access control entry for an agent.",
            "parameters": {
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent ID to revoke access from"
                    },
                    "server_id": {
                        "type": "string",
                        "description": "Server ID to revoke access for"
                    },
                    "entry_type": {
                        "type": "string",
                        "enum": ["server_grant", "tool_grant"],
                        "description": "Type of access entry to revoke"
                    },
                    "tool_name": {
                        "type": "string",
                        "description": "Tool name (for tool_grant revocation)"
                    }
                },
                "required": ["agent_id", "server_id", "entry_type"]
            }
        }
    })
}

fn audit_replay_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.audit.replay",
            "description": "Replay audit log entries since a given seq (DB id) or timestamp.",
            "parameters": {
                "type": "object",
                "properties": {
                    "since_seq": {
                        "type": "integer",
                        "description": "Replay entries with id > since_seq"
                    },
                    "since_timestamp": {
                        "type": "string",
                        "description": "Replay entries with timestamp > since_timestamp (RFC3339)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of entries to return (default 100)"
                    }
                }
            }
        }
    })
}

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
                    },
                    "server_type": {
                        "type": "string",
                        "enum": ["basic", "coordinator"],
                        "description": concat!(
                            "Server type. 'basic' (default): standard MGP server. ",
                            "'coordinator': adds delegate() helper for calling other MCP servers ",
                            "via the kernel API (MGP §5.6, §19.1). Coordinator servers receive ",
                            "CLOTO_KERNEL_URL and CLOTO_API_KEY as environment variables.",
                        ),
                        "default": "basic"
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
    let server_type = args
        .get("server_type")
        .and_then(|v| v.as_str())
        .unwrap_or("basic");
    if server_type != "basic" && server_type != "coordinator" {
        return Err(anyhow::anyhow!(
            "Invalid server_type '{}': must be 'basic' or 'coordinator'",
            server_type
        ));
    }

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
    if let Err(violations) = validate_mcp_code(code, super::mcp_mgp::CodeSafetyLevel::Standard) {
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

    // Generate script from template (MGP-capable)
    let server_id = format!("agent.{name}");
    let mgp_version = super::mcp_mgp::MGP_VERSION;
    let desc_escaped = description.replace('"', r#"\""#);

    let script = if server_type == "coordinator" {
        format!(
            r#""""MGP Coordinator Server: {name} — {desc}"""
import asyncio
import json
import os
import datetime
import httpx

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

app = Server("{name}")

KERNEL_URL = os.environ.get("CLOTO_KERNEL_URL", "http://127.0.0.1:8081")


async def delegate(server_id: str, tool_name: str, arguments: dict,
                   original_actor: str, chain: list = None) -> dict:
    """Delegate a tool call to another MCP server via the kernel (MGP section 5.6)."""
    delegation_chain = list(chain or [])
    delegation_chain.append({{
        "server_id": "{server_id}",
        "tool_name": tool_name,
        "timestamp": datetime.datetime.utcnow().isoformat() + "Z"
    }})
    if len(delegation_chain) > 3:
        raise ValueError("Delegation chain depth exceeds maximum of 3")
    payload = {{
        "server_id": server_id,
        "tool_name": tool_name,
        "arguments": {{
            **arguments,
            "_mgp": {{
                "delegation": {{
                    "original_actor": original_actor,
                    "delegated_via": "{server_id}",
                    "chain": delegation_chain
                }}
            }}
        }}
    }}
    api_key = os.environ.get("CLOTO_API_KEY", "")
    async with httpx.AsyncClient(timeout=30) as client:
        resp = await client.post(
            f"{{KERNEL_URL}}/api/mcp/call",
            json=payload,
            headers={{"X-API-Key": api_key}}
        )
        resp.raise_for_status()
        return resp.json()


{code}

async def main():
    init_options = app.create_initialization_options()
    if init_options.capabilities:
        init_options.capabilities.experimental = {{
            "mgp": {{
                "version": "{mgp_version}",
                "extensions": ["tool_security", "delegation"],
                "server_id": "{server_id}",
                "trust_level": "untrusted"
            }}
        }}
    async with stdio_server() as (read, write):
        await app.run(read, write, init_options)

if __name__ == "__main__":
    asyncio.run(main())
"#,
            name = name,
            desc = desc_escaped,
            code = code,
            mgp_version = mgp_version,
            server_id = server_id,
        )
    } else {
        format!(
            r#""""MCP Server: {name} — {desc}"""
import asyncio
import json

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

app = Server("{name}")

{code}

async def main():
    init_options = app.create_initialization_options()
    if init_options.capabilities:
        init_options.capabilities.experimental = {{
            "mgp": {{
                "version": "{mgp_version}",
                "extensions": ["tool_security"],
                "server_id": "{server_id}",
                "trust_level": "untrusted"
            }}
        }}
    async with stdio_server() as (read, write):
        await app.run(read, write, init_options)

if __name__ == "__main__":
    asyncio.run(main())
"#,
            name = name,
            desc = desc_escaped,
            code = code,
            mgp_version = mgp_version,
            server_id = server_id,
        )
    };

    // Write script file
    let scripts_dir = std::path::Path::new("data/mcp_scripts");
    if !scripts_dir.exists() {
        std::fs::create_dir_all(scripts_dir)
            .map_err(|e| anyhow::anyhow!("Failed to create data/mcp_scripts directory: {}", e))?;
    }
    let script_path = scripts_dir.join(format!("mcp_{name}.py"));
    std::fs::write(&script_path, &script)
        .map_err(|e| anyhow::anyhow!("Failed to write script: {}", e))?;

    // Build env vars (coordinator servers get kernel URL for delegation calls)
    let mut env = std::collections::HashMap::new();
    if server_type == "coordinator" {
        let port = std::env::var("PORT").unwrap_or_else(|_| "8081".to_string());
        env.insert(
            "CLOTO_KERNEL_URL".to_string(),
            format!("http://127.0.0.1:{}", port),
        );
        if let Ok(api_key) = std::env::var("CLOTO_API_KEY") {
            env.insert("CLOTO_API_KEY".to_string(), api_key);
        }
    }

    // Register and connect the server
    let mgp_config = Some(super::mcp_mgp::MgpServerConfig {
        trust_level: Some("untrusted".to_string()),
    });
    let tool_names = manager
        .add_server(
            server_id.clone(),
            "python".to_string(),
            vec![script_path.to_string_lossy().to_string()],
            Some(script),
            Some(description.to_string()),
            mgp_config,
            env,
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

// ============================================================
// Access Control Kernel Tools (MGP §5)
// ============================================================

/// Execute mgp.access.query — query access entries or resolve tool access.
pub(super) async fn execute_access_query(manager: &McpClientManager, args: Value) -> Result<Value> {
    if !manager.yolo_mode.load(Ordering::Relaxed) {
        return Ok(serde_json::json!({
            "status": "rejected",
            "reason": "Access control tools require YOLO mode (operator-level privilege).",
        }));
    }

    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: agent_id"))?;

    let server_id = args
        .get("server_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: server_id"))?;
    let tool_name = args.get("tool_name").and_then(|v| v.as_str());

    // If tool_name provided → resolve specific tool access
    if let Some(tn) = tool_name {
        let permission =
            crate::db::resolve_tool_access(manager.pool(), agent_id, server_id, tn).await?;
        return Ok(serde_json::json!({
            "agent_id": agent_id,
            "server_id": server_id,
            "tool_name": tn,
            "permission": permission,
        }));
    }

    // Otherwise → list entries for agent + server
    let entries = crate::db::get_access_entries_for_agent(manager.pool(), agent_id).await?;
    let entries_json: Vec<Value> = entries
        .iter()
        .filter(|e| e.server_id == server_id)
        .map(|e| {
            serde_json::json!({
                "entry_type": e.entry_type,
                "server_id": e.server_id,
                "tool_name": e.tool_name,
                "permission": e.permission,
                "granted_by": e.granted_by,
                "granted_at": e.granted_at,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "agent_id": agent_id,
        "server_id": server_id,
        "entries": entries_json,
    }))
}

/// Execute mgp.access.grant — create an access control entry.
pub(super) async fn execute_access_grant(manager: &McpClientManager, args: Value) -> Result<Value> {
    if !manager.yolo_mode.load(Ordering::Relaxed) {
        return Ok(serde_json::json!({
            "status": "rejected",
            "reason": "Access control tools require YOLO mode (operator-level privilege).",
        }));
    }

    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: agent_id"))?;
    let server_id = args
        .get("server_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: server_id"))?;
    let entry_type = args
        .get("entry_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: entry_type"))?;
    let permission = args
        .get("permission")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: permission"))?;
    let tool_name = args.get("tool_name").and_then(|v| v.as_str());
    let justification = args.get("justification").and_then(|v| v.as_str());

    let parsed_entry_type: crate::db::mcp::EntryType =
        serde_json::from_value(serde_json::Value::String(entry_type.to_string()))
            .map_err(|_| anyhow::anyhow!("Invalid entry_type: '{}'", entry_type))?;
    let parsed_permission: crate::db::mcp::PermissionLevel =
        serde_json::from_value(serde_json::Value::String(permission.to_string()))
            .map_err(|_| anyhow::anyhow!("Invalid permission: '{}'", permission))?;

    let entry = crate::db::AccessControlEntry {
        id: None,
        entry_type: parsed_entry_type,
        agent_id: agent_id.to_string(),
        server_id: server_id.to_string(),
        tool_name: tool_name.map(str::to_string),
        permission: parsed_permission,
        granted_by: Some("kernel".to_string()),
        granted_at: chrono::Utc::now().to_rfc3339(),
        expires_at: None,
        justification: justification.map(str::to_string),
        metadata: None,
    };

    let id = crate::db::save_access_control_entry(manager.pool(), &entry).await?;

    // Audit log
    crate::db::spawn_audit_log(
        manager.pool().clone(),
        crate::db::AuditLogEntry {
            timestamp: chrono::Utc::now(),
            event_type: "ACCESS_GRANTED".to_string(),
            actor_id: Some("kernel".to_string()),
            target_id: Some(format!("{}:{}", server_id, agent_id)),
            permission: Some(permission.to_string()),
            result: "success".to_string(),
            reason: justification.unwrap_or("mgp.access.grant").to_string(),
            metadata: tool_name.map(|tn| serde_json::json!({"tool_name": tn})),
            trace_id: None,
        },
    );

    info!(
        "Access granted: agent={}, server={}, type={}, permission={}",
        agent_id, server_id, entry_type, permission
    );

    Ok(serde_json::json!({
        "status": "granted",
        "id": id,
        "agent_id": agent_id,
        "server_id": server_id,
        "entry_type": entry_type,
        "permission": permission,
    }))
}

/// Execute mgp.access.revoke — delete an access control entry.
pub(super) async fn execute_access_revoke(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    if !manager.yolo_mode.load(Ordering::Relaxed) {
        return Ok(serde_json::json!({
            "status": "rejected",
            "reason": "Access control tools require YOLO mode (operator-level privilege).",
        }));
    }

    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: agent_id"))?;
    let server_id = args
        .get("server_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: server_id"))?;
    let entry_type = args
        .get("entry_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: entry_type"))?;
    let tool_name = args.get("tool_name").and_then(|v| v.as_str());

    let deleted =
        crate::db::delete_access_entry(manager.pool(), agent_id, server_id, entry_type, tool_name)
            .await?;

    // Audit log
    crate::db::spawn_audit_log(
        manager.pool().clone(),
        crate::db::AuditLogEntry {
            timestamp: chrono::Utc::now(),
            event_type: "ACCESS_REVOKED".to_string(),
            actor_id: Some("kernel".to_string()),
            target_id: Some(format!("{}:{}", server_id, agent_id)),
            permission: None,
            result: "success".to_string(),
            reason: "mgp.access.revoke".to_string(),
            metadata: tool_name.map(|tn| serde_json::json!({"tool_name": tn})),
            trace_id: None,
        },
    );

    info!(
        "Access revoked: agent={}, server={}, type={}, deleted={}",
        agent_id, server_id, entry_type, deleted
    );

    Ok(serde_json::json!({
        "status": "revoked",
        "deleted_count": deleted,
        "agent_id": agent_id,
        "server_id": server_id,
        "entry_type": entry_type,
    }))
}

// ============================================================
// Audit Replay Kernel Tool (MGP §5.6)
// ============================================================

/// Execute mgp.audit.replay — replay audit log entries.
pub(super) async fn execute_audit_replay(manager: &McpClientManager, args: Value) -> Result<Value> {
    if !manager.yolo_mode.load(Ordering::Relaxed) {
        return Ok(serde_json::json!({
            "status": "rejected",
            "reason": "Audit tools require YOLO mode (operator-level privilege).",
        }));
    }

    let since_seq = args.get("since_seq").and_then(serde_json::Value::as_i64);
    let since_timestamp = args.get("since_timestamp").and_then(|v| v.as_str());
    let limit = args
        .get("limit")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(100);

    let entries =
        crate::db::query_audit_logs_since(manager.pool(), since_seq, since_timestamp, limit)
            .await?;

    let events_json: Vec<Value> = entries
        .iter()
        .map(|(id, e)| {
            serde_json::json!({
                "seq": id,
                "timestamp": e.timestamp.to_rfc3339(),
                "event_type": e.event_type,
                "actor": {
                    "type": "kernel",
                    "id": e.actor_id,
                },
                "target": {
                    "server_id": e.target_id,
                    "tool_name": e.metadata.as_ref()
                        .and_then(|m| m.get("tool_name"))
                        .and_then(|v| v.as_str()),
                },
                "permission": e.permission,
                "result": e.result,
                "reason": e.reason,
                "metadata": e.metadata,
            })
        })
        .collect();

    let next_seq = events_json.last().and_then(|e| e.get("seq")).cloned();
    let has_more = i64::try_from(events_json.len()).unwrap_or(i64::MAX) == limit;

    Ok(serde_json::json!({
        "events": events_json,
        "has_more": has_more,
        "next_seq": next_seq,
    }))
}

// ============================================================
// Tier 3: Lifecycle Kernel Tools (MGP §11)
// ============================================================

fn health_ping_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.health.ping",
            "description": "Check if a specific MCP server is alive and responsive.",
            "parameters": {
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "ID of the MCP server to ping"
                    }
                },
                "required": ["server_id"]
            }
        }
    })
}

fn health_status_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.health.status",
            "description": "Get detailed health status of an MCP server including state, tools, and MGP info.",
            "parameters": {
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "ID of the MCP server to query"
                    }
                },
                "required": ["server_id"]
            }
        }
    })
}

fn lifecycle_shutdown_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.lifecycle.shutdown",
            "description": "Initiate graceful shutdown of an MCP server (Draining → Disconnected).",
            "parameters": {
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "ID of the MCP server to shut down"
                    },
                    "reason": {
                        "type": "string",
                        "enum": ["operator_request", "configuration_change", "resource_limit", "idle_timeout", "kernel_shutdown"],
                        "description": "Shutdown reason category"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "Timeout in milliseconds before forced stop (default 5000)"
                    }
                },
                "required": ["server_id", "reason"]
            }
        }
    })
}

/// Execute mgp.health.ping — check server liveness.
pub(super) async fn execute_health_ping(manager: &McpClientManager, args: Value) -> Result<Value> {
    let server_id = args
        .get("server_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: server_id"))?;

    let state = manager.state.read().await;
    let Some(handle) = state.servers.get(server_id) else {
        return Ok(serde_json::json!({
            "server_id": server_id,
            "status": "not_found",
        }));
    };

    let start = std::time::Instant::now();
    let is_alive = handle.client.as_ref().is_some_and(|c| c.is_alive());
    let _elapsed_ms = start.elapsed().as_millis();

    let health = if is_alive && handle.status.is_operational() {
        "healthy"
    } else if is_alive {
        "degraded"
    } else {
        "unhealthy"
    };

    let uptime_secs = handle.connected_at.map(|t| t.elapsed().as_secs());

    Ok(serde_json::json!({
        "status": health,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "uptime_secs": uptime_secs,
        "server_id": server_id,
    }))
}

/// Execute mgp.health.status — detailed server health info.
pub(super) async fn execute_health_status(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    let server_id = args
        .get("server_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: server_id"))?;

    let state = manager.state.read().await;
    let Some(handle) = state.servers.get(server_id) else {
        return Ok(serde_json::json!({
            "server_id": server_id,
            "status": "not_found",
        }));
    };

    let status = if handle.status.is_operational() {
        "healthy"
    } else {
        "unhealthy"
    };

    let uptime_secs = handle.connected_at.map(|t| t.elapsed().as_secs());

    Ok(serde_json::json!({
        "server_id": server_id,
        "status": status,
        "uptime_secs": uptime_secs,
        "tools_available": handle.tools.iter().filter(|_| handle.status.is_operational()).count(),
        "tools_total": handle.tools.len(),
        "pending_requests": 0,
        "resources": {},
        "checks": {
            "mgp_negotiated": handle.mgp_negotiated.is_some(),
        },
    }))
}

/// Execute mgp.lifecycle.shutdown — graceful shutdown with draining.
pub(super) async fn execute_lifecycle_shutdown(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    let server_id = args
        .get("server_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: server_id"))?
        .to_string();
    let reason = args
        .get("reason")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: reason"))?;
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(5000);

    manager.drain_server(&server_id, reason, timeout_ms).await?;

    Ok(serde_json::json!({
        "accepted": true,
        "pending_requests": 0,
        "estimated_drain_ms": timeout_ms,
    }))
}

// ============================================================
// Tier 3: Streaming Kernel Tools (MGP §12)
// ============================================================

fn stream_cancel_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.stream.cancel",
            "description": "Cancel an active streaming tool call.",
            "parameters": {
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "ID of the MCP server"
                    },
                    "request_id": {
                        "type": "integer",
                        "description": "Request ID of the streaming call to cancel"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Reason for cancellation (default: user_cancelled)"
                    }
                },
                "required": ["server_id", "request_id"]
            }
        }
    })
}

fn stream_pace_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.stream.pace",
            "description": "Control the output rate of a streaming tool call (§12.8).",
            "parameters": {
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Server hosting the streaming tool"
                    },
                    "request_id": {
                        "type": "number",
                        "description": "The streaming request ID to pace"
                    },
                    "max_chunks_per_second": {
                        "type": "number",
                        "description": "Maximum chunks per second the server should send"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Reason for pacing"
                    }
                },
                "required": ["server_id", "request_id", "max_chunks_per_second"]
            }
        }
    })
}

/// Execute mgp.stream.cancel — cancel a streaming call.
pub(super) async fn execute_stream_cancel(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    let server_id = args
        .get("server_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: server_id"))?;
    let request_id = args
        .get("request_id")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: request_id"))?;

    let reason = args
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("user_cancelled");

    let result =
        super::mcp_streaming::cancel_stream(manager, server_id, request_id, reason).await?;

    manager.stream_assembler.remove(server_id, request_id);

    Ok(serde_json::json!({
        "server_id": server_id,
        "request_id": request_id,
        "cancelled": true,
        "partial_result": result.get("partial_result"),
    }))
}

/// Execute mgp.stream.pace — control output rate of a streaming tool call.
pub(super) async fn execute_stream_pace(manager: &McpClientManager, args: Value) -> Result<Value> {
    let server_id = args
        .get("server_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: server_id"))?;
    let request_id = args
        .get("request_id")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: request_id"))?;
    let max_chunks = args
        .get("max_chunks_per_second")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: max_chunks_per_second"))?
        as u32;
    let reason = args.get("reason").and_then(|v| v.as_str());

    super::mcp_streaming::send_pace(manager, server_id, request_id, max_chunks, reason).await?;

    Ok(serde_json::json!({
        "server_id": server_id,
        "request_id": request_id,
        "max_chunks_per_second": max_chunks,
        "paced": true,
    }))
}

// ============================================================
// Tier 3: Event Kernel Tools (MGP §13)
// ============================================================

fn events_subscribe_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.events.subscribe",
            "description": "Subscribe an MCP server to event channels.",
            "parameters": {
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "ID of the subscribing MCP server"
                    },
                    "channels": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Event channels to subscribe to (e.g., ['lifecycle', 'tools'])"
                    },
                    "filter": {
                        "type": "object",
                        "description": "Optional filter criteria for events"
                    }
                },
                "required": ["channels"]
            }
        }
    })
}

fn events_unsubscribe_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.events.unsubscribe",
            "description": "Unsubscribe an MCP server from event channels.",
            "parameters": {
                "type": "object",
                "properties": {
                    "subscription_id": {
                        "type": "string",
                        "description": "Subscription ID to remove"
                    }
                },
                "required": ["subscription_id"]
            }
        }
    })
}

fn events_replay_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.events.replay",
            "description": "Replay buffered events from after a given sequence number.",
            "parameters": {
                "type": "object",
                "properties": {
                    "subscription_id": {
                        "type": "string",
                        "description": "Subscription ID to replay events for"
                    },
                    "after_seq": {
                        "type": "integer",
                        "description": "Replay events with _mgp.seq strictly greater than this value"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of events to return (default: 100, max: 1000)"
                    }
                },
                "required": ["subscription_id", "after_seq"]
            }
        }
    })
}

fn events_pending_callbacks_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.events.pending_callbacks",
            "description": "List all pending (unresponded) callbacks with their age.",
            "parameters": {
                "type": "object",
                "properties": {}
            }
        }
    })
}

/// Execute mgp.events.subscribe — register event subscription.
pub(super) async fn execute_events_subscribe(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    super::mcp_events::subscribe(manager, args).await
}

/// Execute mgp.events.unsubscribe — remove event subscription.
pub(super) async fn execute_events_unsubscribe(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    super::mcp_events::unsubscribe(manager, args).await
}

/// Execute mgp.events.replay — replay buffered events.
pub(super) async fn execute_events_replay(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    super::mcp_events::replay(manager, args).await
}

/// Execute mgp.events.pending_callbacks — list pending (unresponded) callbacks.
pub(super) async fn execute_events_pending_callbacks(
    manager: &McpClientManager,
    _args: Value,
) -> Result<Value> {
    let pending = manager.events.pending_callbacks();
    let items: Vec<Value> = pending
        .into_iter()
        .map(|(id, server_id, message, options, age_secs)| {
            serde_json::json!({
                "callback_id": id,
                "server_id": server_id,
                "message": message,
                "options": options,
                "age_secs": age_secs,
            })
        })
        .collect();
    Ok(serde_json::json!({ "pending": items, "count": items.len() }))
}

// ============================================================
// Tier 3: Callback Kernel Tools (MGP §13)
// ============================================================

fn callback_respond_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.callback.respond",
            "description": "Respond to a pending callback request from an MCP server.",
            "parameters": {
                "type": "object",
                "properties": {
                    "callback_id": {
                        "type": "string",
                        "description": "ID of the callback to respond to"
                    },
                    "response": {
                        "type": "string",
                        "description": "Response value or selected option"
                    }
                },
                "required": ["callback_id", "response"]
            }
        }
    })
}

/// Execute mgp.callback.respond — respond to a pending callback.
pub(super) async fn execute_callback_respond(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    super::mcp_events::respond_to_callback(manager, args).await
}

// ── Inter-Agent Delegation ──

/// Schema for `ask_agent` — inter-agent question/delegation tool.
fn ask_agent_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "ask_agent",
            "description": "Ask another agent a question or delegate a task. The target agent processes the prompt using its own LLM engine and system prompt, then returns a response. Context isolation is enforced: the target agent cannot see the caller's conversation history.",
            "parameters": {
                "type": "object",
                "properties": {
                    "target_agent_id": {
                        "type": "string",
                        "description": "The agent ID (not display name). IDs use the format 'agent.<name>' (e.g., 'agent.cloto_default'). Call mgp.discovery.list first to get the exact agent IDs."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The question or task to send to the target agent"
                    },
                    "context": {
                        "type": "string",
                        "description": "Optional additional context to include with the prompt"
                    }
                },
                "required": ["target_agent_id", "prompt"]
            }
        }
    })
}

/// Maximum delegation chain depth (prevents infinite delegation loops).
const MAX_DELEGATION_DEPTH: usize = 3;

/// Execute `ask_agent` — delegate a question/task to another agent.
///
/// The calling agent's `agent_id` is injected by the kernel (anti-spoofing).
/// The target agent processes the prompt with its own LLM engine and system prompt.
/// Context isolation: the target agent does NOT receive the caller's conversation history.
#[allow(clippy::too_many_lines)]
pub(super) async fn execute_ask_agent(manager: &McpClientManager, args: Value) -> Result<Value> {
    // 1. YOLO mode check — inter-agent delegation is a privileged operation
    if !manager.yolo_mode.load(Ordering::Relaxed) {
        return Ok(serde_json::json!({
            "status": "rejected",
            "reason": "ask_agent requires YOLO mode to be enabled (inter-agent delegation is a privileged operation)."
        }));
    }

    // 2. Extract parameters
    let target_agent_id = args
        .get("target_agent_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: target_agent_id"))?;

    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: prompt"))?;

    let context = args.get("context").and_then(|v| v.as_str()).unwrap_or("");

    // 3. Get calling agent's ID (injected by kernel anti-spoofing)
    let caller_agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    // 4. Prevent self-delegation
    if caller_agent_id == target_agent_id {
        return Ok(serde_json::json!({
            "status": "rejected",
            "reason": "Cannot delegate to self."
        }));
    }

    // 5. Delegation chain validation
    let delegation_chain: Vec<String> = args
        .get("_delegation_chain")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    if delegation_chain.len() >= MAX_DELEGATION_DEPTH {
        return Ok(serde_json::json!({
            "status": "rejected",
            "reason": format!(
                "Delegation chain depth {} exceeds maximum of {}.",
                delegation_chain.len(),
                MAX_DELEGATION_DEPTH
            ),
            "chain": delegation_chain,
        }));
    }

    // 6. Circular reference detection
    if delegation_chain.contains(&target_agent_id.to_string()) {
        return Ok(serde_json::json!({
            "status": "rejected",
            "reason": format!(
                "Circular delegation detected: '{}' is already in the delegation chain.",
                target_agent_id
            ),
            "chain": delegation_chain,
        }));
    }

    // 7. Look up target agent
    let agent_mgr = super::agents::AgentManager::new(manager.pool().clone(), 30_000);

    // Resolve caller agent display name for dialogue events
    let caller_agent_name = agent_mgr
        .get_agent_config(caller_agent_id)
        .await
        .map_or_else(|_| caller_agent_id.to_string(), |(meta, _)| meta.name);

    let dialogue_id = format!("dlg-{}", chrono::Utc::now().timestamp_millis());

    let Ok((agent_meta, engine_id)) = agent_mgr.get_agent_config(target_agent_id).await else {
        return Ok(serde_json::json!({
            "status": "error",
            "reason": format!("Agent '{}' not found.", target_agent_id)
        }));
    };

    // 8. Check agent is enabled
    if !agent_meta.enabled {
        return Ok(serde_json::json!({
            "status": "error",
            "reason": format!("Agent '{}' is powered off.", target_agent_id)
        }));
    }

    // 9. Check engine exists
    if !manager.has_server(&engine_id).await {
        return Ok(serde_json::json!({
            "status": "error",
            "reason": format!(
                "Engine '{}' for agent '{}' is not available.",
                engine_id, target_agent_id
            )
        }));
    }

    // 10. Build the message content
    let full_prompt = if context.is_empty() {
        prompt.to_string()
    } else {
        format!("{}\n\nContext:\n{}", prompt, context)
    };

    // 11. Construct the think() call arguments
    //     Uses the same format as SystemHandler::engine_think() (system.rs:1051-1060)
    let think_args = serde_json::json!({
        "agent": serde_json::to_value(&agent_meta)?,
        "message": {
            "id": format!("delegation-{}", chrono::Utc::now().timestamp_millis()),
            "source": { "Agent": { "id": caller_agent_id } },
            "content": full_prompt,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "metadata": {
                "delegation": "true",
                "delegated_by": caller_agent_id,
            }
        },
        "context": [],
    });

    info!(
        "ask_agent: {} → {} (engine: {}, chain depth: {})",
        caller_agent_id,
        target_agent_id,
        engine_id,
        delegation_chain.len() + 1
    );

    // Emit AgentDialogue "pending" event
    let chain_depth = u8::try_from(delegation_chain.len() + 1).unwrap_or(u8::MAX);
    manager
        .emit_kernel_event(cloto_shared::ClotoEventData::AgentDialogue {
            dialogue_id: dialogue_id.clone(),
            caller_agent_id: caller_agent_id.to_string(),
            caller_agent_name: caller_agent_name.clone(),
            target_agent_id: target_agent_id.to_string(),
            target_agent_name: agent_meta.name.clone(),
            prompt: prompt.to_string(),
            engine_id: engine_id.clone(),
            response: None,
            chain_depth,
            status: "pending".to_string(),
        })
        .await;

    // 12. Run mini agentic loop (think_with_tools if available, else plain think)
    let supports_tools = manager
        .has_server_tool(&engine_id, "think_with_tools")
        .await;
    let tools = if supports_tools {
        manager
            .collect_tool_schemas_for_agent(target_agent_id)
            .await
    } else {
        vec![]
    };

    let response =
        match run_ask_agent_loop(manager, &engine_id, &think_args, &tools, target_agent_id).await {
            Ok(content) => content,
            Err(e) => {
                // Emit AgentDialogue "error" event
                manager
                    .emit_kernel_event(cloto_shared::ClotoEventData::AgentDialogue {
                        dialogue_id: dialogue_id.clone(),
                        caller_agent_id: caller_agent_id.to_string(),
                        caller_agent_name: caller_agent_name.clone(),
                        target_agent_id: target_agent_id.to_string(),
                        target_agent_name: agent_meta.name.clone(),
                        prompt: prompt.to_string(),
                        engine_id: engine_id.clone(),
                        response: Some(format!("Engine call failed: {}", e)),
                        chain_depth,
                        status: "error".to_string(),
                    })
                    .await;
                // Audit: delegation failed
                manager
                    .broadcast_audit_event(&crate::db::AuditLogEntry {
                        timestamp: chrono::Utc::now(),
                        event_type: "DELEGATION_FAILED".to_string(),
                        actor_id: Some(caller_agent_id.to_string()),
                        target_id: Some(target_agent_id.to_string()),
                        permission: None,
                        result: "error".to_string(),
                        reason: e.to_string(),
                        metadata: None,
                        trace_id: None,
                    })
                    .await;
                return Ok(serde_json::json!({
                    "status": "error",
                    "reason": format!("Engine call failed: {}", e),
                    "source_agent": caller_agent_id,
                    "target_agent": target_agent_id,
                }));
            }
        };

    // Emit AgentDialogue "success" event
    manager
        .emit_kernel_event(cloto_shared::ClotoEventData::AgentDialogue {
            dialogue_id: dialogue_id.clone(),
            caller_agent_id: caller_agent_id.to_string(),
            caller_agent_name: caller_agent_name.clone(),
            target_agent_id: target_agent_id.to_string(),
            target_agent_name: agent_meta.name.clone(),
            prompt: prompt.to_string(),
            engine_id: engine_id.clone(),
            response: Some(response.clone()),
            chain_depth,
            status: "success".to_string(),
        })
        .await;

    // 14. Audit log: DELEGATION_EXECUTED
    manager
        .broadcast_audit_event(&crate::db::AuditLogEntry {
            timestamp: chrono::Utc::now(),
            event_type: "DELEGATION_EXECUTED".to_string(),
            actor_id: Some(caller_agent_id.to_string()),
            target_id: Some(target_agent_id.to_string()),
            permission: None,
            result: "success".to_string(),
            reason: String::new(),
            metadata: Some(serde_json::json!({
                "engine_id": engine_id,
                "chain_depth": delegation_chain.len() + 1,
            })),
            trace_id: None,
        })
        .await;

    // 15. Return structured response
    Ok(serde_json::json!({
        "status": "success",
        "source_agent": caller_agent_id,
        "target_agent": target_agent_id,
        "engine_id": engine_id,
        "response": response,
    }))
}

// ────────────────────────────────────────────────────────────────────
// GUI Documentation Tools
// ────────────────────────────────────────────────────────────────────

fn gui_map_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "gui.map",
            "description": "Returns the ClotoCore dashboard component map — a structured overview of all UI pages, components, and their purposes. Use this first to identify which source files are relevant to the user's question, then use gui.read to read specific files.",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }
    })
}

fn gui_read_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "gui.read",
            "description": "Read a dashboard source file to understand UI implementation details. The path must be relative to dashboard/src/ (e.g., 'components/AgentTerminal.tsx'). Use gui.map first to identify which files to read.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path relative to dashboard/src/ (e.g., 'components/AgentTerminal.tsx', 'hooks/useAgents.ts')"
                    }
                },
                "required": ["path"]
            }
        }
    })
}

/// Execute gui.map: read and return the component map file.
pub(super) async fn execute_gui_map(_manager: &McpClientManager, _args: Value) -> Result<Value> {
    let map_path = std::path::Path::new("docs/gui/component-map.md");
    if !map_path.exists() {
        return Ok(serde_json::json!({
            "status": "error",
            "reason": "Component map file not found at docs/gui/component-map.md"
        }));
    }

    let content = tokio::fs::read_to_string(map_path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read component map: {}", e))?;

    Ok(serde_json::json!({
        "status": "success",
        "content": content,
    }))
}

/// Allowed file extensions for gui.read (source code only).
const GUI_READ_ALLOWED_EXTENSIONS: &[&str] = &["tsx", "ts", "json", "css", "md"];

/// Maximum file size for gui.read (200KB).
const GUI_READ_MAX_SIZE: u64 = 200 * 1024;

/// Execute gui.read: read a specific dashboard source file with path traversal protection.
pub(super) async fn execute_gui_read(_manager: &McpClientManager, args: Value) -> Result<Value> {
    let rel_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

    // Reject obviously malicious input
    if rel_path.contains("..") || rel_path.starts_with('/') || rel_path.starts_with('\\') {
        return Ok(serde_json::json!({
            "status": "error",
            "reason": "Invalid path: must be relative to dashboard/src/ without '..' traversal"
        }));
    }

    // Check extension
    let ext = std::path::Path::new(rel_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if !GUI_READ_ALLOWED_EXTENSIONS.contains(&ext) {
        return Ok(serde_json::json!({
            "status": "error",
            "reason": format!(
                "File type '.{}' not allowed. Allowed: {}",
                ext,
                GUI_READ_ALLOWED_EXTENSIONS.join(", ")
            )
        }));
    }

    // Build and canonicalize paths
    let base_dir = std::path::Path::new("dashboard/src");
    let target = base_dir.join(rel_path);

    // Canonicalize both to resolve symlinks and verify containment
    let Ok(canonical_base) = base_dir.canonicalize() else {
        return Ok(serde_json::json!({
            "status": "error",
            "reason": "Dashboard source directory not found"
        }));
    };
    let Ok(canonical_target) = target.canonicalize() else {
        return Ok(serde_json::json!({
            "status": "error",
            "reason": format!("File not found: {}", rel_path)
        }));
    };

    // Path traversal protection: ensure resolved path is under dashboard/src/
    if !canonical_target.starts_with(&canonical_base) {
        return Ok(serde_json::json!({
            "status": "error",
            "reason": "Path traversal detected: resolved path is outside dashboard/src/"
        }));
    }

    // Check file size
    let metadata = tokio::fs::metadata(&canonical_target)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read file metadata: {}", e))?;
    if metadata.len() > GUI_READ_MAX_SIZE {
        return Ok(serde_json::json!({
            "status": "error",
            "reason": format!("File too large ({} bytes, max {} bytes)", metadata.len(), GUI_READ_MAX_SIZE)
        }));
    }

    let content = tokio::fs::read_to_string(&canonical_target)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read file: {}", e))?;

    Ok(serde_json::json!({
        "status": "success",
        "path": rel_path,
        "size_bytes": metadata.len(),
        "content": content,
    }))
}

// ────────────────────────────────────────────────────────────────────
// ask_agent Mini Agentic Loop
// ────────────────────────────────────────────────────────────────────

/// Maximum tool iterations for ask_agent (keeps delegations concise).
const ASK_AGENT_MAX_TOOL_ITERATIONS: u8 = 5;
/// Tool execution timeout in seconds.
const ASK_AGENT_TOOL_TIMEOUT_SECS: u64 = 30;

/// Run a mini agentic loop for ask_agent: think_with_tools → execute tools → repeat.
/// Falls back to plain `think` if no tools are available.
#[allow(clippy::too_many_lines)]
async fn run_ask_agent_loop(
    manager: &McpClientManager,
    engine_id: &str,
    think_args: &Value,
    tools: &[Value],
    target_agent_id: &str,
) -> anyhow::Result<String> {
    // No tools available → plain think (single inference)
    if tools.is_empty() {
        let result = manager
            .call_server_tool(engine_id, "think", think_args.clone())
            .await?;
        return Ok(extract_think_response(&result));
    }

    // Build initial args for think_with_tools
    let mut loop_args = think_args.clone();
    if let Some(obj) = loop_args.as_object_mut() {
        obj.insert("tools".to_string(), Value::Array(tools.to_vec()));
    }

    let mut tool_history: Vec<Value> = Vec::new();

    for iteration in 0..ASK_AGENT_MAX_TOOL_ITERATIONS {
        // Add accumulated tool_history
        if let Some(obj) = loop_args.as_object_mut() {
            obj.insert(
                "tool_history".to_string(),
                Value::Array(tool_history.clone()),
            );
        }

        let result = manager
            .call_server_tool(engine_id, "think_with_tools", loop_args.clone())
            .await?;

        match parse_think_result(&result)? {
            cloto_shared::ThinkResult::Final(content) => {
                info!(
                    "ask_agent loop: final response at iteration {}",
                    iteration + 1
                );
                return Ok(content);
            }
            cloto_shared::ThinkResult::ToolCalls {
                assistant_content,
                calls,
            } => {
                // Build assistant message for tool_history
                let tool_calls_json: Vec<Value> = calls
                    .iter()
                    .map(|tc| {
                        serde_json::json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": serde_json::to_string(&tc.arguments)
                                    .unwrap_or_default()
                            }
                        })
                    })
                    .collect();

                tool_history.push(serde_json::json!({
                    "role": "assistant",
                    "content": assistant_content.as_deref().unwrap_or(""),
                    "tool_calls": tool_calls_json,
                }));

                // Execute each tool call
                for call in &calls {
                    // Anti-spoofing: inject target agent's ID
                    let mut safe_args = call.arguments.clone();
                    if let Some(obj) = safe_args.as_object_mut() {
                        obj.insert(
                            "agent_id".to_string(),
                            Value::String(target_agent_id.to_string()),
                        );
                    }

                    // Resolve tool name → server ID (avoids recursive async through execute_tool_internal)
                    let server_id = manager.resolve_tool_server(&call.name).await;
                    let tool_result = if let Some(sid) = server_id {
                        tokio::time::timeout(
                            std::time::Duration::from_secs(ASK_AGENT_TOOL_TIMEOUT_SECS),
                            manager.call_server_tool(&sid, &call.name, safe_args),
                        )
                        .await
                    } else {
                        Ok(Err(anyhow::anyhow!(
                            "Tool '{}' not found in any connected server",
                            call.name
                        )))
                    };

                    let result_text = match tool_result {
                        Ok(Ok(v)) => {
                            // Extract text from CallToolResult
                            v.content
                                .iter()
                                .filter_map(|c| match c {
                                    super::mcp_protocol::ToolContent::Text { text } => {
                                        Some(text.clone())
                                    }
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                        Ok(Err(e)) => format!("Error: {e}"),
                        Err(_) => "Error: tool execution timed out".to_string(),
                    };

                    info!(
                        "ask_agent loop: tool {} executed (iteration {})",
                        call.name,
                        iteration + 1
                    );

                    tool_history.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": call.id,
                        "content": result_text,
                    }));
                }
            }
        }
    }

    // Max iterations reached — return last assistant content or error
    Err(anyhow::anyhow!(
        "ask_agent agentic loop exceeded {} iterations",
        ASK_AGENT_MAX_TOOL_ITERATIONS
    ))
}

/// Extract text response from a plain `think` CallToolResult.
fn extract_think_response(result: &super::mcp_protocol::CallToolResult) -> String {
    let text: String = result
        .content
        .iter()
        .filter_map(|c| match c {
            super::mcp_protocol::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
        parsed
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or(&text)
            .to_string()
    } else {
        text
    }
}

/// Parse ThinkResult from MCP think_with_tools() response.
/// Standalone version of SystemHandler::parse_mcp_think_result for use in ask_agent.
fn parse_think_result(
    result: &super::mcp_protocol::CallToolResult,
) -> anyhow::Result<cloto_shared::ThinkResult> {
    use cloto_shared::ToolCall;

    for content in &result.content {
        if let super::mcp_protocol::ToolContent::Text { text } = content {
            let json: Value = serde_json::from_str(text)
                .map_err(|e| anyhow::anyhow!("MCP engine returned invalid JSON: {}", e))?;

            if let Some(error) = json.get("error").and_then(|e| e.as_str()) {
                return Err(anyhow::anyhow!("Engine error: {}", error));
            }

            let result_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("final");

            if result_type == "tool_calls" {
                let assistant_content = json
                    .get("assistant_content")
                    .and_then(|c| c.as_str())
                    .map(std::string::ToString::to_string);
                let calls_json = json
                    .get("calls")
                    .and_then(|c| c.as_array())
                    .cloned()
                    .unwrap_or_default();

                let calls: Vec<ToolCall> = calls_json
                    .iter()
                    .filter_map(|tc| {
                        let id = tc.get("id")?.as_str()?.to_string();
                        let name = tc.get("name")?.as_str()?.to_string();
                        let arguments = tc
                            .get("arguments")
                            .cloned()
                            .unwrap_or(serde_json::json!({}));
                        Some(ToolCall {
                            id,
                            name,
                            arguments,
                        })
                    })
                    .collect();

                return Ok(cloto_shared::ThinkResult::ToolCalls {
                    assistant_content,
                    calls,
                });
            }

            let content = json
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            return Ok(cloto_shared::ThinkResult::Final(content));
        }
    }
    Err(anyhow::anyhow!(
        "MCP engine returned no parseable ThinkResult"
    ))
}
