//! Default engine-routing plugin: Cost-First Router (CFR), fallback, escalation.
//!
//! bug-293 (§1.4 Data Sovereignty): the `engine_routing` agent metadata is a
//! plugin-specific schema. The kernel must not interpret it. This in-tree plugin
//! owns the `RoutingRule` schema and the `serde_json` deserialization; the kernel
//! handler only forwards opaque `metadata` (JSON) and consumes the resulting
//! `EngineSelection`. The kernel never names the `engine_routing` key or the
//! `RoutingRule` fields.
//!
//! Evaluates per-agent `engine_routing` metadata to select the optimal LLM engine
//! for a given message based on content heuristics, availability, and cost tiers.

/// Marker string in LLM responses that triggers CFR escalation.
const ESCALATION_MARKER: &str = "[[ESCALATE]]";

#[derive(serde::Deserialize)]
struct RoutingRule {
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
            ];
            return tool_keywords
                .iter()
                .any(|k| message.to_lowercase().contains(k));
        }
        false
    }
}

/// Result of engine routing evaluation.
pub(crate) struct EngineSelection {
    pub engine_id: String,
    pub cfr: bool,
    pub escalate_to: Option<String>,
    pub fallback: Option<String>,
}

pub(crate) fn evaluate_engine_routing(
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
pub(crate) fn needs_escalation(content: &str) -> bool {
    content.contains(ESCALATION_MARKER)
}

/// Check if an error is retriable (rate limit, server error, connection).
pub(crate) fn is_retriable_error(e: &anyhow::Error) -> bool {
    let msg = e.to_string();
    msg.contains("rate_limited")
        || msg.contains("provider_error")
        || msg.contains("connection_failed")
        || msg.contains("timeout")
        || msg.contains("429")
        || msg.contains("502")
        || msg.contains("503")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn rules_metadata(json: &str) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("engine_routing".to_string(), json.to_string());
        m
    }

    #[test]
    fn no_metadata_falls_back_to_default() {
        let sel = evaluate_engine_routing("hi", &HashMap::new(), &[], "default-engine");
        assert_eq!(sel.engine_id, "default-engine");
        assert!(!sel.cfr);
        assert!(sel.escalate_to.is_none());
        assert!(sel.fallback.is_none());
    }

    #[test]
    fn malformed_json_falls_back_to_default() {
        let md = rules_metadata("not json");
        let sel = evaluate_engine_routing("hi", &md, &["e1".into()], "default-engine");
        assert_eq!(sel.engine_id, "default-engine");
    }

    #[test]
    fn contains_rule_selects_engine_when_connected() {
        let md = rules_metadata(
            r#"[{"match":"contains:code","engine":"fast"},{"match":"default","engine":"smart"}]"#,
        );
        let connected = vec!["fast".to_string(), "smart".to_string()];
        let sel = evaluate_engine_routing("write code please", &md, &connected, "default-engine");
        assert_eq!(sel.engine_id, "fast");
    }

    #[test]
    fn rule_skipped_when_engine_not_connected() {
        let md = rules_metadata(
            r#"[{"match":"default","engine":"fast"},{"match":"default","engine":"smart"}]"#,
        );
        // Only "smart" is connected, so "fast" rule is skipped.
        let connected = vec!["smart".to_string()];
        let sel = evaluate_engine_routing("anything", &md, &connected, "default-engine");
        assert_eq!(sel.engine_id, "smart");
    }

    #[test]
    fn cfr_disabled_when_escalate_target_offline() {
        let md = rules_metadata(
            r#"[{"match":"default","engine":"fast","cfr":true,"escalate_to":"smart"}]"#,
        );
        // escalate_to "smart" is not connected → cfr must be disabled.
        let connected = vec!["fast".to_string()];
        let sel = evaluate_engine_routing("x", &md, &connected, "default-engine");
        assert_eq!(sel.engine_id, "fast");
        assert!(!sel.cfr);
        assert!(sel.escalate_to.is_none());
    }

    #[test]
    fn escalation_marker_detected() {
        assert!(needs_escalation("blah [[ESCALATE]] blah"));
        assert!(!needs_escalation("no marker here"));
    }
}
