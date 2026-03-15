use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::db_timeout;

/// Retry delay in milliseconds for chat message insertion on conflict.
const CHAT_RETRY_DELAY_MS: u64 = 200;

// ─── Chat Persistence Layer ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageRow {
    pub id: String,
    pub agent_id: String,
    pub user_id: String,
    pub source: String,
    pub content: String, // JSON string of ContentBlock[]
    pub metadata: Option<String>,
    pub created_at: i64,
    pub parent_id: Option<String>,
    pub branch_index: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AttachmentRow {
    pub id: String,
    pub message_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub storage_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_data: Option<Vec<u8>>,
    pub disk_path: Option<String>,
    pub created_at: i64,
}

/// Save a chat message to the database
pub async fn save_chat_message(pool: &SqlitePool, msg: &ChatMessageRow) -> anyhow::Result<()> {
    let query_future = sqlx::query(
        "INSERT INTO chat_messages (id, agent_id, user_id, source, content, metadata, created_at, parent_id, branch_index)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&msg.id)
    .bind(&msg.agent_id)
    .bind(&msg.user_id)
    .bind(&msg.source)
    .bind(&msg.content)
    .bind(&msg.metadata)
    .bind(msg.created_at)
    .bind(&msg.parent_id)
    .bind(msg.branch_index)
    .execute(pool);

    db_timeout(query_future).await?;

    Ok(())
}

/// Save a chat message with one retry on failure.
/// Logs context (agent_id, message_id) on error for traceability.
pub async fn save_chat_message_reliable(
    pool: &SqlitePool,
    msg: &ChatMessageRow,
) -> anyhow::Result<()> {
    match save_chat_message(pool, msg).await {
        Ok(()) => Ok(()),
        Err(first_err) => {
            tracing::warn!(
                agent_id = %msg.agent_id,
                message_id = %msg.id,
                source = %msg.source,
                "Chat persist failed (attempt 1/2): {}. Retrying...",
                first_err,
            );
            tokio::time::sleep(std::time::Duration::from_millis(CHAT_RETRY_DELAY_MS)).await;
            match save_chat_message(pool, msg).await {
                Ok(()) => {
                    tracing::info!(
                        agent_id = %msg.agent_id,
                        message_id = %msg.id,
                        "Chat persist succeeded on retry",
                    );
                    Ok(())
                }
                Err(retry_err) => {
                    tracing::error!(
                        agent_id = %msg.agent_id,
                        message_id = %msg.id,
                        source = %msg.source,
                        "Chat persist FAILED after 2 attempts: {}",
                        retry_err,
                    );
                    Err(retry_err)
                }
            }
        }
    }
}

/// Row type returned by chat message queries.
type ChatMessageTuple = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
    Option<String>,
    i32,
);

/// Get chat messages with cursor-based pagination (ordered by created_at DESC)
pub async fn get_chat_messages(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: &str,
    before_ts: Option<i64>,
    limit: i64,
    max_limit: i64,
) -> anyhow::Result<Vec<ChatMessageRow>> {
    let limit = limit.min(max_limit);

    // Include both the requesting user's messages AND system messages (CRON jobs).
    // Without this, messages persisted with user_id='system' are silently excluded (bug-317).
    let rows: Vec<ChatMessageTuple> = if let Some(before) = before_ts {
        let query_future = sqlx::query_as::<_, ChatMessageTuple>(
            "SELECT id, agent_id, user_id, source, content, metadata, created_at, parent_id, branch_index
             FROM chat_messages
             WHERE agent_id = ? AND (user_id = ? OR user_id = 'system') AND created_at < ?
             ORDER BY created_at DESC
             LIMIT ?",
        )
        .bind(agent_id)
        .bind(user_id)
        .bind(before)
        .bind(limit)
        .fetch_all(pool);

        db_timeout(query_future).await?
    } else {
        let query_future = sqlx::query_as::<_, ChatMessageTuple>(
            "SELECT id, agent_id, user_id, source, content, metadata, created_at, parent_id, branch_index
             FROM chat_messages
             WHERE agent_id = ? AND (user_id = ? OR user_id = 'system')
             ORDER BY created_at DESC
             LIMIT ?",
        )
        .bind(agent_id)
        .bind(user_id)
        .bind(limit)
        .fetch_all(pool);

        db_timeout(query_future).await?
    };

    let messages = rows
        .into_iter()
        .map(
            |(
                id,
                agent_id,
                user_id,
                source,
                content,
                metadata,
                created_at,
                parent_id,
                branch_index,
            )| ChatMessageRow {
                id,
                agent_id,
                user_id,
                source,
                content,
                metadata,
                created_at,
                parent_id,
                branch_index,
            },
        )
        .collect();

    Ok(messages)
}

/// Get the next available branch_index for a given parent_id
pub async fn get_next_branch_index(pool: &SqlitePool, parent_id: &str) -> anyhow::Result<i32> {
    let row: Option<(i32,)> = db_timeout(
        sqlx::query_as::<_, (i32,)>(
            "SELECT COALESCE(MAX(branch_index), -1) FROM chat_messages WHERE parent_id = ?",
        )
        .bind(parent_id)
        .fetch_optional(pool),
    )
    .await?;

    Ok(row.map_or(0, |(max,)| max + 1))
}

/// Get a single chat message by ID
pub async fn get_chat_message_by_id(
    pool: &SqlitePool,
    message_id: &str,
) -> anyhow::Result<Option<ChatMessageRow>> {
    let row: Option<ChatMessageTuple> = db_timeout(
        sqlx::query_as::<_, ChatMessageTuple>(
            "SELECT id, agent_id, user_id, source, content, metadata, created_at, parent_id, branch_index
             FROM chat_messages WHERE id = ?",
        )
        .bind(message_id)
        .fetch_optional(pool),
    )
    .await?;

    Ok(row.map(
        |(
            id,
            agent_id,
            user_id,
            source,
            content,
            metadata,
            created_at,
            parent_id,
            branch_index,
        )| {
            ChatMessageRow {
                id,
                agent_id,
                user_id,
                source,
                content,
                metadata,
                created_at,
                parent_id,
                branch_index,
            }
        },
    ))
}

/// Delete all chat messages (and cascade to attachments) for an agent/user pair
pub async fn delete_chat_messages(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: &str,
) -> anyhow::Result<u64> {
    // First get message IDs for disk attachment cleanup
    // Include system messages (CRON jobs) — mirrors the GET query (bug-317 follow-up)
    let ids_future = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM chat_messages WHERE agent_id = ? AND (user_id = ? OR user_id = 'system')",
    )
    .bind(agent_id)
    .bind(user_id)
    .fetch_all(pool);

    let msg_ids: Vec<String> = db_timeout(ids_future)
        .await?
        .into_iter()
        .map(|(id,)| id)
        .collect();

    // Get disk paths for cleanup
    let disk_paths = get_disk_attachment_paths(pool, &msg_ids).await?;

    // Delete messages (attachments cascade via ON DELETE CASCADE)
    let delete_future = sqlx::query(
        "DELETE FROM chat_messages WHERE agent_id = ? AND (user_id = ? OR user_id = 'system')",
    )
    .bind(agent_id)
    .bind(user_id)
    .execute(pool);

    let result = db_timeout(delete_future).await?;

    // Clean up disk files (best-effort)
    for path in disk_paths {
        let _ = tokio::fs::remove_file(&path).await;
    }

    Ok(result.rows_affected())
}

/// Save a chat attachment
pub async fn save_attachment(pool: &SqlitePool, att: &AttachmentRow) -> anyhow::Result<()> {
    let query_future = sqlx::query(
        "INSERT INTO chat_attachments (id, message_id, filename, mime_type, size_bytes, storage_type, inline_data, disk_path, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&att.id)
    .bind(&att.message_id)
    .bind(&att.filename)
    .bind(&att.mime_type)
    .bind(att.size_bytes)
    .bind(&att.storage_type)
    .bind(&att.inline_data)
    .bind(&att.disk_path)
    .bind(att.created_at)
    .execute(pool);

    db_timeout(query_future).await?;

    Ok(())
}

/// Get attachments for a specific message
pub async fn get_attachments_for_message(
    pool: &SqlitePool,
    message_id: &str,
) -> anyhow::Result<Vec<AttachmentRow>> {
    db_timeout(
        sqlx::query_as::<_, AttachmentRow>(
            "SELECT id, message_id, filename, mime_type, size_bytes, storage_type, inline_data, disk_path, created_at
             FROM chat_attachments
             WHERE message_id = ?",
        )
        .bind(message_id)
        .fetch_all(pool),
    )
    .await
}

/// Get an attachment by ID
pub async fn get_attachment_by_id(
    pool: &SqlitePool,
    attachment_id: &str,
) -> anyhow::Result<Option<AttachmentRow>> {
    db_timeout(
        sqlx::query_as::<_, AttachmentRow>(
            "SELECT id, message_id, filename, mime_type, size_bytes, storage_type, inline_data, disk_path, created_at
             FROM chat_attachments
             WHERE id = ?",
        )
        .bind(attachment_id)
        .fetch_optional(pool),
    )
    .await
}

/// Helper: get disk paths for attachments belonging to given message IDs
async fn get_disk_attachment_paths(
    pool: &SqlitePool,
    message_ids: &[String],
) -> anyhow::Result<Vec<String>> {
    if message_ids.is_empty() {
        return Ok(vec![]);
    }
    // Build placeholders for IN clause
    let placeholders: Vec<&str> = message_ids.iter().map(|_| "?").collect();
    let sql = format!(
        "SELECT disk_path FROM chat_attachments WHERE message_id IN ({}) AND storage_type = 'disk' AND disk_path IS NOT NULL",
        placeholders.join(",")
    );

    let mut query = sqlx::query_as::<_, (String,)>(&sql);
    for id in message_ids {
        query = query.bind(id);
    }

    let rows = db_timeout(query.fetch_all(pool)).await?;

    Ok(rows.into_iter().map(|(path,)| path).collect())
}
