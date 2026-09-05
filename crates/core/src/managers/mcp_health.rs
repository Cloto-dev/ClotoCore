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
/// Follows the `tokio::select!` + `ShutdownSignal` shutdown pattern from events.rs.
pub(super) fn spawn_health_monitor(
    manager: Arc<McpClientManager>,
    shutdown: crate::shutdown::ShutdownSignal,
    interval_secs: u64,
    setup_in_progress: Arc<AtomicBool>,
    setup_done: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            tokio::select! {
                () = shutdown.raised() => {
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
                        std::time::Duration::from_mins(5),
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

        // bug-433: the snapshot is stale after the backoff sleep. If the operator
        // explicitly stopped/drained this server during the wait, its status is
        // now Disconnected/Draining — do NOT resurrect it. Only proceed if it is
        // still in the dead/Error state (or self-transitioned to Restarting).
        {
            let state = manager.state.read().await;
            match state.servers.get(&server_id).map(|h| h.status.clone()) {
                Some(ServerStatus::Disconnected | ServerStatus::Draining) => {
                    debug!(
                        server_id = %server_id,
                        "Skipping auto-restart — operator stopped/drained the server during backoff"
                    );
                    continue;
                }
                None => {
                    debug!(server_id = %server_id, "Skipping auto-restart — server removed during backoff");
                    continue;
                }
                _ => {}
            }
        }

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
                        handle.status = ServerStatus::Error(format!("Auto-restart failed: {}", e));
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

#[cfg(test)]
mod tests {
    //! Characterization tests for the MCP health monitor.
    //!
    //! Every server here is registered with an empty `command`, which
    //! `validate_command` rejects before anything is spawned — so an
    //! "auto-restart" runs the real code path end to end without ever creating
    //! a child process.

    use super::*;
    use crate::managers::mcp_protocol::{McpServerConfig, RestartStrategy};
    use sqlx::SqlitePool;
    use std::sync::atomic::AtomicU64;

    async fn manager() -> Arc<McpClientManager> {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", None)
            .await
            .unwrap();
        Arc::new(McpClientManager::new(pool, false, 120, 30))
    }

    /// A policy that restarts on failure with an effectively instant backoff.
    fn on_failure(max_restarts: u32, backoff_ms: u64) -> RestartPolicy {
        RestartPolicy {
            strategy: RestartStrategy::OnFailure,
            max_restarts,
            restart_window_secs: 300,
            backoff_base_ms: backoff_ms,
            backoff_max_ms: backoff_ms,
        }
    }

    async fn register_server(
        manager: &McpClientManager,
        id: &str,
        status: ServerStatus,
        policy: Option<RestartPolicy>,
    ) {
        let config = McpServerConfig {
            id: id.to_string(),
            // Empty on purpose: the command whitelist rejects it, so a restart
            // attempt fails deterministically without spawning anything.
            command: String::new(),
            restart_policy: policy,
            ..Default::default()
        };
        manager.state.write().await.servers.insert(
            id.to_string(),
            super::super::mcp_types::McpServerHandle {
                id: id.to_string(),
                config,
                client: None,
                tools: Vec::new(),
                handshake: None,
                mgp_negotiated: None,
                status,
                audit_seq: Arc::new(AtomicU64::new(0)),
                connected_at: None,
                isolation_profile: None,
                protocol_era: None,
                instructions: None,
            },
        );
    }

    async fn status_of(manager: &McpClientManager, id: &str) -> Option<ServerStatus> {
        manager
            .state
            .read()
            .await
            .servers
            .get(id)
            .map(|h| h.status.clone())
    }

    async fn set_status(manager: &McpClientManager, id: &str, status: ServerStatus) {
        if let Some(handle) = manager.state.write().await.servers.get_mut(id) {
            handle.status = status;
        }
    }

    /// Poll until `cond` holds or ~5s elapse.
    async fn wait_until<F, Fut>(mut cond: F) -> bool
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        for _ in 0..500 {
            if cond().await {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        false
    }

    #[tokio::test]
    async fn a_dead_server_whose_policy_forbids_restart_is_left_in_its_error_state() {
        // No restart_policy and no auto_restart ⇒ RestartStrategy::Never.
        let manager = manager().await;
        register_server(&manager, "srv", ServerStatus::Error("crashed".into()), None).await;
        let gate = AtomicBool::new(false);

        check_and_restart_dead_servers(&manager, &gate).await;

        assert_eq!(
            status_of(&manager, "srv").await,
            Some(ServerStatus::Error("crashed".into())),
            "the default policy never resurrects a dead server"
        );
    }

    #[tokio::test]
    async fn a_server_with_no_client_counts_as_dead_only_when_its_status_says_error() {
        // `client: None` alone is not death — a Registered/Connected handle with
        // no client attached is skipped entirely.
        let manager = manager().await;
        let policy = Some(on_failure(5, 1));
        register_server(
            &manager,
            "registered",
            ServerStatus::Registered,
            policy.clone(),
        )
        .await;
        register_server(
            &manager,
            "connected",
            ServerStatus::Connected,
            policy.clone(),
        )
        .await;
        register_server(&manager, "stopped", ServerStatus::Disconnected, policy).await;
        let gate = AtomicBool::new(false);

        check_and_restart_dead_servers(&manager, &gate).await;

        assert_eq!(
            status_of(&manager, "registered").await,
            Some(ServerStatus::Registered)
        );
        assert_eq!(
            status_of(&manager, "connected").await,
            Some(ServerStatus::Connected)
        );
        assert_eq!(
            status_of(&manager, "stopped").await,
            Some(ServerStatus::Disconnected)
        );
    }

    #[tokio::test]
    async fn a_failed_auto_restart_leaves_the_server_in_an_error_that_names_the_failure() {
        let manager = manager().await;
        register_server(
            &manager,
            "srv",
            ServerStatus::Error("crashed".into()),
            Some(on_failure(5, 1)),
        )
        .await;
        let gate = AtomicBool::new(false);

        check_and_restart_dead_servers(&manager, &gate).await;

        let Some(ServerStatus::Error(message)) = status_of(&manager, "srv").await else {
            panic!("a failed restart must leave an Error status");
        };
        assert!(
            message.starts_with("Auto-restart failed:"),
            "the reason replaces the old one instead of being swallowed: {message}"
        );
    }

    #[tokio::test]
    async fn an_operator_stop_during_the_backoff_cancels_the_pending_auto_restart() {
        // bug-433: the dead-server snapshot is taken before the backoff sleep,
        // so the status is re-read afterwards and a deliberate stop wins.
        let manager = manager().await;
        register_server(
            &manager,
            "srv",
            ServerStatus::Error("crashed".into()),
            Some(on_failure(5, 400)),
        )
        .await;

        let worker = {
            let manager = manager.clone();
            tokio::spawn(async move {
                let gate = AtomicBool::new(false);
                check_and_restart_dead_servers(&manager, &gate).await;
            })
        };

        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        set_status(&manager, "srv", ServerStatus::Disconnected).await;
        worker.await.unwrap();

        assert_eq!(
            status_of(&manager, "srv").await,
            Some(ServerStatus::Disconnected),
            "an operator stop during the backoff must not be overwritten"
        );
    }

    #[tokio::test]
    async fn a_server_removed_during_the_backoff_is_not_recreated_by_the_restart() {
        let manager = manager().await;
        register_server(
            &manager,
            "srv",
            ServerStatus::Error("crashed".into()),
            Some(on_failure(5, 400)),
        )
        .await;

        let worker = {
            let manager = manager.clone();
            tokio::spawn(async move {
                let gate = AtomicBool::new(false);
                check_and_restart_dead_servers(&manager, &gate).await;
            })
        };

        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        manager.state.write().await.servers.remove("srv");
        worker.await.unwrap();

        assert!(
            status_of(&manager, "srv").await.is_none(),
            "a deregistered server must not come back"
        );
    }

    #[tokio::test]
    async fn the_restart_budget_stops_further_attempts_once_max_restarts_is_reached() {
        let manager = manager().await;
        register_server(
            &manager,
            "srv",
            ServerStatus::Error("crashed".into()),
            Some(on_failure(2, 1)),
        )
        .await;
        let gate = AtomicBool::new(false);

        // Two attempts are allowed; each one fails and rewrites the status.
        for _ in 0..2 {
            check_and_restart_dead_servers(&manager, &gate).await;
        }
        set_status(
            &manager,
            "srv",
            ServerStatus::Error("budget sentinel".into()),
        )
        .await;

        check_and_restart_dead_servers(&manager, &gate).await;

        assert_eq!(
            status_of(&manager, "srv").await,
            Some(ServerStatus::Error("budget sentinel".into())),
            "the third attempt is refused by the budget, so nothing rewrites the status"
        );
    }

    #[tokio::test]
    async fn an_install_starting_mid_sweep_aborts_the_remaining_restarts() {
        // The flag is re-read inside the loop; here it is already set, so the
        // very first dead server aborts the sweep.
        let manager = manager().await;
        register_server(
            &manager,
            "srv",
            ServerStatus::Error("crashed".into()),
            Some(on_failure(5, 1)),
        )
        .await;
        let gate = AtomicBool::new(true);

        check_and_restart_dead_servers(&manager, &gate).await;

        assert_eq!(
            status_of(&manager, "srv").await,
            Some(ServerStatus::Error("crashed".into())),
            "no restart is attempted while an install is running"
        );
    }

    #[tokio::test]
    async fn the_health_monitor_skips_its_sweep_until_setup_is_done() {
        let manager = manager().await;
        register_server(
            &manager,
            "srv",
            ServerStatus::Error("crashed".into()),
            Some(on_failure(5, 1)),
        )
        .await;
        let shutdown = crate::shutdown::ShutdownSignal::new();
        let setup_in_progress = Arc::new(AtomicBool::new(false));
        let setup_done = Arc::new(AtomicBool::new(false));

        spawn_health_monitor(
            manager.clone(),
            shutdown.clone(),
            1,
            setup_in_progress.clone(),
            setup_done.clone(),
        );

        // The first tick fires immediately and must be gated out.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(
            status_of(&manager, "srv").await,
            Some(ServerStatus::Error("crashed".into())),
            "nothing is restarted while setup is incomplete"
        );

        setup_done.store(true, Ordering::Relaxed);
        let probe = manager.clone();
        assert!(
            wait_until(|| {
                let m = probe.clone();
                async move {
                    matches!(
                        status_of(&m, "srv").await,
                        Some(ServerStatus::Error(ref msg)) if msg.starts_with("Auto-restart failed:")
                    )
                }
            })
            .await,
            "once setup is done the next tick does sweep for dead servers"
        );

        shutdown.raise();
    }

    #[tokio::test]
    async fn the_health_monitor_task_ends_when_shutdown_is_raised() {
        let manager = manager().await;
        let shutdown = crate::shutdown::ShutdownSignal::new();

        spawn_health_monitor(
            manager.clone(),
            shutdown.clone(),
            3600,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(true)),
        );

        // The spawned task owns the second Arc; it releases it only on exit.
        assert!(
            Arc::strong_count(&manager) > 1,
            "the monitor task should be holding the manager"
        );

        // Raised once, on purpose: the monitor is briefly elsewhere (its first
        // `interval.tick()` fires immediately), and a latched signal is still
        // there when it comes back round to check.
        shutdown.raise();
        assert!(
            wait_until(|| {
                let stopped = Arc::strong_count(&manager) == 1;
                async move { stopped }
            })
            .await,
            "the task must drop the manager when it breaks out of its loop"
        );
    }
}
