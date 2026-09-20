//! Unread on the roster: an agent that has *said something* unread, as opposed
//! to one that is waiting on an answer.
//!
//! The notification store answers the second question only, so a scheduled run
//! that reported, or a reply that landed after the person walked away, left no
//! mark anywhere. These pin what counts as unread, what clears it, and that the
//! migration does not light up every agent at once on the day it lands.

use cloto_core::db;
use sqlx::SqlitePool;

const MIGRATION: &str =
    include_str!("../../migrations/20260921000000_add_conversation_last_read.sql");

async fn pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::init_db(&pool, "sqlite::memory:", None).await.unwrap();
    sqlx::query("INSERT INTO agents (id, name, description, status, default_engine_id, required_capabilities, metadata, enabled) VALUES ('agent.a', 'A', '', 'online', 'engine.test', '[]', '{}', 1)")
        .execute(&pool).await.unwrap();
    pool
}

/// A conversation with an explicit read time, so each test states its own
/// starting point rather than inheriting one.
async fn conversation(pool: &SqlitePool, id: &str, updated_at: i64, last_read_at: Option<i64>) {
    sqlx::query(
        "INSERT INTO conversations (id, agent_id, user_id, title, created_at, updated_at, archived_at, last_read_at)
         VALUES (?, 'agent.a', 'default', '', 0, ?, NULL, ?)",
    )
    .bind(id)
    .bind(updated_at)
    .bind(last_read_at)
    .execute(pool)
    .await
    .unwrap();
}

async fn message(pool: &SqlitePool, id: &str, conversation_id: &str, source: &str, at: i64) {
    sqlx::query(
        "INSERT INTO chat_messages (id, agent_id, user_id, source, content, created_at, conversation_id)
         VALUES (?, 'agent.a', 'default', ?, '[]', ?, ?)",
    )
    .bind(id)
    .bind(source)
    .bind(at)
    .bind(conversation_id)
    .execute(pool)
    .await
    .unwrap();
}

async fn unread(pool: &SqlitePool) -> Vec<String> {
    db::agents_with_unread(pool, "default").await.unwrap()
}

#[tokio::test]
async fn an_agent_that_spoke_after_the_last_look_is_unread() {
    let p = pool().await;
    conversation(&p, "c1", 200, Some(100)).await;
    message(&p, "m1", "c1", "agent", 200).await;
    assert_eq!(unread(&p).await, vec!["agent.a".to_string()]);
}

#[tokio::test]
async fn an_agent_that_spoke_before_the_last_look_is_read() {
    let p = pool().await;
    conversation(&p, "c1", 200, Some(300)).await;
    message(&p, "m1", "c1", "agent", 200).await;
    assert!(unread(&p).await.is_empty());
}

/// The person's own last line must not mark their own conversation unread —
/// that would put a mark on every agent they ever wrote to and walked away from,
/// which says the opposite of what happened.
#[tokio::test]
async fn the_persons_own_message_does_not_make_it_unread() {
    let p = pool().await;
    conversation(&p, "c1", 400, Some(100)).await;
    message(&p, "m1", "c1", "user", 400).await;
    assert!(unread(&p).await.is_empty());
}

/// `system` is the third source rows carry. It is the kernel's own bookkeeping,
/// not the agent addressing the person, so it does not raise a mark either.
#[tokio::test]
async fn a_system_message_does_not_make_it_unread() {
    let p = pool().await;
    conversation(&p, "c1", 400, Some(100)).await;
    message(&p, "m1", "c1", "system", 400).await;
    assert!(unread(&p).await.is_empty());
}

/// A thread the agent started, that the person has never opened.
#[tokio::test]
async fn a_conversation_never_opened_is_unread() {
    let p = pool().await;
    conversation(&p, "c1", 200, None).await;
    message(&p, "m1", "c1", "agent", 200).await;
    assert_eq!(unread(&p).await, vec!["agent.a".to_string()]);
}

/// Archiving is how a thread is put away, and something put away is not waiting.
#[tokio::test]
async fn an_archived_conversation_is_not_unread() {
    let p = pool().await;
    conversation(&p, "c1", 200, None).await;
    message(&p, "m1", "c1", "agent", 200).await;
    sqlx::query("UPDATE conversations SET archived_at = 500 WHERE id = 'c1'")
        .execute(&p)
        .await
        .unwrap();
    assert!(unread(&p).await.is_empty());
}

#[tokio::test]
async fn looking_at_the_conversation_clears_it() {
    let p = pool().await;
    conversation(&p, "c1", 200, Some(100)).await;
    message(&p, "m1", "c1", "agent", 200).await;
    assert!(
        !unread(&p).await.is_empty(),
        "the mark has to be there to be cleared"
    );

    assert!(db::mark_conversation_read(&p, "c1", 300).await.unwrap());
    assert!(unread(&p).await.is_empty());
}

/// One agent, two threads: reading one does not speak for the other. The row
/// keeps its mark while anything the agent said is still unread.
#[tokio::test]
async fn reading_one_thread_does_not_clear_another() {
    let p = pool().await;
    conversation(&p, "c1", 200, Some(100)).await;
    message(&p, "m1", "c1", "agent", 200).await;
    conversation(&p, "c2", 200, Some(100)).await;
    message(&p, "m2", "c2", "agent", 200).await;

    db::mark_conversation_read(&p, "c1", 300).await.unwrap();
    assert_eq!(unread(&p).await, vec!["agent.a".to_string()]);

    db::mark_conversation_read(&p, "c2", 300).await.unwrap();
    assert!(unread(&p).await.is_empty());
}

/// An agent is named once however much it said, because the roster draws one
/// mark per row.
#[tokio::test]
async fn an_agent_is_named_once_however_much_it_said() {
    let p = pool().await;
    conversation(&p, "c1", 200, Some(100)).await;
    message(&p, "m1", "c1", "agent", 150).await;
    message(&p, "m2", "c1", "agent", 200).await;
    conversation(&p, "c2", 200, None).await;
    message(&p, "m3", "c2", "agent", 200).await;
    assert_eq!(unread(&p).await, vec!["agent.a".to_string()]);
}

/// The statement is taken out of the migration itself, so this measures what
/// ships rather than a second copy of it written here.
///
/// Without the backfill, every conversation that existed before this migration
/// reads as never opened, and the roster lights up beside every agent on the day
/// it lands — for history the person has already seen.
#[tokio::test]
async fn the_migration_backfills_existing_conversations_as_read() {
    let p = pool().await;
    conversation(&p, "c1", 200, None).await;
    message(&p, "m1", "c1", "agent", 200).await;
    assert!(
        !unread(&p).await.is_empty(),
        "unread before the backfill runs"
    );

    let backfill = MIGRATION
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("UPDATE conversations"))
        .expect("the migration no longer carries a backfill statement");
    assert!(
        backfill.contains("last_read_at") && backfill.contains("updated_at"),
        "the backfill does not relate the two columns: {backfill}"
    );
    sqlx::query(sqlx::AssertSqlSafe(
        backfill.trim_end_matches(';').to_string(),
    ))
    .execute(&p)
    .await
    .unwrap();

    assert!(
        unread(&p).await.is_empty(),
        "history that existed before this migration should not arrive unread"
    );
}

/// Both routes are registered.
///
/// The kernel's router is built inline during boot, so a running kernel cannot
/// be asked what it registered, and a handler test passes just as happily for a
/// handler nobody routes to. Measured once by deleting a registration: no other
/// test went red.
#[test]
fn the_unread_routes_are_registered() {
    let lib_rs = include_str!("../../src/lib.rs");
    for route in [
        "\"/chat/unread\"",
        "\"/chat/{agent_id}/conversations/{conversation_id}/read\"",
    ] {
        assert!(
            lib_rs.contains(route),
            "{route} is not registered; its handler would be unreachable"
        );
    }
    assert!(
        lib_rs.contains("handlers::chat::unread_agents"),
        "the unread route does not reach its handler"
    );
    assert!(
        lib_rs.contains("handlers::chat::mark_conversation_read"),
        "the read route does not reach its handler"
    );
}
