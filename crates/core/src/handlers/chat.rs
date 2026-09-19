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
    /// Restrict the list to one conversation. Without it the list is the
    /// agent/user pair's whole history, as it was before conversations.
    pub conversation_id: Option<String>,
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
        params.conversation_id.as_deref(),
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
    /// The conversation to file the message under. Absent, the agent's
    /// default conversation for this user (created on demand).
    pub conversation_id: Option<String>,
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
    let user_id = payload
        .user_id
        .clone()
        .unwrap_or_else(|| DEFAULT_USER_ID.to_string());
    let conversation_id = match payload.conversation_id.clone() {
        Some(id) => id,
        None => db::ensure_default_conversation(&state.pool, &agent_id, &user_id, now).await?,
    };
    let metadata_str = payload.metadata.map(|v| v.to_string());

    let msg = ChatMessageRow {
        id: payload.id.clone(),
        agent_id: agent_id.clone(),
        user_id,
        source: payload.source.clone(),
        content: content_str,
        metadata: metadata_str,
        created_at: now,
        parent_id: None,
        branch_index: 0,
        conversation_id: Some(conversation_id.clone()),
    };

    db::save_chat_message(&state.pool, &msg).await?;
    db::touch_conversation(&state.pool, &conversation_id, now).await?;
    if payload.source == "user" {
        let first_title = db::title_from_first_message(&text_of_content(&payload.content));
        if !first_title.is_empty() {
            db::set_title_if_empty(&state.pool, &conversation_id, &first_title).await?;
        }
    }

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
                                        // >64KB: store on disk. bug-458: key the
                                        // attachment directory on the server-minted
                                        // `att_id` (a fresh UUID), NOT the
                                        // client-controlled `msg.id`, which is
                                        // unvalidated and would otherwise allow
                                        // `..`/absolute path traversal into an
                                        // arbitrary filesystem write.
                                        let dir = state.data_dir.join("attachments").join(&att_id);
                                        let path = dir.join(&filename);
                                        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
                                            error!("Failed to create attachment dir: {}", e);
                                            continue;
                                        }
                                        if let Err(e) = tokio::fs::write(&path, &decoded).await {
                                            error!("Failed to write attachment file: {}", e);
                                            continue;
                                        }
                                        (
                                            "disk".to_string(),
                                            None,
                                            Some(path.to_string_lossy().to_string()),
                                        )
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
/// Serve an attachment file.
///
/// Accepts the admin API key in `X-API-Key` or as `?token=`: the dashboard
/// renders attachments through `<img src>` / `<audio src>`, which cannot set a
/// header, so a header-only check left every stored attachment a 403 in the
/// browser.
#[allow(clippy::implicit_hasher)]
pub async fn get_attachment(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
    Path(attachment_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    super::check_auth_with_query(&state, &headers, &query)?;

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

#[derive(Deserialize)]
pub struct StopResponseRequest {
    /// The id of the message whose reply is to stop.
    pub source_message_id: String,
}

/// Stop the reply an agent is producing to one message.
///
/// **Route:** `POST /api/chat/:agent_id/stop`
///
/// Answers `{"stopped": true}` when that reply was still being produced (or
/// was queued behind the agent's previous turn): it is dropped where it was,
/// nothing of it is stored, and a `ResponseStopped` event is sent instead of a
/// `ThoughtResponse`. `{"stopped": false}` means there was no such reply to
/// stop — it had already finished, so what it produced stands.
pub async fn stop_response(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Json(payload): Json<StopResponseRequest>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;
    let stopped = state
        .response_stops
        .stop(&agent_id, &payload.source_message_id);
    ok_data(serde_json::json!({ "stopped": stopped }))
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

    // bug-474: the lookup is by message_id alone — verify the message actually
    // belongs to the path agent, or a caller could re-inject one agent's content
    // into another agent's stream by mismatching agent_id/message_id.
    if original.agent_id != agent_id {
        return Err(AppError::NotFound(format!(
            "Message '{}' not found for agent '{}'",
            message_id, agent_id
        )));
    }

    // Mirror post_message's existence/enabled gate (retry dispatches a live event).
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
        metadata: {
            let mut m = std::collections::HashMap::from([
                ("target_agent_id".to_string(), agent_id),
                ("skip_user_persist".to_string(), "true".to_string()),
                ("parent_id".to_string(), message_id),
            ]);
            if let Some(cid) = original.conversation_id.clone() {
                m.insert("conversation_id".to_string(), cid);
            }
            m
        },
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

// --- Speaking to one agent (docs/PANEL_WRITE_GATE_DESIGN.md §4.2) ---

/// The name a message sent through `send_to_agent` carries. The kernel does not
/// know the name a person chose in their browser, and the dashboard falls back
/// to this same word when they chose none.
const DEFAULT_USER_NAME: &str = "User";

#[derive(Deserialize)]
pub struct SendRequest {
    pub conversation_id: String,
    pub content: String,
}

/// The message `send_to_agent` hands to the agent. Everything that decides who
/// is speaking to whom comes from the path and the conversation row, never from
/// the request: the sender is always the person who owns the conversation.
#[must_use]
pub fn message_for_send(
    agent_id: &str,
    conversation: &db::ConversationRow,
    content: &str,
) -> cloto_shared::ClotoMessage {
    let mut msg = cloto_shared::ClotoMessage::new(
        cloto_shared::MessageSource::User {
            id: conversation.user_id.clone(),
            name: DEFAULT_USER_NAME.to_string(),
        },
        content.to_string(),
    );
    msg.target_agent = Some(agent_id.to_string());
    msg.metadata
        .insert("target_agent_id".to_string(), agent_id.to_string());
    msg.metadata
        .insert("conversation_id".to_string(), conversation.id.clone());
    msg
}

/// POST /api/chat/:agent_id/send — say something to the agent in the path, in
/// one of its conversations, and have it answer.
///
/// `POST /api/chat` does the same with the target and the sender in the body,
/// which is why a connector panel cannot be allowed it: an exact path would pin
/// nothing. Here the path names the agent, the conversation must belong to it,
/// and the sender is the conversation's owner. The reply is filed under the
/// returned `id` with `-resp` appended.
pub async fn send_to_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Json(payload): Json<SendRequest>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;
    let (agent, _) = state
        .agent_manager
        .get_agent_config(&agent_id)
        .await
        .map_err(|_| AppError::NotFound(format!("agent '{agent_id}'")))?;
    if !agent.enabled {
        return Err(AppError::Validation(format!(
            "Agent '{agent_id}' is powered off"
        )));
    }
    let content = payload.content.trim();
    if content.is_empty() {
        return Err(AppError::Validation("content is required".to_string()));
    }
    let conversation = owned_conversation(&state, &agent_id, &payload.conversation_id).await?;
    let msg = message_for_send(&agent_id, &conversation, content);
    let id = msg.id.clone();
    let envelope =
        crate::EnvelopedEvent::system(cloto_shared::ClotoEventData::MessageReceived(msg));
    if let Err(e) = state.event_tx.send(envelope).await {
        error!("Failed to send chat message event: {}", e);
        return Err(AppError::Internal(anyhow::anyhow!(
            "Failed to accept message"
        )));
    }
    ok_data(serde_json::json!({ "id": id, "conversation_id": conversation.id }))
}

// --- Search across conversations ---

#[derive(Deserialize)]
pub struct SearchMessagesQuery {
    /// Whitespace-separated terms; a message matches when it contains all of them.
    pub q: Option<String>,
    pub user_id: Option<String>,
    pub limit: Option<i64>,
}

/// GET /api/chat/search
/// Messages across every agent's conversations (archived ones included) whose
/// text contains every term of `q`, newest first. `total` counts every match
/// and `truncated` says when `results` holds fewer than that.
pub async fn search_messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<SearchMessagesQuery>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    let query = params.q.as_deref().unwrap_or("").trim();
    if query.is_empty() {
        return Err(AppError::Validation("q is required".to_string()));
    }
    let user_id = params.user_id.as_deref().unwrap_or(DEFAULT_USER_ID);
    let limit = params
        .limit
        .unwrap_or(50)
        .max(1)
        .min(state.config.max_chat_query_limit);

    let found = db::search_chat_messages(&state.pool, user_id, query, limit).await?;
    let truncated = found.truncated();
    ok_data(serde_json::json!({
        "query": query,
        "results": found.hits,
        "total": found.total,
        "truncated": truncated,
    }))
}

// --- Conversations (docs/CONVERSATIONS_DESIGN.md §3) ---

#[derive(Deserialize)]
pub struct ConversationsQuery {
    pub user_id: Option<String>,
    #[serde(default)]
    pub include_archived: bool,
}

/// GET /api/chat/:agent_id/conversations
pub async fn list_conversations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Query(params): Query<ConversationsQuery>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;
    let user_id = params.user_id.as_deref().unwrap_or(DEFAULT_USER_ID);
    let conversations =
        db::list_conversations(&state.pool, &agent_id, user_id, params.include_archived).await?;
    ok_data(serde_json::json!({ "conversations": conversations }))
}

#[derive(Deserialize, Default)]
pub struct CreateConversationRequest {
    pub user_id: Option<String>,
}

/// POST /api/chat/:agent_id/conversations — a fresh, untitled conversation.
pub async fn create_conversation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    payload: Option<Json<CreateConversationRequest>>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;
    require_agent(&state, &agent_id).await?;
    let payload = payload.map(|Json(p)| p).unwrap_or_default();
    let user_id = payload.user_id.as_deref().unwrap_or(DEFAULT_USER_ID);
    let row = db::create_conversation(
        &state.pool,
        &agent_id,
        user_id,
        chrono::Utc::now().timestamp_millis(),
    )
    .await?;
    ok_data(serde_json::to_value(row).unwrap_or(serde_json::Value::Null))
}

#[derive(Deserialize)]
pub struct UpdateConversationRequest {
    pub title: Option<String>,
    /// `true` archives (hidden, kept whole), `false` unarchives; absent leaves it.
    pub archived: Option<bool>,
}

#[derive(Deserialize, Default)]
pub struct ConversationReadQuery {
    pub limit: Option<i64>,
}

/// GET /api/chat/:agent_id/conversations/:conversation_id — the conversation and
/// its newest messages, newest first.
///
/// The same messages are reachable as `GET /api/chat/:agent_id/messages?conversation_id=`,
/// but a connector panel can only declare paths, not query strings. Taking the
/// id from the path is what lets a panel read the thread it writes to.
pub async fn get_conversation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((agent_id, conversation_id)): Path<(String, String)>,
    Query(params): Query<ConversationReadQuery>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;
    let conversation = owned_conversation(&state, &agent_id, &conversation_id).await?;
    let limit = params
        .limit
        .unwrap_or(50)
        .max(1)
        .min(state.config.max_chat_query_limit);
    let messages = db::get_chat_messages(
        &state.pool,
        &agent_id,
        &conversation.user_id,
        Some(&conversation.id),
        None,
        limit + 1,
        state.config.max_chat_query_limit,
    )
    .await?;
    #[allow(clippy::cast_possible_wrap)]
    let has_more = messages.len() as i64 > limit;
    let messages: Vec<ChatMessageRow> = messages.into_iter().take(limit as usize).collect();
    ok_data(serde_json::json!({
        "conversation": conversation,
        "messages": messages,
        "has_more": has_more,
    }))
}

/// PATCH /api/chat/:agent_id/conversations/:conversation_id — rename, archive, unarchive.
pub async fn update_conversation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((agent_id, conversation_id)): Path<(String, String)>,
    Json(payload): Json<UpdateConversationRequest>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;
    let row = owned_conversation(&state, &agent_id, &conversation_id).await?;
    if let Some(title) = payload.title.as_deref() {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 200 {
            return Err(AppError::Validation(
                "title must be 1–200 characters".to_string(),
            ));
        }
        db::rename_conversation(&state.pool, &row.id, title).await?;
    }
    if let Some(archived) = payload.archived {
        let archived_at = archived.then(|| chrono::Utc::now().timestamp_millis());
        db::set_conversation_archived(&state.pool, &row.id, archived_at).await?;
    }
    let updated = db::get_conversation(&state.pool, &row.id)
        .await?
        .ok_or_else(|| AppError::NotFound("conversation".to_string()))?;
    ok_data(serde_json::to_value(updated).unwrap_or(serde_json::Value::Null))
}

/// DELETE /api/chat/:agent_id/conversations/:conversation_id — immediate and permanent.
pub async fn delete_conversation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((agent_id, conversation_id)): Path<(String, String)>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;
    let row = owned_conversation(&state, &agent_id, &conversation_id).await?;
    let deleted = db::delete_conversation(&state.pool, &row.id)
        .await?
        .unwrap_or(0);
    ok_data(serde_json::json!({ "deleted_messages": deleted }))
}

#[derive(Deserialize, Default)]
pub struct BulkConversationsRequest {
    pub user_id: Option<String>,
}

/// POST /api/chat/:agent_id/conversations/archive-all
pub async fn archive_all_conversations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    payload: Option<Json<BulkConversationsRequest>>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;
    let payload = payload.map(|Json(p)| p).unwrap_or_default();
    let user_id = payload.user_id.as_deref().unwrap_or(DEFAULT_USER_ID);
    let archived = db::archive_all_conversations(
        &state.pool,
        &agent_id,
        user_id,
        chrono::Utc::now().timestamp_millis(),
    )
    .await?;
    ok_data(serde_json::json!({ "archived": archived }))
}

/// POST /api/chat/:agent_id/conversations/delete-all — archived ones included.
pub async fn delete_all_conversations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    payload: Option<Json<BulkConversationsRequest>>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;
    let payload = payload.map(|Json(p)| p).unwrap_or_default();
    let user_id = payload.user_id.as_deref().unwrap_or(DEFAULT_USER_ID);
    let deleted = db::delete_all_conversations(&state.pool, &agent_id, user_id).await?;
    ok_data(serde_json::json!({ "deleted": deleted }))
}

async fn require_agent(state: &AppState, agent_id: &str) -> AppResult<()> {
    state
        .agent_manager
        .get_agent_config(agent_id)
        .await
        .map(|_| ())
        .map_err(|_| AppError::NotFound(format!("agent '{agent_id}'")))
}

/// A conversation looked up by id, and only when it belongs to the agent in
/// the path — an id is never enough on its own to reach another agent's thread.
async fn owned_conversation(
    state: &AppState,
    agent_id: &str,
    conversation_id: &str,
) -> AppResult<db::ConversationRow> {
    match db::get_conversation(&state.pool, conversation_id).await? {
        Some(row) if row.agent_id == agent_id => Ok(row),
        _ => Err(AppError::NotFound("conversation".to_string())),
    }
}

/// The text of a content-block array, for the first-line title.
fn text_of_content(content: &serde_json::Value) -> String {
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

// --- Helpers ---

fn base64_decode(input: &str) -> Result<Vec<u8>, ()> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|_| ())
}
