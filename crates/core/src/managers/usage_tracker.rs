//! Ephemeral per-agent "last response usage" store.
//!
//! Mind MCP servers return OpenAI-style `{usage: {prompt_tokens, completion_tokens,
//! total_tokens}}` on every `think`/`think_with_tools` call. The kernel extracts
//! that number on each final reply, normalizes Anthropic's {input_tokens,
//! output_tokens} to the same shape, and pins it to the agent so the Dashboard
//! can show "2,341 / 32,768 tok" in the agent header.
//!
//! Intentionally ephemeral (in-memory, process-lifetime). Persisting per-turn
//! token counts would balloon the DB and has no value across restarts — the UI
//! only needs "what did the most recent response cost?".

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};

/// Per-agent summary of the most recent LLM response's token usage.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LastUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    /// Snapshot of the provider's `context_length` at record time, so the UI can
    /// compute "input/max" without a second DB lookup.
    pub context_length: Option<i64>,
    pub provider_id: String,
    pub model_id: String,
    /// True when `prompt_tokens` came from the pre-flight char-based estimate
    /// rather than the provider's reported usage (e.g. if the MCP server didn't
    /// include the `usage` field).
    pub is_estimate: bool,
    pub updated_at: DateTime<Utc>,
}

/// Process-lifetime cache of `LastUsage` keyed by `agent.id`.
#[derive(Default, Clone)]
pub struct UsageStore {
    inner: Arc<Mutex<HashMap<String, LastUsage>>>,
}

impl UsageStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, agent_id: &str, usage: LastUsage) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.insert(agent_id.to_string(), usage);
        }
    }

    #[must_use]
    pub fn get(&self, agent_id: &str) -> Option<LastUsage> {
        self.inner.lock().ok()?.get(agent_id).cloned()
    }
}

/// Extract & normalize the `usage` field from an LLM response body.
///
/// Handles OpenAI's `{prompt_tokens, completion_tokens, total_tokens}` and
/// Anthropic's `{input_tokens, output_tokens}` (no total — we sum the two).
/// Returns `None` if neither shape is present or the numbers can't be parsed.
#[must_use]
pub fn normalize_usage(v: &serde_json::Value) -> Option<(u32, u32, u32)> {
    let obj = v.as_object()?;
    let prompt = obj
        .get("prompt_tokens")
        .or_else(|| obj.get("input_tokens"))
        .and_then(serde_json::Value::as_u64)?;
    let completion = obj
        .get("completion_tokens")
        .or_else(|| obj.get("output_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let total = obj
        .get("total_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(prompt + completion);
    Some((
        u32::try_from(prompt).unwrap_or(u32::MAX),
        u32::try_from(completion).unwrap_or(u32::MAX),
        u32::try_from(total).unwrap_or(u32::MAX),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn records_and_reads_back() {
        let store = UsageStore::new();
        let u = LastUsage {
            prompt_tokens: 1000,
            completion_tokens: 200,
            total_tokens: 1200,
            context_length: Some(4096),
            provider_id: "local".into(),
            model_id: "qwen/qwen3.5-9b".into(),
            is_estimate: false,
            updated_at: Utc::now(),
        };
        store.record("agent.x", u.clone());
        let got = store.get("agent.x").unwrap();
        assert_eq!(got.prompt_tokens, 1000);
        assert_eq!(got.completion_tokens, 200);
        assert_eq!(got.context_length, Some(4096));
    }

    #[test]
    fn later_record_overwrites_earlier() {
        let store = UsageStore::new();
        let base = LastUsage {
            prompt_tokens: 100,
            completion_tokens: 10,
            total_tokens: 110,
            context_length: None,
            provider_id: "p".into(),
            model_id: "m".into(),
            is_estimate: false,
            updated_at: Utc::now(),
        };
        store.record("agent.x", base.clone());
        store.record(
            "agent.x",
            LastUsage {
                prompt_tokens: 500,
                ..base
            },
        );
        assert_eq!(store.get("agent.x").unwrap().prompt_tokens, 500);
    }

    #[test]
    fn normalize_openai_shape() {
        let u = json!({"prompt_tokens": 42, "completion_tokens": 8, "total_tokens": 50});
        assert_eq!(normalize_usage(&u), Some((42, 8, 50)));
    }

    #[test]
    fn normalize_anthropic_shape() {
        let u = json!({"input_tokens": 30, "output_tokens": 10});
        assert_eq!(normalize_usage(&u), Some((30, 10, 40)));
    }

    #[test]
    fn normalize_handles_missing_total() {
        let u = json!({"prompt_tokens": 5, "completion_tokens": 3});
        assert_eq!(normalize_usage(&u), Some((5, 3, 8)));
    }

    #[test]
    fn normalize_rejects_empty() {
        assert_eq!(normalize_usage(&json!({})), None);
        assert_eq!(normalize_usage(&json!(null)), None);
    }
}
