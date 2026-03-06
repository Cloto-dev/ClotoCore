//! MCP server health monitor.
//!
//! Periodically checks for dead MCP server processes and auto-restarts
//! them when `auto_restart` is enabled in the server configuration.

use super::mcp::McpClientManager;
use super::mcp_types::ServerStatus;
use std::sync::Arc;
use tracing::{error, info, warn};

/// Spawn a background task that periodically checks for dead MCP servers
/// and auto-restarts them if `auto_restart` is enabled in their config.
/// Follows the `tokio::select!` + `Arc<Notify>` shutdown pattern from events.rs.
pub(super) fn spawn_health_monitor(
    manager: Arc<McpClientManager>,
    shutdown: Arc<tokio::sync::Notify>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tokio::select! {
                () = shutdown.notified() => {
                    info!("MCP health monitor shutting down");
                    break;
                }
                _ = interval.tick() => {
                    check_and_restart_dead_servers(&manager).await;
                }
            }
        }
    });
}

/// Scan all registered MCP servers and restart any that have died
/// (process exited / channel closed) if their config has `auto_restart: true`.
async fn check_and_restart_dead_servers(manager: &McpClientManager) {
    let dead_servers: Vec<String> = {
        let servers = manager.servers.read().await;
        servers
            .iter()
            .filter_map(|(id, handle)| {
                if !handle.config.auto_restart {
                    return None;
                }
                let is_dead = match &handle.client {
                    Some(client) => !client.is_alive(),
                    None => matches!(handle.status, ServerStatus::Error(_)),
                };
                if is_dead {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect()
    };

    for server_id in dead_servers {
        warn!(server_id = %server_id, "MCP server died, attempting auto-restart");
        match manager.restart_server(&server_id).await {
            Ok(tools) => {
                info!(
                    server_id = %server_id,
                    tools = tools.len(),
                    "MCP server auto-restarted successfully"
                );
            }
            Err(e) => {
                error!(
                    server_id = %server_id,
                    error = %e,
                    "MCP server auto-restart failed"
                );
                // Update status to Error so the UI reflects the failure
                let mut servers = manager.servers.write().await;
                if let Some(handle) = servers.get_mut(&server_id) {
                    handle.status = ServerStatus::Error(format!("Auto-restart failed: {}", e));
                }
            }
        }
    }
}
