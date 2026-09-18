//! The turns a model reads for a conversation (docs/CONVERSATIONS_DESIGN.md §2c).
//!
//! Two pure steps, kept out of the dispatch path so they can be tested against
//! mutations without an engine: turning stored chat rows into the messages an
//! engine takes, and merging those with long-term recall and the in-memory
//! transcript into one timeline.

use cloto_shared::{ClotoMessage, MessageSource};
use std::collections::HashSet;

use crate::db::ChatMessageRow;

/// A stored row as the engine sees it. The content column holds content
/// blocks; the engine reads text, so the text blocks are joined and the rest
/// (images, audio) are named rather than dropped silently.
#[must_use]
pub fn row_to_message(row: &ChatMessageRow) -> ClotoMessage {
    let content = text_of_blocks(&row.content);
    let source = match row.source.as_str() {
        "user" => MessageSource::User {
            id: row.user_id.clone(),
            name: row.user_id.clone(),
        },
        "agent" => MessageSource::Agent {
            id: row.agent_id.clone(),
        },
        _ => MessageSource::System,
    };
    let timestamp =
        chrono::DateTime::from_timestamp_millis(row.created_at).unwrap_or_else(chrono::Utc::now);
    ClotoMessage {
        id: row.id.clone(),
        source,
        target_agent: Some(row.agent_id.clone()),
        content,
        timestamp,
        metadata: std::collections::HashMap::new(),
    }
}

fn text_of_blocks(content: &str) -> String {
    let Ok(blocks) = serde_json::from_str::<serde_json::Value>(content) else {
        return content.to_string();
    };
    let Some(blocks) = blocks.as_array() else {
        return content.to_string();
    };
    let mut parts: Vec<String> = Vec::new();
    for block in blocks {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    parts.push(text.to_string());
                }
            }
            Some(other) => parts.push(format!("[{other}]")),
            None => {}
        }
    }
    parts.join("\n")
}

/// Oldest-first messages for the rows the database returned oldest-first.
#[must_use]
pub fn rows_to_messages(rows: &[ChatMessageRow]) -> Vec<ClotoMessage> {
    rows.iter().map(row_to_message).collect()
}

/// One timeline for the engine: the conversation's own turns, long-term
/// recall, and the in-memory transcript, de-duplicated by message id and
/// sorted by time. The message being answered (`current_id`) is never in it —
/// the engine receives that one through its own argument.
///
/// Precedence on a duplicate id is the conversation's row: it is the stored
/// truth, where recall may carry a trimmed copy.
#[must_use]
pub fn merge_context(
    recall: Vec<ClotoMessage>,
    conversation: Vec<ClotoMessage>,
    transcript: Vec<ClotoMessage>,
    current_id: &str,
) -> Vec<ClotoMessage> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut merged: Vec<ClotoMessage> = Vec::new();
    for m in conversation.into_iter().chain(recall).chain(transcript) {
        if !m.id.is_empty() && m.id == current_id {
            continue;
        }
        if m.id.is_empty() || seen.insert(m.id.clone()) {
            merged.push(m);
        }
    }
    merged.sort_by_key(|m| m.timestamp);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, source: &str, content: &str, at: i64) -> ChatMessageRow {
        ChatMessageRow {
            id: id.into(),
            agent_id: "agent.a".into(),
            user_id: "u1".into(),
            source: source.into(),
            content: content.into(),
            metadata: None,
            created_at: at,
            parent_id: None,
            branch_index: 0,
            conversation_id: Some("c1".into()),
        }
    }

    fn msg(id: &str, at: i64) -> ClotoMessage {
        let mut m = ClotoMessage::new(MessageSource::System, id.to_string());
        m.id = id.into();
        m.timestamp = chrono::DateTime::from_timestamp_millis(at).unwrap();
        m
    }

    #[test]
    fn a_row_becomes_the_message_the_engine_takes() {
        let r = row(
            "m1",
            "user",
            r#"[{"type":"text","text":"hello"},{"type":"image","url":"data:..."},{"type":"text","text":"again"}]"#,
            1_700_000_000_000,
        );
        let m = row_to_message(&r);
        assert_eq!(m.id, "m1");
        assert_eq!(m.content, "hello\n[image]\nagain");
        assert!(matches!(m.source, MessageSource::User { ref id, .. } if id == "u1"));
        assert_eq!(m.timestamp.timestamp_millis(), 1_700_000_000_000);

        let a = row_to_message(&row("m2", "agent", r#"[{"type":"text","text":"hi"}]"#, 1));
        assert!(matches!(a.source, MessageSource::Agent { ref id } if id == "agent.a"));
        // Content that is not a block list is passed through as it is.
        assert_eq!(
            row_to_message(&row("m3", "user", "plain", 1)).content,
            "plain"
        );
    }

    #[test]
    fn the_merge_is_one_timeline_without_the_message_being_answered() {
        let conversation = vec![msg("c1", 10), msg("c2", 20), msg("now", 30)];
        let recall = vec![msg("r1", 5), msg("c2", 20)];
        let transcript = vec![msg("t1", 25), msg("c1", 10)];
        let merged = merge_context(recall, conversation, transcript, "now");
        let ids: Vec<&str> = merged.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["r1", "c1", "c2", "t1"]);
    }

    #[test]
    fn a_duplicate_id_keeps_the_conversation_row() {
        let mut stored = msg("x", 10);
        stored.content = "the whole text".into();
        let mut recalled = msg("x", 10);
        recalled.content = "the whole".into();
        let merged = merge_context(vec![recalled], vec![stored], vec![], "other");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].content, "the whole text");
    }
}
