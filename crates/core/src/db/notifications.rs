//! The notification store — where an item that is waiting for a person lives.
//!
//! Three kinds share one store because they share one destination (the reader's
//! inbox), while staying distinguishable by what they do to the agent that
//! raised them:
//!
//! - `approval` holds the agent until someone answers.
//! - `proposal` holds nothing; the agent asks and carries on.
//! - `notice` only informs.
//!
//! The kernel already emits events for the first and the third, but an event is
//! gone the moment it is broadcast. Whoever was not looking at the screen never
//! learns that they were asked, and a restart erases the fact entirely. This
//! module is the part that outlives both.
//!
//! Severity is stored as the RFC 5424 identifier and reuses
//! [`cloto_shared::McpLogLevel`] — the same eight values the MCP logging levels
//! carry. That reuse is the point: a private three-value scale here would have to
//! be mapped onto those eight by hand, and hand-maintained tables drift.

use chrono::{DateTime, Utc};
use cloto_shared::McpLogLevel;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::db_timeout;

/// What an item does to the agent that raised it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationKind {
    /// Holds the agent until someone answers.
    Approval,
    /// The agent asks and carries on; there is no deadline and nothing waits.
    Proposal,
    /// Informational only.
    Notice,
}

/// One row of the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationItem {
    /// Stable id chosen by the producer (an approval id, a tool call id). The
    /// side that later resolves the item finds its row by this.
    pub item_id: String,
    pub kind: NotificationKind,
    pub severity: McpLogLevel,
    pub agent_id: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub created_at: DateTime<Utc>,
    pub read_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    /// How the item settled, in the producer's own words.
    pub decision: Option<String>,
    /// Whether this item is holding an agent right now. Not derived from
    /// severity: a reader filtering by severity must still see everything that
    /// is stuck, or the filter starves agents silently.
    pub blocking: bool,
    pub metadata: Option<serde_json::Value>,
}

impl NotificationItem {
    /// A new unread, unresolved item.
    pub fn new(
        item_id: impl Into<String>,
        kind: NotificationKind,
        severity: McpLogLevel,
        title: impl Into<String>,
    ) -> Self {
        Self {
            item_id: item_id.into(),
            kind,
            severity,
            agent_id: None,
            title: title.into(),
            body: None,
            created_at: Utc::now(),
            read_at: None,
            resolved_at: None,
            decision: None,
            blocking: false,
            metadata: None,
        }
    }

    #[must_use]
    pub fn agent(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    #[must_use]
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Mark that this item is holding its agent.
    #[must_use]
    pub fn blocking(mut self) -> Self {
        self.blocking = true;
        self
    }

    #[must_use]
    pub fn metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// The identifier for a severity, read out of the enum's own serde form.
///
/// Written this way rather than as a `match` on purpose: a second list of the
/// eight values here is exactly the drift that reusing the shared enum was meant
/// to prevent.
fn severity_identifier(severity: McpLogLevel) -> String {
    match serde_json::to_value(severity) {
        Ok(serde_json::Value::String(s)) => s,
        // Unreachable for a unit-variant enum, and a database write is the wrong
        // place to panic over it.
        _ => "info".to_string(),
    }
}

fn severity_from_identifier(raw: &str) -> McpLogLevel {
    serde_json::from_value(serde_json::Value::String(raw.to_string())).unwrap_or(McpLogLevel::Info)
}

fn kind_identifier(kind: NotificationKind) -> String {
    match serde_json::to_value(kind) {
        Ok(serde_json::Value::String(s)) => s,
        _ => "notice".to_string(),
    }
}

fn kind_from_identifier(raw: &str) -> NotificationKind {
    serde_json::from_value(serde_json::Value::String(raw.to_string()))
        .unwrap_or(NotificationKind::Notice)
}

fn parse_time(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw).map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc))
}

type NotificationRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
);

fn row_to_item(row: NotificationRow) -> NotificationItem {
    let (
        item_id,
        kind,
        severity,
        agent_id,
        title,
        body,
        created_at,
        read_at,
        resolved_at,
        decision,
        blocking,
        metadata,
    ) = row;
    NotificationItem {
        item_id,
        kind: kind_from_identifier(&kind),
        severity: severity_from_identifier(&severity),
        agent_id,
        title,
        body,
        created_at: parse_time(&created_at),
        read_at: read_at.as_deref().map(parse_time),
        resolved_at: resolved_at.as_deref().map(parse_time),
        decision,
        blocking: blocking != 0,
        metadata: metadata.and_then(|raw| serde_json::from_str(&raw).ok()),
    }
}

const SELECT_COLUMNS: &str = "item_id, kind, severity, agent_id, title, body, created_at, \
                              read_at, resolved_at, decision, blocking, metadata";

/// Write one item to the store.
///
/// A duplicate `item_id` is an error rather than a silent no-op: the producers
/// mint their own ids, so a collision means two different things were given the
/// same name and one of them would otherwise vanish.
pub async fn record_notification(pool: &SqlitePool, item: NotificationItem) -> anyhow::Result<()> {
    let query_future = sqlx::query(
        "INSERT INTO notifications (item_id, kind, severity, agent_id, title, body, created_at, \
         read_at, resolved_at, decision, blocking, metadata)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&item.item_id)
    .bind(kind_identifier(item.kind))
    .bind(severity_identifier(item.severity))
    .bind(&item.agent_id)
    .bind(&item.title)
    .bind(&item.body)
    .bind(item.created_at.to_rfc3339())
    .bind(item.read_at.map(|t| t.to_rfc3339()))
    .bind(item.resolved_at.map(|t| t.to_rfc3339()))
    .bind(&item.decision)
    .bind(i64::from(item.blocking))
    .bind(item.metadata.as_ref().map(ToString::to_string))
    .execute(pool);

    db_timeout(query_future).await?;
    Ok(())
}

/// Write one item, treating a repeat of the same `item_id` as the same item.
///
/// For producers whose id is derived from what is being waited on rather than
/// minted fresh each time — an MCP server blocked on a capability is one waiting
/// item no matter how many times the start is retried. Returns whether a row was
/// actually written.
pub async fn record_notification_once(
    pool: &SqlitePool,
    item: NotificationItem,
) -> anyhow::Result<bool> {
    let query_future = sqlx::query(
        "INSERT INTO notifications (item_id, kind, severity, agent_id, title, body, created_at, \
         read_at, resolved_at, decision, blocking, metadata)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(item_id) DO NOTHING",
    )
    .bind(&item.item_id)
    .bind(kind_identifier(item.kind))
    .bind(severity_identifier(item.severity))
    .bind(&item.agent_id)
    .bind(&item.title)
    .bind(&item.body)
    .bind(item.created_at.to_rfc3339())
    .bind(item.read_at.map(|t| t.to_rfc3339()))
    .bind(item.resolved_at.map(|t| t.to_rfc3339()))
    .bind(&item.decision)
    .bind(i64::from(item.blocking))
    .bind(item.metadata.as_ref().map(ToString::to_string))
    .execute(pool);

    Ok(db_timeout(query_future).await?.rows_affected() > 0)
}

/// Write one item without making the caller wait for the disk.
///
/// The same shape as [`super::spawn_audit_log`], and for the same reason: a
/// notice about something that already happened must not be able to slow down
/// the thing that happened. **Not** for an `approval` — there the row has to
/// exist before the agent starts waiting, or the bell can show an empty inbox
/// while an agent is stuck behind it.
pub fn spawn_notification(pool: SqlitePool, item: NotificationItem) {
    tokio::spawn(async move {
        let item_id = item.item_id.clone();
        if let Err(e) = record_notification(&pool, item).await {
            tracing::error!(item_id = %item_id, "Failed to record notification: {}", e);
        }
    });
}

/// Read one item back.
pub async fn get_notification(
    pool: &SqlitePool,
    item_id: &str,
) -> anyhow::Result<Option<NotificationItem>> {
    // The audit `AssertSqlSafe` asks for: nothing derived from `item_id` reaches
    // the SQL text. The only thing interpolated is `SELECT_COLUMNS`, a constant
    // in this file, and the id itself is `bind`ed and travels as a parameter.
    // The constant is shared with the listing below so the column order cannot
    // drift away from the tuple both decode into.
    let sql = format!("SELECT {SELECT_COLUMNS} FROM notifications WHERE item_id = ?");
    let query_future = sqlx::query_as::<_, NotificationRow>(sqlx::AssertSqlSafe(sql))
        .bind(item_id)
        .fetch_optional(pool);

    Ok(db_timeout(query_future).await?.map(row_to_item))
}

/// Newest first. `unresolved_only` narrows to what is still waiting on someone.
pub async fn list_notifications(
    pool: &SqlitePool,
    limit: i64,
    unresolved_only: bool,
) -> anyhow::Result<Vec<NotificationItem>> {
    let filter = if unresolved_only {
        "WHERE resolved_at IS NULL "
    } else {
        ""
    };
    // The audit `AssertSqlSafe` asks for: the two interpolations are both
    // constants in this file — `SELECT_COLUMNS`, and a `filter` picked from two
    // literals by a `bool`. Neither varies with caller data; `limit` is `bind`ed
    // and travels as a parameter.
    let sql =
        format!("SELECT {SELECT_COLUMNS} FROM notifications {filter}ORDER BY id DESC LIMIT ?");
    let query_future = sqlx::query_as::<_, NotificationRow>(sqlx::AssertSqlSafe(sql))
        .bind(limit)
        .fetch_all(pool);

    Ok(db_timeout(query_future)
        .await?
        .into_iter()
        .map(row_to_item)
        .collect())
}

/// Note that a reader has seen the item. Returns whether a row changed, so a
/// caller can tell "already read" apart from "no such item".
pub async fn mark_notification_read(pool: &SqlitePool, item_id: &str) -> anyhow::Result<bool> {
    let query_future =
        sqlx::query("UPDATE notifications SET read_at = ? WHERE item_id = ? AND read_at IS NULL")
            .bind(Utc::now().to_rfc3339())
            .bind(item_id)
            .execute(pool);

    Ok(db_timeout(query_future).await?.rows_affected() > 0)
}

/// Settle an item: record how it ended and stop it from blocking.
///
/// Resolving is separate from reading on purpose — seeing that you were asked is
/// not answering, and the bell has to keep showing the ones that were only read.
pub async fn resolve_notification(
    pool: &SqlitePool,
    item_id: &str,
    decision: &str,
) -> anyhow::Result<bool> {
    let query_future = sqlx::query(
        "UPDATE notifications SET resolved_at = ?, decision = ?, blocking = 0 \
         WHERE item_id = ? AND resolved_at IS NULL",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(decision)
    .bind(item_id)
    .execute(pool);

    Ok(db_timeout(query_future).await?.rows_affected() > 0)
}
