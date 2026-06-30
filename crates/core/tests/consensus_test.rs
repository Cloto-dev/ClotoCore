//! Integration tests for the in-kernel consensus orchestration (an earlier decision).
//!
//! The happy path (real engines produce proposals → synthesis → unified
//! answer) depends on live MCP engines and is verified against the running app
//! — there is no Rust `ReasoningEngine` in the codebase to mock (every engine
//! is an MCP server). These tests instead pin the structural guarantees that
//! the redesign must hold regardless of engines:
//!
//!   1. A consensus request always terminates with **exactly one**
//!      `ThoughtResponse`, stamped with the **originating agent id** (bug-417:
//!      previously the synthetic id made the answer invisible to the user's
//!      chat) — never a silent timeout (fail-safe).
//!   2. The terminal answer is **persisted** to the originating agent's chat
//!      history (bug-417: it must survive reload and appear where the user
//!      asked).
//!   3. Per-step `ConsensusProgress` events are emitted for the dashboard's
//!      Consensus tab, and engine failures carry an MGP §14 error code.
//!   4. The consensus path no longer emits the retired `ConsensusRequested` /
//!      `ThoughtRequested` events (the dead event contract is gone).
//!
//! See docs/CONSENSUS_REVIVAL_DESIGN.md.

use cloto_core::handlers::system::SystemHandler;
use cloto_core::managers::mcp_mgp::MGP_ERR_SERVER_NOT_READY;
use cloto_core::managers::{AgentManager, PluginRegistry};
use cloto_shared::{ClotoEventData, ClotoMessage, MessageSource};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::mpsc;

/// The consensus answer is now delivered as the originating agent's own
/// response (so it lands in the chat where the user asked), not under a
/// detached synthetic id.
const ORIGINATING_AGENT_ID: &str = "agent.test";

async fn build_handler(
    engines: Vec<String>,
) -> (
    SystemHandler,
    mpsc::Receiver<cloto_core::EnvelopedEvent>,
    SqlitePool,
) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();

    let agent_id = ORIGINATING_AGENT_ID;
    sqlx::query("INSERT INTO agents (id, name, description, status, default_engine_id, required_capabilities, metadata, enabled) VALUES (?, 'Test Agent', 'Desc', 'online', 'engine.test', '[\"Reasoning\"]', '{}', 1)")
        .bind(agent_id)
        .execute(&pool)
        .await
        .unwrap();

    let registry = Arc::new(PluginRegistry::new(5, 10, 50));
    let agent_manager = AgentManager::new(pool.clone(), 90_000);
    let (event_tx, event_rx) = mpsc::channel(64);
    let metrics = Arc::new(cloto_core::managers::SystemMetrics::new());

    let handler = SystemHandler::new(
        registry,
        agent_manager,
        agent_id.to_string(),
        event_tx,
        10, // memory_context_limit
        metrics,
        engines,
        "consensus:".to_string(),
        16, // max_agentic_iterations
        30, // tool_execution_timeout_secs
        Arc::new(dashmap::DashMap::new()),
        Arc::new(dashmap::DashMap::new()),
        pool.clone(),
        Arc::new(dashmap::DashMap::new()),
        5,     // memory_timeout_secs
        false, // mcp_streaming_enabled
    );

    (handler, event_rx, pool)
}

fn drain(rx: &mut mpsc::Receiver<cloto_core::EnvelopedEvent>) -> Vec<ClotoEventData> {
    let mut out = Vec::new();
    while let Ok(env) = rx.try_recv() {
        out.push(env.event.data.clone());
    }
    out
}

fn consensus_msg() -> ClotoMessage {
    ClotoMessage::new(
        MessageSource::User {
            id: "user1".into(),
            name: "User".into(),
        },
        "consensus: what is the best approach?".into(),
    )
}

/// Assert the common invariants every consensus run must satisfy: exactly one
/// terminal `ThoughtResponse` stamped with the **originating** agent id (so the
/// user's chat unblocks), engine_id "consensus", and no retired
/// `ConsensusRequested` / `ThoughtRequested` events on the bus.
fn assert_single_terminal_response(events: &[ClotoEventData]) -> String {
    let mut responses = events.iter().filter_map(|e| match e {
        ClotoEventData::ThoughtResponse {
            agent_id,
            engine_id,
            content,
            ..
        } => Some((agent_id.clone(), engine_id.clone(), content.clone())),
        _ => None,
    });

    let (agent_id, engine_id, content) = responses
        .next()
        .expect("consensus must emit exactly one ThoughtResponse, got none");
    assert!(
        responses.next().is_none(),
        "consensus must emit exactly one ThoughtResponse, got more than one"
    );
    assert_eq!(
        agent_id, ORIGINATING_AGENT_ID,
        "terminal response must be stamped with the originating agent id (bug-417)"
    );
    assert_eq!(engine_id, "consensus", "engine_id must be 'consensus'");

    assert!(
        !events.iter().any(|e| matches!(
            e,
            ClotoEventData::ConsensusRequested { .. } | ClotoEventData::ThoughtRequested { .. }
        )),
        "the retired ConsensusRequested / ThoughtRequested events must not be emitted"
    );

    content
}

/// The terminal consensus answer/error MUST be persisted to the originating
/// agent's chat history (bug-417) so it appears where the user asked and
/// survives a reload.
async fn assert_response_persisted(pool: &SqlitePool, expected_substring: &str) {
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT content FROM chat_messages WHERE agent_id = ? AND source = ?")
            .bind(ORIGINATING_AGENT_ID)
            .bind("agent")
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "exactly one consensus agent response must be persisted, got {}",
        rows.len()
    );
    assert!(
        rows[0].contains(expected_substring),
        "persisted consensus response should contain {expected_substring:?}, got: {}",
        rows[0]
    );
}

/// Fewer engines configured than `min_proposals` (default 2) → immediate
/// fail-safe response, no engine ever runs, no ConsensusProgress steps.
#[tokio::test]
async fn consensus_too_few_engines_emits_single_failsafe_persisted() {
    let (handler, mut rx, pool) = build_handler(vec!["mind.alpha".to_string()]).await;

    handler.handle_message(consensus_msg()).await.unwrap();

    let events = drain(&mut rx);
    let content = assert_single_terminal_response(&events);
    assert!(
        content.contains("[Consensus unavailable]"),
        "expected the too-few-engines fail-safe message, got: {content}"
    );
    // No engines run before the quorum-precheck early return.
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ClotoEventData::ConsensusProgress { .. })),
        "no ConsensusProgress steps should be emitted when fewer than min engines are configured"
    );
    assert_response_persisted(&pool, "[Consensus unavailable]").await;
}

/// Enough engines configured but none are registered/resolvable → every
/// proposal loop errors → quorum is not reached → fail-safe response. Proves
/// the kernel never hangs, each engine surfaces a ConsensusProgress step with an
/// MGP §14 error code (SERVER_NOT_READY), and the error is delivered to chat.
#[tokio::test]
async fn consensus_all_engines_unavailable_quorum_failsafe_mgp_errors() {
    let (handler, mut rx, pool) =
        build_handler(vec!["mind.alpha".to_string(), "mind.beta".to_string()]).await;

    handler.handle_message(consensus_msg()).await.unwrap();

    let events = drain(&mut rx);
    let content = assert_single_terminal_response(&events);
    assert!(
        content.contains("[Consensus failed]"),
        "expected the quorum-not-reached fail-safe message, got: {content}"
    );

    // Each configured engine gets a pending step and an error step.
    let progress: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ClotoEventData::ConsensusProgress {
                phase,
                engine_id,
                status,
                mgp_error_code,
                ..
            } => Some((
                phase.clone(),
                engine_id.clone(),
                status.clone(),
                *mgp_error_code,
            )),
            _ => None,
        })
        .collect();

    let pending = progress
        .iter()
        .filter(|(_, _, s, _)| s == "pending")
        .count();
    assert_eq!(
        pending, 2,
        "both engines should emit a pending proposal step"
    );

    let errors: Vec<_> = progress
        .iter()
        .filter(|(_, _, s, _)| s == "error")
        .collect();
    assert_eq!(errors.len(), 2, "both engines should fail to resolve");
    for (phase, engine, _, code) in &errors {
        assert_eq!(phase, "proposal");
        assert_eq!(
            *code,
            Some(MGP_ERR_SERVER_NOT_READY),
            "unresolved MGP engine '{engine}' must surface SERVER_NOT_READY"
        );
    }

    assert_response_persisted(&pool, "[Consensus failed]").await;
}
