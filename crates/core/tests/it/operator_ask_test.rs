//! Asking the operator a question without stopping for the answer.
//!
//! The tool this covers is the point of the whole notification line: an agent
//! can raise something for a person and carry on, and the person answers when
//! they get to it. Two properties decide whether that works, and both fail
//! quietly if broken, so both are pinned here:
//!
//! * **It is offered without YOLO mode.** The privileged kernel tools are gated
//!   on YOLO, and YOLO is the mode where the operator has said not to ask.
//!   Putting the asking tool behind that gate would mean an agent can raise a
//!   question only once questions have been switched off, and nothing would
//!   report it — the tool would simply never appear.
//! * **It does not block.** A proposal that arrived with `blocking` set would be
//!   counted as holding an agent, and "just checking" would become a stop.

use cloto_core::db;
use cloto_core::managers::mcp::{Caller, McpClientManager};
use sqlx::SqlitePool;
use std::sync::Arc;

const AGENT: &str = "agent.test";
const ASK: &str = "mgp.operator.ask";
const REPLIES: &str = "mgp.operator.replies";

async fn manager(yolo: bool) -> (Arc<McpClientManager>, SqlitePool) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::init_db(&pool, "sqlite::memory:", None).await.unwrap();
    (
        Arc::new(McpClientManager::new(pool.clone(), yolo, 120, 30)),
        pool,
    )
}

fn tool_names(schemas: &[serde_json::Value]) -> Vec<String> {
    schemas
        .iter()
        .filter_map(|s| {
            s.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(String::from)
        })
        .collect()
}

#[tokio::test]
async fn the_asking_tool_is_offered_to_an_agent_that_has_no_privileges() {
    let (mgr, _pool) = manager(false).await;

    let names = tool_names(&mgr.collect_tool_schemas_for_agent(AGENT).await);

    assert!(
        names.iter().any(|n| n == ASK),
        "without YOLO the agent was offered {names:?} — asking a person is not a privilege, \
         and gating it on the mode that means 'stop asking me' would make it unreachable \
         exactly when it is wanted"
    );
    assert!(
        names.iter().any(|n| n == REPLIES),
        "the answer is unreachable: {names:?}"
    );
}

#[tokio::test]
async fn an_explicit_deny_still_takes_the_tool_away() {
    let (mgr, pool) = manager(false).await;
    db::save_access_control_entry(
        &pool,
        &db::mcp::AccessControlEntry {
            id: None,
            entry_type: db::mcp::EntryType::ToolGrant,
            agent_id: AGENT.to_string(),
            server_id: "kernel".to_string(),
            tool_name: Some(ASK.to_string()),
            permission: db::mcp::PermissionLevel::Deny,
            granted_by: Some("test".to_string()),
            granted_at: chrono::Utc::now().to_rfc3339(),
            expires_at: None,
            justification: None,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let names = tool_names(&mgr.collect_tool_schemas_for_agent(AGENT).await);

    // Unconditional is not the same as ungovernable: the operator keeps the one
    // lever every other kernel tool answers to.
    assert!(!names.iter().any(|n| n == ASK), "the deny was ignored");
    assert!(
        names.iter().any(|n| n == REPLIES),
        "denying one tool took the other with it"
    );
}

#[tokio::test]
async fn a_question_is_stored_as_a_proposal_that_holds_nobody_up() {
    let (mgr, pool) = manager(false).await;

    let out = mgr
        .execute_tool(
            &Caller::Agent(AGENT.to_string()),
            ASK,
            serde_json::json!({
                "title": "Shall I retire the market-scan timer?",
                "body": "It has not produced a finding in ten days.",
            }),
        )
        .await
        .expect("asking must not fail");

    let item_id = out["item_id"].as_str().expect("the caller gets an id back");
    let item = db::get_notification(&pool, item_id)
        .await
        .unwrap()
        .expect("the question is in the store");

    assert_eq!(item.kind, db::NotificationKind::Proposal);
    assert_eq!(item.agent_id.as_deref(), Some(AGENT));
    assert_eq!(item.title, "Shall I retire the market-scan timer?");
    assert!(
        !item.blocking,
        "a proposal that blocks is an approval wearing the wrong name, and it would be \
         counted on the badge as holding an agent that is in fact still working"
    );
    assert!(item.resolved_at.is_none(), "nobody has answered it yet");
    // Declared, not derived — the opposite of the approval gate, and only safe
    // because nothing waits on this one.
    assert_eq!(item.severity, cloto_shared::McpLogLevel::Notice);
}

#[tokio::test]
async fn the_declared_severity_is_taken_as_given() {
    let (mgr, pool) = manager(false).await;

    let out = mgr
        .execute_tool(
            &Caller::Agent(AGENT.to_string()),
            ASK,
            serde_json::json!({ "title": "Budget is nearly spent", "severity": "error" }),
        )
        .await
        .unwrap();

    let item = db::get_notification(&pool, out["item_id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(item.severity, cloto_shared::McpLogLevel::Error);
    assert!(!item.blocking, "loud is still not blocking");
}

#[tokio::test]
async fn an_answer_reaches_the_agent_that_asked_and_an_unanswered_question_does_not() {
    let (mgr, pool) = manager(false).await;
    let caller = Caller::Agent(AGENT.to_string());

    let answered = mgr
        .execute_tool(&caller, ASK, serde_json::json!({ "title": "May I?" }))
        .await
        .unwrap()["item_id"]
        .as_str()
        .unwrap()
        .to_string();
    let untouched = mgr
        .execute_tool(&caller, ASK, serde_json::json!({ "title": "And this?" }))
        .await
        .unwrap()["item_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Nothing has been answered, so there is nothing to read.
    let empty = mgr
        .execute_tool(&caller, REPLIES, serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(empty["replies"].as_array().unwrap().len(), 0);

    db::resolve_notification(&pool, &answered, "yes, and log why")
        .await
        .unwrap();

    let out = mgr
        .execute_tool(&caller, REPLIES, serde_json::json!({}))
        .await
        .unwrap();
    let replies = out["replies"].as_array().unwrap();

    assert_eq!(
        replies.len(),
        1,
        "the unanswered question came back too — an agent reading 'not yet' as a reply is \
         the failure this filter exists to prevent"
    );
    assert_eq!(replies[0]["item_id"].as_str().unwrap(), answered);
    assert_eq!(replies[0]["answer"].as_str().unwrap(), "yes, and log why");
    assert!(replies[0]["answered_at"].is_string());
    assert_ne!(replies[0]["item_id"].as_str().unwrap(), untouched);
}

#[tokio::test]
async fn one_agent_does_not_read_another_agent_s_answers() {
    let (mgr, pool) = manager(false).await;

    let theirs = mgr
        .execute_tool(
            &Caller::Agent("agent.other".to_string()),
            ASK,
            serde_json::json!({ "title": "Not yours" }),
        )
        .await
        .unwrap()["item_id"]
        .as_str()
        .unwrap()
        .to_string();
    db::resolve_notification(&pool, &theirs, "no")
        .await
        .unwrap();

    let out = mgr
        .execute_tool(
            &Caller::Agent(AGENT.to_string()),
            REPLIES,
            serde_json::json!({}),
        )
        .await
        .unwrap();

    assert_eq!(
        out["replies"].as_array().unwrap().len(),
        0,
        "the reply went to the wrong agent"
    );
}

/// Handler tests answer "does this function work", never "is anything wired to
/// it". The kernel's router is assembled inline during boot and the tool
/// dispatcher is a `match` arm, so a version of this feature with both halves
/// written and neither connected passes every test above. These read the wiring
/// itself, the way `published_state_test.rs` does.
#[test]
fn the_tool_and_the_answer_route_are_actually_registered() {
    let dispatch = include_str!("../../src/managers/mcp.rs");
    assert!(
        dispatch.contains("TOOL_NAME_OPERATOR_ASK =>"),
        "nothing dispatches the asking tool — an agent calling it would be told no such tool"
    );
    assert!(
        dispatch.contains("TOOL_NAME_OPERATOR_REPLIES =>"),
        "nothing dispatches the replies tool"
    );
    assert!(
        dispatch.contains("operator_ask_schema()"),
        "the schema is never injected, so no agent is ever told the tool exists"
    );
    assert!(
        dispatch.contains("operator_replies_schema()"),
        "the replies schema is never injected"
    );

    let wiring = include_str!("../../src/lib.rs");
    assert!(
        wiring.contains("\"/notifications/{item_id}/answer\""),
        "the answer route is not registered — the question would be unanswerable, which is \
         the one thing this feature cannot be"
    );
    assert!(
        wiring.contains("handlers::notifications::answer_notification"),
        "the registered route does not reach the handler that settles the item"
    );
}
