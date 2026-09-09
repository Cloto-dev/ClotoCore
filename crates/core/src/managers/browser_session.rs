//! Browser sessions, for the one caller that cannot hold a key.
//!
//! Every other caller of the admin API can carry `X-API-Key`: the desktop shell
//! injects it, a spawned server is handed it in its environment, an operator's
//! script reads it from a file. A browser cannot. It loads images, audio, the VRM
//! model and the SSE stream by URL, and a URL cannot carry a header — which is why
//! the key ended up in `?token=`, where it also ends up in history, `Referer` and
//! every proxy log between the browser and the origin.
//!
//! A session is the other shape. The credential lives in a cookie the browser
//! attaches by itself, to same-origin requests, including the ones started by
//! markup rather than by script. Nothing has to put it in a URL, so nothing does.
//!
//! # What the TTL is here, and why it is not what [`super::agent_token`]'s is
//!
//! That module says its TTL is housekeeping, and it is right to: a token there
//! grants exactly what its agent already had through every other path, so a leak
//! hands the holder nothing new. **This is the opposite case.** A browser held no
//! admin credential at all, and a session hands it every one. The TTL is the only
//! thing that bounds how long a stolen cookie stays useful, so it is a boundary
//! here, and short by intent rather than long by convenience.
//!
//! # What a session is not
//!
//! It is not an identity. It says "this browser was authorised", not "this is
//! who". Identity — the operator behind Cloudflare Access, say — is what the
//! *minting* route decides before calling [`SessionStore::mint`]; the store keeps
//! only the label it was given, for diagnostics. Keeping those separate is what
//! lets a second minting route be added without touching the credential.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use tokio::sync::RwLock;

/// Cookie the kernel reads a browser session from.
///
/// The kernel is the source of truth for this name; the dashboard mirrors it
/// from here rather than spelling it again.
pub const SESSION_COOKIE: &str = "cloto_session";

/// How long a session stays resolvable when the caller asks for the default.
///
/// Eight hours: long enough that a working day does not end in a surprise
/// logout, short enough that a cookie copied off a shared machine is not still
/// admin next week. Unlike the agent-token TTL this one is load-bearing (see the
/// module docs), so it is deliberately not measured in days.
pub const DEFAULT_TTL_MINUTES: i64 = 480;

/// Checked when the crate builds, not when the suite runs. This TTL is the only
/// bound on a stolen cookie, so a change that stretched it into "effectively
/// forever" should fail the build rather than a review.
const _: () = assert!(DEFAULT_TTL_MINUTES <= 24 * 60);

struct Entry {
    /// Who the minting route said this was. Diagnostics only — nothing
    /// authorises on it, because the store is not the identity layer.
    label: String,
    expires_at: DateTime<Utc>,
}

/// In-memory map of session fingerprint → the label it was minted for.
///
/// Deliberately not persisted, for the same reason [`super::agent_token`] is
/// not: a restart invalidates every session, and a browser that has to sign in
/// again after the kernel restarted is being told the truth.
#[derive(Default)]
pub struct SessionStore {
    entries: RwLock<HashMap<String, Entry>>,
}

impl SessionStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a session for `label`, valid for `ttl`.
    ///
    /// The raw token is returned exactly once and is not recoverable from the
    /// store afterwards — only its fingerprint is kept.
    pub async fn mint(&self, label: &str, ttl: Duration) -> String {
        let token = crate::apikey::generate();
        let entry = Entry {
            label: label.to_string(),
            expires_at: Utc::now() + ttl,
        };
        let mut entries = self.entries.write().await;
        Self::prune(&mut entries, Utc::now());
        entries.insert(fingerprint(&token), entry);
        token
    }

    /// Mint with [`DEFAULT_TTL_MINUTES`].
    pub async fn mint_default(&self, label: &str) -> String {
        self.mint(label, Duration::minutes(DEFAULT_TTL_MINUTES))
            .await
    }

    /// The label this session was minted for, or `None` if it is unknown or
    /// expired.
    ///
    /// An expired session answers exactly as an unknown one does, for the reason
    /// [`super::agent_token::AgentTokenStore::resolve`] gives: telling the two
    /// apart confirms that a session once existed, which is a fact about someone
    /// else's browser.
    pub async fn resolve(&self, token: &str) -> Option<String> {
        let fp = fingerprint(token);
        let entries = self.entries.read().await;
        entries
            .get(&fp)
            .filter(|e| e.expires_at > Utc::now())
            .map(|e| e.label.clone())
    }

    /// Drop this session. Returns whether it was live.
    ///
    /// This is what sign-out calls, and it is why sign-out has to be reachable
    /// *with the cookie*: a browser that wants to stop being admin holds nothing
    /// else to prove it may.
    pub async fn revoke(&self, token: &str) -> bool {
        let fp = fingerprint(token);
        let mut entries = self.entries.write().await;
        entries
            .remove(&fp)
            .is_some_and(|e| e.expires_at > Utc::now())
    }

    /// Drop every session. For a key rotation: the admin credential that
    /// authorised these is gone, so the sessions standing on it should be too.
    pub async fn revoke_all(&self) {
        self.entries.write().await.clear();
    }

    /// Number of live (unexpired) sessions. For tests and diagnostics.
    pub async fn live_count(&self) -> usize {
        let now = Utc::now();
        let entries = self.entries.read().await;
        entries.values().filter(|e| e.expires_at > now).count()
    }

    fn prune(entries: &mut HashMap<String, Entry>, now: DateTime<Utc>) {
        entries.retain(|_, e| e.expires_at > now);
    }
}

/// Read the session token out of a `Cookie` header.
///
/// Written by hand rather than pulled from a cookie crate because this is the
/// only cookie the kernel reads, and a parser is easier to reason about than a
/// dependency when the thing being parsed is a full-admin credential. It takes
/// the *first* match: a request carrying two `cloto_session` values is
/// ambiguous, and picking the last would let an attacker who can append a
/// cookie override one the browser already holds.
#[must_use]
pub fn token_from_cookie_header(raw: &str) -> Option<&str> {
    raw.split(';')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (name.trim(), value.trim()))
        .find(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| value)
        .filter(|value| !value.is_empty())
}

/// The `Set-Cookie` value that installs `token`.
///
/// Every attribute here is doing something:
/// - `HttpOnly` keeps script from reading a full-admin credential, which is the
///   whole reason this is a cookie and not a value the SPA stores.
/// - `SameSite=Strict` is what stops a cross-site request from carrying it;
///   without it, a link on another page could act as the operator.
/// - `Path=/` because the SPA shell is served from the root and the API from
///   `/api`, and one credential covers both.
/// - `Max-Age` matches the store's TTL so the browser forgets it at the same
///   moment the kernel does. A cookie that outlives its entry is a login screen
///   arriving as an unexplained 403.
/// - `Secure` is caller-controlled: it is correct behind a tunnel that
///   terminates TLS at the edge (the browser's view is https), and it makes the
///   cookie undeliverable over plain http to a loopback port, which is exactly
///   how the desktop shell talks to the kernel.
#[must_use]
pub fn set_cookie_value(token: &str, ttl: Duration, secure: bool) -> String {
    let mut out = format!(
        "{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
        ttl.num_seconds().max(0)
    );
    if secure {
        out.push_str("; Secure");
    }
    out
}

/// The `Set-Cookie` value that clears the session cookie.
#[must_use]
pub fn clear_cookie_value(secure: bool) -> String {
    let mut out = format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0");
    if secure {
        out.push_str("; Secure");
    }
    out
}

/// Domain-separated SHA-256 of a session token.
///
/// The store holds fingerprints rather than tokens so that a memory dump, a
/// panic payload or a debug print of the map does not spill anything a caller
/// could present. The salt is defined here and copied from nowhere — in
/// particular it is not [`super::agent_token`]'s, so a fingerprint from one
/// store can never resolve in the other.
fn fingerprint(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"cloto-browser-session:");
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_minted_session_resolves_to_its_label() {
        let store = SessionStore::new();
        let token = store.mint_default("operator").await;
        assert_eq!(store.resolve(&token).await.as_deref(), Some("operator"));
        assert_eq!(store.live_count().await, 1);
    }

    #[tokio::test]
    async fn an_unknown_token_resolves_to_nothing() {
        let store = SessionStore::new();
        store.mint_default("operator").await;
        assert!(store.resolve("not-a-session").await.is_none());
    }

    #[tokio::test]
    async fn an_expired_session_answers_as_an_unknown_one() {
        let store = SessionStore::new();
        let token = store.mint("operator", Duration::seconds(-1)).await;
        assert!(store.resolve(&token).await.is_none());
        assert_eq!(store.live_count().await, 0);
    }

    #[tokio::test]
    async fn revoke_ends_the_session_and_says_it_was_live() {
        let store = SessionStore::new();
        let token = store.mint_default("operator").await;
        assert!(store.revoke(&token).await);
        assert!(store.resolve(&token).await.is_none());
        assert!(
            !store.revoke(&token).await,
            "a second revoke has nothing to end"
        );
    }

    #[tokio::test]
    async fn revoke_all_ends_every_session() {
        let store = SessionStore::new();
        let a = store.mint_default("a").await;
        let b = store.mint_default("b").await;
        store.revoke_all().await;
        assert!(store.resolve(&a).await.is_none());
        assert!(store.resolve(&b).await.is_none());
    }

    #[tokio::test]
    async fn minting_prunes_what_has_expired() {
        let store = SessionStore::new();
        store.mint("stale", Duration::seconds(-1)).await;
        store.mint_default("fresh").await;
        assert_eq!(store.live_count().await, 1);
    }

    #[tokio::test]
    async fn two_sessions_do_not_share_a_token() {
        let store = SessionStore::new();
        let a = store.mint_default("a").await;
        let b = store.mint_default("b").await;
        assert_ne!(a, b);
        assert_eq!(store.resolve(&a).await.as_deref(), Some("a"));
        assert_eq!(store.resolve(&b).await.as_deref(), Some("b"));
    }

    #[test]
    fn the_cookie_header_parser_finds_the_session_among_others() {
        assert_eq!(
            token_from_cookie_header("theme=dark; cloto_session=abc; lang=ja"),
            Some("abc")
        );
        assert_eq!(token_from_cookie_header("cloto_session=abc"), Some("abc"));
        assert_eq!(
            token_from_cookie_header("  cloto_session = abc  "),
            Some("abc")
        );
    }

    #[test]
    fn the_parser_declines_what_is_not_a_session() {
        assert_eq!(token_from_cookie_header(""), None);
        assert_eq!(token_from_cookie_header("theme=dark"), None);
        assert_eq!(token_from_cookie_header("cloto_session="), None);
        // A name that merely contains ours is a different cookie.
        assert_eq!(token_from_cookie_header("xcloto_session=abc"), None);
        assert_eq!(token_from_cookie_header("cloto_session_x=abc"), None);
    }

    #[test]
    fn a_duplicated_cookie_takes_the_first_value() {
        // Appending a cookie must not override one the browser already holds.
        assert_eq!(
            token_from_cookie_header("cloto_session=real; cloto_session=injected"),
            Some("real")
        );
    }

    #[test]
    fn the_set_cookie_value_carries_every_attribute_it_needs() {
        let v = set_cookie_value("tok", Duration::minutes(10), true);
        assert!(v.starts_with("cloto_session=tok;"));
        for attr in [
            "HttpOnly",
            "SameSite=Strict",
            "Path=/",
            "Max-Age=600",
            "Secure",
        ] {
            assert!(v.contains(attr), "missing {attr} in {v}");
        }
    }

    #[test]
    fn secure_is_omitted_when_the_caller_says_so() {
        let v = set_cookie_value("tok", Duration::minutes(10), false);
        assert!(!v.contains("Secure"), "{v}");
        assert!(
            v.contains("HttpOnly"),
            "the other attributes still stand: {v}"
        );
    }

    #[test]
    fn a_negative_ttl_never_becomes_a_negative_max_age() {
        let v = set_cookie_value("tok", Duration::seconds(-5), false);
        assert!(v.contains("Max-Age=0"), "{v}");
    }

    #[test]
    fn clearing_expires_the_cookie_immediately() {
        let v = clear_cookie_value(true);
        assert!(v.contains("Max-Age=0"), "{v}");
        assert!(v.starts_with("cloto_session=;"), "{v}");
    }

    #[test]
    fn the_fingerprint_is_not_the_token_and_is_domain_separated() {
        let token = "abc";
        assert_ne!(fingerprint(token), token);
        // Same input, different salt from the agent-token store: a fingerprint
        // from one store must never resolve in the other.
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        sha2::Digest::update(&mut hasher, b"cloto-agent-token:");
        sha2::Digest::update(&mut hasher, token.as_bytes());
        let agent_fp = hex::encode(sha2::Digest::finalize(hasher));
        assert_ne!(fingerprint(token), agent_fp);
    }
}
