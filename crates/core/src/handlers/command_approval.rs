//! Command approval types and gate logic for the agentic loop.
//!
//! When an LLM requests tool calls that have a "sandbox" validator (e.g., `execute_command`),
//! the approval gate checks trust status and, if needed, pauses execution to request
//! user approval via the dashboard.

use std::collections::HashSet;
use std::sync::Arc;

use cloto_shared::{ClotoEvent, ClotoEventData, ClotoId, ToolCall};
use dashmap::DashMap;
use sqlx::SqlitePool;
use tokio::sync::oneshot;
use tracing::{info, warn};

use crate::managers::McpClientManager;

/// User's decision on a command approval request.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandApprovalDecision {
    Approve,
    Trust,
    Deny,
}

/// In-memory pending command approval requests (approval_id → oneshot sender).
pub type PendingApprovals = Arc<DashMap<String, oneshot::Sender<CommandApprovalDecision>>>;

/// Session-scoped trusted command names (agent_id → set of command names).
/// Cleared on kernel restart.
pub type SessionTrustedCommands = Arc<DashMap<String, HashSet<String>>>;

/// Emit an event through the event channel (shared helper for approval gate).
async fn emit_event(
    sender: &tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
    trace_id: ClotoId,
    data: ClotoEventData,
) {
    let envelope = crate::EnvelopedEvent {
        event: Arc::new(ClotoEvent::with_trace(trace_id, data)),
        issuer: None,
        correlation_id: Some(trace_id),
        depth: 0,
    };
    if let Err(e) = sender.send(envelope).await {
        warn!("⚠️ Failed to emit approval event: {}", e);
    }
}

/// Extract denied call IDs from a list of untrusted commands.
fn extract_denied_ids(untrusted_cmds: &[serde_json::Value]) -> HashSet<String> {
    untrusted_cmds
        .iter()
        .filter_map(|cmd| cmd.get("call_id").and_then(|v| v.as_str()))
        .map(str::to_string)
        .collect()
}

/// Cron source: auto-approve all tool calls dispatched by the scheduler.
///
/// Cron jobs run unattended — there is no human to press Approve in SecurityGuard
/// or CommandApprovalCard, so the 60s timeout would deny every destructive call.
/// Users accept this tradeoff when they create a cron job (the UI warns about it),
/// and an audit log entry records every bypassed call so the trail is preserved.
async fn handle_cron_approval(
    calls: &[ToolCall],
    agent_id: &str,
    trace_id: ClotoId,
    pool: &SqlitePool,
    sender: &tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
) {
    let tool_names: Vec<&str> = calls.iter().map(|c| c.name.as_str()).collect();
    if tool_names.is_empty() {
        return;
    }

    info!(
        agent_id = %agent_id,
        tools = ?tool_names,
        "⏰ Cron source: HITL approval bypassed"
    );

    let approval_id = uuid::Uuid::new_v4().to_string();
    crate::db::spawn_audit_log(
        pool.clone(),
        crate::db::AuditLogEntry {
            timestamp: chrono::Utc::now(),
            event_type: "CRON_AUTO_APPROVED".to_string(),
            actor_id: Some(agent_id.to_string()),
            target_id: Some(approval_id.clone()),
            permission: None,
            result: "SUCCESS".to_string(),
            reason: format!(
                "Cron scheduler auto-approved {} tool call(s): {:?}",
                tool_names.len(),
                tool_names
            ),
            metadata: None,
            trace_id: Some(trace_id.to_string()),
        },
    );

    emit_event(
        sender,
        trace_id,
        ClotoEventData::CommandApprovalResult {
            approval_id,
            decision: "cron_auto_approved".to_string(),
        },
    )
    .await;
}

/// YOLO mode: auto-approve all sandboxed commands with audit logging.
async fn handle_yolo_approval(
    calls: &[ToolCall],
    agent_id: &str,
    trace_id: ClotoId,
    pool: &SqlitePool,
    sender: &tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
) {
    let sandboxed_tools: Vec<&str> = calls
        .iter()
        .filter_map(|c| c.arguments.get("command").and_then(|v| v.as_str()))
        .collect();
    if sandboxed_tools.is_empty() {
        return;
    }

    info!(
        agent_id = %agent_id,
        commands = ?sandboxed_tools,
        "⚡ YOLO mode: commands auto-approved"
    );

    let approval_id = uuid::Uuid::new_v4().to_string();
    crate::db::spawn_audit_log(
        pool.clone(),
        crate::db::AuditLogEntry {
            timestamp: chrono::Utc::now(),
            event_type: "YOLO_AUTO_APPROVED".to_string(),
            actor_id: Some(agent_id.to_string()),
            target_id: Some(approval_id.clone()),
            permission: None,
            result: "SUCCESS".to_string(),
            reason: format!(
                "YOLO auto-approved {} commands: {:?}",
                sandboxed_tools.len(),
                sandboxed_tools
            ),
            metadata: None,
            trace_id: Some(trace_id.to_string()),
        },
    );

    emit_event(
        sender,
        trace_id,
        ClotoEventData::CommandApprovalResult {
            approval_id,
            decision: "auto_approved".to_string(),
        },
    )
    .await;
}

/// Collect untrusted commands that need approval (not in DB or session trust).
async fn collect_untrusted_commands(
    calls: &[ToolCall],
    agent_id: &str,
    mcp_manager: Option<&Arc<McpClientManager>>,
    session_trusted: &SessionTrustedCommands,
    pool: &SqlitePool,
) -> Vec<serde_json::Value> {
    let mut untrusted_cmds: Vec<serde_json::Value> = Vec::new();
    for call in calls {
        let has_sandbox_validator = if let Some(mcp) = mcp_manager {
            mcp.get_tool_validator(&call.name).as_deref() == Some("sandbox")
        } else {
            false
        };

        // L12: Check MGP risk level first, fall back to MCP annotations
        if !has_sandbox_validator {
            let risk_level = if let Some(mcp) = mcp_manager {
                mcp.get_tool_risk_level(&call.name).await
            } else {
                None
            };

            let needs_approval = match risk_level {
                // MGP negotiated: use kernel-derived risk level
                Some(crate::managers::mcp_mgp::RiskLevel::Safe) => false,
                Some(_) => true, // Moderate or Dangerous
                // No MGP: fall back to MCP annotations (default destructive per spec)
                None => {
                    if let Some(mcp) = mcp_manager {
                        mcp.is_tool_destructive(&call.name).await
                    } else {
                        false
                    }
                }
            };

            if needs_approval {
                let session_is_trusted = session_trusted
                    .get(agent_id)
                    .is_some_and(|set| set.contains(call.name.as_str()));
                if !session_is_trusted {
                    untrusted_cmds.push(serde_json::json!({
                        "call_id": call.id,
                        "command": format!("[destructive] {}", call.name),
                        "command_name": call.name,
                    }));
                }
            }
            continue;
        }

        let Some(cmd_str) = call.arguments.get("command").and_then(|v| v.as_str()) else {
            continue;
        };
        let db_trusted = crate::db::is_command_trusted(pool, agent_id, cmd_str)
            .await
            .unwrap_or(false);
        let cmd_name = cmd_str.split_whitespace().next().unwrap_or(cmd_str);
        let session_is_trusted = session_trusted
            .get(agent_id)
            .is_some_and(|set| set.contains(cmd_name));
        if !db_trusted && !session_is_trusted {
            untrusted_cmds.push(serde_json::json!({
                "call_id": call.id,
                "command": cmd_str,
                "command_name": cmd_name,
            }));
        }
    }
    untrusted_cmds
}

/// Process the user's approval decision and return denied call IDs.
async fn process_approval_decision(
    decision: Result<
        Result<CommandApprovalDecision, oneshot::error::RecvError>,
        tokio::time::error::Elapsed,
    >,
    approval_id: &str,
    agent_id: &str,
    untrusted_cmds: &[serde_json::Value],
    session_trusted: &SessionTrustedCommands,
    pool: &SqlitePool,
    trace_id: ClotoId,
    sender: &tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
) -> HashSet<String> {
    match decision {
        Ok(Ok(CommandApprovalDecision::Approve)) => {
            for cmd in untrusted_cmds {
                if let Some(c) = cmd.get("command").and_then(|v| v.as_str()) {
                    let _ = crate::db::add_trusted_command(pool, agent_id, c).await;
                }
            }
            info!(approval_id = %approval_id, "✅ Commands approved (exact)");
            emit_event(
                sender,
                trace_id,
                ClotoEventData::CommandApprovalResult {
                    approval_id: approval_id.to_string(),
                    decision: "approved".to_string(),
                },
            )
            .await;
            HashSet::new()
        }
        Ok(Ok(CommandApprovalDecision::Trust)) => {
            for cmd in untrusted_cmds {
                if let Some(n) = cmd.get("command_name").and_then(|v| v.as_str()) {
                    session_trusted
                        .entry(agent_id.to_string())
                        .or_default()
                        .insert(n.to_string());
                }
            }
            info!(approval_id = %approval_id, "✅ Command names trusted (session)");
            emit_event(
                sender,
                trace_id,
                ClotoEventData::CommandApprovalResult {
                    approval_id: approval_id.to_string(),
                    decision: "trusted".to_string(),
                },
            )
            .await;
            HashSet::new()
        }
        Ok(Ok(CommandApprovalDecision::Deny)) => {
            warn!(approval_id = %approval_id, "🚫 Commands denied by user");
            emit_event(
                sender,
                trace_id,
                ClotoEventData::CommandApprovalResult {
                    approval_id: approval_id.to_string(),
                    decision: "denied by user".to_string(),
                },
            )
            .await;
            extract_denied_ids(untrusted_cmds)
        }
        Ok(Err(_)) | Err(_) => {
            let reason = if decision.is_err() {
                "timeout (60s)"
            } else {
                "channel closed"
            };
            warn!(approval_id = %approval_id, reason = reason, "🚫 Commands denied (no response)");
            info!(
                approval_id = %approval_id,
                agent_id = %agent_id,
                commands = ?untrusted_cmds,
                reason = reason,
                "📋 Approval gate audit: commands blocked due to {}", reason
            );
            emit_event(
                sender,
                trace_id,
                ClotoEventData::CommandApprovalResult {
                    approval_id: approval_id.to_string(),
                    decision: reason.to_string(),
                },
            )
            .await;
            extract_denied_ids(untrusted_cmds)
        }
    }
}

/// Run the command approval gate for a batch of tool calls.
///
/// Returns a set of call IDs that were denied (should be skipped during execution).
/// Approved/trusted calls are NOT in the returned set.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_approval_gate(
    calls: &[ToolCall],
    agent_id: &str,
    trace_id: ClotoId,
    yolo_mode: bool,
    cron_source: bool,
    mcp_manager: Option<&Arc<McpClientManager>>,
    pending_approvals: &PendingApprovals,
    session_trusted: &SessionTrustedCommands,
    pool: &SqlitePool,
    sender: &tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
) -> HashSet<String> {
    if cron_source {
        handle_cron_approval(calls, agent_id, trace_id, pool, sender).await;
        return HashSet::new();
    }

    if yolo_mode {
        handle_yolo_approval(calls, agent_id, trace_id, pool, sender).await;
        return HashSet::new();
    }

    let untrusted_cmds =
        collect_untrusted_commands(calls, agent_id, mcp_manager, session_trusted, pool).await;

    if untrusted_cmds.is_empty() {
        return HashSet::new();
    }

    let approval_id = uuid::Uuid::new_v4().to_string();
    info!(agent_id = %agent_id, count = untrusted_cmds.len(), "🔒 Commands require approval");

    let (atx, arx) = oneshot::channel();
    pending_approvals.insert(approval_id.clone(), atx);

    emit_event(
        sender,
        trace_id,
        ClotoEventData::CommandApprovalRequested {
            approval_id: approval_id.clone(),
            agent_id: agent_id.to_string(),
            commands: untrusted_cmds.clone(),
        },
    )
    .await;

    let decision = tokio::time::timeout(std::time::Duration::from_mins(1), arx).await;
    pending_approvals.remove(&approval_id);

    process_approval_decision(
        decision,
        &approval_id,
        agent_id,
        &untrusted_cmds,
        session_trusted,
        pool,
        trace_id,
        sender,
    )
    .await
}

#[cfg(test)]
mod tests {
    //! Characterization tests for the command approval gate.
    //!
    //! `execute_command` is the one tool the kernel statically maps to the
    //! "sandbox" validator, so it is the shortest route into the real gate.
    //! Decisions are delivered through `pending_approvals` exactly as the
    //! dashboard's approval endpoint does.

    use super::*;
    use cloto_shared::ClotoEventData;
    use sqlx::SqlitePool;

    const AGENT: &str = "agent.test";

    struct Harness {
        pool: SqlitePool,
        mcp: Arc<McpClientManager>,
        pending: PendingApprovals,
        trusted: SessionTrustedCommands,
        tx: tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
        rx: tokio::sync::mpsc::Receiver<crate::EnvelopedEvent>,
    }

    async fn harness() -> Harness {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", None)
            .await
            .unwrap();
        let mcp = Arc::new(McpClientManager::new(pool.clone(), false, 120, 30));
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        Harness {
            pool,
            mcp,
            pending: Arc::new(DashMap::new()),
            trusted: Arc::new(DashMap::new()),
            tx,
            rx,
        }
    }

    impl Harness {
        /// Run the gate inline. Only safe when the call cannot block on an
        /// operator decision (trusted commands, YOLO, cron, paused time).
        async fn gate(
            &self,
            calls: &[ToolCall],
            yolo_mode: bool,
            cron_source: bool,
        ) -> HashSet<String> {
            run_approval_gate(
                calls,
                AGENT,
                ClotoId::new(),
                yolo_mode,
                cron_source,
                Some(&self.mcp),
                &self.pending,
                &self.trusted,
                &self.pool,
                &self.tx,
            )
            .await
        }

        /// Run the gate on its own task so the test can answer its request.
        fn spawn_gate(
            &self,
            agent: &str,
            calls: Vec<ToolCall>,
        ) -> tokio::task::JoinHandle<HashSet<String>> {
            let pool = self.pool.clone();
            let mcp = self.mcp.clone();
            let pending = self.pending.clone();
            let trusted = self.trusted.clone();
            let tx = self.tx.clone();
            let agent = agent.to_string();
            tokio::spawn(async move {
                run_approval_gate(
                    &calls,
                    &agent,
                    ClotoId::new(),
                    false,
                    false,
                    Some(&mcp),
                    &pending,
                    &trusted,
                    &pool,
                    &tx,
                )
                .await
            })
        }

        /// Block until the gate emits its request, returning the approval id.
        async fn next_request(&mut self) -> String {
            let envelope = tokio::time::timeout(std::time::Duration::from_secs(5), self.rx.recv())
                .await
                .expect("the gate must emit an approval request")
                .expect("channel open");
            match &envelope.event.data {
                ClotoEventData::CommandApprovalRequested { approval_id, .. } => approval_id.clone(),
                other => panic!("expected CommandApprovalRequested, got {other:?}"),
            }
        }

        /// Answer the gate's outstanding request.
        async fn answer(&mut self, decision: CommandApprovalDecision) -> String {
            let approval_id = self.next_request().await;
            let (_, sender) = self
                .pending
                .remove(&approval_id)
                .expect("the gate registers its oneshot before emitting the request");
            sender.send(decision).expect("the gate is still listening");
            approval_id
        }

        fn drain(&mut self) -> Vec<ClotoEventData> {
            let mut out = Vec::new();
            while let Ok(envelope) = self.rx.try_recv() {
                out.push(envelope.event.data.clone());
            }
            out
        }
    }

    fn shell_call(id: &str, command: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            name: "execute_command".to_string(),
            arguments: serde_json::json!({ "command": command }),
        }
    }

    fn was_requested(events: &[ClotoEventData]) -> bool {
        events
            .iter()
            .any(|e| matches!(e, ClotoEventData::CommandApprovalRequested { .. }))
    }

    fn decisions(events: &[ClotoEventData]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                ClotoEventData::CommandApprovalResult { decision, .. } => Some(decision.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn denying_an_untrusted_command_returns_its_call_id_as_blocked() {
        let mut h = harness().await;
        let gate = h.spawn_gate(AGENT, vec![shell_call("call-1", "rm -rf /tmp/x")]);

        let approval_id = h.answer(CommandApprovalDecision::Deny).await;
        let denied = gate.await.unwrap();

        assert_eq!(
            denied,
            HashSet::from(["call-1".to_string()]),
            "a denied call is reported so the executor skips it"
        );
        assert_eq!(decisions(&h.drain()), vec!["denied by user".to_string()]);
        assert!(
            !h.pending.contains_key(&approval_id),
            "the pending entry is cleaned up after the decision"
        );
    }

    #[tokio::test]
    async fn approving_a_command_trusts_that_exact_string_so_the_next_run_is_not_gated() {
        let mut h = harness().await;

        let gate = h.spawn_gate(AGENT, vec![shell_call("call-1", "ls -la /tmp")]);
        h.answer(CommandApprovalDecision::Approve).await;
        assert!(gate.await.unwrap().is_empty());
        h.drain();

        // The consequence: the identical command no longer asks.
        assert!(h
            .gate(&[shell_call("call-2", "ls -la /tmp")], false, false)
            .await
            .is_empty());
        assert!(
            !was_requested(&h.drain()),
            "an approved command must not be re-requested"
        );

        // Approve is exact-string trust, so a different argument asks again.
        let second = h.spawn_gate(AGENT, vec![shell_call("call-3", "ls -la /etc")]);
        h.answer(CommandApprovalDecision::Deny).await;
        assert_eq!(second.await.unwrap(), HashSet::from(["call-3".to_string()]));
    }

    #[tokio::test]
    async fn trusting_a_command_trusts_only_its_first_word_for_the_rest_of_the_session() {
        // Quirk with teeth: Trust keys on `command_name`, the first
        // whitespace-separated token. Trusting `rm -rf /tmp/x` therefore lets
        // every later `rm ...` through without asking.
        let mut h = harness().await;

        let gate = h.spawn_gate(AGENT, vec![shell_call("call-1", "rm -rf /tmp/x")]);
        h.answer(CommandApprovalDecision::Trust).await;
        assert!(gate.await.unwrap().is_empty());
        h.drain();

        assert_eq!(
            h.trusted.get(AGENT).map(|set| set.contains("rm")),
            Some(true),
            "only the binary name is remembered"
        );
        assert!(
            h.gate(&[shell_call("call-2", "rm -rf /var/else")], false, false)
                .await
                .is_empty(),
            "a different argument list to the same binary is no longer gated"
        );
        assert!(!was_requested(&h.drain()));

        // Session trust is per agent — another agent is still asked.
        let other = h.spawn_gate("agent.other", vec![shell_call("call-3", "rm -rf /tmp/x")]);
        h.answer(CommandApprovalDecision::Deny).await;
        assert_eq!(other.await.unwrap(), HashSet::from(["call-3".to_string()]));
    }

    #[tokio::test]
    async fn the_gate_denies_everything_it_asked_about_when_the_timeout_expires() {
        let mut h = harness().await;
        let gate = h.spawn_gate(
            AGENT,
            vec![shell_call("call-1", "curl http://host.invalid")],
        );

        // Wait for the request on the real clock (the gate hits SQLite before
        // it starts waiting, and a paused clock trips sqlx's acquire timeout),
        // then freeze time so the runtime fast-forwards to the 60s deadline
        // instead of the test sleeping for a minute.
        h.next_request().await;
        tokio::time::pause();

        let denied = gate.await.unwrap();

        assert_eq!(denied, HashSet::from(["call-1".to_string()]));
        assert_eq!(
            decisions(&h.drain()),
            vec!["timeout (60s)".to_string()],
            "the emitted decision names the timeout"
        );
        assert!(h.pending.is_empty(), "the pending entry is not leaked");
    }

    #[tokio::test]
    async fn the_gate_denies_when_the_approval_channel_is_dropped_without_a_decision() {
        let mut h = harness().await;
        let gate = h.spawn_gate(AGENT, vec![shell_call("call-1", "cat /etc/passwd")]);

        let approval_id = h.next_request().await;
        // Drop the responder without answering (what a dashboard reload does).
        drop(h.pending.remove(&approval_id));

        assert_eq!(gate.await.unwrap(), HashSet::from(["call-1".to_string()]));
        assert_eq!(decisions(&h.drain()), vec!["channel closed".to_string()]);
    }

    #[tokio::test]
    async fn yolo_mode_approves_sandboxed_commands_without_asking_and_records_the_bypass() {
        let mut h = harness().await;

        let denied = h
            .gate(&[shell_call("call-1", "rm -rf /tmp/x")], true, false)
            .await;

        assert!(denied.is_empty(), "YOLO denies nothing");
        let events = h.drain();
        assert!(!was_requested(&events), "YOLO must not ask the operator");
        assert_eq!(decisions(&events), vec!["auto_approved".to_string()]);
        assert!(
            h.trusted.is_empty(),
            "YOLO is a per-run bypass, not a trust grant"
        );
    }

    #[tokio::test]
    async fn yolo_mode_stays_silent_when_no_call_carries_a_command_argument() {
        // Quirk: `handle_yolo_approval` looks only at `arguments.command`, so a
        // destructive tool without one is approved with no event and no audit
        // row at all — that call's bypass leaves no trace.
        let mut h = harness().await;

        let denied = h
            .gate(
                &[ToolCall {
                    id: "call-1".into(),
                    name: "delete_everything".into(),
                    arguments: serde_json::json!({ "scope": "all" }),
                }],
                true,
                false,
            )
            .await;

        assert!(denied.is_empty());
        assert!(
            h.drain().is_empty(),
            "neither a request nor a result event is emitted"
        );
    }

    #[tokio::test]
    async fn a_cron_dispatched_call_bypasses_the_gate_ahead_of_the_yolo_check() {
        let mut h = harness().await;

        let denied = h
            .gate(&[shell_call("call-1", "rm -rf /tmp/x")], false, true)
            .await;

        assert!(denied.is_empty());
        let events = h.drain();
        assert!(!was_requested(&events));
        assert_eq!(
            decisions(&events),
            vec!["cron_auto_approved".to_string()],
            "the cron branch runs before the YOLO branch and labels itself"
        );
    }

    #[tokio::test]
    async fn a_command_already_trusted_in_the_database_never_reaches_the_gate() {
        let mut h = harness().await;
        crate::db::add_trusted_command(&h.pool, AGENT, "git status")
            .await
            .unwrap();

        assert!(h
            .gate(&[shell_call("call-1", "git status")], false, false)
            .await
            .is_empty());
        assert!(h.drain().is_empty(), "no operator interaction at all");

        // DB trust is scoped to the agent that owns it.
        let other = h.spawn_gate("agent.other", vec![shell_call("call-2", "git status")]);
        h.answer(CommandApprovalDecision::Deny).await;
        assert_eq!(other.await.unwrap(), HashSet::from(["call-2".to_string()]));
    }

    #[tokio::test]
    async fn without_an_mcp_manager_nothing_is_ever_considered_to_need_approval() {
        // Worth pinning before a headless rewrite: the gate's entire risk
        // classification lives behind `mcp_manager`. With `None` — the shape a
        // stripped-down embedding would naturally produce — even
        // `execute_command` sails through ungated.
        let h = harness().await;

        let denied = run_approval_gate(
            &[shell_call("call-1", "rm -rf /")],
            AGENT,
            ClotoId::new(),
            false,
            false,
            None,
            &h.pending,
            &h.trusted,
            &h.pool,
            &h.tx,
        )
        .await;

        assert!(denied.is_empty());
    }
}
