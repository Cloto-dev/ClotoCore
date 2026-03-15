//! MCP server health monitor.
//!
//! Periodically checks for dead MCP server processes and auto-restarts
//! them using LifecycleManager restart policies and backoff (§11.6).

use super::mcp::McpClientManager;
use super::mcp_protocol::RestartPolicy;
use super::mcp_types::ServerStatus;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Spawn a background task that periodically checks for dead MCP servers
/// and auto-restarts them based on their restart policy (§11.6).
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
                    // Clean up responded callbacks older than 5 minutes (§13.4)
                    let cleaned = manager.events.cleanup_stale_callbacks(
                        std::time::Duration::from_secs(300),
                    );
                    if cleaned > 0 {
                        debug!(count = cleaned, "Cleaned up stale callbacks");
                    }
                }
            }
        }
    });
}

/// Scan all registered MCP servers and restart any that have died
/// (process exited / channel closed) if their restart policy allows it.
async fn check_and_restart_dead_servers(manager: &McpClientManager) {
    let dead_servers: Vec<(String, ServerStatus, RestartPolicy)> = {
        let state = manager.state.read().await;
        state
            .servers
            .iter()
            .filter_map(|(id, handle)| {
                let policy = handle.config.effective_restart_policy();
                let is_dead = match &handle.client {
                    Some(client) => !client.is_alive(),
                    None => matches!(handle.status, ServerStatus::Error(_)),
                };
                if is_dead {
                    Some((id.clone(), handle.status.clone(), policy))
                } else {
                    None
                }
            })
            .collect()
    };

    for (server_id, status, policy) in dead_servers {
        if !manager
            .lifecycle
            .should_restart(&server_id, &policy, &status)
        {
            debug!(
                server_id = %server_id,
                strategy = ?policy.strategy,
                "Restart policy denied restart for dead server"
            );
            continue;
        }

        let backoff = manager.lifecycle.calculate_backoff(&server_id, &policy);
        warn!(
            server_id = %server_id,
            backoff_ms = %backoff.as_millis(),
            "MCP server died, waiting backoff before auto-restart"
        );
        tokio::time::sleep(backoff).await;

        match manager.restart_server(&server_id).await {
            Ok(tools) => {
                info!(
                    server_id = %server_id,
                    tools = tools.len(),
                    "MCP server auto-restarted successfully"
                );
                manager.lifecycle.reset_counter(&server_id);

                super::mcp_lifecycle::emit_lifecycle_notification(
                    manager,
                    &server_id,
                    "Error",
                    "Connected",
                    "Auto-restart succeeded",
                )
                .await;

                super::mcp_events::deliver_event(
                    manager,
                    "lifecycle",
                    &serde_json::json!({
                        "server_id": server_id,
                        "previous_state": "Error",
                        "new_state": "Connected",
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                    }),
                )
                .await;
            }
            Err(e) => {
                error!(
                    server_id = %server_id,
                    error = %e,
                    "MCP server auto-restart failed"
                );
                let mut state = manager.state.write().await;
                if let Some(handle) = state.servers.get_mut(&server_id) {
                    handle.status = ServerStatus::Error(format!("Auto-restart failed: {}", e));
                }

                super::mcp_lifecycle::emit_lifecycle_notification(
                    manager,
                    &server_id,
                    "Connected",
                    "Error",
                    &format!("Auto-restart failed: {}", e),
                )
                .await;

                super::mcp_events::deliver_event(
                    manager,
                    "lifecycle",
                    &serde_json::json!({
                        "server_id": server_id,
                        "previous_state": "Connected",
                        "new_state": "Error",
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                    }),
                )
                .await;
            }
        }
    }
}
