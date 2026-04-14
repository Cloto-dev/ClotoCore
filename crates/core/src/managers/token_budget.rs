//! Pre-flight token budget check for outgoing LLM requests.
//!
//! The kernel builds a `think` / `think_with_tools` payload (system prompt +
//! recalled memory + conversation context + tool schemas) and passes it to a
//! mind MCP server. If the total exceeds the configured provider
//! `context_length`, the provider returns HTTP 400 — today the user only sees
//! "Provider 'local' returned an error (400)" with no guidance on which part
//! of the request was too large.
//!
//! This module estimates the request size up-front (UTF-8 byte length / 3 —
//! CJK-safe, conservative) and breaks down the total into the four components
//! so the error surface can point at the dominant contributor.
//!
//! The `/3` divisor is deliberately pessimistic: typical OpenAI-style BPE
//! tokenizers produce ~1 token per 3–4 English bytes and ~1 token per 3 UTF-8
//! bytes for CJK. A 15% safety margin on top leaves room for the response as
//! well as estimator drift.

use serde_json::Value;

/// Estimate tokens from a serialized JSON blob using a UTF-8-byte heuristic.
///
/// Assumes `~3 bytes per token`, which is on the conservative side for both
/// English (actual: 4 bytes/token) and CJK (actual: 3 bytes/token). We'd
/// rather reject a borderline request than let it fail upstream with a 400.
#[must_use]
pub fn estimate_request_tokens(v: &Value) -> usize {
    // Serialize without pretty-printing; we're measuring raw payload size.
    let bytes = serde_json::to_vec(v).map(|b| b.len()).unwrap_or(0);
    bytes / 3
}

fn estimate_str_tokens(s: &str) -> usize {
    s.len() / 3
}

/// Component of the request that contributed most to the token count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DominantComponent {
    SystemPrompt,
    Context,
    Tools,
    Message,
}

impl DominantComponent {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SystemPrompt => "system_prompt",
            Self::Context => "context",
            Self::Tools => "tools",
            Self::Message => "message",
        }
    }
}

/// Verdict from [`check_budget`] with enough detail to show the user
/// a meaningful error (which component to shrink).
#[derive(Debug, Clone, serde::Serialize)]
pub struct BudgetDecision {
    pub exceeds: bool,
    pub estimated_input: usize,
    pub context_length: i64,
    /// Required headroom = estimated response tokens + safety buffer.
    pub headroom_reserved: usize,
    pub dominant_component: DominantComponent,
    pub system_tokens: usize,
    pub context_tokens: usize,
    pub tools_tokens: usize,
    pub message_tokens: usize,
}

/// Constant headroom reserved for the model's response + padding for estimator drift.
/// Providers reject the prompt if there's no room to generate; 1024 tokens is a
/// reasonable floor for agentic replies (short completions won't fail this check,
/// long ones may need more — but the user can always raise `context_length`).
pub const RESPONSE_HEADROOM: usize = 1024;

/// Fraction of `context_length` we allow the input to occupy before flagging overflow.
/// 0.85 leaves 15% on top of the explicit response headroom to cover estimator drift.
pub const SAFETY_FRACTION: f64 = 0.85;

/// Check whether the request fits within the provider's configured context window.
///
/// Arguments are the four components the kernel already has on hand when it
/// builds the MCP payload. `context_length` comes from `llm_providers.context_length`
/// — callers should skip the check entirely when it's `None`.
#[must_use]
pub fn check_budget(
    system_prompt: &str,
    context: &Value,
    tools: &Value,
    message: &Value,
    context_length: i64,
) -> BudgetDecision {
    let system_tokens = estimate_str_tokens(system_prompt);
    let context_tokens = estimate_request_tokens(context);
    let tools_tokens = estimate_request_tokens(tools);
    let message_tokens = estimate_request_tokens(message);
    let estimated_input = system_tokens + context_tokens + tools_tokens + message_tokens;

    let dominant_component = [
        (DominantComponent::SystemPrompt, system_tokens),
        (DominantComponent::Context, context_tokens),
        (DominantComponent::Tools, tools_tokens),
        (DominantComponent::Message, message_tokens),
    ]
    .into_iter()
    .max_by_key(|&(_, n)| n)
    .map_or(DominantComponent::Tools, |(c, _)| c);

    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let allowed_input = (context_length as f64 * SAFETY_FRACTION) as usize;
    let budget_ceiling = allowed_input.saturating_sub(RESPONSE_HEADROOM);
    let exceeds = estimated_input > budget_ceiling;

    BudgetDecision {
        exceeds,
        estimated_input,
        context_length,
        headroom_reserved: RESPONSE_HEADROOM,
        dominant_component,
        system_tokens,
        context_tokens,
        tools_tokens,
        message_tokens,
    }
}

/// Human-readable summary for logs. Frontend uses the structured fields directly.
#[must_use]
pub fn describe_overflow(d: &BudgetDecision) -> String {
    format!(
        "estimated input {} tokens exceeds budget (context_length={}, headroom={}); \
         dominant component: {} ({} tokens)",
        d.estimated_input,
        d.context_length,
        d.headroom_reserved,
        d.dominant_component.as_str(),
        match d.dominant_component {
            DominantComponent::SystemPrompt => d.system_tokens,
            DominantComponent::Context => d.context_tokens,
            DominantComponent::Tools => d.tools_tokens,
            DominantComponent::Message => d.message_tokens,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ascii_estimate_is_conservative() {
        // 300 ASCII chars = 300 bytes → 100 tokens (overshoots tiktoken's ~75).
        let s = "a".repeat(300);
        assert_eq!(estimate_str_tokens(&s), 100);
    }

    #[test]
    fn cjk_estimate_is_accurate() {
        // 100 CJK chars = 300 bytes → 100 tokens (matches actual Qwen / cl100k rate).
        let s = "あ".repeat(100);
        assert_eq!(s.len(), 300); // sanity: 3 bytes/char
        assert_eq!(estimate_str_tokens(&s), 100);
    }

    #[test]
    fn fits_within_small_context() {
        let d = check_budget(
            "hi",
            &json!([]),
            &json!([]),
            &json!({"content": "hello"}),
            4096,
        );
        assert!(!d.exceeds);
        assert_eq!(d.dominant_component, DominantComponent::Message);
    }

    #[test]
    fn exceeds_with_oversized_tools() {
        // 6000 tokens of tool schemas vs 4096 context → overflow, dominant=tools.
        let fake_tool = json!({"name": "x".repeat(18000)}); // ~6000 tokens
        let d = check_budget("hi", &json!([]), &fake_tool, &json!({}), 4096);
        assert!(d.exceeds);
        assert_eq!(d.dominant_component, DominantComponent::Tools);
        assert!(d.tools_tokens > 4000);
    }

    #[test]
    fn respects_response_headroom() {
        // Context length exactly equal to input → fails because no headroom for response.
        let s = "a".repeat(3000); // 1000 tokens
        let d = check_budget(&s, &json!([]), &json!([]), &json!({}), 1000);
        assert!(d.exceeds);
    }

    #[test]
    fn dominant_is_context_when_history_dominates() {
        // 45_000 bytes → 15_000 tokens; 4096 ctx → overflow, dominant = context.
        let big_context = json!(vec!["b".repeat(45_000); 1]);
        let d = check_budget("hi", &big_context, &json!([]), &json!({}), 4096);
        assert!(d.exceeds);
        assert_eq!(d.dominant_component, DominantComponent::Context);
    }
}
