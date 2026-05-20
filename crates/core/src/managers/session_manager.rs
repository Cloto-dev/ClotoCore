//! T1 (in-flight conversation) state for the agentic loop.
//!
//! `SessionManager` holds the live transcript and `tool_history` for each
//! `(agent_id, bridge_session_id)` pair so the agentic loop can resume
//! mid-conversation without rebuilding context from the source-of-truth
//! plugin (Discord REST, CPersona recall, etc.) on every callback. Bridges
//! still own their source-specific "what counts as a continuous session"
//! semantics (Discord's `ChunkTracker` time-gap, slash command lifecycle,
//! etc.) and emit a stable `bridge_session_id` per turn; the kernel just
//! keys T1 by `{agent_id, bridge_session_id}` and treats the id as opaque
//! per [Principle 1.2 Capability over Concrete Type].
//!
//! # Tier model
//!
//! Each session lives in one of four tiers, picked from `last_active`:
//!
//! - **HotActive** (0–30 min): full transcript + full tool_history, prompt
//!   cache is warm, the agentic loop appends in place.
//! - **HotStashed** (30–60 min): transcript + tool_history still intact but
//!   prompt cache is cold; if the user returns inside the stash window, the
//!   loop resumes seamlessly.
//! - **Warm** (60 min – 24 h): tool_history is GC'd (mid-task plans go stale
//!   fast), transcript is kept so the LLM still sees "we were talking";
//!   callers should summarise the transcript before feeding it back.
//! - **Cold** (≥ 24 h): the session is evicted from memory. Individual
//!   messages have already been persisted to CPersona via the existing
//!   per-turn `store` calls (`handlers/system.rs:store_user_message`,
//!   `store_assistant_message`), and `maybe_archive_episode` will fold them
//!   into a summary once the unarchived threshold is hit — no separate
//!   archive call is needed here.
//!
//! # Persistence
//!
//! State is **in-memory only** per [Principle 1.4 Data Sovereignty]: no new
//! kernel SQL table, no new migration. Kernel restart drops every active
//! T1, which is acceptable because CPersona already carries the durable
//! record. The user experience after restart is "same agent, same memories,
//! fresh active turn" — the same fallback the system already exhibits when
//! a chunk gap rolls over to a new bridge_session_id.

use cloto_shared::ClotoMessage;
use dashmap::DashMap;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Default time-gap after which a session falls out of `HotActive`.
pub const DEFAULT_HOT_GAP: Duration = Duration::from_mins(30);
/// Default extra window where tool_history is still recoverable (soft archive).
pub const DEFAULT_STASH_TTL: Duration = Duration::from_mins(30);
/// Default upper bound for keeping a transcript before Cold eviction.
pub const DEFAULT_WARM_TTL: Duration = Duration::from_hours(24);

/// Opaque per-agent session key.
///
/// `bridge_session_id` is whatever the source bridge emits in
/// `msg.metadata["external_session_id"]` (or wherever else a future bridge
/// chooses to put it). The kernel never parses it.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct SessionKey {
    pub agent_id: String,
    pub bridge_session_id: String,
}

impl SessionKey {
    #[must_use]
    pub fn new(agent_id: impl Into<String>, bridge_session_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            bridge_session_id: bridge_session_id.into(),
        }
    }
}

/// Lifecycle tier of a `T1` session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionTier {
    HotActive,
    HotStashed,
    Warm,
    Cold,
}

impl SessionTier {
    /// Pick the tier for an `elapsed` duration using the configured policy.
    #[must_use]
    pub fn from_elapsed(
        elapsed: Duration,
        hot_gap: Duration,
        stash_ttl: Duration,
        warm_ttl: Duration,
    ) -> Self {
        if elapsed <= hot_gap {
            Self::HotActive
        } else if elapsed <= hot_gap + stash_ttl {
            Self::HotStashed
        } else if elapsed <= warm_ttl {
            Self::Warm
        } else {
            Self::Cold
        }
    }
}

/// In-flight conversation state for one (agent, bridge_session) pair.
#[derive(Debug)]
pub struct AgentSession {
    /// Ordered transcript — append-only during HotActive/HotStashed, may be
    /// summarised by the caller during Warm.
    pub transcript: Vec<ClotoMessage>,
    /// Latest tool_history emitted by the agentic loop. Cleared on the
    /// HotStashed → Warm transition.
    pub tool_history: Vec<Value>,
    /// `Instant` of the last append. Drives tier transitions and Cold
    /// eviction.
    pub last_active: Instant,
    /// First-touch time, retained for diagnostics.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl AgentSession {
    fn new() -> Self {
        Self {
            transcript: Vec::new(),
            tool_history: Vec::new(),
            last_active: Instant::now(),
            created_at: chrono::Utc::now(),
        }
    }

    /// Tier as observed at `now`.
    #[must_use]
    pub fn tier(
        &self,
        now: Instant,
        hot_gap: Duration,
        stash_ttl: Duration,
        warm_ttl: Duration,
    ) -> SessionTier {
        let elapsed = now.saturating_duration_since(self.last_active);
        SessionTier::from_elapsed(elapsed, hot_gap, stash_ttl, warm_ttl)
    }

    /// Append a message and refresh `last_active`.
    pub fn append(&mut self, msg: ClotoMessage) {
        self.transcript.push(msg);
        self.last_active = Instant::now();
    }

    /// Overwrite the tool_history snapshot and refresh `last_active`.
    pub fn record_tool_history(&mut self, history: Vec<Value>) {
        self.tool_history = history;
        self.last_active = Instant::now();
    }
}

/// Process-lifetime store of `AgentSession`s, keyed by [`SessionKey`].
///
/// Cloneable wrapper around an `Arc<DashMap>` so handlers can hold it
/// without taking explicit refs. Per the module doc, this is intentionally
/// in-memory only.
#[derive(Clone)]
pub struct SessionManager {
    inner: Arc<DashMap<SessionKey, AgentSession>>,
    hot_gap: Duration,
    stash_ttl: Duration,
    warm_ttl: Duration,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::with_policy(DEFAULT_HOT_GAP, DEFAULT_STASH_TTL, DEFAULT_WARM_TTL)
    }
}

impl SessionManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with a custom tier policy (used by tests).
    #[must_use]
    pub fn with_policy(hot_gap: Duration, stash_ttl: Duration, warm_ttl: Duration) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            hot_gap,
            stash_ttl,
            warm_ttl,
        }
    }

    /// Pull the configured policy, mostly so the background cleanup task can
    /// share thresholds with `tier()` decisions.
    #[must_use]
    pub fn policy(&self) -> (Duration, Duration, Duration) {
        (self.hot_gap, self.stash_ttl, self.warm_ttl)
    }

    /// Currently-tracked session count, for diagnostics / tests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// True when no sessions are tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Append `msg` to the session identified by `key`, creating it if
    /// needed, and return the resulting tier observed at `Instant::now()`.
    ///
    /// The returned tier is informational — callers don't need to branch on
    /// it for the basic forward path. It's there so the agentic loop can
    /// decide whether to summarise the transcript when it returns Warm.
    pub fn append_message(&self, key: &SessionKey, msg: ClotoMessage) -> SessionTier {
        let mut entry = self
            .inner
            .entry(key.clone())
            .or_insert_with(AgentSession::new);
        entry.append(msg);
        entry.tier(Instant::now(), self.hot_gap, self.stash_ttl, self.warm_ttl)
    }

    /// Record the agentic loop's emitted `tool_history` on the session, or
    /// no-op if the session is missing (e.g. concurrently evicted).
    pub fn record_tool_history(&self, key: &SessionKey, history: Vec<Value>) {
        if let Some(mut entry) = self.inner.get_mut(key) {
            entry.record_tool_history(history);
        }
    }

    /// Read a snapshot of the transcript for `key`, or empty if no session.
    /// The clone is intentional: callers feed it straight into the LLM
    /// context builder and shouldn't hold the DashMap lock across awaits.
    #[must_use]
    pub fn snapshot_transcript(&self, key: &SessionKey) -> Vec<ClotoMessage> {
        self.inner
            .get(key)
            .map(|s| s.transcript.clone())
            .unwrap_or_default()
    }

    /// Read a snapshot of the tool_history.
    #[must_use]
    pub fn snapshot_tool_history(&self, key: &SessionKey) -> Vec<Value> {
        self.inner
            .get(key)
            .map(|s| s.tool_history.clone())
            .unwrap_or_default()
    }

    /// Current tier for `key` (or `Cold` if the session is missing — same
    /// observable behaviour as "the session has been evicted").
    #[must_use]
    pub fn tier_of(&self, key: &SessionKey) -> SessionTier {
        self.inner.get(key).map_or(SessionTier::Cold, |s| {
            s.tier(Instant::now(), self.hot_gap, self.stash_ttl, self.warm_ttl)
        })
    }

    /// Apply tier transitions to every session.
    ///
    /// - HotStashed → Warm clears `tool_history` (mid-task plans are stale)
    /// - Warm → Cold evicts the entry.
    ///
    /// Returns the number of entries that were evicted, so the caller can
    /// log/meter cleanup pressure.
    #[must_use]
    pub fn run_cleanup(&self) -> usize {
        let now = Instant::now();
        let (hot, stash, warm) = (self.hot_gap, self.stash_ttl, self.warm_ttl);
        let mut evicted = 0;
        self.inner.retain(|_, session| {
            let tier = session.tier(now, hot, stash, warm);
            match tier {
                SessionTier::Cold => {
                    evicted += 1;
                    false
                }
                SessionTier::Warm => {
                    if !session.tool_history.is_empty() {
                        session.tool_history.clear();
                    }
                    true
                }
                SessionTier::HotActive | SessionTier::HotStashed => true,
            }
        });
        evicted
    }

    /// Manually evict a session (used by tests and prospective explicit-reset
    /// flows like `/reset`; the agentic loop itself never evicts).
    pub fn evict(&self, key: &SessionKey) {
        self.inner.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cloto_shared::MessageSource;
    use std::collections::HashMap;

    fn user_msg(content: &str) -> ClotoMessage {
        ClotoMessage {
            id: "m".into(),
            source: MessageSource::User {
                id: "u".into(),
                name: "u".into(),
            },
            target_agent: None,
            content: content.into(),
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn from_elapsed_picks_correct_tier() {
        let h = Duration::from_secs(30);
        let s = Duration::from_secs(30);
        let w = Duration::from_secs(120);
        assert_eq!(
            SessionTier::from_elapsed(Duration::from_secs(0), h, s, w),
            SessionTier::HotActive
        );
        assert_eq!(
            SessionTier::from_elapsed(Duration::from_secs(30), h, s, w),
            SessionTier::HotActive
        );
        assert_eq!(
            SessionTier::from_elapsed(Duration::from_secs(31), h, s, w),
            SessionTier::HotStashed
        );
        assert_eq!(
            SessionTier::from_elapsed(Duration::from_secs(60), h, s, w),
            SessionTier::HotStashed
        );
        assert_eq!(
            SessionTier::from_elapsed(Duration::from_secs(61), h, s, w),
            SessionTier::Warm
        );
        assert_eq!(
            SessionTier::from_elapsed(Duration::from_secs(120), h, s, w),
            SessionTier::Warm
        );
        assert_eq!(
            SessionTier::from_elapsed(Duration::from_secs(121), h, s, w),
            SessionTier::Cold
        );
    }

    #[test]
    fn append_creates_and_grows_transcript() {
        let sm = SessionManager::new();
        let key = SessionKey::new("agent.a", "discord:123:1");
        let tier = sm.append_message(&key, user_msg("hello"));
        assert_eq!(tier, SessionTier::HotActive);
        sm.append_message(&key, user_msg("world"));
        let t = sm.snapshot_transcript(&key);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].content, "hello");
        assert_eq!(t[1].content, "world");
    }

    #[test]
    fn distinct_keys_are_isolated() {
        let sm = SessionManager::new();
        let a = SessionKey::new("agent.a", "discord:1:1");
        let b = SessionKey::new("agent.a", "discord:2:1");
        sm.append_message(&a, user_msg("A"));
        sm.append_message(&b, user_msg("B"));
        assert_eq!(sm.snapshot_transcript(&a).len(), 1);
        assert_eq!(sm.snapshot_transcript(&b).len(), 1);
        assert_eq!(sm.snapshot_transcript(&a)[0].content, "A");
    }

    #[test]
    fn distinct_agent_ids_are_isolated_on_same_session() {
        let sm = SessionManager::new();
        let a = SessionKey::new("agent.sapphy", "discord:42:1");
        let b = SessionKey::new("agent.ks22", "discord:42:1");
        sm.append_message(&a, user_msg("A1"));
        sm.append_message(&b, user_msg("B1"));
        assert_eq!(sm.snapshot_transcript(&a)[0].content, "A1");
        assert_eq!(sm.snapshot_transcript(&b)[0].content, "B1");
    }

    #[test]
    fn record_tool_history_overwrites() {
        let sm = SessionManager::new();
        let key = SessionKey::new("agent.a", "s1");
        sm.append_message(&key, user_msg("turn 1"));
        sm.record_tool_history(&key, vec![serde_json::json!({"name": "search"})]);
        let h = sm.snapshot_tool_history(&key);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0]["name"], "search");

        sm.record_tool_history(
            &key,
            vec![
                serde_json::json!({"name": "search"}),
                serde_json::json!({"name": "read"}),
            ],
        );
        assert_eq!(sm.snapshot_tool_history(&key).len(), 2);
    }

    #[test]
    fn record_tool_history_on_missing_session_is_noop() {
        let sm = SessionManager::new();
        let key = SessionKey::new("agent.a", "ghost");
        sm.record_tool_history(&key, vec![serde_json::json!({"name": "x"})]);
        assert!(sm.is_empty());
        assert_eq!(sm.snapshot_tool_history(&key), Vec::<Value>::new());
    }

    #[test]
    fn tier_of_missing_session_is_cold() {
        let sm = SessionManager::new();
        let key = SessionKey::new("agent.a", "nope");
        assert_eq!(sm.tier_of(&key), SessionTier::Cold);
    }

    #[test]
    fn cleanup_evicts_cold_sessions() {
        // 0 ms thresholds → everything immediately Cold.
        let sm = SessionManager::with_policy(
            Duration::from_millis(0),
            Duration::from_millis(0),
            Duration::from_millis(0),
        );
        let k = SessionKey::new("agent.a", "s1");
        sm.append_message(&k, user_msg("hi"));
        std::thread::sleep(Duration::from_millis(2));
        let evicted = sm.run_cleanup();
        assert_eq!(evicted, 1);
        assert!(sm.is_empty());
    }

    #[test]
    fn cleanup_clears_tool_history_in_warm() {
        // Hot=0, stash=0, warm=10s → anything past 0 is Warm (until 10s).
        let sm = SessionManager::with_policy(
            Duration::from_millis(0),
            Duration::from_millis(0),
            Duration::from_secs(10),
        );
        let k = SessionKey::new("agent.a", "s1");
        sm.append_message(&k, user_msg("hi"));
        sm.record_tool_history(&k, vec![serde_json::json!({"name": "x"})]);
        std::thread::sleep(Duration::from_millis(2));

        assert_eq!(sm.tier_of(&k), SessionTier::Warm);
        sm.run_cleanup();

        // Session retained (still Warm), tool_history wiped, transcript kept.
        assert_eq!(sm.snapshot_transcript(&k).len(), 1);
        assert_eq!(sm.snapshot_tool_history(&k), Vec::<Value>::new());
    }

    #[test]
    fn cleanup_keeps_hot_sessions_intact() {
        let sm = SessionManager::new();
        let k = SessionKey::new("agent.a", "s1");
        sm.append_message(&k, user_msg("hi"));
        sm.record_tool_history(&k, vec![serde_json::json!({"name": "x"})]);
        let evicted = sm.run_cleanup();
        assert_eq!(evicted, 0);
        assert_eq!(sm.snapshot_transcript(&k).len(), 1);
        assert_eq!(sm.snapshot_tool_history(&k).len(), 1);
    }

    #[test]
    fn evict_removes_session() {
        let sm = SessionManager::new();
        let k = SessionKey::new("agent.a", "s1");
        sm.append_message(&k, user_msg("hi"));
        sm.evict(&k);
        assert!(sm.is_empty());
        // ClotoMessage doesn't implement PartialEq, so probe via len.
        assert_eq!(sm.snapshot_transcript(&k).len(), 0);
    }
}
