//! MGP Tier 4 — Server Discovery (§15).
//!
//! Provides three kernel tools for runtime server registry management:
//! `mgp.discovery.list`, `mgp.discovery.register`, `mgp.discovery.deregister`.

use super::mcp::McpClientManager;
use super::mcp_types::ServerSource;
use anyhow::Result;
use serde_json::Value;
use std::sync::atomic::Ordering;
use tracing::{debug, info};

// ============================================================
// Kernel Tool Schemas (§15.4)
// ============================================================

/// Return all §15 discovery kernel tool schemas.
pub(super) fn discovery_tool_schemas() -> Vec<Value> {
    vec![
        discovery_list_schema(),
        discovery_register_schema(),
        discovery_deregister_schema(),
    ]
}

fn discovery_list_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.discovery.list",
            "description": "Query connected and registered MCP servers with optional filtering.",
            "parameters": {
                "type": "object",
                "properties": {
                    "filter": {
                        "type": "object",
                        "properties": {
                            "extensions": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Required extensions (server must have ALL)"
                            },
                            "permissions": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Required permissions (server must have ALL)"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["connected", "disconnected", "all"],
                                "description": "Filter by server status (default: connected)"
                            }
                        },
                        "description": "Filter criteria for server listing"
                    }
                }
            }
        }
    })
}

fn discovery_register_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.discovery.register",
            "description": "Register a new MCP server at runtime (requires YOLO mode).",
            "parameters": {
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Server identifier"
                    },
                    "command": {
                        "type": "string",
                        "description": "Command to start the server"
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Command arguments"
                    },
                    "transport": {
                        "type": "string",
                        "enum": ["stdio", "http"],
                        "description": "Transport protocol"
                    },
                    "mgp": {
                        "type": "object",
                        "properties": {
                            "extensions": {
                                "type": "array",
                                "items": { "type": "string" }
                            },
                            "permissions_required": {
                                "type": "array",
                                "items": { "type": "string" }
                            },
                            "trust_level": {
                                "type": "string",
                                "enum": ["core", "standard", "experimental", "untrusted"]
                            }
                        },
                        "description": "MGP configuration for the server"
                    },
                    "created_by": {
                        "type": "string",
                        "description": "Agent or user that initiated registration"
                    },
                    "justification": {
                        "type": "string",
                        "description": "Reason for registering this server"
                    }
                },
                "required": ["id", "command", "transport"]
            }
        }
    })
}

fn discovery_deregister_schema() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "mgp.discovery.deregister",
            "description": "Remove a dynamically registered MCP server. Config-loaded servers cannot be deregistered.",
            "parameters": {
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Server identifier to deregister"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Reason for deregistration"
                    }
                },
                "required": ["id"]
            }
        }
    })
}

// ============================================================
// Kernel Tool Executors (§15.4)
// ============================================================

/// Execute mgp.discovery.list — query connected and registered servers.
pub(super) async fn execute_discovery_list(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    let filter = args.get("filter");
    let filter_extensions: Option<Vec<String>> = filter
        .and_then(|f| f.get("extensions"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        });
    let filter_permissions: Option<Vec<String>> = filter
        .and_then(|f| f.get("permissions"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        });
    let filter_status = filter
        .and_then(|f| f.get("status"))
        .and_then(|v| v.as_str())
        .unwrap_or("connected");

    let mut servers_json = Vec::new();

    {
        let state = manager.state.read().await;

        // Connected/active servers
        for handle in state.servers.values() {
            // Status filter
            let is_connected = handle.status.is_operational();
            if filter_status == "connected" && !is_connected {
                continue;
            }
            if filter_status == "disconnected" && is_connected {
                continue;
            }

            // Skip mind.* (engine-internal)
            if handle.id.starts_with("mind.") {
                continue;
            }

            let mgp = handle.mgp_negotiated.as_ref();

            // Extension filter
            if let Some(ref required_ext) = filter_extensions {
                let server_ext: Vec<String> =
                    mgp.map(|m| m.active_extensions.clone()).unwrap_or_default();
                if !required_ext.iter().all(|e| server_ext.contains(e)) {
                    continue;
                }
            }

            // Permission filter
            if let Some(ref required_perm) = filter_permissions {
                if !required_perm
                    .iter()
                    .all(|p| handle.config.required_permissions.contains(p))
                {
                    continue;
                }
            }

            let tools: Vec<String> = handle.tools.iter().map(|t| t.name.clone()).collect();
            let trust_level = mgp.map(|m| format!("{:?}", m.trust_level).to_lowercase());

            servers_json.push(serde_json::json!({
                "id": handle.id,
                "status": handle.status,
                "mgp_version": mgp.map(|m| m.version.as_str()),
                "extensions": mgp.map(|m| &m.active_extensions).cloned().unwrap_or_default(),
                "tools": tools,
                "trust_level": trust_level,
            }));
        }

        // Include stopped servers when filter is "all" or "disconnected"
        if filter_status == "all" || filter_status == "disconnected" {
            for (id, (config, _source)) in &state.stopped_configs {
                // Check if already included from active servers
                if servers_json
                    .iter()
                    .any(|s| s.get("id").and_then(|v| v.as_str()) == Some(id))
                {
                    continue;
                }
                servers_json.push(serde_json::json!({
                    "id": id,
                    "status": "Disconnected",
                    "mgp_version": null,
                    "extensions": [],
                    "tools": [],
                    "trust_level": config.mgp.as_ref().and_then(|m| m.trust_level.as_deref()),
                }));
            }
        }
    }

    debug!(count = servers_json.len(), filter = %filter_status, "Discovery list completed");

    Ok(serde_json::json!({
        "servers": servers_json,
    }))
}

/// Execute mgp.discovery.register — register a runtime server.
pub(super) async fn execute_discovery_register(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    if !manager.yolo_mode.load(Ordering::Relaxed) {
        return Err(anyhow::Error::new(
            super::mcp_mgp::MgpError::permission_denied(
                "mgp.discovery.register requires YOLO mode to be enabled",
            ),
        ));
    }

    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: id"))?;
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: command"))?;
    let transport = args
        .get("transport")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: transport"))?;

    // Check for duplicate
    {
        let state = manager.state.read().await;
        if state.servers.contains_key(id) {
            return Err(anyhow::Error::new(
                super::mcp_mgp::MgpError::server_already_registered(format!(
                    "Server '{}' is already registered",
                    id
                )),
            ));
        }
    }

    let cmd_args: Vec<String> = args
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let mgp_config = args
        .get("mgp")
        .and_then(|v| serde_json::from_value::<super::mcp_mgp::MgpServerConfig>(v.clone()).ok());

    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let auth_token = args
        .get("auth_token")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    let config = super::mcp_protocol::McpServerConfig {
        id: id.to_string(),
        command: command.to_string(),
        args: cmd_args,
        env: std::collections::HashMap::new(),
        transport: transport.to_string(),
        url,
        auth_token,
        auto_restart: None,
        required_permissions: Vec::new(),
        tool_validators: std::collections::HashMap::new(),
        display_name: None,
        mgp: mgp_config,
        restart_policy: None,
        seal: None,
        isolation: None,
    };

    info!(id = %id, command = %command, "Registering dynamic server via mgp.discovery.register");

    match manager.connect_server(config, ServerSource::Dynamic).await {
        Ok(tools) => Ok(serde_json::json!({
            "id": id,
            "status": "connected",
            "message": format!("Server registered and connected with {} tool(s)", tools.len()),
        })),
        Err(e) => Ok(serde_json::json!({
            "id": id,
            "status": "error",
            "message": format!("Registration failed: {}", e),
        })),
    }
}

/// Execute mgp.discovery.deregister — remove a dynamically registered server.
pub(super) async fn execute_discovery_deregister(
    manager: &McpClientManager,
    args: Value,
) -> Result<Value> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: id"))?;

    // Check source — only allow deregistering dynamic servers
    {
        let state = manager.state.read().await;
        if let Some(handle) = state.servers.get(id) {
            if handle.source == ServerSource::Config {
                return Err(anyhow::Error::new(
                    super::mcp_mgp::MgpError::cannot_deregister_config(format!(
                        "Cannot deregister config-loaded server '{}'. Use lifecycle.shutdown instead.",
                        id
                    )),
                ));
            }
        }
    }

    let reason = args
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("deregistered via mgp.discovery.deregister");

    info!(id = %id, reason = %reason, "Deregistering server via mgp.discovery.deregister");

    manager.disconnect_server(id).await?;

    Ok(serde_json::json!({
        "id": id,
        "status": "deregistered",
        "message": format!("Server '{}' deregistered: {}", id, reason),
    }))
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_schemas_valid() {
        let schemas = discovery_tool_schemas();
        assert_eq!(schemas.len(), 3);

        // Verify each schema has the expected tool name
        let names: Vec<&str> = schemas
            .iter()
            .filter_map(|s| s["function"]["name"].as_str())
            .collect();
        assert!(names.contains(&"mgp.discovery.list"));
        assert!(names.contains(&"mgp.discovery.register"));
        assert!(names.contains(&"mgp.discovery.deregister"));

        // register has required fields
        let register = &schemas[1];
        let required = register["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("id")));
        assert!(required.iter().any(|v| v.as_str() == Some("command")));
        assert!(required.iter().any(|v| v.as_str() == Some("transport")));
    }
}
