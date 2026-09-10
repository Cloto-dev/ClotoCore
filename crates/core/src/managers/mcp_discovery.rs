//! MGP Tier 4 — Server Discovery (§15).
//!
//! Provides three kernel tools for runtime server registry management:
//! `mgp.discovery.list`, `mgp.discovery.register`, `mgp.discovery.deregister`.

use super::mcp::McpClientManager;
use cloto_shared::{RejectionCode, ToolFailure, ToolRejection};
use serde_json::Value;
use std::sync::atomic::Ordering;
use tracing::{debug, info};

/// Local alias shadowing `anyhow::Result`. See `mcp_kernel_tool.rs` for rationale.
type Result<T> = std::result::Result<T, ToolFailure>;

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
    // Scoped to the servers the caller holds a grant for, by the same predicate
    // the acting tools ask "may I touch this one?" with. Unscoped, this was the
    // one place that answered "what is on this host" for free: every server id,
    // its status, its trust level and its whole tool list, to a caller holding
    // no grant at all. §16's `mgp.tools.discover` is the discovery an agent is
    // actually offered — it is injected on every turn, and it is a search, so
    // it answers "is there a tool for this" without enumerating the host. This
    // one is not in any model's tool list; naming it takes knowledge from
    // outside the kernel, which is the shape of reconnaissance rather than of
    // use.
    //
    // Read before the state lock: it goes to the database, and holding the
    // server map across that would make every listing wait on it.
    let granted = super::mcp_kernel_tool::granted_servers(manager, &args).await?;

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
            // Grant filter, before every other one: a server this caller may
            // not reach is not a server it may be told about.
            if !granted.contains(&handle.id) {
                continue;
            }

            // Status filter
            let is_connected = handle.status.is_operational();
            if filter_status == "connected" && !is_connected {
                continue;
            }
            if filter_status == "disconnected" && is_connected {
                continue;
            }

            // Skip reasoning engines (think/think_with_tools are engine-internal).
            // Detected by tool surface, not id prefix, so bare-id engines are all
            // hidden.
            if handle.is_reasoning_engine() {
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

        // Stopped servers are now in `state.servers` with Disconnected status,
        // so they are already included by the main iterator above.
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
        return Err(ToolFailure::Rejection(ToolRejection {
            code: RejectionCode::YoloRequired,
            reason: "This tool is restricted to privileged (YOLO) mode, which is currently disabled by the operator. The kernel will reject identical requests until the operator re-enables privileged mode in the dashboard.".to_string(),
            remediation_hint: Some("Ask the operator to enable YOLO mode in Settings → Security.".to_string()),
            retryable: true,
            details: None,
        }));
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
            return Err(
                anyhow::Error::new(super::mcp_mgp::MgpError::server_already_registered(
                    format!("Server '{}' is already registered", id),
                ))
                .into(),
            );
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
        display_name: None,
        mgp: mgp_config,
        restart_policy: None,
        seal: None,
        isolation: None,
        marketplace_id: None,
        protocol_era: None,
    };

    info!(id = %id, command = %command, "Registering dynamic server via mgp.discovery.register");

    match manager.connect_server(config).await {
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
    // Registering a server is privileged; disconnecting one is the same reach in
    // the other direction, and `mgp.discovery.list` hands out the ids to aim at.
    // The two halves of this pair took different answers to that question until
    // now — registering refused outside privileged mode, deregistering did not.
    if !manager.yolo_mode.load(Ordering::Relaxed) {
        return Err(ToolFailure::Rejection(ToolRejection {
            code: RejectionCode::YoloRequired,
            reason: "This tool is restricted to privileged (YOLO) mode, which is currently disabled by the operator. The kernel will reject identical requests until the operator re-enables privileged mode in the dashboard.".to_string(),
            remediation_hint: Some("Ask the operator to enable YOLO mode in Settings → Security.".to_string()),
            retryable: true,
            details: None,
        }));
    }

    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: id"))?;

    // Verify server exists before attempting deregistration
    {
        let state = manager.state.read().await;
        if !state.servers.contains_key(id) {
            return Err(anyhow::anyhow!("Server '{}' not found", id).into());
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

    #[tokio::test]
    async fn execute_discovery_register_rejects_when_yolo_off() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", None)
            .await
            .unwrap();
        let mgr = McpClientManager::new(pool, false, 120, 30);
        let args = serde_json::json!({
            "id": "test_server",
            "command": "echo",
            "transport": "stdio"
        });
        match execute_discovery_register(&mgr, args).await {
            Err(ToolFailure::Rejection(r)) => {
                assert_eq!(r.code, RejectionCode::YoloRequired);
                assert!(r.retryable);
                assert!(r.reason.contains("privileged"));
            }
            other => panic!("expected YoloRequired rejection, got {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // Who the listing answers
    // ------------------------------------------------------------------

    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;

    async fn manager() -> McpClientManager {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", None)
            .await
            .unwrap();
        McpClientManager::new(pool, false, 120, 30)
    }

    async fn connected(manager: &McpClientManager, id: &str, tool: &str) {
        manager.state.write().await.servers.insert(
            id.to_string(),
            super::super::mcp_types::McpServerHandle {
                id: id.to_string(),
                config: crate::managers::mcp_protocol::McpServerConfig {
                    id: id.to_string(),
                    ..Default::default()
                },
                client: None,
                tools: vec![crate::managers::mcp_protocol::McpTool {
                    name: tool.to_string(),
                    description: None,
                    input_schema: serde_json::json!({}),
                    annotations: None,
                }],
                handshake: None,
                mgp_negotiated: None,
                status: super::super::mcp_types::ServerStatus::Connected,
                audit_seq: Arc::new(AtomicU64::new(0)),
                connected_at: None,
                isolation_profile: None,
                protocol_era: None,
                instructions: None,
            },
        );
    }

    async fn grant(manager: &McpClientManager, agent_id: &str, server_id: &str) {
        crate::db::save_access_control_entry(
            manager.pool(),
            &crate::db::AccessControlEntry {
                id: None,
                entry_type: crate::db::mcp::EntryType::ServerGrant,
                agent_id: agent_id.to_string(),
                server_id: server_id.to_string(),
                tool_name: None,
                permission: crate::db::mcp::PermissionLevel::Allow,
                granted_by: Some("test".to_string()),
                granted_at: chrono::Utc::now().to_rfc3339(),
                expires_at: None,
                justification: None,
                metadata: None,
            },
        )
        .await
        .unwrap();
    }

    async fn listed(manager: &McpClientManager, args: Value) -> Vec<String> {
        let out = execute_discovery_list(manager, args)
            .await
            .expect("a read-only listing must answer, not refuse");
        out["servers"]
            .as_array()
            .expect("servers is a list")
            .iter()
            .map(|s| s["id"].as_str().unwrap().to_string())
            .collect()
    }

    /// The listing answers "which servers may I see?", and that is the same
    /// question the acting tools answer as "may I touch this one?". Unscoped it
    /// handed a caller with no grant every server id on the host, its status and
    /// its whole tool list — the ids to objects it is refused any other access
    /// to, which is what a reconnaissance surface is.
    #[tokio::test]
    async fn a_caller_sees_only_the_servers_it_holds_a_grant_for() {
        let mgr = manager().await;
        connected(&mgr, "mine", "read_note").await;
        connected(&mgr, "theirs", "wire_money").await;
        grant(&mgr, "agent.one", "mine").await;

        let seen = listed(&mgr, serde_json::json!({ "agent_id": "agent.one" })).await;

        assert_eq!(seen, vec!["mine".to_string()]);
    }

    /// Not even the names of the tools. A tool list is the most useful half of
    /// the disclosure: it says what is worth asking for and under which id.
    #[tokio::test]
    async fn a_server_it_may_not_reach_discloses_no_tool_names() {
        let mgr = manager().await;
        connected(&mgr, "theirs", "wire_money").await;
        grant(&mgr, "agent.one", "mine").await;

        let out = execute_discovery_list(&mgr, serde_json::json!({ "agent_id": "agent.one" }))
            .await
            .expect("a read-only listing must answer, not refuse");

        assert!(
            !out.to_string().contains("wire_money"),
            "an ungranted server's tools must not appear anywhere in the response: {out}"
        );
    }

    /// A caller with a grant is not being punished for the scoping: the server
    /// it may reach still arrives whole.
    #[tokio::test]
    async fn a_granted_server_still_arrives_with_its_tools() {
        let mgr = manager().await;
        connected(&mgr, "mine", "read_note").await;
        grant(&mgr, "agent.one", "mine").await;

        let out = execute_discovery_list(&mgr, serde_json::json!({ "agent_id": "agent.one" }))
            .await
            .expect("a read-only listing must answer, not refuse");

        assert_eq!(out["servers"][0]["tools"][0], "read_note");
    }

    /// Refusing an unattributed call was not an option: `agent_id` is forced in
    /// by the anti-spoofing shim, not by the registry, so a call arriving
    /// through the registry carries none — and `capability_gate_test` requires
    /// this read-only tool to stay reachable. Showing nothing is the safe
    /// direction and keeps it so.
    #[tokio::test]
    async fn an_unattributed_listing_shows_nothing_rather_than_everything() {
        let mgr = manager().await;
        connected(&mgr, "theirs", "wire_money").await;

        assert!(listed(&mgr, serde_json::json!({})).await.is_empty());
    }

    /// The grant filter runs before the status filter, so asking for the
    /// disconnected ones is not a way around it.
    #[tokio::test]
    async fn asking_for_every_status_does_not_widen_the_grant() {
        let mgr = manager().await;
        connected(&mgr, "theirs", "wire_money").await;

        let seen = listed(
            &mgr,
            serde_json::json!({
                "agent_id": "agent.one",
                "filter": { "status": "all" },
            }),
        )
        .await;

        assert!(seen.is_empty(), "got {seen:?}");
    }
}
