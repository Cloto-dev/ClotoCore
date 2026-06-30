//! Integration tests for the in-kernel consensus orchestration (Goal #138).
//!
//! The happy path (real engines produce proposals → synthesis → unified
//! answer) depends on live MCP engines and is verified against the running app
//! — there is no Rust `ReasoningEngine` in the codebase to mock (every engine
//! is an MCP server). These tests instead pin the structural guarantees that
//! the redesign must hold regardless of engines:
//!
//!   1. A consensus request always terminates with **exactly one**
//!      `ThoughtResponse`, stamped with the synthetic agent id — never a silent
//!      timeout (fail-safe).
//!   2. The consensus path no longer emits the retired `ConsensusRequested` /
//!      `ThoughtRequested` events (the dead event contract is gone).
//!
//! See docs/CONSENSUS_REVIVAL_DESIGN.md.

use cloto_core::handlers::system::SystemHandler;
use cloto_core::managers::{AgentManager, PluginRegistry};
use cloto_shared::{ClotoEventData, ClotoMessage, MessageSource};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::mpsc;

const SYNTHETIC_AGENT_ID: &str = "system.consensus";

async fn build_handler(
    engines: Vec<String>,
) -> (SystemHandler, mpsc::Receiver<cloto_core::EnvelopedEvent>) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();

    let agent_id = "agent.test";
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
        pool,
        Arc::new(dashmap::DashMap::new()),
        5,     // memory_timeout_secs
        false, // mcp_streaming_enabled
    );

    (handler, event_rx)
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
/// terminal `ThoughtResponse` stamped with the synthetic agent id, and no
/// retired `ConsensusRequested` / `ThoughtRequested` events on the bus.
fn assert_single_synthetic_response(events: &[ClotoEventData]) -> String {
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
        agent_id, SYNTHETIC_AGENT_ID,
        "terminal response must be stamped with the synthetic agent id"
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

/// Fewer engines configured than `min_proposals` (default 2) → immediate
/// fail-safe response, no engine ever runs.
#[tokio::test]
async fn consensus_too_few_engines_emits_single_synthetic_failsafe() {
    let (handler, mut rx) = build_handler(vec!["mind.alpha".to_string()]).await;

    handler.handle_message(consensus_msg()).await.unwrap();

    let events = drain(&mut rx);
    let content = assert_single_synthetic_response(&events);
    assert!(
        content.contains("[Consensus unavailable]"),
        "expected the too-few-engines fail-safe message, got: {content}"
    );
}

/// Enough engines configured but none are registered/resolvable → every
/// proposal loop errors → quorum is not reached → fail-safe response. Proves
/// the kernel never hangs waiting on proposals that will never arrive.
#[tokio::test]
async fn consensus_all_engines_unavailable_quorum_failsafe_no_legacy_events() {
    let (handler, mut rx) =
        build_handler(vec!["mind.alpha".to_string(), "mind.beta".to_string()]).await;

    handler.handle_message(consensus_msg()).await.unwrap();

    let events = drain(&mut rx);
    let content = assert_single_synthetic_response(&events);
    assert!(
        content.contains("[Consensus failed]"),
        "expected the quorum-not-reached fail-safe message, got: {content}"
    );
}
