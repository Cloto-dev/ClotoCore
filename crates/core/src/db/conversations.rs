//! Conversations: the persistent unit of the dashboard chat
//! (`docs/CONVERSATIONS_DESIGN.md`).
//!
//! A conversation is a thread a person leaves and returns to. It never ends;
//! it is archived (hidden, kept whole, reversible) or deleted (immediate).
//! The model reads a conversation's own messages as its context, so the rows
//! here are what the person sees *and* what the model sees.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::chat::get_disk_attachment_paths;
use super::db_timeout;

/// The conversation a message lands in when it names none: one per
/// `(agent, user)`, created on demand, and the same id the migration gave the
/// history that existed before conversations did.
#[must_use]
pub fn default_conversation_id(agent_id: &str, user_id: &str) -> String {
    format!("default:{agent_id}:{user_id}")
}

/// A title cut from the first line of the first message, before the model
/// names the conversation.
pub const FIRST_LINE_TITLE_CHARS: usize = 60;

#[must_use]
pub fn title_from_first_message(content: &str) -> String {
    let line = content.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let line = line.trim();
    if line.chars().count() <= FIRST_LINE_TITLE_CHARS {
        return line.to_string();
    }
    let mut cut: String = line.chars().take(FIRST_LINE_TITLE_CHARS).collect();
    cut.push('…');
    cut
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConversationRow {
    pub id: String,
    pub agent_id: String,
    pub user_id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
}

/// A conversation as the list shows it: the row plus how many messages it holds.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConversationListItem {
    pub id: String,
    pub agent_id: String,
    pub user_id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
    pub message_count: i64,
}

/// Create a conversation with a fresh id.
pub async fn create_conversation(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: &str,
    now_ms: i64,
) -> anyhow::Result<ConversationRow> {
    let id = uuid::Uuid::new_v4().to_string();
    insert_conversation(pool, &id, agent_id, user_id, "", now_ms).await?;
    Ok(ConversationRow {
        id,
        agent_id: agent_id.to_string(),
        user_id: user_id.to_string(),
        title: String::new(),
        created_at: now_ms,
        updated_at: now_ms,
        archived_at: None,
    })
}

async fn insert_conversation(
    pool: &SqlitePool,
    id: &str,
    agent_id: &str,
    user_id: &str,
    title: &str,
    now_ms: i64,
) -> anyhow::Result<()> {
    db_timeout(
        sqlx::query(
            "INSERT INTO conversations (id, agent_id, user_id, title, created_at, updated_at, archived_at)
             VALUES (?, ?, ?, ?, ?, ?, NULL)",
        )
        .bind(id)
        .bind(agent_id)
        .bind(user_id)
        .bind(title)
        .bind(now_ms)
        .bind(now_ms)
        .execute(pool),
    )
    .await?;
    Ok(())
}

/// The conversation a message without an id lands in, created if absent.
pub async fn ensure_default_conversation(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: &str,
    now_ms: i64,
) -> anyhow::Result<String> {
    let id = default_conversation_id(agent_id, user_id);
    db_timeout(
        sqlx::query(
            "INSERT OR IGNORE INTO conversations (id, agent_id, user_id, title, created_at, updated_at, archived_at)
             VALUES (?, ?, ?, '', ?, ?, NULL)",
        )
        .bind(&id)
        .bind(agent_id)
        .bind(user_id)
        .bind(now_ms)
        .bind(now_ms)
        .execute(pool),
    )
    .await?;
    Ok(id)
}

pub async fn get_conversation(
    pool: &SqlitePool,
    id: &str,
) -> anyhow::Result<Option<ConversationRow>> {
    db_timeout(
        sqlx::query_as::<_, ConversationRow>(
            "SELECT id, agent_id, user_id, title, created_at, updated_at, archived_at
             FROM conversations WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(pool),
    )
    .await
}

/// List an agent's conversations for a user, newest activity first. System
/// conversations (cron output) are listed alongside the user's, as the
/// message list has always shown system rows next to the user's own.
pub async fn list_conversations(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: &str,
    include_archived: bool,
) -> anyhow::Result<Vec<ConversationListItem>> {
    let archived_clause = if include_archived {
        ""
    } else {
        "AND c.archived_at IS NULL"
    };
    // `archived_clause` is one of two literals chosen above; nothing from the
    // caller reaches the SQL text.
    let sql = format!(
        "SELECT c.id, c.agent_id, c.user_id, c.title, c.created_at, c.updated_at, c.archived_at,
                (SELECT COUNT(*) FROM chat_messages m WHERE m.conversation_id = c.id) AS message_count
         FROM conversations c
         WHERE c.agent_id = ? AND (c.user_id = ? OR c.user_id = 'system') {archived_clause}
         ORDER BY c.updated_at DESC"
    );
    db_timeout(
        sqlx::query_as::<_, ConversationListItem>(sqlx::AssertSqlSafe(sql))
            .bind(agent_id)
            .bind(user_id)
            .fetch_all(pool),
    )
    .await
}

pub async fn rename_conversation(pool: &SqlitePool, id: &str, title: &str) -> anyhow::Result<bool> {
    let result = db_timeout(
        sqlx::query("UPDATE conversations SET title = ? WHERE id = ?")
            .bind(title)
            .bind(id)
            .execute(pool),
    )
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Give a conversation its first title, and only its first: a title already
/// set — by hand or by the model — is never overwritten here.
pub async fn set_title_if_empty(pool: &SqlitePool, id: &str, title: &str) -> anyhow::Result<bool> {
    let result = db_timeout(
        sqlx::query("UPDATE conversations SET title = ? WHERE id = ? AND title = ''")
            .bind(title)
            .bind(id)
            .execute(pool),
    )
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Archive (`Some(now)`) or unarchive (`None`). `updated_at` is left alone so an
/// unarchived conversation returns to its old place in the list.
pub async fn set_conversation_archived(
    pool: &SqlitePool,
    id: &str,
    archived_at: Option<i64>,
) -> anyhow::Result<bool> {
    let result = db_timeout(
        sqlx::query("UPDATE conversations SET archived_at = ? WHERE id = ?")
            .bind(archived_at)
            .bind(id)
            .execute(pool),
    )
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Record activity: the newest message's time.
pub async fn touch_conversation(pool: &SqlitePool, id: &str, now_ms: i64) -> anyhow::Result<()> {
    db_timeout(
        sqlx::query("UPDATE conversations SET updated_at = MAX(updated_at, ?) WHERE id = ?")
            .bind(now_ms)
            .bind(id)
            .execute(pool),
    )
    .await?;
    Ok(())
}

pub async fn count_messages(pool: &SqlitePool, id: &str) -> anyhow::Result<i64> {
    let (n,): (i64,) = db_timeout(
        sqlx::query_as("SELECT COUNT(*) FROM chat_messages WHERE conversation_id = ?")
            .bind(id)
            .fetch_one(pool),
    )
    .await?;
    Ok(n)
}

/// Delete a conversation and its messages. Attachments cascade; disk files are
/// removed best-effort. Returns the number of messages removed, or `None` when
/// no such conversation exists.
pub async fn delete_conversation(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<u64>> {
    if get_conversation(pool, id).await?.is_none() {
        return Ok(None);
    }
    let ids: Vec<String> = db_timeout(
        sqlx::query_as::<_, (String,)>("SELECT id FROM chat_messages WHERE conversation_id = ?")
            .bind(id)
            .fetch_all(pool),
    )
    .await?
    .into_iter()
    .map(|(m,)| m)
    .collect();
    let disk_paths = get_disk_attachment_paths(pool, &ids).await?;

    let deleted = db_timeout(
        sqlx::query("DELETE FROM chat_messages WHERE conversation_id = ?")
            .bind(id)
            .execute(pool),
    )
    .await?
    .rows_affected();
    db_timeout(
        sqlx::query("DELETE FROM conversations WHERE id = ?")
            .bind(id)
            .execute(pool),
    )
    .await?;

    for path in disk_paths {
        let _ = tokio::fs::remove_file(&path).await;
    }
    Ok(Some(deleted))
}

/// Archive every live conversation of an agent for a user. Returns how many.
pub async fn archive_all_conversations(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: &str,
    now_ms: i64,
) -> anyhow::Result<u64> {
    let result = db_timeout(
        sqlx::query(
            "UPDATE conversations SET archived_at = ?
             WHERE agent_id = ? AND (user_id = ? OR user_id = 'system') AND archived_at IS NULL",
        )
        .bind(now_ms)
        .bind(agent_id)
        .bind(user_id)
        .execute(pool),
    )
    .await?;
    Ok(result.rows_affected())
}

/// Delete every conversation of an agent for a user, archived ones included.
/// Returns how many conversations were removed.
pub async fn delete_all_conversations(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: &str,
) -> anyhow::Result<u64> {
    let ids: Vec<String> = db_timeout(
        sqlx::query_as::<_, (String,)>(
            "SELECT id FROM conversations WHERE agent_id = ? AND (user_id = ? OR user_id = 'system')",
        )
        .bind(agent_id)
        .bind(user_id)
        .fetch_all(pool),
    )
    .await?
    .into_iter()
    .map(|(id,)| id)
    .collect();
    let mut removed = 0;
    for id in &ids {
        if delete_conversation(pool, id).await?.is_some() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// The turns the model reads: the conversation's newest `budget` messages,
/// returned oldest-first. Branches are not collapsed here — every stored turn
/// of the conversation is a turn the person saw.
pub async fn get_conversation_context(
    pool: &SqlitePool,
    conversation_id: &str,
    budget: usize,
) -> anyhow::Result<Vec<super::chat::ChatMessageRow>> {
    #[allow(clippy::cast_possible_wrap)]
    let limit = budget as i64;
    let mut rows =
        super::chat::get_chat_messages_in_conversation(pool, conversation_id, limit).await?;
    rows.reverse();
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_title_is_the_first_non_empty_line_cut_to_length() {
        assert_eq!(
            title_from_first_message("\n\n  hello there  \nsecond"),
            "hello there"
        );
        let long = "x".repeat(FIRST_LINE_TITLE_CHARS + 5);
        let t = title_from_first_message(&long);
        assert_eq!(t.chars().count(), FIRST_LINE_TITLE_CHARS + 1);
        assert!(t.ends_with('…'));
        assert_eq!(title_from_first_message("   \n \n"), "");
    }
}
