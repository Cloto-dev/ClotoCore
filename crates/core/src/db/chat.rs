use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::db_timeout;

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        "INSERT INTO chat_messages (id, agent_id, user_id, source, content, metadata, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&msg.id)
    .bind(&msg.agent_id)
    .bind(&msg.user_id)
    .bind(&msg.source)
    .bind(&msg.content)
    .bind(&msg.metadata)
    .bind(msg.created_at)
    .execute(pool);

    db_timeout(query_future).await?;

    Ok(())
}

/// Row type returned by chat message queries.
type ChatMessageTuple = (String, String, String, String, String, Option<String>, i64);

/// Get chat messages with cursor-based pagination (ordered by created_at DESC)
pub async fn get_chat_messages(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: &str,
    before_ts: Option<i64>,
    limit: i64,
) -> anyhow::Result<Vec<ChatMessageRow>> {
    let limit = limit.min(200);

    let rows: Vec<ChatMessageTuple> = if let Some(before) = before_ts {
        let query_future = sqlx::query_as::<_, ChatMessageTuple>(
            "SELECT id, agent_id, user_id, source, content, metadata, created_at
             FROM chat_messages
             WHERE agent_id = ? AND user_id = ? AND created_at < ?
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
            "SELECT id, agent_id, user_id, source, content, metadata, created_at
             FROM chat_messages
             WHERE agent_id = ? AND user_id = ?
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
            |(id, agent_id, user_id, source, content, metadata, created_at)| ChatMessageRow {
                id,
                agent_id,
                user_id,
                source,
                content,
                metadata,
                created_at,
            },
        )
        .collect();

    Ok(messages)
}

/// Delete all chat messages (and cascade to attachments) for an agent/user pair
pub async fn delete_chat_messages(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: &str,
) -> anyhow::Result<u64> {
    // First get message IDs for disk attachment cleanup
    let ids_future = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM chat_messages WHERE agent_id = ? AND user_id = ?",
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
    let delete_future = sqlx::query("DELETE FROM chat_messages WHERE agent_id = ? AND user_id = ?")
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
    let query_future = sqlx::query_as::<_, (String, String, String, String, i64, String, Option<Vec<u8>>, Option<String>, i64)>(
        "SELECT id, message_id, filename, mime_type, size_bytes, storage_type, inline_data, disk_path, created_at
         FROM chat_attachments
         WHERE message_id = ?"
    )
    .bind(message_id)
    .fetch_all(pool);

    let rows = db_timeout(query_future).await?;

    let attachments = rows
        .into_iter()
        .map(
            |(
                id,
                message_id,
                filename,
                mime_type,
                size_bytes,
                storage_type,
                inline_data,
                disk_path,
                created_at,
            )| {
                AttachmentRow {
                    id,
                    message_id,
                    filename,
                    mime_type,
                    size_bytes,
                    storage_type,
                    inline_data,
                    disk_path,
                    created_at,
                }
            },
        )
        .collect();

    Ok(attachments)
}

/// Get an attachment by ID
pub async fn get_attachment_by_id(
    pool: &SqlitePool,
    attachment_id: &str,
) -> anyhow::Result<Option<AttachmentRow>> {
    let query_future = sqlx::query_as::<_, (String, String, String, String, i64, String, Option<Vec<u8>>, Option<String>, i64)>(
        "SELECT id, message_id, filename, mime_type, size_bytes, storage_type, inline_data, disk_path, created_at
         FROM chat_attachments
         WHERE id = ?"
    )
    .bind(attachment_id)
    .fetch_optional(pool);

    let row = db_timeout(query_future).await?;

    Ok(row.map(
        |(
            id,
            message_id,
            filename,
            mime_type,
            size_bytes,
            storage_type,
            inline_data,
            disk_path,
            created_at,
        )| {
            AttachmentRow {
                id,
                message_id,
                filename,
                mime_type,
                size_bytes,
                storage_type,
                inline_data,
                disk_path,
                created_at,
            }
        },
    ))
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
