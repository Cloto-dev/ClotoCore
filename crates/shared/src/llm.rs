//! Shared utilities for OpenAI-compatible LLM API plugins.
//!
//! These free functions extract the common patterns shared by Cerebras, DeepSeek,
//! and any future plugin that targets the OpenAI chat completions API format.

use crate::{AgentMetadata, ExivMessage, HttpRequest, MessageSource, ThinkResult, ToolCall};
use std::collections::HashMap;

/// Build the standard OpenAI-compatible messages array.
///
/// Returns `[system_message, ...context_messages, user_message]`.
/// The caller may append additional entries (e.g. tool_history) after this.
pub fn build_chat_messages(
    agent: &AgentMetadata,
    message: &ExivMessage,
    context: &[ExivMessage],
) -> Vec<serde_json::Value> {
    let mut messages = Vec::with_capacity(context.len() + 2);

    messages.push(serde_json::json!({
        "role": "system",
        "content": format!("You are {}. {}.", agent.name, agent.description)
    }));

    for msg in context {
        let role = match msg.source {
            MessageSource::User { .. } => "user",
            MessageSource::Agent { .. } => "assistant",
            MessageSource::System => "system",
        };
        messages.push(serde_json::json!({ "role": role, "content": msg.content }));
    }

    messages.push(serde_json::json!({ "role": "user", "content": message.content }));
    messages
}

/// Build an `HttpRequest` for an OpenAI-compatible chat completions endpoint.
///
/// When `tools` is `Some` and non-empty, the `"tools"` field is included in the body.
pub fn build_chat_request(
    url: &str,
    api_key: &str,
    model_id: &str,
    messages: Vec<serde_json::Value>,
    tools: Option<&[serde_json::Value]>,
) -> HttpRequest {
    let mut body = serde_json::json!({
        "model": model_id,
        "messages": messages,
        "stream": false
    });

    if let Some(t) = tools {
        if !t.is_empty() {
            body["tools"] = serde_json::json!(t);
        }
    }

    let mut headers = HashMap::new();
    headers.insert("Authorization".to_string(), format!("Bearer {}", api_key));
    headers.insert("Content-Type".to_string(), "application/json".to_string());

    HttpRequest {
        method: "POST".to_string(),
        url: url.to_string(),
        headers,
        body: Some(body.to_string()),
    }
}

/// Parse a chat completions response body, extracting the text content.
///
/// Returns an error if the API returned an error object or the response is malformed.
pub fn parse_chat_content(
    response_body: &str,
    provider_name: &str,
) -> anyhow::Result<String> {
    let json: serde_json::Value = serde_json::from_str(response_body)?;

    if let Some(error) = json.get("error") {
        let msg = error.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
        return Err(anyhow::anyhow!("{} API Error: {}", provider_name, msg));
    }

    json.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!(
            "Invalid {} API response: missing choices[0].message.content",
            provider_name
        ))
}

/// Parse a chat completions response body, returning either final text or tool calls.
///
/// Handles the `finish_reason == "tool_calls"` convention and the presence of a
/// `tool_calls` array in the assistant message.
pub fn parse_chat_think_result(
    response_body: &str,
    provider_name: &str,
) -> anyhow::Result<ThinkResult> {
    let json: serde_json::Value = serde_json::from_str(response_body)?;

    if let Some(error) = json.get("error") {
        let msg = error.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
        return Err(anyhow::anyhow!("{} API Error: {}", provider_name, msg));
    }

    let choice = json.get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| anyhow::anyhow!("Invalid API response: missing choices[0]"))?;
    let message_obj = choice.get("message")
        .ok_or_else(|| anyhow::anyhow!("Invalid API response: missing message"))?;
    let finish_reason = choice.get("finish_reason")
        .and_then(|v| v.as_str()).unwrap_or("stop");

    if finish_reason == "tool_calls" || message_obj.get("tool_calls").is_some() {
        if let Some(tool_calls_arr) = message_obj.get("tool_calls").and_then(|v| v.as_array()) {
            let calls: Vec<ToolCall> = tool_calls_arr.iter().filter_map(|tc| {
                let id = tc.get("id")?.as_str()?.to_string();
                let function = tc.get("function")?;
                let name = function.get("name")?.as_str()?.to_string();
                let arguments_str = function.get("arguments")?.as_str()?;
                let arguments = match serde_json::from_str(arguments_str) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(tool = %name, error = %e, "Malformed tool_call arguments, using empty object");
                        serde_json::json!({})
                    }
                };
                Some(ToolCall { id, name, arguments })
            }).collect();

            if !calls.is_empty() {
                let assistant_content = message_obj.get("content")
                    .and_then(|v| v.as_str()).map(|s| s.to_string());
                return Ok(ThinkResult::ToolCalls { assistant_content, calls });
            }
        }
    }

    let content = message_obj.get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid API response: missing content"))?
        .to_string();
    Ok(ThinkResult::Final(content))
}
