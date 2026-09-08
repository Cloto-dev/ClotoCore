//! Agent-scoped tokens, for callers that must not be trusted to name themselves.
//!
//! The kernel's per-agent capability gate ([`crate::db::resolve_tool_access`],
//! applied in `McpClientManager::enforce_caller_grant`) only works if the kernel
//! knows *which* agent is calling. Every existing path knows that structurally:
//! the agentic loop dispatches on behalf of an agent it already holds. One path
//! does not — `POST /api/mcp/call`, the coordinator endpoint, which authenticates
//! with the admin key and therefore runs as [`Caller::System`], bypassing the gate
//! outright.
//!
//! That was sound while the only holders of the admin key were the kernel and
//! servers it spawned. It stops being sound as soon as an external CLI harness is
//! the caller: the harness runs a shell, so anything reachable from its process
//! tree is reachable by the model inside it. A caller that can read the admin key
//! can also decline to attach the `_mgp.delegation` envelope that would have
//! narrowed it — and §5.6.1's permission intersection only runs when that envelope
//! is present. Self-declared identity is not a boundary when the declarer is the
//! party being restrained.
//!
//! A token minted here is the other shape: the kernel derives the caller from the
//! token, so the caller never gets to say who it is.
//!
//! # What the TTL is and is not
//!
//! The TTL is housekeeping — it bounds the size of the map, nothing more. It is
//! **not** the security boundary. A token grants exactly the permissions its agent
//! already has through every other dispatch path, so a token that leaks out of a
//! harness hands that harness nothing it did not already hold. What the token
//! withholds is *other agents'* permissions, and that does not decay with time.
//!
//! # Relationship to the LLM proxy token
//!
//! [`crate::managers::llm_proxy`] reaches for the same idea one level over: a
//! secret that is not the admin key, handed to a child in its environment, scoped
//! to a single surface, dying with the kernel process. This differs in one respect
//! only, and it is the point of the module — the proxy's token is shared by every
//! child, because it answers "may you spend provider credit". Ours must be per
//! agent, because it answers "who are you", and one shared answer to that question
//! is the self-declared identity it exists to replace.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use tokio::sync::RwLock;

/// Header the kernel reads an agent token from.
///
/// The kernel is the source of truth for this name; a bridge that presents the
/// token mirrors it from here.
pub const AGENT_TOKEN_HEADER: &str = "X-Agent-Token";

/// Agent-metadata key a dispatch carries a minted token in.
///
/// The kernel is the source of truth for this name too; an engine that reads
/// the token mirrors it from here. It shares the metadata map with keys the
/// system-prompt renderer reads, and is deliberately not one of them — see
/// `McpClientManager::enrich_agent_for_dispatch`.
pub const METADATA_AGENT_TOKEN: &str = "agent_token";

/// Environment variable naming the kernel's own address, injected into every
/// spawned MCP server.
///
/// A child that was handed a token still has to know where to present it, and
/// the kernel is the only party that knows what it bound. The name matches the
/// one the coordinator template already reads, so a server written against
/// either finds it.
pub const KERNEL_URL_ENV: &str = "CLOTO_KERNEL_URL";

/// MGP extension a server declares to ask for [`METADATA_AGENT_TOKEN`].
///
/// Negotiation intersects this with the kernel's own list, so declaring it is
/// a request rather than a grant.
pub const AGENT_TOKEN_EXTENSION: &str = "agent_token";

/// How long a minted token stays resolvable when the caller asks for the default.
///
/// Long enough that a slow harness run does not lose its tools half-way through,
/// which is the only failure this number can cause — see the module docs for why
/// it is not a security parameter.
pub const DEFAULT_TTL_MINUTES: i64 = 60;

/// Checked when the crate builds, not when the suite runs: lowering the TTL past
/// the length of a plausible run is the one failure this number can cause on its
/// own, and a build error says so before anything ships.
const _: () = assert!(DEFAULT_TTL_MINUTES >= 30);

struct Entry {
    agent_id: String,
    expires_at: DateTime<Utc>,
}

/// In-memory map of token fingerprint → agent identity.
///
/// Deliberately not persisted. A restart invalidates every token, which is
/// correct: the runs that held them did not survive the restart either.
#[derive(Default)]
pub struct AgentTokenStore {
    entries: RwLock<HashMap<String, Entry>>,
}

impl AgentTokenStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a token bound to `agent_id`, valid for `ttl`.
    ///
    /// The raw token is returned exactly once and is not recoverable from the
    /// store afterwards — only its fingerprint is kept.
    pub async fn mint(&self, agent_id: &str, ttl: Duration) -> String {
        let token = crate::apikey::generate();
        let entry = Entry {
            agent_id: agent_id.to_string(),
            expires_at: Utc::now() + ttl,
        };
        let mut entries = self.entries.write().await;
        Self::prune(&mut entries, Utc::now());
        entries.insert(fingerprint(&token), entry);
        token
    }

    /// Mint with [`DEFAULT_TTL_MINUTES`].
    pub async fn mint_default(&self, agent_id: &str) -> String {
        self.mint(agent_id, Duration::minutes(DEFAULT_TTL_MINUTES))
            .await
    }

    /// The agent this token names, or `None` if it is unknown or expired.
    ///
    /// An expired entry answers exactly as an unknown one does. The caller must
    /// not be able to tell "your token ran out" from "no such token" — the first
    /// confirms a token was once valid, which is a fact about someone else's run.
    pub async fn resolve(&self, token: &str) -> Option<String> {
        let fp = fingerprint(token);
        let entries = self.entries.read().await;
        entries
            .get(&fp)
            .filter(|e| e.expires_at > Utc::now())
            .map(|e| e.agent_id.clone())
    }

    /// Drop every token naming `agent_id`.
    ///
    /// For the case where an agent is disabled or deleted mid-run: the grants
    /// backing it are gone, and a token that outlives them would resolve to an
    /// identity the rest of the kernel no longer honours.
    pub async fn revoke_agent(&self, agent_id: &str) {
        let mut entries = self.entries.write().await;
        entries.retain(|_, e| e.agent_id != agent_id);
    }

    /// Number of live (unexpired) tokens. For tests and diagnostics.
    pub async fn live_count(&self) -> usize {
        let now = Utc::now();
        let entries = self.entries.read().await;
        entries.values().filter(|e| e.expires_at > now).count()
    }

    fn prune(entries: &mut HashMap<String, Entry>, now: DateTime<Utc>) {
        entries.retain(|_, e| e.expires_at > now);
    }
}

/// Domain-separated SHA-256 of a token.
///
/// The store holds fingerprints rather than tokens so that a memory dump, a
/// panic payload or a debug print of the map does not spill anything a caller
/// could present. The salt is defined here and copied from nowhere.
fn fingerprint(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"cloto-agent-token:");
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_minted_token_resolves_to_the_agent_it_names() {
        let store = AgentTokenStore::new();
        let token = store.mint_default("agent.growth").await;
        assert_eq!(store.resolve(&token).await.as_deref(), Some("agent.growth"));
    }

    #[tokio::test]
    async fn two_agents_do_not_share_a_token() {
        let store = AgentTokenStore::new();
        let a = store.mint_default("agent.a").await;
        let b = store.mint_default("agent.b").await;
        assert_ne!(a, b, "each mint must produce a distinct secret");
        assert_eq!(store.resolve(&a).await.as_deref(), Some("agent.a"));
        assert_eq!(store.resolve(&b).await.as_deref(), Some("agent.b"));
    }

    #[tokio::test]
    async fn an_unknown_token_resolves_to_nothing() {
        let store = AgentTokenStore::new();
        store.mint_default("agent.a").await;
        assert_eq!(store.resolve("not-a-token").await, None);
        // A token-shaped string is no better than an obviously wrong one.
        assert_eq!(store.resolve(&crate::apikey::generate()).await, None);
    }

    #[tokio::test]
    async fn an_expired_token_resolves_to_nothing() {
        let store = AgentTokenStore::new();
        let token = store.mint("agent.a", Duration::seconds(-1)).await;
        assert_eq!(
            store.resolve(&token).await,
            None,
            "an expired token must answer exactly as an unknown one does"
        );
    }

    #[tokio::test]
    async fn revoking_an_agent_drops_its_tokens_and_leaves_the_others() {
        let store = AgentTokenStore::new();
        let doomed = store.mint_default("agent.a").await;
        let spared = store.mint_default("agent.b").await;
        store.revoke_agent("agent.a").await;
        assert_eq!(store.resolve(&doomed).await, None);
        assert_eq!(store.resolve(&spared).await.as_deref(), Some("agent.b"));
    }

    #[tokio::test]
    async fn minting_prunes_what_has_expired() {
        let store = AgentTokenStore::new();
        store.mint("agent.old", Duration::seconds(-1)).await;
        assert_eq!(store.live_count().await, 0);
        store.mint_default("agent.new").await;
        assert_eq!(
            store.live_count().await,
            1,
            "the expired entry must not still be occupying the map"
        );
    }

    #[test]
    fn the_store_does_not_keep_the_token_itself() {
        let token = "a-secret-that-must-not-be-recoverable";
        let fp = fingerprint(token);
        assert_ne!(fp, token);
        assert!(
            !fp.contains(token),
            "the fingerprint must not embed the token"
        );
        assert_eq!(fp, fingerprint(token), "fingerprinting must be stable");
        assert_ne!(
            fp,
            fingerprint("a-secret-that-must-not-be-recoverable "),
            "a different token must fingerprint differently"
        );
    }
}
