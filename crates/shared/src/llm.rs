//! Shared utilities for OpenAI-compatible LLM API plugins.
//!
//! These free functions extract the common patterns shared by Cerebras, DeepSeek,
//! and any future plugin that targets the OpenAI chat completions API format.

use crate::{AgentMetadata, ClotoMessage, MessageSource, ThinkResult, ToolCall};
/// Build the system prompt for a Cloto agent.
///
/// Automatically injects platform context (identity, privacy, capabilities)
/// so agents self-identify as Cloto agents without requiring manual description setup.
/// The user-supplied `description` serves as role/persona definition layered on top.
fn build_system_prompt(agent: &AgentMetadata) -> String {
    let has_memory = agent
        .metadata
        .get("preferred_memory")
        .is_some_and(|m| !m.is_empty());

    let memory_line = if has_memory {
        "You have persistent memory — you can recall past conversations with your operator.\n"
    } else {
        ""
    };

    format!(
        "You are {name}, an AI agent running on the Cloto platform.\n\
         Cloto is a local, self-hosted AI container system — all data stays on your \
         operator's hardware and is never sent to any external service.\n\
         {memory}You can extend your own capabilities by creating new skills at runtime.\n\
         \n\
         {description}",
        name = agent.name,
        memory = memory_line,
        description = agent.description,
    )
}

/// Build the standard OpenAI-compatible messages array.
///
/// Returns `[system_message, ...context_messages, user_message]`.
/// The caller may append additional entries (e.g. tool_history) after this.
#[must_use]
pub fn build_chat_messages(
    agent: &AgentMetadata,
    message: &ClotoMessage,
    context: &[ClotoMessage],
) -> Vec<serde_json::Value> {
    let mut messages = Vec::with_capacity(context.len() + 2);

    messages.push(serde_json::json!({
        "role": "system",
        "content": build_system_prompt(agent)
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

/// Parse a chat completions response body, returning either final text or tool calls.
///
/// Handles the `finish_reason == "tool_calls"` convention and the presence of a
/// `tool_calls` array in the assistant message.
pub fn parse_chat_think_result(
    response_body: &str,
    provider_name: &str,
) -> anyhow::Result<ThinkResult> {
    let json: serde_json::Value = serde_json::from_str(response_body).map_err(|e| {
        anyhow::anyhow!(
            "{} API response is not valid JSON: {} | body: {}",
            provider_name,
            e,
            &response_body[..response_body.len().min(500)]
        )
    })?;

    // Standard OpenAI error format
    if let Some(error) = json.get("error") {
        let msg = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or_else(|| error.as_str().unwrap_or("Unknown error"));
        return Err(anyhow::anyhow!("{} API Error: {}", provider_name, msg));
    }
    // Cerebras non-standard error format
    if json
        .get("type")
        .and_then(|t| t.as_str())
        .is_some_and(|t| t.contains("error"))
    {
        let msg = json
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Unknown error");
        return Err(anyhow::anyhow!("{} API Error: {}", provider_name, msg));
    }

    let choice = json.get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| {
            tracing::error!(provider = %provider_name, body = %response_body, "Unexpected API response structure");
            anyhow::anyhow!(
                "Invalid {} API response: missing choices[0] | body: {}",
                provider_name,
                &response_body[..response_body.len().min(500)]
            )
        })?;
    let message_obj = choice
        .get("message")
        .ok_or_else(|| anyhow::anyhow!("Invalid API response: missing message"))?;
    let finish_reason = choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("stop");

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
                let assistant_content = message_obj
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string);
                return Ok(ThinkResult::ToolCalls {
                    assistant_content,
                    calls,
                });
            }
        }
    }

    let content = message_obj
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid API response: missing content"))?
        .to_string();
    Ok(ThinkResult::Final(content))
}
