//! MGP Tier 3 — Lifecycle Management (§11).
//!
//! Provides restart policy evaluation, backoff calculation, and lifecycle
//! event notification for MCP servers.

use super::mcp::McpClientManager;
use super::mcp_protocol::{RestartPolicy, RestartStrategy};
use super::mcp_types::ServerStatus;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{info, warn};

pub(super) struct RestartCounter {
    pub count: u32,
    pub window_start: Instant,
    pub last_restart: Instant,
}

pub(super) struct LifecycleManager {
    restart_counters: Mutex<HashMap<String, RestartCounter>>,
}

impl LifecycleManager {
    pub fn new() -> Self {
        Self {
            restart_counters: Mutex::new(HashMap::new()),
        }
    }

    /// Determine whether a server should be restarted based on its policy and status.
    pub fn should_restart(
        &self,
        server_id: &str,
        policy: &RestartPolicy,
        status: &ServerStatus,
    ) -> bool {
        match policy.strategy {
            RestartStrategy::Never => false,
            RestartStrategy::OnFailure => {
                if !matches!(status, ServerStatus::Error(_)) {
                    return false;
                }
                self.check_restart_budget(server_id, policy)
            }
            RestartStrategy::Always => self.check_restart_budget(server_id, policy),
        }
    }

    /// Check whether the restart budget (max_restarts within restart_window_secs) allows another restart.
    fn check_restart_budget(&self, server_id: &str, policy: &RestartPolicy) -> bool {
        let mut counters = self.restart_counters.lock().unwrap_or_else(|e| {
            warn!("LifecycleManager mutex was poisoned, recovering");
            e.into_inner()
        });
        let now = Instant::now();

        let counter = counters
            .entry(server_id.to_string())
            .or_insert(RestartCounter {
                count: 0,
                window_start: now,
                last_restart: now,
            });

        // Reset window if expired
        let window = Duration::from_secs(policy.restart_window_secs);
        if now.duration_since(counter.window_start) > window {
            counter.count = 0;
            counter.window_start = now;
        }

        counter.count < policy.max_restarts
    }

    /// Record a restart attempt and calculate the backoff duration.
    pub fn calculate_backoff(&self, server_id: &str, policy: &RestartPolicy) -> Duration {
        let mut counters = self.restart_counters.lock().unwrap_or_else(|e| {
            warn!("LifecycleManager mutex was poisoned, recovering");
            e.into_inner()
        });
        let now = Instant::now();

        let counter = counters
            .entry(server_id.to_string())
            .or_insert(RestartCounter {
                count: 0,
                window_start: now,
                last_restart: now,
            });

        counter.count += 1;
        counter.last_restart = now;

        // Exponential backoff: base_ms * 2^(count-1), capped at max_ms
        let exp = (counter.count - 1).min(20); // avoid overflow
        let backoff_ms = policy.backoff_base_ms.saturating_mul(1u64 << exp);
        let capped_ms = backoff_ms.min(policy.backoff_max_ms);
        Duration::from_millis(capped_ms)
    }

    /// Reset restart counter for a server (e.g., after successful connection).
    pub fn reset_counter(&self, server_id: &str) {
        let mut counters = self.restart_counters.lock().unwrap_or_else(|e| {
            warn!("LifecycleManager mutex was poisoned, recovering");
            e.into_inner()
        });
        counters.remove(server_id);
    }
}

/// Send a `notifications/mgp.lifecycle` notification to all servers that negotiated the "lifecycle" extension.
pub(super) async fn emit_lifecycle_notification(
    manager: &McpClientManager,
    server_id: &str,
    from: &str,
    to: &str,
    reason: &str,
) {
    let state = manager.state.read().await;
    for handle in state.servers.values() {
        if !handle.status.is_operational() {
            continue;
        }
        let has_lifecycle = handle
            .mgp_negotiated
            .as_ref()
            .is_some_and(|m| m.active_extensions.iter().any(|e| e == "lifecycle"));
        if !has_lifecycle {
            continue;
        }
        let Some(client) = &handle.client else {
            continue;
        };
        let params = serde_json::json!({
            "server_id": server_id,
            "previous_state": from,
            "new_state": to,
            "reason": reason,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });
        if let Err(e) = client
            .send_notification("notifications/mgp.lifecycle", Some(params))
            .await
        {
            tracing::debug!(
                server = %handle.id,
                error = %e,
                "Failed to send lifecycle notification"
            );
        }
    }
    info!(
        server = %server_id,
        from = %from,
        to = %to,
        reason = %reason,
        "Lifecycle transition"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_policy(strategy: RestartStrategy) -> RestartPolicy {
        RestartPolicy {
            strategy,
            max_restarts: 3,
            restart_window_secs: 300,
            backoff_base_ms: 100,
            backoff_max_ms: 5000,
        }
    }

    #[test]
    fn never_strategy_always_false() {
        let lm = LifecycleManager::new();
        let policy = test_policy(RestartStrategy::Never);
        assert!(!lm.should_restart("s1", &policy, &ServerStatus::Error("fail".into())));
        assert!(!lm.should_restart("s1", &policy, &ServerStatus::Connected));
    }

    #[test]
    fn on_failure_only_on_error() {
        let lm = LifecycleManager::new();
        let policy = test_policy(RestartStrategy::OnFailure);
        assert!(lm.should_restart("s1", &policy, &ServerStatus::Error("fail".into())));
        assert!(!lm.should_restart("s1", &policy, &ServerStatus::Disconnected));
        assert!(!lm.should_restart("s1", &policy, &ServerStatus::Connected));
    }

    #[test]
    fn always_strategy_on_any_status() {
        let lm = LifecycleManager::new();
        let policy = test_policy(RestartStrategy::Always);
        assert!(lm.should_restart("s1", &policy, &ServerStatus::Error("fail".into())));
        assert!(lm.should_restart("s1", &policy, &ServerStatus::Disconnected));
    }

    #[test]
    fn max_restarts_respected() {
        let lm = LifecycleManager::new();
        let policy = test_policy(RestartStrategy::Always);
        // Use up the budget
        for _ in 0..3 {
            assert!(lm.should_restart("s1", &policy, &ServerStatus::Error("fail".into())));
            lm.calculate_backoff("s1", &policy);
        }
        // 4th should be denied
        assert!(!lm.should_restart("s1", &policy, &ServerStatus::Error("fail".into())));
    }

    #[test]
    fn backoff_exponential_capped() {
        let lm = LifecycleManager::new();
        let policy = test_policy(RestartStrategy::Always);

        let d1 = lm.calculate_backoff("s1", &policy);
        assert_eq!(d1, Duration::from_millis(100)); // 100 * 2^0

        let d2 = lm.calculate_backoff("s1", &policy);
        assert_eq!(d2, Duration::from_millis(200)); // 100 * 2^1

        let d3 = lm.calculate_backoff("s1", &policy);
        assert_eq!(d3, Duration::from_millis(400)); // 100 * 2^2

        let d4 = lm.calculate_backoff("s1", &policy);
        assert_eq!(d4, Duration::from_millis(800)); // 100 * 2^3

        let d5 = lm.calculate_backoff("s1", &policy);
        assert_eq!(d5, Duration::from_millis(1600)); // 100 * 2^4

        let d6 = lm.calculate_backoff("s1", &policy);
        assert_eq!(d6, Duration::from_millis(3200)); // 100 * 2^5

        let d7 = lm.calculate_backoff("s1", &policy);
        assert_eq!(d7, Duration::from_millis(5000)); // capped at max
    }

    #[test]
    fn reset_counter_allows_restart_again() {
        let lm = LifecycleManager::new();
        let policy = test_policy(RestartStrategy::Always);
        for _ in 0..3 {
            lm.calculate_backoff("s1", &policy);
        }
        assert!(!lm.should_restart("s1", &policy, &ServerStatus::Error("fail".into())));
        lm.reset_counter("s1");
        assert!(lm.should_restart("s1", &policy, &ServerStatus::Error("fail".into())));
    }
}
