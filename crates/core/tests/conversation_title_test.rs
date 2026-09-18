//! A conversation's title is the engine's to give only while nobody has given one.
//!
//! After the first exchange the kernel asks the agent's engine to name the
//! conversation (`SystemHandler::after_reply_persisted`). That question is a
//! whole engine run — for an agent on a CLI harness, one more `codex exec` or
//! `claude -p` against a subscription quota — so it must not be asked of a
//! conversation that already carries a chosen name: the kernel would only throw
//! the answer away. Scheduled runs that open a conversation and name it before
//! the first message are exactly that shape.
//!
//! The engine here is real as far as the kernel can tell: a python MCP server
//! connected through `McpClientManager::connect_server` and reached by the same
//! dispatch a production engine is. It counts the `think` calls it answers and
//! reports the count through a second tool, so the tests count engine runs
//! instead of inferring them from what ended up in the database.

use cloto_core::db;
use cloto_core::handlers::system::SystemHandler;
use cloto_core::managers::mcp_protocol::McpServerConfig;
use cloto_core::managers::{AgentManager, Caller, McpClientManager, PluginRegistry};
use cloto_core::test_utils::create_test_app_state;
use cloto_shared::{ClotoMessage, MessageSource};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const AGENT: &str = "agent.a";
const ENGINE: &str = "engine.fake";
const REPLY: &str = "the reply";
const ENGINE_TITLE: &str = "Named by the engine";

/// A stdio MCP server with two tools: `think`, which answers every request and
/// counts it, and `calls`, which reports the count. A request that asks for a
/// conversation title gets a title; anything else gets the reply. Started with
/// the argument `slow-title`, it takes a second over a title — long enough for
/// a test to rename the conversation while the engine is thinking. Requests are
/// answered one at a time, so a `calls` sent meanwhile returns after the title.
const FAKE_ENGINE: &str = r#"
import sys, json, time
slow_title = sys.argv[1:] == ["slow-title"]
calls = 0
def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    rid = req.get("id")
    if rid is None:
        continue
    method = req.get("method")
    if method == "initialize":
        version = (req.get("params") or {}).get("protocolVersion", "2025-06-18")
        send({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": version,
              "capabilities": {"tools": {}}, "serverInfo": {"name": "fake-engine", "version": "0"}}})
    elif method == "tools/list":
        schema = {"type": "object"}
        send({"jsonrpc": "2.0", "id": rid, "result": {"tools": [
            {"name": "think", "description": "answer", "inputSchema": schema},
            {"name": "calls", "description": "think calls answered so far", "inputSchema": schema}]}})
    elif method == "tools/call":
        params = req.get("params") or {}
        if params.get("name") == "calls":
            text = str(calls)
        else:
            calls += 1
            asked = ((params.get("arguments") or {}).get("message") or {}).get("content", "")
            if asked.startswith("Give this conversation a title"):
                if slow_title:
                    time.sleep(1)
                text = "Named by the engine"
            else:
                text = "the reply"
        send({"jsonrpc": "2.0", "id": rid, "result": {"content": [{"type": "text", "text": text}]}})
    else:
        send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "method not found"}})
"#;

struct Rig {
    pool: SqlitePool,
    mcp: Arc<McpClientManager>,
    handler: SystemHandler,
}

fn python3_available() -> bool {
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping: python3 not found");
        return false;
    }
    true
}

/// An agent whose engine is the fake, granted to use it, and a handler whose
/// registry reaches the fake through a manager that has connected it.
async fn rig() -> Option<Rig> {
    rig_with(&[]).await
}

async fn rig_with(engine_args: &[&str]) -> Option<Rig> {
    if !python3_available() {
        return None;
    }
    let pool = create_test_app_state(Some("test-key".into()))
        .await
        .pool
        .clone();
    sqlx::query(
        "INSERT INTO agents (id, name, description, status, default_engine_id, \
         required_capabilities, metadata, enabled) \
         VALUES (?, 'A', 'd', 'online', ?, '[]', '{}', 1)",
    )
    .bind(AGENT)
    .bind(ENGINE)
    .execute(&pool)
    .await
    .unwrap();
    // The grant below refers to the server's row.
    sqlx::query(
        "INSERT INTO mcp_servers (name, command, created_at, default_policy) \
         VALUES (?, 'python3', 0, 'opt-in')",
    )
    .bind(ENGINE)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO mcp_access_control \
         (entry_type, agent_id, server_id, tool_name, permission, granted_at) \
         VALUES ('server_grant', ?, ?, NULL, 'allow', 't0')",
    )
    .bind(AGENT)
    .bind(ENGINE)
    .execute(&pool)
    .await
    .unwrap();

    let mcp = Arc::new(McpClientManager::new(pool.clone(), false, 120, 30));
    let mut args = vec!["-c", FAKE_ENGINE];
    args.extend_from_slice(engine_args);
    let config: McpServerConfig = serde_json::from_value(serde_json::json!({
        "id": ENGINE,
        "command": "python3",
        "args": args,
    }))
    .unwrap();
    mcp.connect_server(config)
        .await
        .expect("the fake engine connects");

    let registry = Arc::new(PluginRegistry::new(5, 10, 50, mcp.clone()));
    let (event_tx, _event_rx) = mpsc::channel(64);
    let handler = SystemHandler::new(
        registry,
        AgentManager::new(pool.clone(), 90_000),
        AGENT.to_string(),
        event_tx,
        10,
        Arc::new(cloto_core::managers::SystemMetrics::new()),
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
    Some(Rig { pool, mcp, handler })
}

fn user_message(text: &str, conversation: &str) -> ClotoMessage {
    let mut metadata = HashMap::new();
    metadata.insert("target_agent_id".to_string(), AGENT.to_string());
    metadata.insert("conversation_id".to_string(), conversation.to_string());
    ClotoMessage {
        id: cloto_shared::ClotoId::new().to_string(),
        source: MessageSource::User {
            id: "default".into(),
            name: "User".into(),
        },
        target_agent: Some(AGENT.to_string()),
        content: text.to_string(),
        timestamp: chrono::Utc::now(),
        metadata,
    }
}

async fn new_conversation(pool: &SqlitePool) -> String {
    let now = chrono::Utc::now().timestamp_millis();
    db::create_conversation(pool, AGENT, "default", now)
        .await
        .unwrap()
        .id
}

async fn title(pool: &SqlitePool, conversation: &str) -> String {
    db::get_conversation(pool, conversation)
        .await
        .unwrap()
        .expect("the conversation exists")
        .title
}

/// The agent's reply in the conversation, as text.
async fn reply(pool: &SqlitePool, conversation: &str) -> String {
    let (content,): (String,) = sqlx::query_as(
        "SELECT content FROM chat_messages WHERE conversation_id = ? AND source = 'agent'",
    )
    .bind(conversation)
    .fetch_one(pool)
    .await
    .expect("one reply was stored");
    let blocks: serde_json::Value = serde_json::from_str(&content).unwrap();
    blocks[0]["text"].as_str().unwrap_or_default().to_string()
}

/// How many `think` calls the fake engine has answered.
async fn engine_runs(mcp: &McpClientManager) -> u64 {
    let result = mcp
        .call_server_tool(&Caller::System, ENGINE, "calls", serde_json::json!({}))
        .await
        .expect("the fake engine reports its count");
    let value = serde_json::to_value(&result).unwrap();
    value["content"][0]["text"]
        .as_str()
        .and_then(|t| t.parse().ok())
        .unwrap_or_else(|| panic!("unreadable count: {value}"))
}

/// The control: a conversation nobody named is asked about, and gets the
/// engine's title. Without it the test below could pass because no title is
/// ever requested at all.
#[tokio::test]
async fn an_unnamed_conversation_is_named_by_the_engine_after_its_first_exchange() {
    let Some(rig) = rig().await else { return };
    let conversation = new_conversation(&rig.pool).await;

    rig.handler
        .handle_message(user_message("hello there", &conversation))
        .await
        .unwrap();
    assert_eq!(reply(&rig.pool, &conversation).await, REPLY);

    let asked = Instant::now();
    while title(&rig.pool, &conversation).await != ENGINE_TITLE {
        assert!(
            asked.elapsed() < Duration::from_secs(30),
            "the engine never named the conversation (title {:?})",
            title(&rig.pool, &conversation).await
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    eprintln!(
        "the engine's title arrived {:?} after the reply",
        asked.elapsed()
    );
    assert_eq!(
        engine_runs(&rig.mcp).await,
        2,
        "one run for the reply, one for the title"
    );
}

/// A first message with no line of text leaves the conversation untitled, and
/// an untitled conversation is still the engine's to name.
///
/// This does not separate the two halves of `title_is_provisional`: with no
/// line of text the first line is empty too, so "untitled" and "titled by its
/// first line" agree here. They part only when writing the first-line title
/// failed, which no test can arrange without a failing database.
#[tokio::test]
async fn a_blank_first_message_leaves_the_title_to_the_engine() {
    let Some(rig) = rig().await else { return };
    let conversation = new_conversation(&rig.pool).await;

    rig.handler
        .handle_message(user_message("  \n  ", &conversation))
        .await
        .unwrap();
    assert_eq!(reply(&rig.pool, &conversation).await, REPLY);

    let asked = Instant::now();
    while title(&rig.pool, &conversation).await != ENGINE_TITLE {
        assert!(
            asked.elapsed() < Duration::from_secs(30),
            "an untitled conversation was never named (title {:?})",
            title(&rig.pool, &conversation).await
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(engine_runs(&rig.mcp).await, 2);
}

/// The name is checked again when the engine's answer arrives: someone may
/// have renamed the conversation while the engine was thinking, and then the
/// answer is discarded.
#[tokio::test]
async fn a_rename_made_while_the_engine_thinks_is_kept() {
    let Some(rig) = rig_with(&["slow-title"]).await else {
        return;
    };
    let conversation = new_conversation(&rig.pool).await;

    rig.handler
        .handle_message(user_message("hello there", &conversation))
        .await
        .unwrap();
    assert_eq!(reply(&rig.pool, &conversation).await, REPLY);
    // The request for a title is on its way (the conversation was unnamed when
    // the reply was stored), and the engine takes a second over it.
    let chosen = "Renamed meanwhile";
    assert!(db::rename_conversation(&rig.pool, &conversation, chosen)
        .await
        .unwrap());

    // The engine answers one request at a time, so a count of 2 is read only
    // once the title request has been answered. A read that overtook the
    // request (the kernel sends it from a task of its own) says 1; ask again.
    let asked = Instant::now();
    while engine_runs(&rig.mcp).await < 2 {
        assert!(
            asked.elapsed() < Duration::from_secs(30),
            "the title was never asked for, though the conversation was unnamed when the reply was stored"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The answer reaches the kernel's task within milliseconds (the control
    // above measures it); give that task well past its time to overwrite.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(title(&rig.pool, &conversation).await, chosen);
}

#[tokio::test]
async fn a_conversation_someone_named_costs_no_engine_run_for_a_title() {
    let Some(rig) = rig().await else { return };
    let conversation = new_conversation(&rig.pool).await;
    let chosen = "Daily report 2026-09-18";
    assert!(db::rename_conversation(&rig.pool, &conversation, chosen)
        .await
        .unwrap());

    rig.handler
        .handle_message(user_message("hello there", &conversation))
        .await
        .unwrap();
    assert_eq!(reply(&rig.pool, &conversation).await, REPLY);

    // A title request is issued from a task spawned when the reply is stored;
    // the control above sees its answer land well inside this window.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        engine_runs(&rig.mcp).await,
        1,
        "only the reply may run the engine — a title for a named conversation would be thrown away"
    );
    assert_eq!(title(&rig.pool, &conversation).await, chosen);
}
