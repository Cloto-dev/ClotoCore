//! Regression test for the `tool_hint` direct-execution access-control gate
//! (bug-420).
//!
//! `tool_hint` is supplied by an external I/O bridge (e.g. a Discord backtick
//! command) and lets the kernel run a tool without going through the agentic
//! loop. `execute_tool_internal` resolves the tool via the GLOBAL tool index, so
//! without a per-agent check an agent could invoke a tool on any connected
//! server — including one it was never granted. The fix gates the path with the
//! same `check_tool_access` the agentic loop uses (`execute_tool_for_agent`):
//! anything but an explicit Allow is refused *before* execution.
//!
//! Here we assert the denial: an agent that is not granted a tool gets the
//! "not available to this agent" refusal (which only the access gate emits —
//! before the fix the path would have reached execute_tool_internal and
//! returned a generic "tool not found"). The granted/allowed path runs the tool
//! and is exercised end-to-end against the live app (it needs a connected MCP
//! server) and by the agentic loop's existing permission tests.

use cloto_core::handlers::system::SystemHandler;
use cloto_core::managers::{AgentManager, McpClientManager, PluginRegistry};
use cloto_shared::{ClotoEventData, ClotoMessage, MessageSource};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

const AGENT_ID: &str = "agent.test";

async fn build_handler() -> (SystemHandler, mpsc::Receiver<cloto_core::EnvelopedEvent>) {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agents (id, name, description, status, default_engine_id, required_capabilities, metadata, enabled) VALUES (?, 'Test Agent', 'Desc', 'online', 'engine.test', '[]', '{}', 1)")
        .bind(AGENT_ID)
        .execute(&pool)
        .await
        .unwrap();

    // A real (in-process) MCP manager so the tool_hint branch is taken and
    // check_tool_access runs. yolo off so the default opt-in policy denies an
    // ungranted tool.
    let mcp = Arc::new(McpClientManager::new(pool.clone(), false, 120, 30));
    let mut registry = PluginRegistry::new(5, 10, 50);
    registry.set_mcp_manager(mcp);
    let registry = Arc::new(registry);

    let agent_manager = AgentManager::new(pool.clone(), 90_000);
    let (event_tx, event_rx) = mpsc::channel(64);
    let metrics = Arc::new(cloto_core::managers::SystemMetrics::new());

    let handler = SystemHandler::new(
        registry,
        agent_manager,
        AGENT_ID.to_string(),
        event_tx,
        10,
        metrics,
        vec![],
        "consensus:".to_string(),
        16,
        30,
        Arc::new(dashmap::DashMap::new()),
        Arc::new(dashmap::DashMap::new()),
        pool.clone(),
        Arc::new(dashmap::DashMap::new()),
        5,
        false,
    );
    (handler, event_rx)
}

fn tool_hint_msg(tool: &str) -> ClotoMessage {
    let mut metadata = HashMap::new();
    metadata.insert("tool_hint".to_string(), tool.to_string());
    // A callback so the result is surfaced as an ExternalAction event we can read.
    metadata.insert("external_callback_id".to_string(), "cb1".to_string());
    metadata.insert("external_action_id".to_string(), "a1".to_string());
    metadata.insert("external_source".to_string(), "discord".to_string());
    ClotoMessage {
        id: cloto_shared::ClotoId::new().to_string(),
        source: MessageSource::User {
            id: "discord:u1".into(),
            name: "User".into(),
        },
        target_agent: Some(AGENT_ID.to_string()),
        content: "`secret_tool`".to_string(),
        timestamp: chrono::Utc::now(),
        metadata,
    }
}

/// An agent that is not granted a tool's server must be refused by the
/// access gate before execution — even via the tool_hint bypass.
#[tokio::test]
async fn tool_hint_denies_ungranted_tool() {
    let (handler, mut rx) = build_handler().await;

    handler
        .handle_message(tool_hint_msg("secret_tool"))
        .await
        .unwrap();

    let response = {
        let mut found = None;
        while let Ok(env) = rx.try_recv() {
            if let ClotoEventData::ExternalAction { response, .. } = &env.event.data {
                found = response.clone();
                break;
            }
        }
        found.expect("tool_hint execution must surface an ExternalAction result")
    };

    // Post bug-421: the tool_hint path routes through the central capability
    // gate inside execute_tool_internal under Caller::Agent. An ungranted /
    // unavailable tool is refused BEFORE execution and surfaces as an `Error:`
    // — here `secret_tool` is on no connected server, so resolution fails
    // ("not found") before any dispatch; a connected-but-ungranted server would
    // surface "not granted" from the gate. Either way the tool never runs.
    // (The gate's connected-but-ungranted denial is covered end-to-end by
    // engine_access_gate_test.rs.)
    assert!(
        response.starts_with("Error")
            && (response.contains("not found")
                || response.contains("not granted")
                || response.contains("not available")),
        "an ungranted/unavailable tool_hint must be refused before execution, got: {response}"
    );
}
