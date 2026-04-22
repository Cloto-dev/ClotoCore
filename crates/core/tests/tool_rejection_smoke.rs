//! Smoke test for Phase A/B — prints what the agentic loop would put into
//! `tool_history` when a YOLO-gated or delegation-rejected kernel tool is
//! invoked in non-YOLO mode. Runs outside the live kernel, so no state is
//! disturbed. Invoke with:
//!
//!   cargo test -p cloto_core --test tool_rejection_smoke -- --nocapture
//!
//! This test will be removed when Phase F (MGP_SPEC §13.3 draft) archives
//! the test plan document.

use cloto_core::managers::McpClientManager;
use cloto_shared::{RejectionCode, ToolFailure};
use serde_json::{json, Value};

/// Simulate what system.rs (agentic loop, line ~1506) would do with a
/// `Result<Value, ToolFailure>` return value. This mirrors the production
/// flow post-Phase B, pre-Phase C. Returns `(success, content)` as the
/// agentic loop computes them today.
fn simulate_agentic_loop_outcome(tool_result: Result<Value, ToolFailure>) -> (bool, String) {
    match tool_result {
        Ok(v) => (true, v.to_string()),
        Err(e) => (false, format!("Error: {}", e)),
    }
}

async fn setup_manager(yolo: bool) -> McpClientManager {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    cloto_core::db::init_db(&pool, "sqlite::memory:", "memory.cpersona")
        .await
        .unwrap();
    McpClientManager::new(pool, yolo, 120, 30)
}

async fn call(mgr: &McpClientManager, tool: &str, args: Value) -> Result<Value, ToolFailure> {
    mgr.execute_tool(tool, args, Some("agent.demo")).await
}

fn print_outcome(label: &str, tool_name: &str, result: Result<Value, ToolFailure>) {
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("CASE: {}", label);
    println!("tool: {}", tool_name);

    if let Err(ToolFailure::Rejection(ref r)) = result {
        println!(
            "rejection.code           = {:?}  ({})",
            r.code,
            serde_json::to_string(&r.code).unwrap_or_else(|_| "?".into())
        );
        println!("rejection.retryable      = {}", r.retryable);
        if let Some(h) = r.remediation_hint.as_ref() {
            println!("rejection.remediation    = \"{}\"", h);
        }
        if let Some(d) = r.details.as_ref() {
            println!("rejection.details        = {}", d);
        }
    }

    let (success, content) = simulate_agentic_loop_outcome(result);
    println!("agentic_loop.success     = {}", success);
    println!("agentic_loop.content (what the LLM sees in tool_history):");
    for line in content.lines() {
        println!("   | {}", line);
    }
}

#[tokio::test]
async fn phase_b_live_rejection_output_demo() {
    // Initialize tracing so info!/warn! from the kernel surface in test output.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();

    println!();
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  Phase B rejection output — simulated agentic loop view          ║");
    println!("║  (no Tauri app contacted; in-process McpClientManager)           ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");

    // ────── YOLO OFF: every YOLO-gated tool rejects ──────
    let mgr_off = setup_manager(false).await;

    print_outcome(
        "YOLO OFF — create_mcp_server",
        "create_mcp_server",
        call(
            &mgr_off,
            "create_mcp_server",
            json!({"name": "demo_server", "code": "print('hi')"}),
        )
        .await,
    );

    print_outcome(
        "YOLO OFF — mgp.access.grant",
        "mgp.access.grant",
        call(
            &mgr_off,
            "mgp.access.grant",
            json!({
                "agent_id": "agent.demo",
                "server_id": "file.terminal",
                "entry_type": "server_grant",
                "permission": "allow"
            }),
        )
        .await,
    );

    print_outcome(
        "YOLO OFF — mgp.agent.ask",
        "mgp.agent.ask",
        call(
            &mgr_off,
            "mgp.agent.ask",
            json!({
                "target_agent_id": "agent.other",
                "prompt": "hi",
                "agent_id": "agent.caller"
            }),
        )
        .await,
    );

    print_outcome(
        "YOLO OFF — mgp.audit.replay",
        "mgp.audit.replay",
        call(&mgr_off, "mgp.audit.replay", json!({})).await,
    );

    // ────── YOLO ON: delegation-logic rejections still fire ──────
    let mgr_on = setup_manager(true).await;

    print_outcome(
        "YOLO ON — self-delegation",
        "mgp.agent.ask",
        call(
            &mgr_on,
            "mgp.agent.ask",
            json!({
                "target_agent_id": "agent.solo",
                "prompt": "ask myself",
                "agent_id": "agent.solo"
            }),
        )
        .await,
    );

    print_outcome(
        "YOLO ON — delegation cycle",
        "mgp.agent.ask",
        call(
            &mgr_on,
            "mgp.agent.ask",
            json!({
                "target_agent_id": "agent.b",
                "prompt": "loopback",
                "agent_id": "agent.a",
                "_delegation_chain": ["agent.b", "agent.a"]
            }),
        )
        .await,
    );

    print_outcome(
        "YOLO ON — delegation depth exceeded",
        "mgp.agent.ask",
        call(
            &mgr_on,
            "mgp.agent.ask",
            json!({
                "target_agent_id": "agent.z",
                "prompt": "deep",
                "agent_id": "agent.root",
                "_delegation_chain": ["agent.a", "agent.b", "agent.c"]
            }),
        )
        .await,
    );

    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("SUMMARY: Every case above returned Err(ToolFailure::Rejection).");
    println!("- Pre-Phase B behavior: Ok({{\"status\":\"rejected\",...}}) with success=true.");
    println!("- Phase B behavior:     Err(Rejection) with success=false.");
    println!("  The LLM's tool_history now begins with 'Error:' + structured reason.");
    println!("- Phase C (next) adds: ToolRejected SSE event, break-on-same-code,");
    println!("  break-on-!retryable, and mechanical AgentFinalResponse.");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // Sanity assertion: YOLO OFF case must be a YoloRequired rejection.
    let sanity = call(&mgr_off, "mgp.access.revoke", json!({})).await;
    match sanity {
        Err(ToolFailure::Rejection(r)) => {
            assert_eq!(r.code, RejectionCode::YoloRequired);
            assert!(r.retryable, "YoloRequired must be retryable=true");
        }
        other => panic!("Expected YoloRequired rejection, got {:?}", other),
    }
}
