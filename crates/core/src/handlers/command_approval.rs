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
            mcp.get_tool_validator(&call.name).await.as_deref() == Some("sandbox")
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
    mcp_manager: Option<&Arc<McpClientManager>>,
    pending_approvals: &PendingApprovals,
    session_trusted: &SessionTrustedCommands,
    pool: &SqlitePool,
    sender: &tokio::sync::mpsc::Sender<crate::EnvelopedEvent>,
) -> HashSet<String> {
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

    let decision = tokio::time::timeout(std::time::Duration::from_secs(60), arx).await;
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
