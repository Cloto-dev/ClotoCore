//! Engine routing rules: Cost-First Router (CFR), fallback, and escalation logic.
//!
//! Evaluates per-agent `engine_routing` metadata to select the optimal LLM engine
//! for a given message based on content heuristics, availability, and cost tiers.

/// Marker string in LLM responses that triggers CFR escalation.
const ESCALATION_MARKER: &str = "[[ESCALATE]]";

#[derive(serde::Deserialize)]
pub(super) struct RoutingRule {
    #[serde(rename = "match")]
    condition: String,
    engine: String,
    /// Cost-First Router: try this engine first, escalate if [[ESCALATE]] in response.
    #[serde(default)]
    cfr: bool,
    /// Engine to escalate to when CFR engine returns [[ESCALATE]].
    escalate_to: Option<String>,
    /// Engine to fall back to on 429/5xx/connection errors.
    fallback: Option<String>,
}

impl RoutingRule {
    fn matches(&self, message: &str) -> bool {
        if self.condition == "default" {
            return true;
        }
        if let Some(kw) = self.condition.strip_prefix("contains:") {
            return message.to_lowercase().contains(&kw.to_lowercase());
        }
        if let Some(len) = self.condition.strip_prefix("length:>") {
            if let Ok(n) = len.parse::<usize>() {
                return message.len() > n;
            }
        }
        if self.condition == "tools_likely" {
            let tool_keywords = [
                "調べ",
                "検索",
                "実行",
                "ファイル",
                "search",
                "run",
                "execute",
                "find",
                "research",
            ];
            return tool_keywords
                .iter()
                .any(|k| message.to_lowercase().contains(k));
        }
        false
    }
}

/// Result of engine routing evaluation.
pub(super) struct EngineSelection {
    pub engine_id: String,
    pub cfr: bool,
    pub escalate_to: Option<String>,
    pub fallback: Option<String>,
}

pub(super) fn evaluate_engine_routing(
    message: &str,
    metadata: &std::collections::HashMap<String, String>,
    connected_servers: &[String],
    default_engine_id: &str,
) -> EngineSelection {
    let selection = (|| -> Option<EngineSelection> {
        let rules_json = metadata.get("engine_routing")?;
        let rules: Vec<RoutingRule> = serde_json::from_str(rules_json).ok()?;

        for rule in &rules {
            if !connected_servers.contains(&rule.engine) {
                continue;
            }
            if rule.matches(message) {
                // Validate escalate_to and fallback targets are connected
                let escalate_to = rule
                    .escalate_to
                    .as_ref()
                    .filter(|e| connected_servers.contains(e))
                    .cloned();
                let fallback = rule
                    .fallback
                    .as_ref()
                    .filter(|f| connected_servers.contains(f))
                    .cloned();
                return Some(EngineSelection {
                    engine_id: rule.engine.clone(),
                    cfr: rule.cfr && escalate_to.is_some(),
                    escalate_to,
                    fallback,
                });
            }
        }
        None
    })();

    selection.unwrap_or(EngineSelection {
        engine_id: default_engine_id.to_string(),
        cfr: false,
        escalate_to: None,
        fallback: None,
    })
}

/// Check if response content contains an escalation marker.
pub(super) fn needs_escalation(content: &str) -> bool {
    content.contains(ESCALATION_MARKER)
}

/// Check if an error is retriable (rate limit, server error, connection).
pub(super) fn is_retriable_error(e: &anyhow::Error) -> bool {
    let msg = e.to_string();
    msg.contains("rate_limited")
        || msg.contains("provider_error")
        || msg.contains("connection_failed")
        || msg.contains("timeout")
        || msg.contains("429")
        || msg.contains("502")
        || msg.contains("503")
}
