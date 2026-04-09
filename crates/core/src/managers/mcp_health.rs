//! MCP server health monitor.
//!
//! Periodically checks for dead MCP server processes and auto-restarts
//! them using LifecycleManager restart policies and backoff (§11.6).

use super::mcp::McpClientManager;
use super::mcp_protocol::RestartPolicy;
use super::mcp_types::ServerStatus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Spawn a background task that periodically checks for dead MCP servers
/// and auto-restarts them based on their restart policy (§11.6).
/// Follows the `tokio::select!` + `Arc<Notify>` shutdown pattern from events.rs.
pub(super) fn spawn_health_monitor(
    manager: Arc<McpClientManager>,
    shutdown: Arc<tokio::sync::Notify>,
    interval_secs: u64,
    setup_in_progress: Arc<AtomicBool>,
    setup_done: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            tokio::select! {
                () = shutdown.notified() => {
                    info!("MCP health monitor shutting down");
                    break;
                }
                _ = interval.tick() => {
                    // Skip auto-restart when setup hasn't completed or install is running.
                    // On clean installs, servers lack Python/venv and die immediately,
                    // causing restart_server() to hold a write lock that blocks
                    // list_servers() in the batch install flow.
                    if !setup_done.load(Ordering::Relaxed)
                        || setup_in_progress.load(Ordering::Relaxed)
                    {
                        debug!("Skipping MCP health check — setup not done or install in progress");
                        continue;
                    }
                    check_and_restart_dead_servers(&manager, &setup_in_progress).await;
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
#[allow(clippy::too_many_lines)]
async fn check_and_restart_dead_servers(
    manager: &McpClientManager,
    setup_in_progress: &AtomicBool,
) {
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
        // Re-check flag inside the loop: batch install may have started
        // while we were processing earlier servers in this batch.
        if setup_in_progress.load(Ordering::Relaxed) {
            debug!("Aborting restart loop — setup started");
            return;
        }

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
                // Scope the write lock — emit_lifecycle_notification acquires
                // a read lock, so the write lock must be released first.
                {
                    let mut state = manager.state.write().await;
                    if let Some(handle) = state.servers.get_mut(&server_id) {
                        handle.status =
                            ServerStatus::Error(format!("Auto-restart failed: {}", e));
                    }
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
