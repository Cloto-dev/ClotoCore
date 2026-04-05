use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use tracing::error;

use crate::db::{self, AttachmentRow, ChatMessageRow};
use crate::{AppError, AppResult, AppState};

/// Default user ID when none is provided in the request.
const DEFAULT_USER_ID: &str = "default";

use super::ok_data;

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

    let user_id = params.user_id.as_deref().unwrap_or(DEFAULT_USER_ID);
    let limit = params
        .limit
        .unwrap_or(50)
        .max(1)
        .min(state.config.max_chat_query_limit);

    let messages = db::get_chat_messages(
        &state.pool,
        &agent_id,
        user_id,
        params.before,
        limit + 1, // fetch one extra to determine has_more
        state.config.max_chat_query_limit,
    )
    .await?;

    #[allow(clippy::cast_possible_wrap)]
    let has_more = messages.len() as i64 > limit;
    let messages: Vec<ChatMessageRow> = messages.into_iter().take(limit as usize).collect();

    ok_data(serde_json::json!({
        "messages": messages,
        "has_more": has_more,
    }))
}

#[derive(Deserialize)]
pub struct PostMessageRequest {
    pub id: String,
    pub source: String,
    pub content: serde_json::Value, // ContentBlock[] as opaque JSON
    pub metadata: Option<serde_json::Value>,
    pub user_id: Option<String>,
}

/// POST /api/chat/:agent_id/messages
/// Save a new chat message
#[allow(clippy::too_many_lines)]
pub async fn post_message(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Json(payload): Json<PostMessageRequest>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    // Block messages to disabled agents
    let (agent, _) = state
        .agent_manager
        .get_agent_config(&agent_id)
        .await
        .map_err(|_| {
            AppError::Cloto(cloto_shared::ClotoError::ValidationError(format!(
                "Agent '{}' not found",
                agent_id
            )))
        })?;
    if !agent.enabled {
        return Err(AppError::Cloto(cloto_shared::ClotoError::ValidationError(
            format!("Agent '{}' is powered off", agent_id),
        )));
    }

    // Validate source
    if crate::db::mcp::MessageSource::from_str_validated(&payload.source).is_none() {
        return Err(AppError::Cloto(cloto_shared::ClotoError::ValidationError(
            "source must be 'user', 'agent', or 'system'".to_string(),
        )));
    }

    // Validate content is a JSON array
    if !payload.content.is_array() {
        return Err(AppError::Cloto(cloto_shared::ClotoError::ValidationError(
            "content must be a JSON array of ContentBlock".to_string(),
        )));
    }

    // M-3: Limit content array length to prevent abuse
    if payload
        .content
        .as_array()
        .is_some_and(|a| a.len() > super::utils::CONTENT_BLOCK_MAX_ITEMS)
    {
        return Err(AppError::Cloto(cloto_shared::ClotoError::ValidationError(
            format!(
                "content array exceeds maximum of {} items",
                super::utils::CONTENT_BLOCK_MAX_ITEMS
            ),
        )));
    }

    let now = chrono::Utc::now().timestamp_millis();
    let content_str = serde_json::to_string(&payload.content)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to serialize content: {}", e)))?;
    let metadata_str = payload.metadata.map(|v| v.to_string());

    let msg = ChatMessageRow {
        id: payload.id.clone(),
        agent_id: agent_id.clone(),
        user_id: payload
            .user_id
            .clone()
            .unwrap_or_else(|| "default".to_string()),
        source: payload.source,
        content: content_str,
        metadata: metadata_str,
        created_at: now,
        parent_id: None,
        branch_index: 0,
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
                                "image/png",
                                "image/jpeg",
                                "image/jpg",
                                "image/gif",
                                "image/webp",
                                "image/svg+xml",
                            ];
                            if !ALLOWED_MIME_TYPES.contains(&mime_type.as_str()) {
                                tracing::warn!(
                                    "Rejected attachment with disallowed MIME type: {}",
                                    mime_type
                                );
                                continue;
                            }
                            let Ok(decoded) = base64_decode(base64_data) else {
                                tracing::warn!("Invalid base64 data in attachment, skipping");
                                continue;
                            };
                            {
                                let att_id = uuid::Uuid::new_v4().to_string();
                                #[allow(clippy::cast_possible_wrap)]
                                let size = decoded.len() as i64;
                                let filename = format!(
                                    "image_{}.{}",
                                    &att_id[..8],
                                    super::utils::mime_to_ext_or(&mime_type, "bin")
                                );

                                #[allow(clippy::cast_possible_wrap)]
                                let (storage_type, inline_data, disk_path) =
                                    if size <= state.config.attachment_inline_threshold as i64 {
                                        // <=64KB: store inline
                                        ("inline".to_string(), Some(decoded), None)
                                    } else {
                                        // >64KB: store on disk
                                        let dir = state.data_dir.join("attachments").join(&msg.id);
                                        let path = dir.join(&filename);
                                        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
                                            error!("Failed to create attachment dir: {}", e);
                                            continue;
                                        }
                                        if let Err(e) = tokio::fs::write(&path, &decoded).await {
                                            error!("Failed to write attachment file: {}", e);
                                            continue;
                                        }
                                        ("disk".to_string(), None, Some(path.to_string_lossy().to_string()))
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

    ok_data(serde_json::json!({
        "id": msg.id,
        "created_at": now,
    }))
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

    let user_id = params.user_id.as_deref().unwrap_or(DEFAULT_USER_ID);
    let deleted_count = db::delete_chat_messages(&state.pool, &agent_id, user_id).await?;

    ok_data(serde_json::json!({
        "deleted_count": deleted_count,
    }))
}

/// GET /api/chat/attachments/:attachment_id
/// Serve an attachment file
pub async fn get_attachment(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(attachment_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    super::check_auth(&state, &headers)?;

    let att = db::get_attachment_by_id(&state.pool, &attachment_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Attachment not found".to_string()))?;

    let data = match att.storage_type.as_str() {
        "inline" => att
            .inline_data
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("Inline attachment has no data")))?,
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
        (
            axum::http::header::CACHE_CONTROL,
            "public, max-age=31536000, immutable".to_string(),
        ),
        (
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", att.filename),
        ),
    ];

    Ok((headers, Bytes::from(data)))
}

/// Retry an agent response: re-sends the original user message for re-generation.
///
/// **Route:** `POST /api/chat/:agent_id/messages/:message_id/retry`
pub async fn retry_response(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((agent_id, message_id)): Path<(String, String)>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    // Look up the original user message
    let original = db::get_chat_message_by_id(&state.pool, &message_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Message '{}' not found", message_id)))?;

    // Extract text content from the stored ContentBlock[] JSON
    let content_text = serde_json::from_str::<serde_json::Value>(&original.content)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .and_then(|blocks| {
            blocks
                .iter()
                .filter_map(|b| {
                    if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                        b.get("text").and_then(|t| t.as_str()).map(String::from)
                    } else {
                        None
                    }
                })
                .reduce(|a, b| format!("{} {}", a, b))
        })
        .unwrap_or_default();

    if content_text.is_empty() {
        return Err(AppError::Validation(
            "Original message has no text content to retry".to_string(),
        ));
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    let retry_id = format!("retry-{}", now_ms);

    let cloto_msg = cloto_shared::ClotoMessage {
        id: retry_id.clone(),
        source: cloto_shared::MessageSource::User {
            id: original.user_id.clone(),
            name: original.user_id.clone(),
        },
        target_agent: Some(agent_id.clone()),
        content: content_text,
        timestamp: chrono::Utc::now(),
        metadata: std::collections::HashMap::from([
            ("target_agent_id".to_string(), agent_id),
            ("skip_user_persist".to_string(), "true".to_string()),
            ("parent_id".to_string(), message_id),
        ]),
    };

    let envelope =
        crate::EnvelopedEvent::system(cloto_shared::ClotoEventData::MessageReceived(cloto_msg));
    if let Err(e) = state.event_tx.send(envelope).await {
        error!("Failed to send retry event: {}", e);
        return Err(AppError::Internal(anyhow::anyhow!(
            "Failed to accept retry"
        )));
    }

    ok_data(serde_json::json!({
        "retry_id": retry_id,
    }))
}

/// Send a chat message into the system.
///
/// **Route:** `POST /api/chat`
///
/// # Authentication
/// Requires valid API key in `X-API-Key` header.
///
/// # Request Body
/// An `ClotoMessage` JSON object containing the message content,
/// sender information, and optional metadata.
///
/// # Behavior
/// Wraps the message as a `MessageReceived` event and publishes
/// it to the event bus for processing by agents and plugins.
///
/// # Response
/// - **200 OK:** `{ "status": "accepted" }`
/// - **403 Forbidden:** Invalid or missing API key
/// - **500 Internal Server Error:** Event bus send failure
pub async fn chat_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(msg): Json<cloto_shared::ClotoMessage>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;
    let envelope =
        crate::EnvelopedEvent::system(cloto_shared::ClotoEventData::MessageReceived(msg));
    if let Err(e) = state.event_tx.send(envelope).await {
        error!("Failed to send chat message event: {}", e);
        return Err(AppError::Internal(anyhow::anyhow!(
            "Failed to accept message"
        )));
    }
    ok_data(serde_json::json!({}))
}

// --- Helpers ---

fn base64_decode(input: &str) -> Result<Vec<u8>, ()> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|_| ())
}
