//! Capability gate (bug-421) — unit + behavioural coverage of the single
//! per-agent MCP access gate and its data layer.
//!
//! The permission *data model* is exercised directly via
//! `resolve_tool_access` / `resolve_explicit_permission` (no in-process kernel),
//! and the PATH 1 gate (`call_server_tool` → `resolve_tool_call_target`
//! → `enforce_caller_grant`) plus the kernel-native Deny-only RBAC are exercised
//! through the public `McpClientManager` surface against an in-memory DB.
//!
//! The headline guarantee — a zero-grant agent's engine is denied (the LM Studio
//! repro) — is the same code path: the engine `think` tool routes through
//! `call_server_tool` under `Caller::Agent`, so `path1_gate_denies_ungranted_server`
//! covers it (an engine is just an MCP server gated by `server_id`).

use cloto_core::db::mcp::PermissionLevel;
use cloto_core::managers::{Caller, McpClientManager};
use sqlx::SqlitePool;

async fn fresh_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    // Runs all migrations incl. 20260701000000 (opt-in flip + engine backfill);
    // on an empty DB both are no-ops, which also smoke-tests that the new
    // migration applies cleanly (no FATAL).
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();
    pool
}

async fn add_server(pool: &SqlitePool, name: &str, default_policy: &str) {
    sqlx::query("INSERT INTO mcp_servers (name, command, created_at, default_policy) VALUES (?, 'noop', 0, ?)")
        .bind(name)
        .bind(default_policy)
        .execute(pool)
        .await
        .unwrap();
}

async fn add_grant(
    pool: &SqlitePool,
    entry_type: &str,
    agent_id: &str,
    server_id: &str,
    tool_name: Option<&str>,
    permission: &str,
) {
    sqlx::query(
        "INSERT INTO mcp_access_control (entry_type, agent_id, server_id, tool_name, permission, granted_at) \
         VALUES (?, ?, ?, ?, ?, 't0')",
    )
    .bind(entry_type)
    .bind(agent_id)
    .bind(server_id)
    .bind(tool_name)
    .bind(permission)
    .execute(pool)
    .await
    .unwrap();
}

// ─────────────────────────── data layer ───────────────────────────

#[tokio::test]
async fn resolve_tool_access_opt_in_truth_table() {
    let pool = fresh_pool().await;
    add_server(&pool, "tool.optin", "opt-in").await;
    add_server(&pool, "tool.optout", "opt-out").await;

    // No grant + opt-in default → Deny (the post-flip default; revoke-staleness fix).
    assert_eq!(
        cloto_core::db::resolve_tool_access(&pool, "a", "tool.optin", "do")
            .await
            .unwrap(),
        PermissionLevel::Deny
    );
    // No grant + opt-out default → Allow.
    assert_eq!(
        cloto_core::db::resolve_tool_access(&pool, "a", "tool.optout", "do")
            .await
            .unwrap(),
        PermissionLevel::Allow
    );
    // Unknown server (no row) → "opt-in" fallback → Deny.
    assert_eq!(
        cloto_core::db::resolve_tool_access(&pool, "a", "ghost.server", "do")
            .await
            .unwrap(),
        PermissionLevel::Deny
    );

    // server_grant allow → Allow; deny → Deny.
    add_grant(&pool, "server_grant", "a", "tool.optin", None, "allow").await;
    assert_eq!(
        cloto_core::db::resolve_tool_access(&pool, "a", "tool.optin", "do")
            .await
            .unwrap(),
        PermissionLevel::Allow
    );
    add_grant(&pool, "server_grant", "b", "tool.optout", None, "deny").await;
    assert_eq!(
        cloto_core::db::resolve_tool_access(&pool, "b", "tool.optout", "do")
            .await
            .unwrap(),
        PermissionLevel::Deny
    );

    // tool_grant takes precedence over server_grant (allow beats a server deny).
    add_grant(&pool, "tool_grant", "b", "tool.optout", Some("do"), "allow").await;
    assert_eq!(
        cloto_core::db::resolve_tool_access(&pool, "b", "tool.optout", "do")
            .await
            .unwrap(),
        PermissionLevel::Allow
    );
}

#[tokio::test]
async fn resolve_explicit_permission_ignores_default_policy() {
    let pool = fresh_pool().await;
    add_server(&pool, "tool.optout", "opt-out").await;

    // No explicit entry → None, even though default_policy=opt-out would Allow.
    // This is what makes the kernel Deny-only RBAC default to Allow without
    // inheriting a server's opt-in default as a deny.
    assert_eq!(
        cloto_core::db::resolve_explicit_permission(&pool, "a", "tool.optout", "do")
            .await
            .unwrap(),
        None
    );

    // Explicit deny is surfaced.
    add_grant(&pool, "server_grant", "a", "tool.optout", None, "deny").await;
    assert_eq!(
        cloto_core::db::resolve_explicit_permission(&pool, "a", "tool.optout", "do")
            .await
            .unwrap(),
        Some(PermissionLevel::Deny)
    );
}

// ─────────────────────────── PATH 1 gate ───────────────────────────

fn err_text<T: std::fmt::Debug>(r: &anyhow::Result<T>) -> String {
    match r {
        Ok(v) => panic!("expected Err, got Ok({v:?})"),
        Err(e) => e.to_string(),
    }
}

#[tokio::test]
async fn path1_gate_denies_ungranted_server() {
    let pool = fresh_pool().await;
    let mgr = McpClientManager::new(pool, false, 120, 30);

    // Ungranted engine server (a `mind.*` server is just an MCP server). The
    // gate denies BEFORE the client lookup — proving an agent with no grant
    // cannot reach its engine (the LM Studio repro).
    let denied = mgr
        .call_server_tool(
            &Caller::Agent("agent.zero".to_string()),
            "mind.x",
            "think",
            serde_json::json!({}),
        )
        .await;
    let msg = err_text(&denied);
    assert!(
        msg.contains("not granted") || msg.contains("Access denied"),
        "ungranted engine must be denied by the gate, got: {msg}"
    );
    assert!(
        !msg.contains("not found") && !msg.contains("not connected"),
        "gate must deny BEFORE the client lookup, got: {msg}"
    );
}

#[tokio::test]
async fn path1_gate_allows_granted_then_fails_at_connection() {
    let pool = fresh_pool().await;
    add_server(&pool, "mind.x", "opt-in").await;
    add_grant(&pool, "server_grant", "agent.ok", "mind.x", None, "allow").await;
    let mgr = McpClientManager::new(pool, false, 120, 30);

    // With the engine granted, the gate passes; execution then fails only
    // because the server is not connected — proving Allow != Deny and that the
    // refusal in the ungranted case came from the gate, not the connection.
    let r = mgr
        .call_server_tool(
            &Caller::Agent("agent.ok".to_string()),
            "mind.x",
            "think",
            serde_json::json!({}),
        )
        .await;
    let msg = err_text(&r);
    assert!(
        msg.contains("not found") || msg.contains("not connected"),
        "granted engine must pass the gate and fail only at connection, got: {msg}"
    );
    assert!(
        !msg.contains("not granted"),
        "granted engine must not be denied by the gate, got: {msg}"
    );
}

#[tokio::test]
async fn path1_gate_system_bypasses() {
    let pool = fresh_pool().await;
    let mgr = McpClientManager::new(pool, false, 120, 30);

    // System bypasses the grant gate entirely: an ungranted server fails only
    // at the client lookup, never at the gate.
    let r = mgr
        .call_server_tool(&Caller::System, "mind.x", "think", serde_json::json!({}))
        .await;
    let msg = err_text(&r);
    assert!(
        msg.contains("not found") || msg.contains("not connected"),
        "System must bypass the gate, got: {msg}"
    );
    assert!(
        !msg.contains("not granted"),
        "System must not be gated, got: {msg}"
    );
}

// ───────────── kernel presentation ⇄ enforcement parity ─────────────

fn kernel_tool_count(schemas: &[serde_json::Value]) -> usize {
    schemas
        .iter()
        .filter(|s| {
            s.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .is_some_and(|n| n.starts_with("mgp."))
        })
        .count()
}

#[tokio::test]
async fn kernel_presentation_defaults_to_allow_and_honors_explicit_deny() {
    let pool = fresh_pool().await;
    add_server(&pool, "kernel", "opt-in").await;
    // yolo on: collect_tool_schemas_for_agent offers kernel-native (mgp.*) tools.
    let mgr = McpClientManager::new(pool.clone(), true, 120, 30);

    // Gap-1 fix: the presentation filter must use resolve_explicit_permission
    // (default Allow), matching enforce_kernel_rbac — NOT resolve_tool_access
    // (which would fall back to the 'kernel' opt-in row → Deny and hide every
    // kernel tool). An agent with no explicit kernel grant sees the kernel tools.
    let free = mgr.collect_tool_schemas_for_agent("agent.free").await;
    let free_kernel = kernel_tool_count(&free);
    assert!(
        free_kernel > 0,
        "default-Allow: an agent with no explicit kernel deny must be offered kernel tools, got {free_kernel}"
    );

    // An explicit deny removes exactly that tool; the rest remain (Deny-only).
    add_grant(
        &pool,
        "tool_grant",
        "agent.blocked",
        "kernel",
        Some("mgp.audit.replay"),
        "deny",
    )
    .await;
    let blocked = mgr.collect_tool_schemas_for_agent("agent.blocked").await;
    let blocked_has_denied = blocked.iter().any(|s| {
        s.get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
            == Some("mgp.audit.replay")
    });
    assert!(
        !blocked_has_denied,
        "an explicit kernel deny must remove that tool from the presented set"
    );
    assert!(
        kernel_tool_count(&blocked) >= free_kernel - 1,
        "only the explicitly-denied kernel tool is removed; the rest remain (default Allow)"
    );
}

// ─────────────────────────── kernel RBAC ───────────────────────────

#[tokio::test]
async fn kernel_rbac_blocks_only_on_explicit_deny() {
    let pool = fresh_pool().await;
    // The synthetic "kernel" server row must exist: mcp_access_control.server_id
    // is a FK onto mcp_servers(name) and the kernel opens SQLite with
    // foreign_keys=ON, so granting on "kernel" requires the row (mirrors the
    // live DB, which carries a `kernel` row). This also confirms the migration's
    // EXISTS(mcp_servers) backfill guard is mandatory.
    add_server(&pool, "kernel", "opt-in").await;
    // An explicit kernel deny for this agent on a kernel-native tool.
    add_grant(
        &pool,
        "tool_grant",
        "agent.blocked",
        "kernel",
        Some("mgp.audit.replay"),
        "deny",
    )
    .await;
    let mgr = McpClientManager::new(pool, true, 120, 30); // yolo on: isolate the RBAC

    // Explicit deny → blocked by the kernel Deny-only RBAC.
    let blocked = mgr
        .execute_tool(
            &Caller::Agent("agent.blocked".to_string()),
            "mgp.audit.replay",
            serde_json::json!({}),
        )
        .await;
    match blocked {
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("cannot use kernel tool") || msg.contains("Access denied"),
                "explicit kernel deny must block, got: {msg}"
            );
        }
        Ok(v) => panic!("explicit kernel deny must block, got Ok({v:?})"),
    }

    // A different agent with NO explicit deny is NOT blocked by the RBAC
    // (default Allow) — it proceeds into the kernel tool's own logic rather than
    // being refused by the gate. We only assert it is not the RBAC denial.
    let other = mgr
        .execute_tool(
            &Caller::Agent("agent.free".to_string()),
            "mgp.audit.replay",
            serde_json::json!({}),
        )
        .await;
    if let Err(e) = other {
        let msg = e.to_string();
        assert!(
            !msg.contains("cannot use kernel tool"),
            "agent without an explicit kernel deny must NOT hit the RBAC denial, got: {msg}"
        );
    }
}
