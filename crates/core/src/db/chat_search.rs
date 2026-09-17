//! Search across the dashboard chat: every conversation's messages, archived
//! conversations included (`docs/CONVERSATIONS_DESIGN.md` §2(f)).
//!
//! The index is `chat_message_search`, kept by triggers on `chat_messages`
//! (migration `20260917120000_add_chat_message_search.sql`). It holds each
//! message's text blocks as trigram tokens, so a substring of Japanese text is
//! found as readily as an English word.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::db_timeout;

/// Terms shorter than this are below what a trigram index can serve, and are
/// matched by scanning the indexed text instead.
// HARDCODED(SQLite FTS5 documentation, "The Trigram Tokenizer"): the tokenizer
// emits three-character tokens, so the index cannot answer a shorter term.
const TRIGRAM_CHARS: usize = 3;

/// Characters of context kept before the first match in a snippet.
const SNIPPET_BEFORE_CHARS: usize = 30;
/// Characters kept from the first match onwards.
const SNIPPET_AFTER_CHARS: usize = 90;

/// One message that matched, with what a result row needs to be shown and
/// opened.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatSearchHit {
    pub message_id: String,
    pub agent_id: String,
    pub conversation_id: Option<String>,
    pub conversation_title: Option<String>,
    /// The conversation is archived: still found, not in the sidebar.
    pub archived: bool,
    pub source: String,
    pub created_at: i64,
    /// The text around the first match, on one line.
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSearch {
    /// Newest first, at most the limit asked for.
    pub hits: Vec<ChatSearchHit>,
    /// Every message that matched, whether or not it is in `hits`.
    pub total: i64,
}

impl ChatSearch {
    /// More matched than were returned. Said outright so a caller never reads
    /// a cut-off list as the whole answer.
    #[must_use]
    pub fn truncated(&self) -> bool {
        usize::try_from(self.total).unwrap_or(usize::MAX) > self.hits.len()
    }
}

/// The whitespace-separated terms of a query. Every term must occur in a
/// message for it to match.
#[must_use]
pub fn search_terms(query: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for term in query.split_whitespace() {
        if !terms.iter().any(|t| t == term) {
            terms.push(term.to_string());
        }
    }
    terms
}

/// Messages of `user_id` (and the `system` user, as the message list reads
/// them) whose text contains every term of `query`, newest first.
///
/// An empty query matches nothing.
pub async fn search_chat_messages(
    pool: &SqlitePool,
    user_id: &str,
    query: &str,
    limit: i64,
) -> anyhow::Result<ChatSearch> {
    let terms = search_terms(query);
    if terms.is_empty() {
        return Ok(ChatSearch {
            hits: Vec::new(),
            total: 0,
        });
    }

    // Long terms go to the index as one MATCH of quoted phrases (implicit AND);
    // short ones are LIKE conditions on the indexed text.
    let (long, short): (Vec<&String>, Vec<&String>) = terms
        .iter()
        .partition(|t| t.chars().count() >= TRIGRAM_CHARS);
    let mut clauses: Vec<&str> = Vec::new();
    let match_expr = (!long.is_empty()).then(|| {
        long.iter()
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" ")
    });
    if match_expr.is_some() {
        clauses.push("s.body MATCH ?");
    }
    let like_patterns: Vec<String> = short.iter().map(|t| like_pattern(t)).collect();
    clauses.extend(std::iter::repeat_n(
        "s.body LIKE ? ESCAPE '\\'",
        like_patterns.len(),
    ));
    clauses.push("(m.user_id = ? OR m.user_id = 'system')");
    let where_clause = clauses.join(" AND ");

    let from = "FROM chat_message_search s
         JOIN chat_message_search_keys k ON k.key = s.rowid
         JOIN chat_messages m ON m.id = k.message_id
         LEFT JOIN conversations c ON c.id = m.conversation_id";

    let count_sql = format!("SELECT COUNT(*) {from} WHERE {where_clause}");
    let rows_sql = format!(
        "SELECT m.id, m.agent_id, m.conversation_id, c.title, c.archived_at, m.source, m.created_at, s.body
         {from} WHERE {where_clause}
         ORDER BY m.created_at DESC, m.id
         LIMIT ?"
    );

    // The audit `AssertSqlSafe` asks for: the interpolated clauses are fixed
    // strings chosen by how many terms there are; every term reaches SQLite as
    // a bind parameter.
    let mut count = sqlx::query_as::<_, (i64,)>(sqlx::AssertSqlSafe(count_sql));
    let mut rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            String,
            i64,
            String,
        ),
    >(sqlx::AssertSqlSafe(rows_sql));
    if let Some(expr) = &match_expr {
        count = count.bind(expr);
        rows = rows.bind(expr);
    }
    for pattern in &like_patterns {
        count = count.bind(pattern);
        rows = rows.bind(pattern);
    }
    count = count.bind(user_id);
    rows = rows.bind(user_id).bind(limit);

    let (total,) = db_timeout(count.fetch_one(pool)).await?;
    let rows = db_timeout(rows.fetch_all(pool)).await?;

    let hits = rows
        .into_iter()
        .map(
            |(
                message_id,
                agent_id,
                conversation_id,
                title,
                archived_at,
                source,
                created_at,
                body,
            )| {
                ChatSearchHit {
                    snippet: snippet(&body, &terms),
                    message_id,
                    agent_id,
                    conversation_id,
                    conversation_title: title,
                    archived: archived_at.is_some(),
                    source,
                    created_at,
                }
            },
        )
        .collect();

    Ok(ChatSearch { hits, total })
}

/// A LIKE pattern matching `term` anywhere, with the pattern's own wildcards
/// in the term taken literally.
fn like_pattern(term: &str) -> String {
    let mut pattern = String::with_capacity(term.len() + 2);
    pattern.push('%');
    for c in term.chars() {
        if matches!(c, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

/// The text around the earliest occurrence of any term, whitespace collapsed
/// to single spaces, with an ellipsis where it was cut.
#[must_use]
pub fn snippet(body: &str, terms: &[String]) -> String {
    let text: Vec<char> = body
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .collect();
    let folded: Vec<char> = text.iter().map(|c| fold(*c)).collect();

    let first = terms
        .iter()
        .filter_map(|term| {
            let needle: Vec<char> = term.chars().map(fold).collect();
            find(&folded, &needle)
        })
        .min()
        .unwrap_or(0);

    let start = first.saturating_sub(SNIPPET_BEFORE_CHARS);
    let end = (first + SNIPPET_AFTER_CHARS).min(text.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&text[start..end]);
    if end < text.len() {
        out.push('…');
    }
    out
}

/// Case folding for finding a match, one character for one so positions in the
/// folded text are positions in the original.
fn fold(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

fn find(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_are_split_on_whitespace_and_deduplicated() {
        assert_eq!(
            search_terms("  カリン  挨拶\tカリン "),
            vec!["カリン".to_string(), "挨拶".to_string()]
        );
        assert!(search_terms(" \n ").is_empty());
    }

    #[test]
    fn like_wildcards_in_a_term_are_literal() {
        assert_eq!(like_pattern("5%"), "%5\\%%");
        assert_eq!(like_pattern("a_b"), "%a\\_b%");
        assert_eq!(like_pattern("c\\d"), "%c\\\\d%");
    }

    #[test]
    fn a_snippet_is_cut_around_the_first_match_on_one_line() {
        let before = "あ".repeat(50);
        let after = "い".repeat(120);
        let body = format!("{before}\n\n見つけたい言葉{after}");
        let s = snippet(&body, &["見つけたい".to_string()]);
        assert!(s.starts_with('…') && s.ends_with('…'), "{s}");
        assert!(
            s.contains(" 見つけたい言葉"),
            "newlines collapse to a space: {s}"
        );
        assert_eq!(
            s.chars().filter(|c| *c == 'あ').count(),
            SNIPPET_BEFORE_CHARS - 1,
            "the context before the match is kept, less the collapsed space"
        );
    }

    #[test]
    fn a_snippet_starts_at_the_earliest_of_several_terms_and_ignores_case() {
        // The prefix is longer than a snippet, so a match that is not found
        // leaves the snippet at the start of the text, without either term.
        let body = format!("{} Beta then alpha", "x ".repeat(100));
        let s = snippet(&body, &["ALPHA".to_string(), "beta".to_string()]);
        assert!(s.starts_with('…'), "the snippet moved to the match: {s}");
        let beta = s.find("Beta").expect("the earlier term is in the snippet");
        assert!(s[..beta].ends_with("x "), "{s}");
    }

    #[test]
    fn a_short_body_is_whole() {
        assert_eq!(snippet("short text", &["text".to_string()]), "short text");
    }
}
