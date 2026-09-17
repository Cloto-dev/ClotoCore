//! Conversations (docs/CONVERSATIONS_DESIGN.md §5): the behaviours the design
//! introduces, each written so the mutation named beside it in the document
//! turns it red.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use cloto_core::db::{self, ChatMessageRow};
use cloto_core::handlers::chat::{
    archive_all_conversations, create_conversation, delete_all_conversations, delete_conversation,
    get_messages, list_conversations, post_message, update_conversation, ConversationsQuery,
    CreateConversationRequest, GetMessagesQuery, PostMessageRequest, UpdateConversationRequest,
};
use cloto_core::handlers::system::SystemHandler;
use cloto_core::managers::{AgentManager, McpClientManager, PluginRegistry};
use cloto_core::test_utils::create_test_app_state;
use cloto_core::AppState;
use cloto_shared::{ClotoMessage, MessageSource};
use sqlx::SqlitePool;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

const API_KEY: &str = "test-key";
const CONVERSATIONS_MIGRATION: i64 = 20_260_917_000_000;

async fn insert_agent(pool: &SqlitePool, id: &str) {
    sqlx::query("INSERT INTO agents (id, name, description, status, default_engine_id, required_capabilities, metadata, enabled) VALUES (?, 'Test Agent', 'Desc', 'online', 'engine.test', '[]', '{}', 1)")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}

fn row(
    id: &str,
    agent: &str,
    user: &str,
    source: &str,
    text: &str,
    at: i64,
    conversation: Option<&str>,
) -> ChatMessageRow {
    ChatMessageRow {
        id: id.into(),
        agent_id: agent.into(),
        user_id: user.into(),
        source: source.into(),
        content: serde_json::json!([{"type": "text", "text": text}]).to_string(),
        metadata: None,
        created_at: at,
        parent_id: None,
        branch_index: 0,
        conversation_id: conversation.map(str::to_string),
    }
}

async fn state() -> Arc<AppState> {
    let s = create_test_app_state(Some(API_KEY.into())).await;
    insert_agent(&s.pool, "agent.a").await;
    insert_agent(&s.pool, "agent.b").await;
    s
}

fn headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("X-API-Key", API_KEY.parse().unwrap());
    h
}

async fn read(
    outcome: Result<Json<serde_json::Value>, cloto_core::AppError>,
) -> (StatusCode, serde_json::Value) {
    let response = match outcome {
        Ok(json) => json.into_response(),
        Err(err) => err.into_response(),
    };
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

async fn create(state: &Arc<AppState>, agent: &str) -> String {
    let (status, body) = read(
        create_conversation(
            State(state.clone()),
            headers(),
            Path(agent.to_string()),
            Some(Json(CreateConversationRequest { user_id: None })),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"]["id"].as_str().unwrap().to_string()
}

async fn list(
    state: &Arc<AppState>,
    agent: &str,
    include_archived: bool,
) -> Vec<serde_json::Value> {
    let (status, body) = read(
        list_conversations(
            State(state.clone()),
            headers(),
            Path(agent.to_string()),
            Query(ConversationsQuery {
                user_id: None,
                include_archived,
            }),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"]["conversations"].as_array().unwrap().clone()
}

// ---------------------------------------------------------------------------
// Migration: old history is kept whole

/// Runs every migration older than the conversations one, seeds history the
/// way it existed before conversations, then runs the rest.
async fn pool_with_history_before_conversations() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let older: Vec<_> = db::MIGRATOR
        .migrations
        .iter()
        .filter(|m| m.version < CONVERSATIONS_MIGRATION)
        .cloned()
        .collect();
    assert!(
        older.len() < db::MIGRATOR.migrations.len(),
        "the conversations migration must be in the set"
    );
    let before = sqlx::migrate::Migrator {
        migrations: Cow::Owned(older),
        ..sqlx::migrate::Migrator::DEFAULT
    };
    before.run(&pool).await.unwrap();
    insert_agent(&pool, "agent.a").await;
    insert_agent(&pool, "agent.b").await;
    // Pre-conversation rows: no conversation_id column yet.
    for (id, agent, user, source, at) in [
        ("m1", "agent.a", "u1", "user", 1_700_000_000_000_i64),
        ("m2", "agent.a", "u1", "agent", 1_700_000_001_000),
        ("m3", "agent.a", "system", "agent", 1_700_100_000_000),
        ("m4", "agent.b", "u1", "user", 1_700_200_000_000),
        ("m5", "agent.a", "u1", "user", 1_700_300_000_000),
    ] {
        sqlx::query("INSERT INTO chat_messages (id, agent_id, user_id, source, content, metadata, created_at, parent_id, branch_index) VALUES (?, ?, ?, ?, '[]', NULL, ?, NULL, 0)")
            .bind(id).bind(agent).bind(user).bind(source).bind(at)
            .execute(&pool).await.unwrap();
    }
    pool
}

#[tokio::test]
async fn the_migration_keeps_every_old_row_and_files_it_once_per_agent_and_user() {
    let pool = pool_with_history_before_conversations().await;
    let (before,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chat_messages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, 5);

    db::MIGRATOR.run(&pool).await.unwrap();

    let (after,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chat_messages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(after, before, "no row may be dropped or duplicated");

    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, agent_id, user_id, conversation_id FROM chat_messages ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for (id, agent, user, conversation) in &rows {
        assert_eq!(
            conversation,
            &db::default_conversation_id(agent, user),
            "row {id} must be filed under its agent/user's default conversation"
        );
    }

    let convs: Vec<(String, String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT id, agent_id, user_id, title, created_at, updated_at FROM conversations ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        convs.len(),
        3,
        "one conversation per (agent, user) that had messages"
    );
    let a_u1 = convs
        .iter()
        .find(|c| c.1 == "agent.a" && c.2 == "u1")
        .unwrap();
    assert_eq!(
        a_u1.4, 1_700_000_000_000,
        "created_at is the oldest message"
    );
    assert_eq!(
        a_u1.5, 1_700_300_000_000,
        "updated_at is the newest message"
    );
    assert_eq!(
        a_u1.3, "2023-11-14 – 2023-11-18",
        "the title is the date range"
    );
}

// ---------------------------------------------------------------------------
// The model reads the conversation

#[tokio::test]
async fn context_is_one_conversation_only_and_never_the_other() {
    let s = state().await;
    let c1 = create(&s, "agent.a").await;
    let c2 = create(&s, "agent.a").await;
    for (i, c) in [(1, &c1), (2, &c2), (3, &c1)] {
        db::save_chat_message(
            &s.pool,
            &row(
                &format!("m{i}"),
                "agent.a",
                "default",
                "user",
                &format!("turn {i}"),
                i,
                Some(c),
            ),
        )
        .await
        .unwrap();
    }
    let ctx = db::get_conversation_context(&s.pool, &c2, 40)
        .await
        .unwrap();
    let ids: Vec<&str> = ctx.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["m2"], "only the second conversation's turn");

    let ctx = db::get_conversation_context(&s.pool, &c1, 40)
        .await
        .unwrap();
    let ids: Vec<&str> = ctx.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["m1", "m3"],
        "the first conversation's turns, oldest first"
    );
}

#[tokio::test]
async fn the_budget_keeps_the_newest_turns_in_order() {
    let s = state().await;
    let c = create(&s, "agent.a").await;
    for i in 1..=50_i64 {
        db::save_chat_message(
            &s.pool,
            &row(
                &format!("m{i:02}"),
                "agent.a",
                "default",
                "user",
                "x",
                i,
                Some(&c),
            ),
        )
        .await
        .unwrap();
    }
    let ctx = db::get_conversation_context(&s.pool, &c, 40).await.unwrap();
    assert_eq!(ctx.len(), 40);
    assert_eq!(
        ctx.first().unwrap().id,
        "m11",
        "the oldest kept is the 11th"
    );
    assert_eq!(ctx.last().unwrap().id, "m50", "the newest is last");
    assert!(
        ctx.windows(2).all(|w| w[0].created_at <= w[1].created_at),
        "oldest first"
    );
}

async fn handler_on(pool: SqlitePool) -> SystemHandler {
    let mcp = Arc::new(McpClientManager::new(pool.clone(), false, 120, 30));
    let registry = Arc::new(PluginRegistry::new(5, 10, 50, mcp));
    let agent_manager = AgentManager::new(pool.clone(), 90_000);
    let (event_tx, _event_rx) = mpsc::channel(64);
    let metrics = Arc::new(cloto_core::managers::SystemMetrics::new());
    SystemHandler::new(
        registry,
        agent_manager,
        "agent.a".to_string(),
        event_tx,
        10,
        metrics,
        vec![],
        "consensus:".to_string(),
        16,
        30,
        Arc::new(dashmap::DashMap::new()),
        Arc::new(dashmap::DashMap::new()),
        pool,
        Arc::new(dashmap::DashMap::new()),
        5,
        false,
    )
}

fn user_message(text: &str, conversation: Option<&str>) -> ClotoMessage {
    let mut metadata = HashMap::new();
    metadata.insert("target_agent_id".to_string(), "agent.a".to_string());
    if let Some(c) = conversation {
        metadata.insert("conversation_id".to_string(), c.to_string());
    }
    ClotoMessage {
        id: cloto_shared::ClotoId::new().to_string(),
        source: MessageSource::User {
            id: "default".into(),
            name: "User".into(),
        },
        target_agent: Some("agent.a".to_string()),
        content: text.to_string(),
        timestamp: chrono::Utc::now(),
        metadata,
    }
}

/// The turns the dispatch path hands the engine come from the database, so a
/// handler built after a "restart" (a new handler on the same pool, with an
/// empty in-memory transcript) sees them.
#[tokio::test]
async fn the_conversation_context_survives_a_restart() {
    let s = state().await;
    let c = create(&s, "agent.a").await;
    let first = handler_on(s.pool.clone()).await;
    // No engine is registered, so the reply is an error — persisted all the same.
    first
        .handle_message(user_message("remember the number 41", Some(&c)))
        .await
        .unwrap();
    drop(first);

    let restarted = handler_on(s.pool.clone()).await;
    let probe = user_message("what was it?", Some(&c));
    let ctx = restarted.conversation_context_for(&probe).await;
    assert!(
        ctx.iter()
            .any(|m| m.content.contains("remember the number 41")),
        "the earlier turn must reach the model from the database: {ctx:?}"
    );
    let stranger = user_message("what was it?", None);
    assert!(
        restarted
            .conversation_context_for(&stranger)
            .await
            .is_empty(),
        "an id-less message continues the default conversation, which is empty here"
    );
}

#[tokio::test]
async fn a_message_without_an_id_lands_in_the_default_conversation_every_time() {
    let s = state().await;
    let handler = handler_on(s.pool.clone()).await;
    handler
        .handle_message(user_message("first", None))
        .await
        .unwrap();
    handler
        .handle_message(user_message("second", None))
        .await
        .unwrap();
    let expected = db::default_conversation_id("agent.a", "default");
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT source, conversation_id FROM chat_messages ORDER BY created_at, rowid",
    )
    .fetch_all(&s.pool)
    .await
    .unwrap();
    assert!(
        rows.len() >= 2,
        "user rows (and their error replies) were stored: {rows:?}"
    );
    assert!(
        rows.iter().all(|(_, c)| c.as_deref() == Some(expected.as_str())),
        "every row — user turns and the replies filed after them — carries the default id: {rows:?}"
    );
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversations")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "one default conversation, not one per message");
    let conv = db::get_conversation(&s.pool, &expected)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(conv.title, "first", "titled from the first message, once");

    // And the thread continues: the next id-less message reads those turns.
    let ctx = handler
        .conversation_context_for(&user_message("third", None))
        .await;
    assert!(
        ctx.iter().any(|m| m.content == "first"),
        "an id-less message continues the default conversation: {ctx:?}"
    );
}

// ---------------------------------------------------------------------------
// The API

#[tokio::test]
async fn archive_hides_without_losing_and_unarchive_restores_the_old_place() {
    let s = state().await;
    let older = create(&s, "agent.a").await;
    let newer = create(&s, "agent.a").await;
    // Pin the order explicitly: two creates in one millisecond would tie.
    for (id, at) in [(&older, 1_000_i64), (&newer, 2_000)] {
        sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(&s.pool)
            .await
            .unwrap();
    }
    db::save_chat_message(
        &s.pool,
        &row(
            "m1",
            "agent.a",
            "default",
            "user",
            "kept",
            1_000,
            Some(&older),
        ),
    )
    .await
    .unwrap();

    let (status, _) = read(
        update_conversation(
            State(s.clone()),
            headers(),
            Path(("agent.a".to_string(), older.clone())),
            Json(UpdateConversationRequest {
                title: None,
                archived: Some(true),
            }),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let live: Vec<String> = list(&s, "agent.a", false)
        .await
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        live,
        vec![newer.clone()],
        "archived is absent from the list"
    );
    let all = list(&s, "agent.a", true).await;
    assert_eq!(all.len(), 2, "present with include_archived");
    let archived = all.iter().find(|c| c["id"] == older.as_str()).unwrap();
    assert!(archived["archived_at"].is_number());
    assert_eq!(archived["message_count"], 1, "messages intact");

    let (status, _) = read(
        update_conversation(
            State(s.clone()),
            headers(),
            Path(("agent.a".to_string(), older.clone())),
            Json(UpdateConversationRequest {
                title: None,
                archived: Some(false),
            }),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let live: Vec<String> = list(&s, "agent.a", false)
        .await
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        live,
        vec![newer, older],
        "back in the list at its old place: archiving did not bump updated_at"
    );
}

#[tokio::test]
async fn delete_removes_the_messages_too_and_only_for_the_owning_agent() {
    let s = state().await;
    let c = create(&s, "agent.a").await;
    db::save_chat_message(
        &s.pool,
        &row("m1", "agent.a", "default", "user", "gone", 1, Some(&c)),
    )
    .await
    .unwrap();

    // Another agent's path cannot reach it.
    let (status, _) = read(
        delete_conversation(
            State(s.clone()),
            headers(),
            Path(("agent.b".to_string(), c.clone())),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(db::get_conversation(&s.pool, &c).await.unwrap().is_some());

    let (status, body) = read(
        delete_conversation(
            State(s.clone()),
            headers(),
            Path(("agent.a".to_string(), c.clone())),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["deleted_messages"], 1);
    assert!(db::get_conversation(&s.pool, &c).await.unwrap().is_none());
    let (status, body) = read(
        get_messages(
            State(s.clone()),
            headers(),
            Path("agent.a".to_string()),
            Query(GetMessagesQuery {
                user_id: None,
                before: None,
                limit: None,
                conversation_id: Some(c.clone()),
            }),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["messages"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn a_rename_sticks_and_the_first_line_title_never_overwrites_it() {
    let s = state().await;
    let c = create(&s, "agent.a").await;
    let (status, body) = read(
        update_conversation(
            State(s.clone()),
            headers(),
            Path(("agent.a".to_string(), c.clone())),
            Json(UpdateConversationRequest {
                title: Some("  My thread  ".into()),
                archived: None,
            }),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["title"], "My thread");

    // The first message would title an untitled conversation; not a named one.
    let (status, _) = read(
        post_message(
            State(s.clone()),
            headers(),
            Path("agent.a".to_string()),
            Json(PostMessageRequest {
                id: "p1".into(),
                source: "user".into(),
                content: serde_json::json!([{"type": "text", "text": "hello there"}]),
                metadata: None,
                user_id: None,
                conversation_id: Some(c.clone()),
            }),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        db::get_conversation(&s.pool, &c)
            .await
            .unwrap()
            .unwrap()
            .title,
        "My thread"
    );

    let fresh = create(&s, "agent.a").await;
    let (status, _) = read(
        post_message(
            State(s.clone()),
            headers(),
            Path("agent.a".to_string()),
            Json(PostMessageRequest {
                id: "p2".into(),
                source: "user".into(),
                content: serde_json::json!([{"type": "text", "text": "  A question\nwith a second line"}]),
                metadata: None,
                user_id: None,
                conversation_id: Some(fresh.clone()),
            }),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        db::get_conversation(&s.pool, &fresh)
            .await
            .unwrap()
            .unwrap()
            .title,
        "A question"
    );
}

#[tokio::test]
async fn the_bulk_actions_cover_every_conversation_of_the_agent_and_user() {
    let s = state().await;
    let a1 = create(&s, "agent.a").await;
    let _a2 = create(&s, "agent.a").await;
    let b1 = create(&s, "agent.b").await;
    db::save_chat_message(
        &s.pool,
        &row("m1", "agent.a", "default", "user", "x", 1, Some(&a1)),
    )
    .await
    .unwrap();

    let (status, body) = read(
        archive_all_conversations(
            State(s.clone()),
            headers(),
            Path("agent.a".to_string()),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["archived"], 2);
    assert!(list(&s, "agent.a", false).await.is_empty());
    assert_eq!(
        list(&s, "agent.b", false).await.len(),
        1,
        "the other agent is untouched"
    );

    let (status, body) = read(
        delete_all_conversations(
            State(s.clone()),
            headers(),
            Path("agent.a".to_string()),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["deleted"], 2, "archived ones included");
    assert!(list(&s, "agent.a", true).await.is_empty());
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chat_messages")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(db::get_conversation(&s.pool, &b1).await.unwrap().is_some());
}

/// The routes exist only if the kernel registers them; the handlers cannot
/// tell. Read out of the source that builds the router, as the published
/// state test does.
#[test]
fn the_kernel_registers_the_conversation_routes() {
    let wiring = include_str!("../src/lib.rs");
    for needle in [
        "\"/chat/{agent_id}/conversations\"",
        "\"/chat/{agent_id}/conversations/{conversation_id}\"",
        "\"/chat/{agent_id}/conversations/archive-all\"",
        "\"/chat/{agent_id}/conversations/delete-all\"",
        "handlers::chat::list_conversations",
        "handlers::chat::create_conversation",
        "handlers::chat::update_conversation",
        "handlers::chat::delete_conversation",
        "handlers::chat::archive_all_conversations",
        "handlers::chat::delete_all_conversations",
        "set_max_conversation_context(config.max_conversation_context)",
    ] {
        assert!(wiring.contains(needle), "{needle} is not wired in lib.rs");
    }
}
