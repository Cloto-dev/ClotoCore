//! The store that lets a question outlive the moment it was asked.
//!
//! Two things are being checked here, and they are different questions. The
//! round-trip tests ask whether the store keeps what it was given. The wiring
//! assertions at the bottom ask whether the kernel actually writes to it — a
//! store with perfect round-trips that nothing calls is an empty inbox, and no
//! test of the store itself can tell you that.

use cloto_core::db::{
    get_notification, list_notifications, mark_notification_read, record_notification,
    record_notification_once, resolve_notification, NotificationItem, NotificationKind,
};
use cloto_shared::McpLogLevel;
use sqlx::SqlitePool;

async fn memory_store() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();
    pool
}

#[tokio::test]
async fn an_item_comes_back_with_everything_it_was_given() {
    let pool = memory_store().await;

    let item = NotificationItem::new(
        "item-1",
        NotificationKind::Approval,
        McpLogLevel::Critical,
        "A destructive command is waiting",
    )
    .agent("agent.alpha")
    .body("rm -rf /var/lib/something")
    .blocking()
    .metadata(serde_json::json!({ "call_id": "c1" }));

    record_notification(&pool, item).await.expect("record");

    let read = get_notification(&pool, "item-1")
        .await
        .expect("read")
        .expect("the item is there");
    assert_eq!(read.kind, NotificationKind::Approval);
    assert_eq!(
        read.severity,
        McpLogLevel::Critical,
        "severity survives as the RFC 5424 identifier rather than being flattened \
         into a display scale"
    );
    assert_eq!(read.agent_id.as_deref(), Some("agent.alpha"));
    assert_eq!(read.title, "A destructive command is waiting");
    assert_eq!(read.body.as_deref(), Some("rm -rf /var/lib/something"));
    assert!(read.blocking);
    assert!(read.read_at.is_none());
    assert!(read.resolved_at.is_none());
    assert_eq!(
        read.metadata
            .and_then(|m| m["call_id"].as_str().map(String::from)),
        Some("c1".to_string())
    );
}

#[tokio::test]
async fn the_stored_severity_is_the_rfc_5424_word_the_rest_of_the_kernel_uses() {
    let pool = memory_store().await;
    record_notification(
        &pool,
        NotificationItem::new("sev", NotificationKind::Notice, McpLogLevel::Warning, "t"),
    )
    .await
    .expect("record");

    // Read the raw column, not the decoded struct: the point of reusing the
    // shared enum is that what lands on disk is the same word MCP logging uses,
    // so a reader joining the two never needs a translation table.
    let raw: String = sqlx::query_scalar("SELECT severity FROM notifications WHERE item_id = ?")
        .bind("sev")
        .fetch_one(&pool)
        .await
        .expect("column read");
    assert_eq!(raw, "warning");
}

#[tokio::test]
async fn an_item_outlives_the_process_that_wrote_it() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cloto_memories.db");
    let url = format!("sqlite:{}", db_path.display());

    {
        let pool = cloto_core::open_kernel_db(&url, None).await.expect("open");
        cloto_core::db::init_db(&pool, &url, None)
            .await
            .expect("init");
        record_notification(
            &pool,
            NotificationItem::new(
                "survives",
                NotificationKind::Approval,
                McpLogLevel::Warning,
                "Asked before the restart",
            )
            .blocking(),
        )
        .await
        .expect("record");
        pool.close().await;
    }

    // A second kernel, on the same file. This is the whole reason the store is
    // not a map in memory: the pending approvals map is a oneshot channel and
    // dies with the process, but "you were asked" has to still be true after it.
    let pool = cloto_core::open_kernel_db(&url, None)
        .await
        .expect("reopen");
    let read = get_notification(&pool, "survives")
        .await
        .expect("read")
        .expect("the item is still there after the restart");
    assert_eq!(read.title, "Asked before the restart");
    assert!(read.blocking);
}

#[tokio::test]
async fn reading_an_item_is_not_answering_it() {
    let pool = memory_store().await;
    record_notification(
        &pool,
        NotificationItem::new("r1", NotificationKind::Approval, McpLogLevel::Error, "t").blocking(),
    )
    .await
    .expect("record");

    assert!(mark_notification_read(&pool, "r1")
        .await
        .expect("read mark"));
    assert!(
        !mark_notification_read(&pool, "r1")
            .await
            .expect("read mark"),
        "a second read changes nothing, so a caller can tell 'already seen' from 'no such item'"
    );

    let seen = get_notification(&pool, "r1")
        .await
        .expect("read")
        .expect("present");
    assert!(seen.read_at.is_some());
    assert!(
        seen.resolved_at.is_none() && seen.blocking,
        "seeing that you were asked is not answering: the item is still holding its agent"
    );

    assert_eq!(
        list_notifications(&pool, 10, true)
            .await
            .expect("list")
            .len(),
        1,
        "and it is still in the unresolved list"
    );
}

#[tokio::test]
async fn a_settled_item_leaves_the_unresolved_list_but_not_the_store() {
    let pool = memory_store().await;
    for id in ["a", "b"] {
        record_notification(
            &pool,
            NotificationItem::new(id, NotificationKind::Notice, McpLogLevel::Info, "t"),
        )
        .await
        .expect("record");
    }

    assert!(resolve_notification(&pool, "a", "approved")
        .await
        .expect("resolve"));
    assert!(
        !resolve_notification(&pool, "a", "approved twice")
            .await
            .expect("resolve"),
        "settling an item twice must not overwrite how it ended the first time"
    );

    let unresolved = list_notifications(&pool, 10, true).await.expect("list");
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].item_id, "b");

    let everything = list_notifications(&pool, 10, false).await.expect("list");
    assert_eq!(everything.len(), 2, "history is kept, not deleted");
    assert_eq!(
        everything[0].item_id, "b",
        "newest first, so the bell opens on what just happened"
    );
}

#[tokio::test]
async fn a_repeated_id_is_one_item_only_where_the_producer_says_so() {
    let pool = memory_store().await;
    let item =
        || NotificationItem::new("dup", NotificationKind::Approval, McpLogLevel::Warning, "t");

    assert!(record_notification_once(&pool, item())
        .await
        .expect("first write"));
    assert!(
        !record_notification_once(&pool, item())
            .await
            .expect("second write"),
        "a retried start of the same blocked server is the same waiting item"
    );

    // The strict path is the other half of that choice: producers that mint a
    // fresh id per request must hear about a collision rather than have one of
    // the two requests silently disappear.
    assert!(
        record_notification(&pool, item()).await.is_err(),
        "a duplicate id on the strict path is an error, not a quiet no-op"
    );

    assert_eq!(
        list_notifications(&pool, 10, false)
            .await
            .expect("list")
            .len(),
        1
    );
}

// ── Is the kernel actually writing to any of this? ──────────────────────────
//
// Testing the handlers answers "does this function work", never "is this
// function reached". The approval gate is covered by driving the real gate (see
// the tests beside it in `handlers::command_approval`); the three remaining
// producers are reached only from inside long-running loops, so what is checked
// here is that the call still sits at the site that emits the matching event.
// Delete one of those calls and this file goes red.

#[test]
fn the_capability_escalation_path_records_what_it_is_blocked_on() {
    let wiring = include_str!("../src/managers/mcp.rs");
    assert!(
        wiring.contains("ClotoEventData::PermissionRequested"),
        "the event this is anchored to has moved — re-anchor the assertion rather than deleting it"
    );
    assert!(
        wiring.contains("crate::db::record_notification_once"),
        "a server blocked on a capability announces it and leaves nothing behind: \
         whoever was not watching at that second never learns the server is stuck"
    );
}

#[test]
fn the_tool_rejection_path_records_the_rejection() {
    let wiring = include_str!("../src/handlers/system.rs");
    assert!(
        wiring.contains("ClotoEventData::ToolRejected"),
        "the event this is anchored to has moved — re-anchor the assertion rather than deleting it"
    );
    assert!(
        wiring.contains("crate::db::spawn_notification"),
        "a rejection reaches the audit log and the screen, but nothing a reader \
         can come back to"
    );
}

#[test]
fn the_shutdown_path_records_why_the_kernel_stopped() {
    let wiring = include_str!("../src/handlers.rs");
    assert!(
        wiring.contains("SystemNotification"),
        "the event this is anchored to has moved — re-anchor the assertion rather than deleting it"
    );
    assert!(
        wiring.contains("crate::db::spawn_notification"),
        "why a kernel is not running is exactly what someone looks for afterwards"
    );
}
