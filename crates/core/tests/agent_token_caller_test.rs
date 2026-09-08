//! Who a `/api/mcp/call` runs as, decided from the credential and nothing else.
//!
//! The gate these tests protect is `enforce_caller_grant`, which is already
//! correct for `Caller::Agent` and deliberately vacuous for `Caller::System`.
//! What was missing was any way for an untrusted caller to *be* an agent: the
//! endpoint authenticated with the admin key, so every caller was System and
//! the gate never ran. An agent token supplies the identity the kernel reads
//! instead of the one the caller declares.
//!
//! Each test therefore asks one question about the credential, and one about
//! what the gate then did. The two are separable in the error text: the gate
//! refuses *before* the client lookup, so "not granted" means the gate spoke
//! and "not connected" means it let the call through — the same distinction
//! `capability_gate_test.rs` relies on.

use cloto_core::managers::agent_token::{AgentTokenStore, AGENT_TOKEN_HEADER};
use cloto_core::managers::{Caller, McpClientManager};
use sqlx::SqlitePool;

async fn fresh_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();
    pool
}

async fn add_server(pool: &SqlitePool, name: &str) {
    sqlx::query(
        "INSERT INTO mcp_servers (name, command, created_at, default_policy)
         VALUES (?, 'noop', 0, 'opt-in')",
    )
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
}

async fn grant_server(pool: &SqlitePool, agent_id: &str, server_id: &str) {
    sqlx::query(
        "INSERT INTO mcp_access_control \
         (entry_type, agent_id, server_id, tool_name, permission, granted_at) \
         VALUES ('server_grant', ?, ?, NULL, 'allow', 't0')",
    )
    .bind(agent_id)
    .bind(server_id)
    .execute(pool)
    .await
    .unwrap();
}

fn err_text<T: std::fmt::Debug>(r: &anyhow::Result<T>) -> String {
    match r {
        Ok(v) => panic!("expected Err, got Ok({v:?})"),
        Err(e) => e.to_string(),
    }
}

// ───────────── the token names an agent, and the gate then applies ─────────────

#[tokio::test]
async fn a_token_holder_is_gated_as_the_agent_the_token_names() {
    let pool = fresh_pool().await;
    let store = AgentTokenStore::new();
    let token = store.mint_default("agent.ungranted").await;

    // The credential resolves to that agent — not to System, which is the whole
    // point: System would have skipped what follows.
    let caller = match store.resolve(&token).await {
        Some(id) => Caller::Agent(id),
        None => panic!("a freshly minted token must resolve"),
    };
    match &caller {
        Caller::Agent(id) => assert_eq!(id, "agent.ungranted"),
        Caller::System => panic!("a token must never resolve to System — that skips the gate"),
    }

    let mgr = McpClientManager::new(pool, false, 120, 30);
    let denied = mgr
        .call_server_tool(&caller, "mind.x", "think", serde_json::json!({}))
        .await;
    let msg = err_text(&denied);
    assert!(
        msg.contains("not granted") || msg.contains("Access denied"),
        "a token holder with no grant must be refused by the gate, got: {msg}"
    );
    assert!(
        !msg.contains("not found") && !msg.contains("not connected"),
        "the refusal must come from the gate, before the client lookup, got: {msg}"
    );
}

#[tokio::test]
async fn granting_the_agent_lets_the_same_token_through() {
    let pool = fresh_pool().await;
    add_server(&pool, "mind.x").await;
    grant_server(&pool, "agent.granted", "mind.x").await;
    let store = AgentTokenStore::new();
    let token = store.mint_default("agent.granted").await;

    let caller = Caller::Agent(store.resolve(&token).await.unwrap());
    let mgr = McpClientManager::new(pool, false, 120, 30);
    let r = mgr
        .call_server_tool(&caller, "mind.x", "think", serde_json::json!({}))
        .await;
    let msg = err_text(&r);
    assert!(
        msg.contains("not found") || msg.contains("not connected"),
        "a granted agent must pass the gate and fail only at connection, got: {msg}"
    );
    assert!(
        !msg.contains("not granted"),
        "a granted agent must not be refused by the gate, got: {msg}"
    );
}

#[tokio::test]
async fn a_token_does_not_carry_another_agents_grants() {
    let pool = fresh_pool().await;
    add_server(&pool, "mind.x").await;
    grant_server(&pool, "agent.rich", "mind.x").await;
    let store = AgentTokenStore::new();
    let poor_token = store.mint_default("agent.poor").await;

    let caller = Caller::Agent(store.resolve(&poor_token).await.unwrap());
    let mgr = McpClientManager::new(pool, false, 120, 30);
    let denied = mgr
        .call_server_tool(&caller, "mind.x", "think", serde_json::json!({}))
        .await;
    let msg = err_text(&denied);
    assert!(
        msg.contains("not granted") || msg.contains("Access denied"),
        "one agent's grant must not travel on another agent's token, got: {msg}"
    );
}

// ───────────── the credential itself ─────────────

#[tokio::test]
async fn an_invalid_token_names_nobody() {
    let store = AgentTokenStore::new();
    store.mint_default("agent.real").await;
    assert_eq!(
        store.resolve("forged").await,
        None,
        "a forged token must not resolve to any agent"
    );
}

#[tokio::test]
async fn the_header_name_is_the_one_a_bridge_would_send() {
    // Pinned because the value crosses a repository boundary: the bridge that
    // presents the token mirrors this name from here. A silent rename here is a
    // silent authentication failure there.
    assert_eq!(AGENT_TOKEN_HEADER, "X-Agent-Token");
}

// ───────────── the endpoint's existing behaviour is untouched ─────────────

#[tokio::test]
async fn system_still_bypasses_when_no_token_is_presented() {
    let pool = fresh_pool().await;
    let mgr = McpClientManager::new(pool, false, 120, 30);

    // The admin-key path is unchanged: still System, still ungated. This is the
    // coordinator pattern, and nothing above was allowed to narrow it.
    let r = mgr
        .call_server_tool(&Caller::System, "mind.x", "think", serde_json::json!({}))
        .await;
    let msg = err_text(&r);
    assert!(
        msg.contains("not found") || msg.contains("not connected"),
        "System must still bypass the gate, got: {msg}"
    );
    assert!(
        !msg.contains("not granted"),
        "System must not become gated, got: {msg}"
    );
}
