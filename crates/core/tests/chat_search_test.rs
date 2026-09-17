//! Search across conversations (`GET /api/chat/search`, `db::search_chat_messages`).
//!
//! Each test names the behaviour it pins; the index lives in a migration and
//! is kept by triggers, so most of these go through real SQLite rather than
//! the Rust that queries it.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use cloto_core::db::{self, ChatMessageRow};
use cloto_core::handlers::chat::{search_messages, SearchMessagesQuery};
use cloto_core::test_utils::create_test_app_state;
use cloto_core::AppState;
use sqlx::SqlitePool;
use std::borrow::Cow;
use std::sync::Arc;

const API_KEY: &str = "test-key";
const SEARCH_MIGRATION: i64 = 20_260_917_120_000;

async fn insert_agent(pool: &SqlitePool, id: &str) {
    sqlx::query("INSERT INTO agents (id, name, description, status, default_engine_id, required_capabilities, metadata, enabled) VALUES (?, 'Test Agent', 'Desc', 'online', 'engine.test', '[]', '{}', 1)")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
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

async fn conversation(pool: &SqlitePool, agent: &str) -> String {
    db::create_conversation(pool, agent, "default", 1)
        .await
        .unwrap()
        .id
}

async fn say(
    pool: &SqlitePool,
    id: &str,
    agent: &str,
    user: &str,
    conversation: &str,
    text: &str,
    at: i64,
) {
    save_raw(
        pool,
        id,
        agent,
        user,
        conversation,
        &serde_json::json!([{"type": "text", "text": text}]).to_string(),
        at,
    )
    .await;
}

async fn save_raw(
    pool: &SqlitePool,
    id: &str,
    agent: &str,
    user: &str,
    conversation: &str,
    content: &str,
    at: i64,
) {
    db::save_chat_message(
        pool,
        &ChatMessageRow {
            id: id.into(),
            agent_id: agent.into(),
            user_id: user.into(),
            source: "user".into(),
            content: content.into(),
            metadata: None,
            created_at: at,
            parent_id: None,
            branch_index: 0,
            conversation_id: Some(conversation.into()),
        },
    )
    .await
    .unwrap();
}

async fn found(pool: &SqlitePool, query: &str) -> Vec<String> {
    db::search_chat_messages(pool, "default", query, 100)
        .await
        .unwrap()
        .hits
        .into_iter()
        .map(|h| h.message_id)
        .collect()
}

async fn search_route(
    state: &Arc<AppState>,
    q: Option<&str>,
    limit: Option<i64>,
) -> (StatusCode, serde_json::Value) {
    let outcome = search_messages(
        State(state.clone()),
        headers(),
        Query(SearchMessagesQuery {
            q: q.map(str::to_string),
            user_id: None,
            limit,
        }),
    )
    .await;
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

// ---------------------------------------------------------------------------
// What is found

/// The reason the index is trigram: a word tokenizer keeps a run of Japanese
/// as one token, and a phrase from inside it never matches.
#[tokio::test]
async fn a_phrase_inside_japanese_text_is_found_across_conversations() {
    let s = state().await;
    let c1 = conversation(&s.pool, "agent.a").await;
    let c2 = conversation(&s.pool, "agent.a").await;
    let c3 = conversation(&s.pool, "agent.b").await;
    say(
        &s.pool,
        "m1",
        "agent.a",
        "default",
        &c1,
        "雑談部屋に「おはようございます」と送信して",
        1_000,
    )
    .await;
    say(
        &s.pool,
        "m2",
        "agent.a",
        "default",
        &c2,
        "今日もおはようございます、カリンです",
        2_000,
    )
    .await;
    say(
        &s.pool,
        "m3",
        "agent.b",
        "default",
        &c3,
        "みなさんおはようございます！",
        3_000,
    )
    .await;
    say(
        &s.pool,
        "m4",
        "agent.b",
        "default",
        &c3,
        "こんばんは",
        4_000,
    )
    .await;

    let hits = db::search_chat_messages(&s.pool, "default", "ようござい", 100)
        .await
        .unwrap()
        .hits;
    assert_eq!(
        hits.iter()
            .map(|h| h.message_id.as_str())
            .collect::<Vec<_>>(),
        ["m3", "m2", "m1"],
        "every conversation's match, newest first"
    );
    assert_eq!(
        hits.iter()
            .map(|h| h.conversation_id.clone().unwrap())
            .collect::<Vec<_>>(),
        [c3, c2, c1]
    );
    assert_eq!(hits[0].agent_id, "agent.b");
}

#[tokio::test]
async fn a_message_matches_only_when_it_holds_every_term() {
    let s = state().await;
    let c = conversation(&s.pool, "agent.a").await;
    say(
        &s.pool,
        "both",
        "agent.a",
        "default",
        &c,
        "alpha and beta together",
        1,
    )
    .await;
    say(
        &s.pool,
        "one",
        "agent.a",
        "default",
        &c,
        "only alpha here",
        2,
    )
    .await;
    assert_eq!(found(&s.pool, "beta  alpha").await, ["both"]);
}

/// Below three characters the index cannot answer; the scan must.
#[tokio::test]
async fn a_term_shorter_than_a_trigram_is_still_found() {
    let s = state().await;
    let c = conversation(&s.pool, "agent.a").await;
    say(
        &s.pool,
        "m1",
        "agent.a",
        "default",
        &c,
        "朝の挨拶を送って",
        1,
    )
    .await;
    say(
        &s.pool,
        "m2",
        "agent.a",
        "default",
        &c,
        "夜の連絡を送って",
        2,
    )
    .await;
    assert_eq!(found(&s.pool, "挨拶").await, ["m1"]);
    // Mixed: one term through the index, one through the scan.
    assert_eq!(found(&s.pool, "送って 挨拶").await, ["m1"]);
}

#[tokio::test]
async fn search_ignores_ascii_case() {
    let s = state().await;
    let c = conversation(&s.pool, "agent.a").await;
    say(
        &s.pool,
        "m1",
        "agent.a",
        "default",
        &c,
        "Deploy the Kernel",
        1,
    )
    .await;
    assert_eq!(found(&s.pool, "kernel").await, ["m1"]);
    assert_eq!(found(&s.pool, "DE").await, ["m1"]);
}

#[tokio::test]
async fn wildcards_and_quotes_in_a_query_are_text() {
    let s = state().await;
    let c = conversation(&s.pool, "agent.a").await;
    say(&s.pool, "m1", "agent.a", "default", &c, "axb", 1).await;
    say(
        &s.pool,
        "m2",
        "agent.a",
        "default",
        &c,
        "say \"hi\" twice",
        2,
    )
    .await;
    assert!(found(&s.pool, "a_").await.is_empty(), "_ is not a wildcard");
    assert!(found(&s.pool, "%").await.is_empty(), "% is not a wildcard");
    assert_eq!(
        found(&s.pool, "\"hi\"").await,
        ["m2"],
        "a quote is matched, not parsed"
    );
}

/// The index holds the text blocks, not the JSON they are stored in.
#[tokio::test]
async fn only_the_text_is_searched_and_odd_content_does_not_break_a_write() {
    let s = state().await;
    let c = conversation(&s.pool, "agent.a").await;
    say(&s.pool, "m1", "agent.a", "default", &c, "hello", 1).await;
    assert!(
        found(&s.pool, "type").await.is_empty(),
        "the JSON keys are not text"
    );
    save_raw(&s.pool, "bad", "agent.a", "default", &c, "{not json", 2).await;
    save_raw(
        &s.pool,
        "obj",
        "agent.a",
        "default",
        &c,
        r#"{"type":"text","text":"object"}"#,
        3,
    )
    .await;
    save_raw(
        &s.pool,
        "img",
        "agent.a",
        "default",
        &c,
        r#"[{"type":"image","url":"x"}]"#,
        4,
    )
    .await;
    assert!(found(&s.pool, "object").await.is_empty());
    let (messages,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chat_messages")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(messages, 4, "every write succeeded");
}

#[tokio::test]
async fn another_users_messages_are_not_found_and_the_system_users_are() {
    let s = state().await;
    let c = conversation(&s.pool, "agent.a").await;
    say(&s.pool, "mine", "agent.a", "default", &c, "shared word", 1).await;
    say(
        &s.pool,
        "theirs",
        "agent.a",
        "someone",
        &c,
        "shared word",
        2,
    )
    .await;
    say(&s.pool, "system", "agent.a", "system", &c, "shared word", 3).await;
    assert_eq!(found(&s.pool, "shared").await, ["system", "mine"]);
}

// ---------------------------------------------------------------------------
// The index follows the table

#[tokio::test]
async fn an_archived_conversation_is_found_and_says_so() {
    let s = state().await;
    let live = conversation(&s.pool, "agent.a").await;
    let archived = conversation(&s.pool, "agent.a").await;
    say(&s.pool, "live", "agent.a", "default", &live, "needle", 1).await;
    say(&s.pool, "old", "agent.a", "default", &archived, "needle", 2).await;
    db::set_conversation_archived(&s.pool, &archived, Some(5))
        .await
        .unwrap();

    let hits = db::search_chat_messages(&s.pool, "default", "needle", 100)
        .await
        .unwrap()
        .hits;
    let flags: Vec<(&str, bool)> = hits
        .iter()
        .map(|h| (h.message_id.as_str(), h.archived))
        .collect();
    assert_eq!(flags, [("old", true), ("live", false)]);
}

#[tokio::test]
async fn a_deleted_conversation_leaves_nothing_findable() {
    let s = state().await;
    let gone = conversation(&s.pool, "agent.a").await;
    let kept = conversation(&s.pool, "agent.a").await;
    say(&s.pool, "g1", "agent.a", "default", &gone, "needle one", 1).await;
    say(&s.pool, "g2", "agent.a", "default", &gone, "needle two", 2).await;
    say(
        &s.pool,
        "k1",
        "agent.a",
        "default",
        &kept,
        "needle three",
        3,
    )
    .await;

    db::delete_conversation(&s.pool, &gone).await.unwrap();

    assert_eq!(found(&s.pool, "needle").await, ["k1"]);
    let (indexed,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chat_message_search")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    let (keys,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chat_message_search_keys")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!((indexed, keys), (1, 1), "the index shrinks with the table");
}

#[tokio::test]
async fn rewritten_content_is_found_by_its_new_text_only() {
    let s = state().await;
    let c = conversation(&s.pool, "agent.a").await;
    say(&s.pool, "m1", "agent.a", "default", &c, "first draft", 1).await;
    sqlx::query("UPDATE chat_messages SET content = ? WHERE id = 'm1'")
        .bind(serde_json::json!([{"type": "text", "text": "final wording"}]).to_string())
        .execute(&s.pool)
        .await
        .unwrap();
    assert!(found(&s.pool, "draft").await.is_empty());
    assert_eq!(found(&s.pool, "wording").await, ["m1"]);
}

/// Runs every migration older than the search one, seeds history, then runs
/// the rest: messages written before the index existed are searchable.
#[tokio::test]
async fn history_from_before_the_index_is_searchable() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let older: Vec<_> = db::MIGRATOR
        .migrations
        .iter()
        .filter(|m| m.version < SEARCH_MIGRATION)
        .cloned()
        .collect();
    assert!(
        older.len() < db::MIGRATOR.migrations.len(),
        "the search migration must be in the set"
    );
    sqlx::migrate::Migrator {
        migrations: Cow::Owned(older),
        ..sqlx::migrate::Migrator::DEFAULT
    }
    .run(&pool)
    .await
    .unwrap();
    insert_agent(&pool, "agent.a").await;
    let c = conversation(&pool, "agent.a").await;
    say(&pool, "old1", "agent.a", "default", &c, "昔の会話の中身", 1).await;
    say(&pool, "old2", "agent.a", "default", &c, "もう一つの中身", 2).await;
    save_raw(&pool, "old3", "agent.a", "default", &c, "[]", 3).await;

    db::MIGRATOR.run(&pool).await.unwrap();

    assert_eq!(found(&pool, "中身").await, ["old2", "old1"]);
    let (keys,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chat_message_search_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(keys, 3, "every existing message gets a key, text or not");

    // And the triggers are live after the backfill.
    say(&pool, "new1", "agent.a", "default", &c, "新しい中身", 4).await;
    assert_eq!(found(&pool, "中身").await, ["new1", "old2", "old1"]);
}

// ---------------------------------------------------------------------------
// The route

/// A caller that asked for five and got two must be told three more exist.
#[tokio::test]
async fn a_cut_list_says_how_many_matched_and_that_it_was_cut() {
    let s = state().await;
    let c = conversation(&s.pool, "agent.a").await;
    for i in 0..5 {
        say(
            &s.pool,
            &format!("m{i}"),
            "agent.a",
            "default",
            &c,
            "needle",
            i,
        )
        .await;
    }

    let (status, body) = search_route(&s, Some("needle"), Some(2)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let data = &body["data"];
    assert_eq!(data["results"].as_array().unwrap().len(), 2);
    assert_eq!(data["total"], 5);
    assert_eq!(data["truncated"], true);
    assert_eq!(data["results"][0]["message_id"], "m4");

    let (_, body) = search_route(&s, Some("needle"), Some(10)).await;
    assert_eq!(body["data"]["results"].as_array().unwrap().len(), 5);
    assert_eq!(body["data"]["total"], 5);
    assert_eq!(body["data"]["truncated"], false);
}

#[tokio::test]
async fn a_result_carries_what_the_palette_shows() {
    let s = state().await;
    let c = conversation(&s.pool, "agent.a").await;
    db::rename_conversation(&s.pool, &c, "Morning routine")
        .await
        .unwrap();
    say(
        &s.pool,
        "m1",
        "agent.a",
        "default",
        &c,
        "Send the greeting to the lounge",
        42,
    )
    .await;

    let (_, body) = search_route(&s, Some("greeting"), None).await;
    let hit = &body["data"]["results"][0];
    assert_eq!(hit["conversation_id"], c.as_str());
    assert_eq!(hit["conversation_title"], "Morning routine");
    assert_eq!(hit["agent_id"], "agent.a");
    assert_eq!(hit["source"], "user");
    assert_eq!(hit["created_at"], 42);
    assert_eq!(hit["archived"], false);
    assert_eq!(hit["snippet"], "Send the greeting to the lounge");
}

#[tokio::test]
async fn a_blank_query_is_refused() {
    let s = state().await;
    for q in [None, Some(""), Some("   ")] {
        let (status, _) = search_route(&s, q, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "q = {q:?}");
    }
}

#[test]
fn the_kernel_registers_the_search_route() {
    let wiring = include_str!("../src/lib.rs");
    for needle in ["\"/chat/search\"", "handlers::chat::search_messages"] {
        assert!(wiring.contains(needle), "{needle} is not wired in lib.rs");
    }
}
