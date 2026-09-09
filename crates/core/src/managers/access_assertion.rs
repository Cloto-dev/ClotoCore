//! Cloudflare Access assertions, verified rather than read.
//!
//! [`super::browser_session`] gives a browser a credential it can hold. It does
//! not say who may be given one: minting is an admin route, so something still
//! has to present the key once. This module is what removes that step, by
//! deciding identity from the edge that already authenticated the human.
//!
//! # Why a header is not enough
//!
//! The kernel listens on loopback, and *every* local process can reach a
//! loopback listener and set any header it likes. `Cf-Access-Authenticated-User-Email`
//! read at face value would therefore hand full admin to anything running on the
//! host — including an agent harness executing commands a model wrote. The admin
//! key is what keeps those out today, because it lives in an environment variable
//! they do not get.
//!
//! So identity here comes from `Cf-Access-Jwt-Assertion`, whose **signature** a
//! local forger cannot produce. Loopback is still required, but as a second
//! condition rather than the only one: a request that did not come through the
//! tunnel has no business here even when it carries a valid assertion.
//!
//! One nearby precedent is worth naming so it is not mis-copied: a *read-only*
//! service behind the same edge may trust these headers on the loopback
//! condition alone, and is right to — the most a forged header buys there is a
//! view. This route grants every admin permission. **When borrowing a rule, look
//! at how much authority the rule was guarding.**
//!
//! # What is verified
//!
//! In order, and with the signature first so that nothing downstream reasons
//! about claims that were never authenticated:
//!
//! 1. the peer is loopback;
//! 2. the JWS has three segments and its header names `RS256` — the algorithm is
//!    pinned, never taken from the token, so an attacker cannot pick one;
//! 3. the signature verifies against the JWKS key named by `kid`;
//! 4. `iss` is our team domain, `aud` contains our application tag, `exp` has not
//!    passed and `nbf` has arrived;
//! 5. the token carries an `email` — a human, not a service token.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration as StdDuration, Instant};

use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::sync::RwLock;

/// Header Cloudflare Access puts the signed assertion in.
// HARDCODED(Cloudflare Zero Trust docs, "Validate JWTs" — the header name is
// chosen by the edge, so there is no local owner to read it from):
// changing it here would simply stop matching what the tunnel sends.
pub const ACCESS_ASSERTION_HEADER: &str = "cf-access-jwt-assertion";

/// How long a fetched key set is used before it is fetched again.
const JWKS_TTL: StdDuration = StdDuration::from_secs(600);

/// Floor on how often an unknown `kid` may trigger a refetch.
///
/// A rotation should be picked up in seconds, but the trigger is attacker-reachable:
/// anyone who can reach the route can name a `kid` that is not in the set. Without
/// a floor, that is an outbound request per attempt.
const JWKS_MIN_REFRESH_INTERVAL: StdDuration = StdDuration::from_secs(60);

/// Tolerance for clock skew between the edge and this host, in seconds.
const CLOCK_LEEWAY_SECONDS: i64 = 60;

/// Largest assertion accepted, in bytes.
///
/// A Cloudflare Access JWT is well under 2 KB. The bound exists so a caller
/// cannot make the kernel base64-decode and hash something arbitrarily large
/// before any of the cheap checks have run.
const MAX_ASSERTION_BYTES: usize = 8192;

/// Why an assertion was not accepted.
///
/// Specific on purpose — the *log* should say which check failed, because
/// "Access sign-in is broken" is otherwise undiagnosable. The **response** stays
/// a bare 403 (see the handler): telling a caller which condition it missed is
/// telling it how to search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessError {
    /// No team domain / audience configured — the route is off.
    NotConfigured,
    /// The request did not arrive over the tunnel.
    NotFromEdge,
    /// The token is not a well-formed JWS.
    Malformed(&'static str),
    /// The header named an algorithm other than RS256.
    UnsupportedAlgorithm,
    /// `kid` is not in the key set (and a refresh did not add it).
    UnknownKey,
    /// The signature did not verify against that key.
    BadSignature,
    /// `exp` has passed.
    Expired,
    /// `nbf` has not arrived.
    NotYetValid,
    /// `iss` is not our team domain.
    WrongIssuer,
    /// `aud` does not contain our application tag.
    WrongAudience,
    /// A valid token, but not one that names a person.
    NoIdentity,
    /// The key set could not be fetched or parsed.
    Jwks(String),
}

impl std::fmt::Display for AccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "Access sign-in is not configured"),
            Self::NotFromEdge => write!(f, "request did not arrive over the tunnel"),
            Self::Malformed(why) => write!(f, "malformed assertion: {why}"),
            Self::UnsupportedAlgorithm => write!(f, "assertion is not RS256"),
            Self::UnknownKey => write!(f, "assertion names a key we do not have"),
            Self::BadSignature => write!(f, "signature did not verify"),
            Self::Expired => write!(f, "assertion has expired"),
            Self::NotYetValid => write!(f, "assertion is not valid yet"),
            Self::WrongIssuer => write!(f, "assertion was issued by another team"),
            Self::WrongAudience => write!(f, "assertion was minted for another application"),
            Self::NoIdentity => write!(f, "assertion names no person"),
            Self::Jwks(why) => write!(f, "key set unavailable: {why}"),
        }
    }
}

/// Where the edge is, and which application we are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessConfig {
    /// Team hostname without a scheme, e.g. `example.cloudflareaccess.com`.
    team_domain: String,
    /// The Access application's AUD tag.
    audience: String,
}

impl AccessConfig {
    /// Read the deployment switches, or `None` if either is missing or unusable.
    ///
    /// Both are required together. Half a configuration is not a weaker one, it
    /// is an ambiguous one — a team domain with no audience would verify that a
    /// token came from the right team while ignoring which application minted
    /// it, and every application in a team shares an issuer.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let team = std::env::var("CLOTO_ACCESS_TEAM_DOMAIN").ok()?;
        let aud = std::env::var("CLOTO_ACCESS_AUD").ok()?;
        Self::new(&team, &aud)
    }

    /// Build from explicit values, normalising the team domain.
    ///
    /// A scheme and a trailing slash are accepted and stripped, because that is
    /// how the value is written down in Cloudflare's dashboard. Anything that
    /// would change *which host is contacted* — a path, a port, an embedded
    /// credential, whitespace — is refused rather than normalised, since the
    /// JWKS URL is built by interpolation and a value that can carry structure
    /// there is a value that can redirect the trust anchor.
    #[must_use]
    pub fn new(team_domain: &str, audience: &str) -> Option<Self> {
        let team = team_domain.trim();
        let team = team
            .strip_prefix("https://")
            .or_else(|| team.strip_prefix("http://"))
            .unwrap_or(team);
        let team = team.trim_end_matches('/');
        let audience = audience.trim();

        if team.is_empty() || audience.is_empty() {
            return None;
        }
        if team
            .chars()
            .any(|c| c == '/' || c == ':' || c == '@' || c == '?' || c == '#' || c.is_whitespace())
        {
            return None;
        }
        if audience.chars().any(char::is_whitespace) {
            return None;
        }

        Some(Self {
            team_domain: team.to_string(),
            audience: audience.to_string(),
        })
    }

    /// The `iss` every assertion from this team carries.
    #[must_use]
    pub fn issuer(&self) -> String {
        format!("https://{}", self.team_domain)
    }

    /// Where the signing keys are published.
    #[must_use]
    pub fn jwks_url(&self) -> String {
        // HARDCODED(Cloudflare Zero Trust docs, "Validate JWTs" — the certs
        // path is fixed by the edge and published per team domain):
        // the only variable part is the host, which the operator configures.
        format!("https://{}/cdn-cgi/access/certs", self.team_domain)
    }

    /// The application tag an assertion must be minted for.
    #[must_use]
    pub fn audience(&self) -> &str {
        &self.audience
    }
}

/// One RSA public key from the JWKS, as raw big-endian components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JwtKey {
    n: Vec<u8>,
    e: Vec<u8>,
}

/// The published keys, by `kid`.
pub type KeySet = HashMap<String, JwtKey>;

/// Who the edge says is calling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedIdentity {
    /// The address Access authenticated. Never empty.
    pub email: String,
    /// Access's own subject id, for logs.
    pub subject: String,
}

impl VerifiedIdentity {
    /// The label a session is minted under.
    ///
    /// Prefixed so a session's provenance is legible in diagnostics: an
    /// `access:` session was authorised by the edge, an `admin-key` one by
    /// something that already held the key.
    #[must_use]
    pub fn session_label(&self) -> String {
        format!("access:{}", self.email)
    }
}

/// `aud` is a string when there is one, an array when there are several.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Audience {
    fn contains(&self, wanted: &str) -> bool {
        match self {
            Self::One(a) => a == wanted,
            Self::Many(all) => all.iter().any(|a| a == wanted),
        }
    }
}

#[derive(Debug, Deserialize)]
struct JwsHeader {
    alg: String,
    kid: Option<String>,
    /// Extensions the producer says we must understand. We understand none, so
    /// its presence is a refusal (RFC 7515 §4.1.11).
    crit: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct Claims {
    iss: Option<String>,
    aud: Option<Audience>,
    exp: Option<i64>,
    nbf: Option<i64>,
    email: Option<String>,
    sub: Option<String>,
}

fn b64url(segment: &str) -> Result<Vec<u8>, AccessError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(segment)
        .map_err(|_| AccessError::Malformed("a segment is not base64url"))
}

/// Turn a JWKS document into the keys we can verify with.
///
/// Entries we cannot use are skipped rather than fatal: a team that also
/// publishes an EC key should not lose its RSA one. An empty result *is* fatal —
/// "no usable keys" and "every token fails" should not be the same silence.
pub fn parse_jwks(raw: &str) -> Result<KeySet, AccessError> {
    #[derive(Deserialize)]
    struct Jwks {
        keys: Vec<Jwk>,
    }
    #[derive(Deserialize)]
    struct Jwk {
        kid: Option<String>,
        kty: Option<String>,
        alg: Option<String>,
        n: Option<String>,
        e: Option<String>,
    }

    let doc: Jwks =
        serde_json::from_str(raw).map_err(|e| AccessError::Jwks(format!("not a key set: {e}")))?;

    let mut out = KeySet::new();
    for k in doc.keys {
        // HARDCODED(RFC 7518 §6.3 — the JWK key type for RSA):
        // the components below (`n`, `e`) are only meaningful for this type.
        if k.kty.as_deref() != Some("RSA") {
            continue;
        }
        // Absent `alg` is allowed (it is optional in RFC 7517); a *different*
        // one is not, so a key published for another algorithm is never used to
        // verify an RS256 signature.
        // HARDCODED(RFC 7518 §3.1 — the JWA registry names this algorithm):
        // it is the one Access signs with, and the only one we verify.
        if let Some(alg) = k.alg.as_deref() {
            if alg != "RS256" {
                continue;
            }
        }
        let (Some(kid), Some(n), Some(e)) = (k.kid, k.n, k.e) else {
            continue;
        };
        let (Ok(n), Ok(e)) = (b64url(&n), b64url(&e)) else {
            continue;
        };
        if kid.is_empty() || n.is_empty() || e.is_empty() {
            continue;
        }
        out.insert(kid, JwtKey { n, e });
    }

    if out.is_empty() {
        return Err(AccessError::Jwks("no usable RSA keys".to_string()));
    }
    Ok(out)
}

/// Verify one assertion against one key set. No I/O, no clock of its own.
///
/// Split out from [`AccessVerifier`] so the rules are a function of their
/// arguments: the tests drive this directly and can therefore state a moment in
/// time rather than sleep through one.
pub fn verify_assertion(
    token: &str,
    keys: &KeySet,
    config: &AccessConfig,
    now: DateTime<Utc>,
) -> Result<VerifiedIdentity, AccessError> {
    if token.is_empty() {
        return Err(AccessError::Malformed("empty"));
    }
    if token.len() > MAX_ASSERTION_BYTES {
        return Err(AccessError::Malformed("implausibly large"));
    }

    let mut parts = token.split('.');
    let (Some(header_b64), Some(payload_b64), Some(signature_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(AccessError::Malformed("a JWS has exactly three segments"));
    };
    if header_b64.is_empty() || payload_b64.is_empty() || signature_b64.is_empty() {
        return Err(AccessError::Malformed("a segment is empty"));
    }

    let header: JwsHeader = serde_json::from_slice(&b64url(header_b64)?)
        .map_err(|_| AccessError::Malformed("header is not a JWS header"))?;

    // Pinned, not dispatched. The token says which algorithm it used; we say
    // which one we accept, and only ever call the RS256 verifier. That is what
    // makes `alg: none` and the HS256-with-the-public-key confusion impossible
    // here rather than merely unhandled.
    // HARDCODED(RFC 7518 §3.1): the same registry name as the JWKS filter
    // above, and the pin that makes every other `alg` a refusal.
    if header.alg != "RS256" {
        return Err(AccessError::UnsupportedAlgorithm);
    }
    if header.crit.is_some() {
        return Err(AccessError::Malformed("header demands unknown extensions"));
    }

    let kid = header
        .kid
        .filter(|k| !k.is_empty())
        .ok_or(AccessError::Malformed("header names no key"))?;
    let key = keys.get(&kid).ok_or(AccessError::UnknownKey)?;

    let signature = b64url(signature_b64)?;
    // The bytes that were signed are the two segments exactly as they arrived —
    // re-encoding them would verify a document the sender never signed.
    let signing_input = &token[..header_b64.len() + 1 + payload_b64.len()];

    ring::signature::RsaPublicKeyComponents {
        n: key.n.as_slice(),
        e: key.e.as_slice(),
    }
    .verify(
        &ring::signature::RSA_PKCS1_2048_8192_SHA256,
        signing_input.as_bytes(),
        &signature,
    )
    .map_err(|_| AccessError::BadSignature)?;

    // Only now are the claims something the edge said, rather than something the
    // caller typed.
    let claims: Claims = serde_json::from_slice(&b64url(payload_b64)?)
        .map_err(|_| AccessError::Malformed("payload is not a claim set"))?;

    if claims.iss.as_deref() != Some(config.issuer().as_str()) {
        return Err(AccessError::WrongIssuer);
    }
    match claims.aud {
        Some(ref aud) if aud.contains(config.audience()) => {}
        _ => return Err(AccessError::WrongAudience),
    }

    let now = now.timestamp();
    let exp = claims.exp.ok_or(AccessError::Expired)?;
    if now > exp + CLOCK_LEEWAY_SECONDS {
        return Err(AccessError::Expired);
    }
    if let Some(nbf) = claims.nbf {
        if now + CLOCK_LEEWAY_SECONDS < nbf {
            return Err(AccessError::NotYetValid);
        }
    }

    // A service token authenticates a machine and carries `common_name` instead
    // of an address. It is deliberately not accepted: this route hands out every
    // admin permission, and the ruling it implements is about the operator
    // behind the edge, not about anything holding a client secret.
    let email = claims
        .email
        .filter(|e| !e.trim().is_empty())
        .ok_or(AccessError::NoIdentity)?;

    Ok(VerifiedIdentity {
        email,
        subject: claims.sub.unwrap_or_default(),
    })
}

struct CachedKeys {
    keys: KeySet,
    fetched_at: Instant,
}

/// Fetches and caches the team's signing keys, and answers whether an assertion
/// is one of theirs.
pub struct AccessVerifier {
    config: Option<AccessConfig>,
    cache: RwLock<Option<CachedKeys>>,
    http: reqwest::Client,
}

impl AccessVerifier {
    /// Build from the environment. Absent configuration is not an error — it is
    /// a deployment that does not sit behind Access, and the route simply
    /// refuses.
    #[must_use]
    pub fn from_env() -> Self {
        Self::new(AccessConfig::from_env())
    }

    #[must_use]
    pub fn new(config: Option<AccessConfig>) -> Self {
        Self {
            config,
            cache: RwLock::new(None),
            http: reqwest::Client::builder()
                .timeout(StdDuration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    /// A verifier whose key set is given rather than fetched.
    ///
    /// The keys are treated as just-fetched, so nothing reaches the network
    /// until the ordinary TTL expires. This is the seam the tests use to
    /// exercise a real signature without a real team domain.
    #[must_use]
    pub fn with_keys(config: AccessConfig, keys: KeySet) -> Self {
        let verifier = Self::new(Some(config));
        *verifier
            .cache
            .try_write()
            .expect("a verifier that nothing else holds yet") = Some(CachedKeys {
            keys,
            fetched_at: Instant::now(),
        });
        verifier
    }

    /// Whether this deployment accepts Access assertions at all.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.config.is_some()
    }

    /// The configuration, for a boot-time log line.
    #[must_use]
    pub fn config(&self) -> Option<&AccessConfig> {
        self.config.as_ref()
    }

    /// Verify an assertion that arrived from `peer`.
    ///
    /// The peer is an argument rather than the handler's business so that the
    /// loopback condition cannot be forgotten at a call site: there is no way to
    /// ask this question without answering where the request came from.
    pub async fn verify(
        &self,
        token: &str,
        peer: IpAddr,
        now: DateTime<Utc>,
    ) -> Result<VerifiedIdentity, AccessError> {
        let config = self.config.as_ref().ok_or(AccessError::NotConfigured)?;
        if !peer.is_loopback() {
            return Err(AccessError::NotFromEdge);
        }

        let keys = self.keys(config, false).await?;
        match verify_assertion(token, &keys, config, now) {
            // An unknown `kid` is what a key rotation looks like from here, so
            // it is the one failure worth a second look with fresh keys.
            Err(AccessError::UnknownKey) => {
                let keys = self.keys(config, true).await?;
                verify_assertion(token, &keys, config, now)
            }
            other => other,
        }
    }

    /// The current key set, fetching if it is missing, stale, or `force`d — and
    /// not more often than [`JWKS_MIN_REFRESH_INTERVAL`].
    async fn keys(&self, config: &AccessConfig, force: bool) -> Result<KeySet, AccessError> {
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.as_ref() {
                let age = cached.fetched_at.elapsed();
                let too_soon = force && age < JWKS_MIN_REFRESH_INTERVAL;
                if too_soon || (!force && age < JWKS_TTL) {
                    return Ok(cached.keys.clone());
                }
            }
        }

        match self.fetch_jwks(config).await {
            Ok(keys) => {
                let mut cache = self.cache.write().await;
                *cache = Some(CachedKeys {
                    keys: keys.clone(),
                    fetched_at: Instant::now(),
                });
                Ok(keys)
            }
            Err(e) => {
                // A fetch that fails while we still hold keys is a network
                // problem, not a reason to stop authenticating anyone: the keys
                // we have were published by the same team and have not been
                // withdrawn by our failing to ask.
                let cache = self.cache.read().await;
                if let Some(cached) = cache.as_ref() {
                    tracing::warn!(error = %e, "Access key set refresh failed; using the last one");
                    return Ok(cached.keys.clone());
                }
                Err(e)
            }
        }
    }

    async fn fetch_jwks(&self, config: &AccessConfig) -> Result<KeySet, AccessError> {
        let url = config.jwks_url();
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| AccessError::Jwks(format!("{url}: {e}")))?;
        if !response.status().is_success() {
            return Err(AccessError::Jwks(format!(
                "{url}: HTTP {}",
                response.status()
            )));
        }
        let body = response
            .text()
            .await
            .map_err(|e| AccessError::Jwks(format!("{url}: {e}")))?;
        parse_jwks(&body)
    }
}

/// Signed assertions and the key set that verifies them.
///
/// Lives here rather than in the test module below because the HTTP layer's
/// tests need the same material: a sign-in that is refused for the wrong
/// reason looks exactly like one refused for the right one, so the only way
/// to prove the route is wired is to make a real assertion succeed.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    pub use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;

    /// Throwaway RSA keys, generated for this test module and used nowhere else.
    ///
    /// They exist so the suite can *sign*: a verifier tested only against
    /// pre-baked strings cannot be extended without reaching for `openssl`, and
    /// a test that is expensive to add is a test that does not get added. They
    /// authorise nothing — no deployment has ever seen them, and the JWKS they
    /// stand in for is fetched from Cloudflare at run time.
    pub const TEST_KEY_A_PKCS8_B64: &str = "MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQD3deWbaKa694RajnVfofdKOR/PHnD42tUr/dMoxF+ekqBIxTB3bliBeWoiPXnKp1kKqPE2HzaOL8n1pk6eaUg4B4imhoLFwKTThP6w7RnLJGFW7jBWkCxT9w3FTzRWo6sAXhBhyL2anjn4gf4UVfsjZ/Qr6rpIQRxfkjNjymTaTQUkM9m38JQ29Xm7h1XDlcsWl0jN6UinPlWJlyrt5lLzCMOZv9RqEguYn64bJFqTPF5TyA53awhou/AO3SfUBNqvcSHTvryxIzVmzat8Alo3qBiNpkqj9oqGdMpOVSnwS4Kkemd4Mh7nAoUnUmGldPew9USSlx6kHhnvuof/C+WBAgMBAAECggEAAZSIp1HnQqli+HsRZ89ud1RfDiEJIqWvF81SpF+AptAT4vMTaKfVO9ptIZPX68He0TEb/Tb8z7KhbQanWN6ePfFaX4nbWuzsgIdIYxPYhtIQJxB1UZAxIEYjGd/0GxuHc4SmQSGZiFu7TglyeGnGJUc8KW2hy+VSi4+w8VGxDC3PhChhqJ4pgDN0ABo158xIyPaz3W0PE8IMOs232xTI9Ct01DGzZ1EliORmUxcTC5fdfGd5Kd3exY+cyJtm7h8ZYAZtVaUTqOpw0B+7Wu0Ps6w7XCSqzBPGZMWi/DGZze3Zq8Kmjmdm0dKlnkIlstbo+T0CoflPUuxy+pC9re+COQKBgQD9OP56DZMZIQ3Oo4uotUqJ4qtlLw/HrzZBk5BUg24AfRqmzvbI+SljD0PMMOumNaKV83TE8ENqcTgLlJSNQHUjKGJnrlvim03LXOT+vUmVT8kOyZbxcwwLh+S8dJH1unpxGO3lK+A+x7V21fOYEOOgUT8Dq8hO9DUmropCrtC99QKBgQD6LLlPJliDu7TF+ZMRrTjaSLojxW8Qhsn/QP4tGGo7PlHSYKLAzT7//LTA4430fhN22rEknhK7+4/qdeNX4sJ9pSb5uEAaoASugrGiP9fjWRPuULx/lEZFKMCHEplNk+zXDyzjhsOEOorf4stkWcb66kYVeuBeYNUhH7dO/iWl3QKBgQDjORlg/H1at0Zkfmz73nIceMHD8g7+6EKPZZLFw4oZ9ijMNjtM7AgvU6tKtzs90jMqy2OktNRJ136rJZCHj6eM/NgQoWziUunj6l+yFrjIuud31X0U/F96mV6vnQq8rbDhe7U9R7nZm+tBz4rekYkwerdI3ATKlGh9ZXG7lJLLYQKBgQC4iUDvx2NHWLBR0HTRdysWqMrVFA+G60YZCQH0lavWo3OLcUjcWwl7nhZeqfvOOyl0ZICCeC9thnR0CB14eIXqVGZZkbWHbj3F1BXfjqRayRxQkDFbEi57WUIa4HdAqDrtr/32nzOdV+mUmCBbl3WVJDYqJgdW1qqf0ltO41016QKBgQCaYPVu1GXInV/oqQPPkvgYIaNw5p2wc0ggvmf4xUwggr31QehLTZMvQTJWrFKovdkvBfaEamLxj08U1omHcNQMw+5zB7dv2TQNylh6450M6jjzgXvHTMyQcwUzWRgRjwMlj87I6TXeeJiswtlo0zC9xURtGsh19Zei4aOZYj2I1Q==";
    pub const TEST_KEY_A_N: &str = "93Xlm2imuveEWo51X6H3Sjkfzx5w-NrVK_3TKMRfnpKgSMUwd25YgXlqIj15yqdZCqjxNh82ji_J9aZOnmlIOAeIpoaCxcCk04T-sO0ZyyRhVu4wVpAsU_cNxU80VqOrAF4QYci9mp45-IH-FFX7I2f0K-q6SEEcX5IzY8pk2k0FJDPZt_CUNvV5u4dVw5XLFpdIzelIpz5ViZcq7eZS8wjDmb_UahILmJ-uGyRakzxeU8gOd2sIaLvwDt0n1ATar3Eh0768sSM1Zs2rfAJaN6gYjaZKo_aKhnTKTlUp8EuCpHpneDIe5wKFJ1JhpXT3sPVEkpcepB4Z77qH_wvlgQ";

    /// A second key, so "signed by something that is not the published key" is a
    /// case the suite can actually produce.
    pub const TEST_KEY_B_PKCS8_B64: &str = "MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDPSECBgOX90vr4I70bKVpcOCfutgV1JqqQZxUs7Bk+ftYODT78/sh+V+XNHJNrWpNC5oPJZH7Zu3vf8meS55QfUb1Iweut9A+oDbYaRQavTC6+fdIC+e22P/Bn3oCdU0fX4AQIt1k5utDnci6Q+/CBRWwDBrtJYNK2BxnzgQm9RGSAX1y+xRt1dEImdXBAToa8sKl98mjM0GCNIxZ/3xQ3EsYg17oQL5Nupd64nOjGhfMsf1llP8QHmMt/dqoNemvrSs7kfxgKfQNPhfpN4f0uhQRTb3j6MZ9b1bmz4i/hwM6L7MAgSKrgMsAGzR2VATe3X8DTgwqJI6H6PXPO2aqtAgMBAAECggEAFknXndVFZci08c+t+ui0bawgJxvtdE5nEsXy0fTFNiIfVD16Y2vmFSfQbwC+nVGM+imdTB+BQFpXlJoVJwe9tqxsZRFtDTRsJo7q4OJBOMJBWHxhA67qL6mqaRDU1ZXp6L2O0X0dnAaJhgmSFkbw8oWLervTka1WmvoigTuD15T8hlvpP0pGDGVyoToEvkkcOSd5POWPjH7snVVYevno/R9hmNFinrIr+V13QyPdYRXQWvUYYqrV68HFCXG7JU/fXezpwVjkFMOKmE2HJR2U+P/+rbiXCdslD7A+7RHY7kPSADwENLEjzHNcfv5Vm3odi5Txb+0qgduBF/aDDZ70ZQKBgQDs5c+5/eyABsXWf5ZwLZdpLYZ4Qg1fj5p42l69oO4z56ZZZnR4wjaXbLWXcv7KW18ZbPd7GSUXIQaclF2Sg305LkfPYGPnW8ahSZUxuA0O1pr6o38TWNDGDGDsmotZh52AJoEsPTTyXQ6rUBd5M27PEFW68wwbM7Fu30ts6aOsiwKBgQDf/xmMn16baHOIXMaW24oDuIQ75prgHULxUNRWiMID1l3nZhvIcKL5C6iLi7BWHEcX6bDMCH2cNG6gSevXvxAZpLLMbhrGZpRnVSqZxaYQuXRryCjlH66zpRGtaPDm/GwGNh8FBYDCFM73rNWpQYE5no6eIVLYjjm/mmWMP7PUpwKBgQCb8yGPeCijk1HTxgQ77td5Bt459omlOfzfyCmMPg/xnXK18auE/50+i/LzM2GlxwbQzxoQMFppYnVeyJDc7bCW3u+pBfRejt0wuib8JwR5my9FBjKWguZVKjr4JzjLBGrbvP1WKSjc0APjJQN+5yvwJfm561wx4BLTQS3/EcOMxwKBgEySBKbYd9vCKfRMWqqJI7W/5pwfaYQBHLgnPF7UYxYyumj2s7qiHmPqA1SojL/y7K6U+RXWNTInjkWG33Mh4hwR+/j8DnUR7dsg9u4X7Xu8GbsacjhYyzynydIwlGExmq/I4nOx/ODbgiCSWXuBY+5RcElH9O0IOV9xJRN7VzrzAoGBAJfJkGgMOjgjcxgLPF7N9Nf3s58m5op2udI6SSQXT5F3lAT0I8dxlP+D5+CNlXicyRLWPpDj6lKuUt6rh0tSW5iefZ5qLjSl8KhrR7yUDHl19QUNmovtYcXRotYGJtrwhBpljPqOX1HG3th0vlojnjEX6twQm53F/ZPKgEm7jYxe";

    pub const TEAM: &str = "example.cloudflareaccess.com";
    pub const AUD: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    pub const KID: &str = "kid-a";

    pub fn config() -> AccessConfig {
        AccessConfig::new(TEAM, AUD).expect("test config is valid")
    }

    pub fn keypair(b64: &str) -> ring::signature::RsaKeyPair {
        let der = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("test key is base64");
        ring::signature::RsaKeyPair::from_pkcs8(&der).expect("test key is a PKCS#8 RSA key")
    }

    pub fn key_a() -> ring::signature::RsaKeyPair {
        keypair(TEST_KEY_A_PKCS8_B64)
    }

    pub fn key_b() -> ring::signature::RsaKeyPair {
        keypair(TEST_KEY_B_PKCS8_B64)
    }

    /// The key set a healthy deployment would have fetched: key A under `kid-a`.
    pub fn published_keys() -> KeySet {
        parse_jwks(&format!(
            r#"{{"keys":[{{"kid":"{KID}","kty":"RSA","alg":"RS256","use":"sig","n":"{TEST_KEY_A_N}","e":"AQAB"}}]}}"#
        ))
        .expect("test key set parses")
    }

    /// Sign `header` + `claims` with `key`, producing a real JWS.
    pub fn sign(key: &ring::signature::RsaKeyPair, header: &str, claims: &str) -> String {
        let signing_input = format!("{}.{}", B64.encode(header), B64.encode(claims));
        let mut signature = vec![0u8; key.public().modulus_len()];
        key.sign(
            &ring::signature::RSA_PKCS1_SHA256,
            &ring::rand::SystemRandom::new(),
            signing_input.as_bytes(),
            &mut signature,
        )
        .expect("test signing succeeds");
        format!("{signing_input}.{}", B64.encode(&signature))
    }

    pub fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).expect("fixed instant")
    }

    pub fn header_json() -> String {
        format!(r#"{{"alg":"RS256","kid":"{KID}","typ":"JWT"}}"#)
    }

    /// Claims that pass everything when checked at `at`, as a mutable starting
    /// point.
    pub fn claims_json_at(at: DateTime<Utc>) -> serde_json::Value {
        serde_json::json!({
            "iss": format!("https://{TEAM}"),
            "aud": [AUD],
            "exp": at.timestamp() + 3600,
            "nbf": at.timestamp() - 60,
            "iat": at.timestamp() - 60,
            "email": "operator@example.com",
            "sub": "sub-123",
        })
    }

    /// Claims for the fixed [`now`] the unit tests below reason about.
    pub fn claims_json() -> serde_json::Value {
        claims_json_at(now())
    }

    pub fn good_token() -> String {
        sign(&key_a(), &header_json(), &claims_json().to_string())
    }

    /// A token that is valid *right now*, for tests that go through a handler.
    ///
    /// A handler reads the wall clock, so a token minted around the fixed
    /// instant above is not merely a different fixture there — it is expired or
    /// not yet valid, and every assertion about the route would then hold for a
    /// reason that has nothing to do with what it claims to test.
    pub fn good_token_now() -> String {
        sign(
            &key_a(),
            &header_json(),
            &claims_json_at(Utc::now()).to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    fn verify(token: &str) -> Result<VerifiedIdentity, AccessError> {
        verify_assertion(token, &published_keys(), &config(), now())
    }

    /// The suite's own instrument check: if the embedded private key and the
    /// embedded modulus ever stopped being a pair, every "rejected" assertion
    /// below would pass for the wrong reason, and the file would still be green.
    #[test]
    fn the_test_key_and_the_published_modulus_are_the_same_key() {
        assert_eq!(
            verify(&good_token()).map(|i| i.email),
            Ok("operator@example.com".to_string()),
            "the embedded key no longer matches the embedded JWKS modulus"
        );
    }

    #[test]
    fn a_valid_assertion_names_the_person_the_edge_authenticated() {
        let identity = verify(&good_token()).expect("valid assertion");
        assert_eq!(identity.email, "operator@example.com");
        assert_eq!(identity.subject, "sub-123");
        assert_eq!(identity.session_label(), "access:operator@example.com");
    }

    #[test]
    fn a_tampered_payload_does_not_verify() {
        let token = good_token();
        let mut parts: Vec<&str> = token.split('.').collect();
        let forged = B64.encode(
            serde_json::json!({
                "iss": format!("https://{TEAM}"),
                "aud": [AUD],
                "exp": now().timestamp() + 3600,
                "email": "attacker@example.com",
            })
            .to_string(),
        );
        parts[1] = &forged;
        assert_eq!(verify(&parts.join(".")), Err(AccessError::BadSignature));
    }

    #[test]
    fn a_signature_from_another_key_does_not_verify() {
        // Signed by key B, but presented under key A's `kid` — the shape a local
        // forger with their own key pair would produce.
        let token = sign(&key_b(), &header_json(), &claims_json().to_string());
        assert_eq!(verify(&token), Err(AccessError::BadSignature));
    }

    #[test]
    fn an_unsigned_assertion_is_refused_by_the_algorithm_pin() {
        let header = format!(r#"{{"alg":"none","kid":"{KID}"}}"#);
        let token = format!(
            "{}.{}.",
            B64.encode(&header),
            B64.encode(claims_json().to_string())
        );
        // Empty third segment: refused before the algorithm is even considered.
        assert_eq!(
            verify(&token),
            Err(AccessError::Malformed("a segment is empty"))
        );

        // And with something in the signature slot, the pin is what refuses it.
        let token = format!(
            "{}.{}.{}",
            B64.encode(&header),
            B64.encode(claims_json().to_string()),
            B64.encode("not-a-signature")
        );
        assert_eq!(verify(&token), Err(AccessError::UnsupportedAlgorithm));
    }

    #[test]
    fn an_hmac_assertion_is_refused_rather_than_verified_with_the_public_key() {
        // The classic confusion: the attacker claims HS256 and signs with the
        // public modulus as the shared secret. It never reaches a verifier.
        use hmac::{Hmac, Mac};
        let header = format!(r#"{{"alg":"HS256","kid":"{KID}"}}"#);
        let signing_input = format!(
            "{}.{}",
            B64.encode(&header),
            B64.encode(claims_json().to_string())
        );
        let mut mac = <Hmac<sha2::Sha256>>::new_from_slice(TEST_KEY_A_N.as_bytes())
            .expect("hmac accepts any key length");
        mac.update(signing_input.as_bytes());
        let token = format!(
            "{signing_input}.{}",
            B64.encode(mac.finalize().into_bytes())
        );
        assert_eq!(verify(&token), Err(AccessError::UnsupportedAlgorithm));
    }

    #[test]
    fn a_header_demanding_unknown_extensions_is_refused() {
        let header = format!(r#"{{"alg":"RS256","kid":"{KID}","crit":["exp"]}}"#);
        let token = sign(&key_a(), &header, &claims_json().to_string());
        assert_eq!(
            verify(&token),
            Err(AccessError::Malformed("header demands unknown extensions"))
        );
    }

    #[test]
    fn a_key_we_do_not_publish_is_unknown_rather_than_trusted() {
        let header = r#"{"alg":"RS256","kid":"kid-somebody-else"}"#;
        let token = sign(&key_a(), header, &claims_json().to_string());
        assert_eq!(verify(&token), Err(AccessError::UnknownKey));
    }

    #[test]
    fn a_header_naming_no_key_is_refused() {
        let token = sign(&key_a(), r#"{"alg":"RS256"}"#, &claims_json().to_string());
        assert_eq!(
            verify(&token),
            Err(AccessError::Malformed("header names no key"))
        );
    }

    #[test]
    fn an_expired_assertion_is_refused() {
        let mut claims = claims_json();
        claims["exp"] = serde_json::json!(now().timestamp() - CLOCK_LEEWAY_SECONDS - 1);
        let token = sign(&key_a(), &header_json(), &claims.to_string());
        assert_eq!(verify(&token), Err(AccessError::Expired));
    }

    #[test]
    fn an_assertion_without_an_expiry_is_refused() {
        let mut claims = claims_json();
        claims.as_object_mut().expect("object").remove("exp");
        let token = sign(&key_a(), &header_json(), &claims.to_string());
        assert_eq!(verify(&token), Err(AccessError::Expired));
    }

    #[test]
    fn an_assertion_that_expired_within_the_leeway_still_passes() {
        let mut claims = claims_json();
        claims["exp"] = serde_json::json!(now().timestamp() - CLOCK_LEEWAY_SECONDS + 1);
        let token = sign(&key_a(), &header_json(), &claims.to_string());
        assert!(
            verify(&token).is_ok(),
            "clock skew is tolerated, not ignored"
        );
    }

    #[test]
    fn an_assertion_from_the_future_is_refused() {
        let mut claims = claims_json();
        claims["nbf"] = serde_json::json!(now().timestamp() + CLOCK_LEEWAY_SECONDS + 1);
        let token = sign(&key_a(), &header_json(), &claims.to_string());
        assert_eq!(verify(&token), Err(AccessError::NotYetValid));
    }

    #[test]
    fn another_teams_assertion_is_refused() {
        let mut claims = claims_json();
        claims["iss"] = serde_json::json!("https://attacker.cloudflareaccess.com");
        let token = sign(&key_a(), &header_json(), &claims.to_string());
        assert_eq!(verify(&token), Err(AccessError::WrongIssuer));
    }

    #[test]
    fn an_assertion_with_no_issuer_is_refused() {
        let mut claims = claims_json();
        claims.as_object_mut().expect("object").remove("iss");
        let token = sign(&key_a(), &header_json(), &claims.to_string());
        assert_eq!(verify(&token), Err(AccessError::WrongIssuer));
    }

    #[test]
    fn an_assertion_for_another_application_is_refused() {
        // Same team, same signing key, different Access application: this is the
        // case `aud` exists for, and the one an issuer check alone would miss.
        let mut claims = claims_json();
        claims["aud"] =
            serde_json::json!(["fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"]);
        let token = sign(&key_a(), &header_json(), &claims.to_string());
        assert_eq!(verify(&token), Err(AccessError::WrongAudience));
    }

    #[test]
    fn an_assertion_with_no_audience_is_refused() {
        let mut claims = claims_json();
        claims.as_object_mut().expect("object").remove("aud");
        let token = sign(&key_a(), &header_json(), &claims.to_string());
        assert_eq!(verify(&token), Err(AccessError::WrongAudience));
    }

    #[test]
    fn an_audience_may_be_a_bare_string_or_a_list() {
        let mut claims = claims_json();
        claims["aud"] = serde_json::json!(AUD);
        let token = sign(&key_a(), &header_json(), &claims.to_string());
        assert!(verify(&token).is_ok(), "a single audience is a string");

        let mut claims = claims_json();
        claims["aud"] = serde_json::json!(["other", AUD]);
        let token = sign(&key_a(), &header_json(), &claims.to_string());
        assert!(verify(&token).is_ok(), "ours among several is still ours");
    }

    #[test]
    fn a_service_token_is_not_a_person() {
        let mut claims = claims_json();
        let obj = claims.as_object_mut().expect("object");
        obj.remove("email");
        obj.insert("common_name".into(), serde_json::json!("ci-runner"));
        let token = sign(&key_a(), &header_json(), &claims.to_string());
        assert_eq!(verify(&token), Err(AccessError::NoIdentity));
    }

    #[test]
    fn a_blank_email_is_not_a_person() {
        let mut claims = claims_json();
        claims["email"] = serde_json::json!("   ");
        let token = sign(&key_a(), &header_json(), &claims.to_string());
        assert_eq!(verify(&token), Err(AccessError::NoIdentity));
    }

    #[test]
    fn things_that_are_not_a_jws_are_refused_before_anything_else() {
        for (token, why) in [
            ("", "empty"),
            ("one-segment", "a JWS has exactly three segments"),
            ("two.segments", "a JWS has exactly three segments"),
            ("a.b.c.d", "a JWS has exactly three segments"),
            (".b.c", "a segment is empty"),
            ("a..c", "a segment is empty"),
            ("a.b.", "a segment is empty"),
        ] {
            assert_eq!(
                verify(token),
                Err(AccessError::Malformed(why)),
                "for {token:?}"
            );
        }
    }

    #[test]
    fn a_segment_that_is_not_base64url_is_refused() {
        let token = "!!!.###.$$$";
        assert_eq!(
            verify(token),
            Err(AccessError::Malformed("a segment is not base64url"))
        );
    }

    #[test]
    fn an_implausibly_large_assertion_is_refused_without_being_parsed() {
        let token = format!(
            "{}.{}.{}",
            "a".repeat(4096),
            "b".repeat(4096),
            "c".repeat(4096)
        );
        assert_eq!(
            verify(&token),
            Err(AccessError::Malformed("implausibly large"))
        );
    }

    #[test]
    fn the_key_set_takes_rsa_signing_keys_and_leaves_the_rest() {
        let keys = parse_jwks(&format!(
            r#"{{"keys":[
                {{"kid":"ec","kty":"EC","crv":"P-256","x":"a","y":"b"}},
                {{"kid":"oct","kty":"oct","n":"{TEST_KEY_A_N}","e":"AQAB"}},
                {{"kid":"enc","kty":"RSA","alg":"RSA-OAEP","n":"{TEST_KEY_A_N}","e":"AQAB"}},
                {{"kty":"RSA","n":"{TEST_KEY_A_N}","e":"AQAB"}},
                {{"kid":"{KID}","kty":"RSA","alg":"RS256","n":"{TEST_KEY_A_N}","e":"AQAB"}}
            ]}}"#
        ))
        .expect("one usable key");
        assert_eq!(keys.len(), 1);
        assert!(keys.contains_key(KID));
        // The `oct` entry above carries RSA-shaped components on purpose: the
        // key type is what decides, not whether `n` and `e` happen to be there.
        assert!(!keys.contains_key("oct"));
    }

    #[test]
    fn a_key_set_with_no_rsa_key_is_an_error_rather_than_an_empty_silence() {
        let err = parse_jwks(r#"{"keys":[{"kid":"ec","kty":"EC","crv":"P-256"}]}"#).unwrap_err();
        assert!(matches!(err, AccessError::Jwks(_)), "{err:?}");
        assert!(parse_jwks(r#"{"keys":[]}"#).is_err());
        assert!(parse_jwks("not json").is_err());
    }

    #[test]
    fn an_rsa_key_without_an_alg_is_still_usable() {
        // `alg` is optional in RFC 7517, and Cloudflare has published keys both
        // ways. Requiring it would turn a rotation into an outage.
        let keys = parse_jwks(&format!(
            r#"{{"keys":[{{"kid":"{KID}","kty":"RSA","n":"{TEST_KEY_A_N}","e":"AQAB"}}]}}"#
        ))
        .expect("usable");
        assert!(keys.contains_key(KID));
    }

    #[test]
    fn the_config_normalises_what_operators_actually_paste() {
        for written in [
            "example.cloudflareaccess.com",
            "https://example.cloudflareaccess.com",
            "https://example.cloudflareaccess.com/",
            "  example.cloudflareaccess.com  ",
        ] {
            let cfg = AccessConfig::new(written, AUD).expect(written);
            assert_eq!(cfg.issuer(), format!("https://{TEAM}"));
            assert_eq!(
                cfg.jwks_url(),
                format!("https://{TEAM}/cdn-cgi/access/certs")
            );
        }
    }

    #[test]
    fn a_team_domain_that_could_move_the_trust_anchor_is_refused() {
        // Each of these would make `jwks_url()` point somewhere other than the
        // host it appears to name.
        for bad in [
            "example.com/../attacker.com",
            "example.com:8080",
            "user@attacker.com",
            "example.com?x=1",
            "example.com#f",
            "exa mple.com",
            "",
            "   ",
            "https://",
        ] {
            assert!(
                AccessConfig::new(bad, AUD).is_none(),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn half_a_configuration_is_no_configuration() {
        assert!(AccessConfig::new(TEAM, "").is_none());
        assert!(AccessConfig::new(TEAM, "   ").is_none());
        assert!(AccessConfig::new("", AUD).is_none());
    }

    #[tokio::test]
    async fn a_request_that_did_not_come_over_the_tunnel_is_refused() {
        let verifier = AccessVerifier::new(Some(config()));
        let token = good_token();
        for peer in ["192.168.0.10", "10.0.0.1", "203.0.113.7"] {
            assert_eq!(
                verifier
                    .verify(&token, peer.parse().expect("addr"), now())
                    .await,
                Err(AccessError::NotFromEdge),
                "{peer} is not the tunnel"
            );
        }
    }

    #[tokio::test]
    async fn an_unconfigured_deployment_accepts_nothing() {
        let verifier = AccessVerifier::new(None);
        assert!(!verifier.is_enabled());
        assert_eq!(
            verifier
                .verify(&good_token(), "127.0.0.1".parse().expect("addr"), now())
                .await,
            Err(AccessError::NotConfigured),
            "an unconfigured verifier refuses before it looks at anything"
        );
    }

    #[tokio::test]
    async fn the_loopback_check_runs_before_any_key_is_fetched() {
        // `example.cloudflareaccess.com` is not a host this suite may contact.
        // If a non-loopback peer got as far as fetching, this test would hang on
        // the network rather than answer.
        let verifier = AccessVerifier::new(Some(config()));
        assert_eq!(
            verifier
                .verify("", "8.8.8.8".parse().expect("addr"), now())
                .await,
            Err(AccessError::NotFromEdge)
        );
    }
}
