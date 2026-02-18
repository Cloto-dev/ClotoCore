use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
    body::Bytes,
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::Arc;
use tracing::error;

use crate::{AppState, AppResult, AppError};
use crate::db::{self, ChatMessageRow, AttachmentRow};

#[derive(Deserialize)]
pub struct GetMessagesQuery {
    pub user_id: Option<String>,
    pub before: Option<i64>,
    pub limit: Option<i64>,
}

/// GET /api/chat/:agent_id/messages
/// Returns paginated chat messages (newest first)
pub async fn get_messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Query(params): Query<GetMessagesQuery>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    let user_id = params.user_id.as_deref().unwrap_or("default");
    let limit = params.limit.unwrap_or(50).min(200);

    let messages = db::get_chat_messages(
        &state.pool,
        &agent_id,
        user_id,
        params.before,
        limit + 1, // fetch one extra to determine has_more
    ).await?;

    let has_more = messages.len() as i64 > limit;
    let messages: Vec<ChatMessageRow> = messages.into_iter().take(limit as usize).collect();

    Ok(Json(serde_json::json!({
        "messages": messages,
        "has_more": has_more,
    })))
}

#[derive(Deserialize)]
pub struct PostMessageRequest {
    pub id: String,
    pub source: String,
    pub content: serde_json::Value,   // ContentBlock[] as opaque JSON
    pub metadata: Option<serde_json::Value>,
}

/// POST /api/chat/:agent_id/messages
/// Save a new chat message
pub async fn post_message(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Json(payload): Json<PostMessageRequest>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    // Validate source
    if !["user", "agent", "system"].contains(&payload.source.as_str()) {
        return Err(AppError::Vers(exiv_shared::ExivError::ValidationError(
            "source must be 'user', 'agent', or 'system'".to_string(),
        )));
    }

    // Validate content is a JSON array
    if !payload.content.is_array() {
        return Err(AppError::Vers(exiv_shared::ExivError::ValidationError(
            "content must be a JSON array of ContentBlock".to_string(),
        )));
    }

    // M-3: Limit content array length to prevent abuse
    const MAX_CONTENT_ITEMS: usize = 20;
    if payload.content.as_array().map_or(false, |a| a.len() > MAX_CONTENT_ITEMS) {
        return Err(AppError::Vers(exiv_shared::ExivError::ValidationError(
            format!("content array exceeds maximum of {} items", MAX_CONTENT_ITEMS),
        )));
    }

    let now = chrono::Utc::now().timestamp_millis();
    let content_str = serde_json::to_string(&payload.content)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to serialize content: {}", e)))?;
    let metadata_str = payload.metadata.map(|v| v.to_string());

    let msg = ChatMessageRow {
        id: payload.id.clone(),
        agent_id: agent_id.clone(),
        user_id: "default".to_string(),
        source: payload.source,
        content: content_str,
        metadata: metadata_str,
        created_at: now,
    };

    db::save_chat_message(&state.pool, &msg).await?;

    // Process inline attachments from content blocks
    if let Some(blocks) = payload.content.as_array() {
        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) == Some("image") {
                if let Some(url) = block.get("url").and_then(|u| u.as_str()) {
                    // Handle base64 data URIs as inline attachments
                    if let Some(data_part) = url.strip_prefix("data:") {
                        if let Some((mime_info, base64_data)) = data_part.split_once(',') {
                            let mime_type = mime_info.trim_end_matches(";base64").to_string();
                            // M-2: Only allow known-safe MIME types
                            const ALLOWED_MIME_TYPES: &[&str] = &[
                                "image/png", "image/jpeg", "image/jpg", "image/gif",
                                "image/webp", "image/svg+xml",
                            ];
                            if !ALLOWED_MIME_TYPES.contains(&mime_type.as_str()) {
                                tracing::warn!("Rejected attachment with disallowed MIME type: {}", mime_type);
                                continue;
                            }
                            let decoded = match base64_decode(base64_data) {
                                Ok(d) => d,
                                Err(()) => {
                                    tracing::warn!("Invalid base64 data in attachment, skipping");
                                    continue;
                                }
                            };
                            {
                                let att_id = uuid::Uuid::new_v4().to_string();
                                let size = decoded.len() as i64;
                                let filename = format!("image_{}.{}", &att_id[..8], mime_to_ext(&mime_type));

                                let (storage_type, inline_data, disk_path) = if size <= 65536 {
                                    // <=64KB: store inline
                                    ("inline".to_string(), Some(decoded), None)
                                } else {
                                    // >64KB: store on disk
                                    let dir = format!("data/attachments/{}", &msg.id);
                                    let path = format!("{}/{}", dir, filename);
                                    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
                                        error!("Failed to create attachment dir: {}", e);
                                        continue;
                                    }
                                    if let Err(e) = tokio::fs::write(&path, &decoded).await {
                                        error!("Failed to write attachment file: {}", e);
                                        continue;
                                    }
                                    ("disk".to_string(), None, Some(path))
                                };

                                let att = AttachmentRow {
                                    id: att_id,
                                    message_id: msg.id.clone(),
                                    filename,
                                    mime_type,
                                    size_bytes: size,
                                    storage_type,
                                    inline_data,
                                    disk_path,
                                    created_at: now,
                                };

                                if let Err(e) = db::save_attachment(&state.pool, &att).await {
                                    error!("Failed to save attachment: {}", e);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(Json(serde_json::json!({
        "id": msg.id,
        "created_at": now,
    })))
}

#[derive(Deserialize)]
pub struct DeleteMessagesQuery {
    pub user_id: Option<String>,
}

/// DELETE /api/chat/:agent_id/messages
/// Delete all messages for an agent/user pair
pub async fn delete_messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Query(params): Query<DeleteMessagesQuery>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    let user_id = params.user_id.as_deref().unwrap_or("default");
    let deleted_count = db::delete_chat_messages(&state.pool, &agent_id, user_id).await?;

    Ok(Json(serde_json::json!({
        "deleted_count": deleted_count,
    })))
}

/// GET /api/chat/attachments/:attachment_id
/// Serve an attachment file
pub async fn get_attachment(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(attachment_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    super::check_auth(&state, &headers)?;

    let att = db::get_attachment_by_id(&state.pool, &attachment_id).await?
        .ok_or_else(|| AppError::NotFound("Attachment not found".to_string()))?;

    let data = match att.storage_type.as_str() {
        "inline" => att.inline_data.ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("Inline attachment has no data"))
        })?,
        "disk" => {
            let path = att.disk_path.ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!("Disk attachment has no path"))
            })?;
            tokio::fs::read(&path).await.map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to read attachment file: {}", e))
            })?
        }
        _ => return Err(AppError::Internal(anyhow::anyhow!("Unknown storage type"))),
    };

    let headers = [
        (axum::http::header::CONTENT_TYPE, att.mime_type.clone()),
        (axum::http::header::CACHE_CONTROL, "public, max-age=31536000, immutable".to_string()),
        (axum::http::header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", att.filename)),
    ];

    Ok((headers, Bytes::from(data)))
}

// --- Helpers ---

fn base64_decode(input: &str) -> Result<Vec<u8>, ()> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|_| ())
}

fn mime_to_ext(mime: &str) -> &str {
    match mime {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        _ => "bin",
    }
}
