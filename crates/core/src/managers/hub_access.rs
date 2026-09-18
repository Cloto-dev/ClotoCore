//! Restricted distribution: this kernel's access token for the hub, and the
//! key that token is bound to.
//!
//! A connector the hub publishes as *restricted* is listed and served only to a
//! kernel that holds an access token covering it and signs each request with
//! the key the token was bound to (`docs/HUB_ACCESS_DESIGN.md`). This module is
//! the kernel's half:
//!
//! - the kernel's Ed25519 key pair, created on first bind;
//! - the stored token and what it opens;
//! - the headers a request to the hub carries ([`request_headers`]), attached
//!   only when the request goes to the hub the token was bound on;
//! - bind and renew, the two calls that change the token;
//! - when the operator should be told that the token is running out.
//!
//! Both secrets live under `data_dir/hub-access/`, written `0600`, in the same
//! class as the admin key. **Neither is ever returned by an API.** What the
//! status route may show is [`AccessStatus`]: a short prefix, the connectors,
//! the expiry and the key fingerprint.
//!
//! An installed restricted connector does not depend on anything here once it
//! is installed: its files, seal and install receipt are local. A token that
//! lapses stops updates, not the connector (design §6).

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, NaiveDateTime, SecondsFormat, Utc};
use mgp_seal::ed25519::{self, KeyId, PrivateKey, PublicKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Prefix of an access token. A publisher token (`chub_`) is a different
/// credential class and is never accepted here.
// HARDCODED(docs/HUB_ACCESS_DESIGN.md §2): the hub mints this prefix; the
// kernel has to recognise it before any request, so it cannot be read.
pub const ACCESS_TOKEN_PREFIX: &str = "chubr_";

/// Key id mixed into every access signature. The hub verifies with the same
/// constant; it keeps these signatures apart from seal signatures made with the
/// same primitive.
// HARDCODED(docs/HUB_ACCESS_DESIGN.md §3): part of the signed wire format; both
// ends must hold the same value, and neither can fetch it from the other.
pub const KERNEL_ACCESS_KEY_ID: &str = "cloto-kernel-access-v1";

// HARDCODED(docs/HUB_ACCESS_DESIGN.md §3): header names of the wire format.
pub const HEADER_SIGNATURE: &str = "X-Cloto-Kernel-Signature";
pub const HEADER_TIMESTAMP: &str = "X-Cloto-Kernel-Timestamp";

/// How long before expiry the operator starts being told.
// HARDCODED(docs/HUB_ACCESS_DESIGN.md §7): a policy of this kernel, not a value
// the hub owns.
pub const RENEWAL_NOTICE_DAYS: i64 = 30;

/// Timeout for a bind or renew call to the hub.
// HARDCODED(docs/HUB_ACCESS_DESIGN.md §4): a local bound on one request; no
// other component owns it.
const HUB_CALL_TIMEOUT_SECS: u64 = 30;

// HARDCODED(docs/HUB_ACCESS_DESIGN.md §5): where this module keeps its files.
const DIR: &str = "hub-access";
const KEY_FILE: &str = "kernel-access.key";
const TOKEN_FILE: &str = "token.json";

/// Serialises everything that changes the stored token or key. Two binds at
/// once would each create a key and one would be stored against the other's
/// binding.
static WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// What is stored about the token. Private to this module: the token itself is
/// in here, and nothing outside should be able to serialise it into a response.
#[derive(Clone, Serialize, Deserialize)]
struct StoredToken {
    token: String,
    token_id: String,
    connector_ids: Vec<String>,
    fingerprint: String,
    expires_at: DateTime<Utc>,
    /// Origin (`scheme://host[:port]`) of the hub the token was bound on. The
    /// token is only ever presented there.
    hub_origin: String,
}

/// What may be shown about the token. No secret is in here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccessStatus {
    pub token_prefix: String,
    pub token_id: String,
    pub connector_ids: Vec<String>,
    pub fingerprint: String,
    pub expires_at: DateTime<Utc>,
    pub hub_origin: String,
    pub stage: ExpiryStage,
}

/// Where the token is in its life, as far as the operator needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpiryStage {
    Valid,
    /// Inside the renewal window: renew now.
    ExpiresSoon,
    /// Past `expires_at`. It cannot be renewed; a person issues a new one on
    /// the hub. Installed connectors keep working.
    Expired,
}

/// Decide the stage at `now`. The window opens exactly
/// [`RENEWAL_NOTICE_DAYS`] before expiry, not a moment earlier.
#[must_use]
pub fn expiry_stage(expires_at: DateTime<Utc>, now: DateTime<Utc>) -> ExpiryStage {
    if now >= expires_at {
        ExpiryStage::Expired
    } else if now >= expires_at - Duration::days(RENEWAL_NOTICE_DAYS) {
        ExpiryStage::ExpiresSoon
    } else {
        ExpiryStage::Valid
    }
}

impl StoredToken {
    fn status(&self, now: DateTime<Utc>) -> AccessStatus {
        AccessStatus {
            token_prefix: token_prefix(&self.token),
            token_id: self.token_id.clone(),
            connector_ids: self.connector_ids.clone(),
            fingerprint: self.fingerprint.clone(),
            expires_at: self.expires_at,
            hub_origin: self.hub_origin.clone(),
            stage: expiry_stage(self.expires_at, now),
        }
    }
}

/// Whether `token` has the shape of an access token. Checked before anything
/// is sent anywhere.
#[must_use]
pub fn is_access_token(token: &str) -> bool {
    let token = token.trim();
    token.starts_with(ACCESS_TOKEN_PREFIX) && token.len() > ACCESS_TOKEN_PREFIX.len()
}

/// Enough of the token to tell two apart on a screen: the class prefix and
/// four characters. Never more.
fn token_prefix(token: &str) -> String {
    let shown: String = token.chars().take(ACCESS_TOKEN_PREFIX.len() + 4).collect();
    format!("{shown}…")
}

// ─── Storage ────────────────────────────────────────────────────────────────

fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR)
}

/// Write `content` to `path` with mode `0600`, atomically: the file is either
/// the old one or the new one, never half of either.
fn write_secret(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        std::io::Write::write_all(&mut file, content.as_bytes())?;
        file.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

fn load_key(data_dir: &Path) -> anyhow::Result<Option<PrivateKey>> {
    let path = dir(data_dir).join(KEY_FILE);
    match std::fs::read_to_string(&path) {
        Ok(raw) => Ok(Some(ed25519::private_key_from_base64(raw.trim()).map_err(
            |e| anyhow::anyhow!("kernel access key is unreadable: {e}"),
        )?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// The kernel's key, created on first use. Kept across forgetting a token, so
/// the next token binds to the same fingerprint the hub has already seen.
fn load_or_create_key(data_dir: &Path) -> anyhow::Result<PrivateKey> {
    if let Some(key) = load_key(data_dir)? {
        return Ok(key);
    }
    let (key, _) = ed25519::generate_keypair(&mut rand::rngs::OsRng);
    write_secret(&dir(data_dir).join(KEY_FILE), &key.to_base64())?;
    Ok(key)
}

fn load_token(data_dir: &Path) -> anyhow::Result<Option<StoredToken>> {
    let path = dir(data_dir).join(TOKEN_FILE);
    match std::fs::read_to_string(&path) {
        Ok(raw) => Ok(Some(serde_json::from_str(&raw)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn store_token(data_dir: &Path, token: &StoredToken) -> anyhow::Result<()> {
    write_secret(
        &dir(data_dir).join(TOKEN_FILE),
        &serde_json::to_string(token)?,
    )?;
    Ok(())
}

/// The stored token's status, or `None` when no token is stored.
pub fn status(data_dir: &Path, now: DateTime<Utc>) -> anyhow::Result<Option<AccessStatus>> {
    Ok(load_token(data_dir)?.map(|t| t.status(now)))
}

/// Connector ids the stored token covers, expired or not. An installed
/// connector on this list is restricted, not dropped from the catalog.
#[must_use]
pub fn covered_connectors(data_dir: &Path) -> Vec<String> {
    load_token(data_dir)
        .ok()
        .flatten()
        .map(|t| t.connector_ids)
        .unwrap_or_default()
}

/// Forget the token. The key stays. Returns whether a token was stored.
pub async fn forget(data_dir: &Path) -> anyhow::Result<bool> {
    let _guard = WRITE_LOCK.lock().await;
    match std::fs::remove_file(dir(data_dir).join(TOKEN_FILE)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

// ─── Signing ────────────────────────────────────────────────────────────────

fn key_id() -> KeyId {
    KeyId::new(KERNEL_ACCESS_KEY_ID).expect("constant key id is valid")
}

/// SHA-256 hex of the public key bytes: what the hub shows people to tell
/// kernels apart.
#[must_use]
pub fn fingerprint(key: &PublicKey) -> String {
    hex::encode(Sha256::digest(key.to_bytes()))
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

/// `METHOD \n PATH-with-query \n TIMESTAMP \n SHA-256(token)`, plus `\n NONCE`
/// on renewal. Must match the hub byte for byte.
fn signing_message(
    method: &str,
    path_and_query: &str,
    timestamp: &str,
    token: &str,
    nonce: Option<&str>,
) -> Vec<u8> {
    let mut s = format!(
        "{method}\n{path_and_query}\n{timestamp}\n{}",
        sha256_hex(token)
    );
    if let Some(nonce) = nonce {
        s.push('\n');
        s.push_str(nonce);
    }
    s.into_bytes()
}

/// `scheme://host[:port]` of a URL, the port omitted when it is the default.
fn origin_of(url: &reqwest::Url) -> Option<String> {
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

fn path_and_query(url: &reqwest::Url) -> String {
    match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_string(),
    }
}

fn signed_headers(
    key: &PrivateKey,
    token: &str,
    method: &str,
    url: &reqwest::Url,
    nonce: Option<&str>,
    now: DateTime<Utc>,
) -> Vec<(String, String)> {
    let timestamp = now.to_rfc3339_opts(SecondsFormat::Secs, true);
    let message = signing_message(method, &path_and_query(url), &timestamp, token, nonce);
    let signature = ed25519::sign(key, &key_id(), &message).to_base64();
    vec![
        ("Authorization".to_string(), format!("Bearer {token}")),
        (HEADER_SIGNATURE.to_string(), signature),
        (HEADER_TIMESTAMP.to_string(), timestamp),
    ]
}

/// The headers to attach to `method url`, or `None` when this request must go
/// out without them.
///
/// Attached only when the request is for the hub the token was bound on **and**
/// that hub is the one this kernel is configured to use (`hub_base`). A catalog
/// entry can point a download anywhere; a token sent to any other origin would
/// be a token handed to whoever runs it.
#[must_use]
pub fn request_headers(
    data_dir: &Path,
    hub_base: Option<&str>,
    method: &str,
    url: &str,
) -> Option<Vec<(String, String)>> {
    let url = reqwest::Url::parse(url).ok()?;
    let configured = origin_of(&reqwest::Url::parse(hub_base?).ok()?)?;
    let target = origin_of(&url)?;
    if target != configured {
        return None;
    }
    let token = load_token(data_dir).ok()??;
    if token.hub_origin != configured {
        return None;
    }
    let key = load_key(data_dir).ok()??;
    Some(signed_headers(
        &key,
        &token.token,
        method,
        &url,
        None,
        Utc::now(),
    ))
}

// ─── Bind and renew ─────────────────────────────────────────────────────────

/// Why a bind or renew did not go through, in terms the operator can act on.
#[derive(Debug)]
pub enum AccessError {
    NotAnAccessToken,
    NoHub,
    NoToken,
    OtherHub { stored: String, configured: String },
    Expired,
    Refused,
    Hub(u16),
    Mismatch(&'static str),
    Other(anyhow::Error),
}

impl std::fmt::Display for AccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnAccessToken => write!(
                f,
                "this is not an access token (it must start with {ACCESS_TOKEN_PREFIX})"
            ),
            Self::NoHub => write!(
                f,
                "the configured catalog is not a hub, so there is nothing to bind a token to"
            ),
            Self::NoToken => write!(f, "no access token is stored"),
            Self::OtherHub { stored, configured } => write!(
                f,
                "the stored token was bound on {stored}, but this kernel now uses {configured}; \
                 forget it and set a token issued by the current hub"
            ),
            Self::Expired => write!(
                f,
                "the token has expired; a new one has to be issued on the hub"
            ),
            Self::Refused => write!(
                f,
                "the hub refused the token (it may be expired, revoked, already bound to \
                 another kernel, or past its bind deadline)"
            ),
            Self::Hub(code) => write!(f, "the hub answered HTTP {code}"),
            Self::Mismatch(what) => {
                write!(f, "the hub's answer does not match this kernel: {what}")
            }
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AccessError {}

impl From<anyhow::Error> for AccessError {
    fn from(e: anyhow::Error) -> Self {
        Self::Other(e)
    }
}

#[derive(Deserialize)]
struct BindResponse {
    token_id: String,
    connector_ids: Vec<String>,
    fingerprint: String,
    expires_at: String,
}

#[derive(Deserialize)]
struct RenewResponse {
    token: String,
    id: String,
    connector_ids: Vec<String>,
    expires_at: String,
}

/// The hub writes times as SQLite `datetime('now')` text (`YYYY-MM-DD
/// HH:MM:SS`, UTC). RFC 3339 is accepted too.
fn parse_hub_time(raw: &str) -> Option<DateTime<Utc>> {
    if let Ok(t) = DateTime::parse_from_rfc3339(raw) {
        return Some(t.with_timezone(&Utc));
    }
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|t| t.and_utc())
}

fn hub_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(HUB_CALL_TIMEOUT_SECS))
        // The token must not follow a redirect it was not sent for.
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

fn endpoint(hub_base: &str, path: &str) -> Result<reqwest::Url, AccessError> {
    reqwest::Url::parse(&format!("{}{path}", hub_base.trim_end_matches('/')))
        .map_err(|_| AccessError::NoHub)
}

async fn post_signed(
    url: &reqwest::Url,
    headers: Vec<(String, String)>,
    body: serde_json::Value,
) -> Result<reqwest::Response, AccessError> {
    let mut request = hub_client()?.post(url.clone()).json(&body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await.map_err(anyhow::Error::from)?;
    match response.status().as_u16() {
        200..=299 => Ok(response),
        401 => Err(AccessError::Refused),
        code => Err(AccessError::Hub(code)),
    }
}

/// Bind `token` to this kernel on the hub at `hub_base` and store it (design
/// §2). The key pair is created if there is none. The request is signed with
/// the key it registers, which proves this kernel holds it.
pub async fn bind(
    data_dir: &Path,
    hub_base: &str,
    token: &str,
) -> Result<AccessStatus, AccessError> {
    let token = token.trim();
    if !is_access_token(token) {
        return Err(AccessError::NotAnAccessToken);
    }
    // HARDCODED(docs/HUB_ACCESS_DESIGN.md §2): the hub's bind route.
    let url = endpoint(hub_base, "/api/access/bind")?;
    let hub_origin = origin_of(&url).ok_or(AccessError::NoHub)?;

    let _guard = WRITE_LOCK.lock().await;
    let key = load_or_create_key(data_dir)?;
    let public = key.public_key();
    let headers = signed_headers(&key, token, "POST", &url, None, Utc::now());
    let response = post_signed(
        &url,
        headers,
        serde_json::json!({ "public_key": public.to_base64() }),
    )
    .await?;
    let bound: BindResponse = response.json().await.map_err(anyhow::Error::from)?;
    let fingerprint = fingerprint(&public);
    if bound.fingerprint != fingerprint {
        return Err(AccessError::Mismatch("the hub recorded a different key"));
    }
    let expires_at =
        parse_hub_time(&bound.expires_at).ok_or(AccessError::Mismatch("unreadable expiry"))?;
    let stored = StoredToken {
        token: token.to_string(),
        token_id: bound.token_id,
        connector_ids: bound.connector_ids,
        fingerprint,
        expires_at,
        hub_origin,
    };
    store_token(data_dir, &stored)?;
    Ok(stored.status(Utc::now()))
}

/// A single-use nonce the hub accepts: 32 characters of `[A-Za-z0-9]`.
fn new_nonce() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Renew the stored token (design §4). The hub issues a new token bound to
/// the same key and revokes this one in the same transaction; the new one
/// replaces what is stored. An expired token cannot renew.
pub async fn renew(data_dir: &Path, hub_base: &str) -> Result<AccessStatus, AccessError> {
    // HARDCODED(docs/HUB_ACCESS_DESIGN.md §4): the hub's renewal route.
    let url = endpoint(hub_base, "/api/access/renew")?;
    let configured = origin_of(&url).ok_or(AccessError::NoHub)?;

    let _guard = WRITE_LOCK.lock().await;
    let stored = load_token(data_dir)?.ok_or(AccessError::NoToken)?;
    if stored.hub_origin != configured {
        return Err(AccessError::OtherHub {
            stored: stored.hub_origin,
            configured,
        });
    }
    if expiry_stage(stored.expires_at, Utc::now()) == ExpiryStage::Expired {
        return Err(AccessError::Expired);
    }
    let key = load_key(data_dir)?.ok_or(AccessError::Mismatch("the bound key is missing"))?;
    let nonce = new_nonce();
    let headers = signed_headers(&key, &stored.token, "POST", &url, Some(&nonce), Utc::now());
    let response = post_signed(&url, headers, serde_json::json!({ "nonce": nonce })).await?;
    let issued: RenewResponse = response.json().await.map_err(anyhow::Error::from)?;
    if !issued.token.starts_with(ACCESS_TOKEN_PREFIX) {
        return Err(AccessError::Mismatch(
            "the new token is not an access token",
        ));
    }
    let expires_at =
        parse_hub_time(&issued.expires_at).ok_or(AccessError::Mismatch("unreadable expiry"))?;
    let renewed = StoredToken {
        token: issued.token,
        token_id: issued.id,
        connector_ids: issued.connector_ids,
        fingerprint: stored.fingerprint,
        expires_at,
        hub_origin: stored.hub_origin,
    };
    store_token(data_dir, &renewed)?;
    Ok(renewed.status(Utc::now()))
}

/// Store a token without a hub, for tests elsewhere in the crate.
#[cfg(test)]
pub(crate) fn store_for_test(
    data_dir: &Path,
    connector_id: &str,
    hub_origin: &str,
    expires_at: DateTime<Utc>,
) {
    load_or_create_key(data_dir).unwrap();
    store_token(
        data_dir,
        &StoredToken {
            token: format!("{ACCESS_TOKEN_PREFIX}{}", "cd".repeat(32)),
            token_id: "T-test".into(),
            connector_ids: vec![connector_id.to_string()],
            fingerprint: "fp".into(),
            expires_at,
            hub_origin: hub_origin.into(),
        },
    )
    .unwrap();
}

/// An already-expired token covering `connector_id`.
#[cfg(test)]
pub(crate) fn store_expired_for_test(data_dir: &Path, connector_id: &str) {
    store_for_test(
        data_dir,
        connector_id,
        "https://hub.example",
        Utc::now() - Duration::days(1),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cloto-hub-access-{tag}-{}-{}",
            std::process::id(),
            new_nonce()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn token() -> String {
        format!("{ACCESS_TOKEN_PREFIX}{}", "ab".repeat(32))
    }

    fn stored(dir: &Path, hub_origin: &str, expires_at: DateTime<Utc>) {
        load_or_create_key(dir).unwrap();
        store_token(
            dir,
            &StoredToken {
                token: token(),
                token_id: "T1".into(),
                connector_ids: vec!["acme-panel".into()],
                fingerprint: "fp".into(),
                expires_at,
                hub_origin: hub_origin.into(),
            },
        )
        .unwrap();
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn the_renewal_window_opens_at_thirty_days_and_not_before() {
        let expires = at("2026-12-19T00:00:00Z");
        let window = expires - Duration::days(RENEWAL_NOTICE_DAYS);
        assert_eq!(
            expiry_stage(expires, window - Duration::seconds(1)),
            ExpiryStage::Valid
        );
        assert_eq!(expiry_stage(expires, window), ExpiryStage::ExpiresSoon);
        assert_eq!(
            expiry_stage(expires, expires - Duration::seconds(1)),
            ExpiryStage::ExpiresSoon
        );
        assert_eq!(expiry_stage(expires, expires), ExpiryStage::Expired);
    }

    #[test]
    fn headers_go_to_the_bound_hub_and_nowhere_else() {
        let dir = temp_dir("origin");
        stored(&dir, "https://hub.example", at("2099-01-01T00:00:00Z"));
        let hub = Some("https://hub.example/api/catalog");

        let to_hub = request_headers(
            &dir,
            hub,
            "GET",
            "https://hub.example/api/connectors/acme-panel/download?version=1.0.0",
        )
        .expect("the bound hub gets the headers");
        assert!(to_hub
            .iter()
            .any(|(n, v)| n == "Authorization" && v == &format!("Bearer {}", token())));

        for elsewhere in [
            "https://github.com/acme/panel/archive.tar.gz",
            "http://hub.example/api/catalog",
            "https://hub.example:8443/api/catalog",
            "https://hub.example.evil/api/catalog",
        ] {
            assert!(
                request_headers(&dir, hub, "GET", elsewhere).is_none(),
                "{elsewhere} must not receive the token"
            );
        }
        // Configured to a different hub than the token was bound on.
        assert!(request_headers(
            &dir,
            Some("https://other.example/api/catalog"),
            "GET",
            "https://other.example/api/catalog"
        )
        .is_none());
        // No hub configured at all.
        assert!(request_headers(&dir, None, "GET", "https://hub.example/api/catalog").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_signature_verifies_over_exactly_what_the_hub_reconstructs() {
        let dir = temp_dir("sig");
        stored(&dir, "https://hub.example", at("2099-01-01T00:00:00Z"));
        let url = "https://hub.example/api/connectors/acme-panel/download?version=1.0.0";
        let headers =
            request_headers(&dir, Some("https://hub.example/api/catalog"), "GET", url).unwrap();
        let get = |name: &str| {
            headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        let timestamp = get(HEADER_TIMESTAMP);
        assert!(timestamp.ends_with('Z'), "RFC 3339 UTC: {timestamp}");
        let public = load_key(&dir).unwrap().unwrap().public_key();
        let signature = ed25519::Signature::from_base64(&get(HEADER_SIGNATURE)).unwrap();
        let expected = signing_message(
            "GET",
            "/api/connectors/acme-panel/download?version=1.0.0",
            &timestamp,
            &token(),
            None,
        );
        assert!(ed25519::verify(&public, &key_id(), &expected, &signature));
        let without_query = signing_message(
            "GET",
            "/api/connectors/acme-panel/download",
            &timestamp,
            &token(),
            None,
        );
        assert!(!ed25519::verify(
            &public,
            &key_id(),
            &without_query,
            &signature
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn status_never_carries_the_token() {
        let dir = temp_dir("status");
        stored(&dir, "https://hub.example", at("2099-01-01T00:00:00Z"));
        let status = status(&dir, Utc::now()).unwrap().unwrap();
        let rendered = serde_json::to_string(&status).unwrap();
        assert!(!rendered.contains(&token()), "{rendered}");
        assert!(!rendered.contains(&"ab".repeat(8)), "{rendered}");
        assert_eq!(status.token_prefix, format!("{ACCESS_TOKEN_PREFIX}abab…"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn secrets_are_written_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("mode");
        stored(&dir, "https://hub.example", at("2099-01-01T00:00:00Z"));
        for file in [KEY_FILE, TOKEN_FILE] {
            let mode = std::fs::metadata(dir.join(DIR).join(file))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{file}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn hub_times_parse_in_both_forms() {
        assert_eq!(
            parse_hub_time("2026-12-19 03:04:05"),
            Some(at("2026-12-19T03:04:05Z"))
        );
        assert_eq!(
            parse_hub_time("2026-12-19T03:04:05Z"),
            Some(at("2026-12-19T03:04:05Z"))
        );
        assert_eq!(parse_hub_time("next week"), None);
    }

    // ─── Against a hub double ───────────────────────────────────────────────

    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Verify what a request carried the way the hub does: rebuild the message
    /// from the method, path and headers, and check the signature with `key`.
    fn hub_verifies(req: &wiremock::Request, key: &PublicKey, nonce: Option<&str>) -> bool {
        let header = |n: &str| req.headers.get(n).and_then(|v| v.to_str().ok()).unwrap();
        let token = header("authorization").strip_prefix("Bearer ").unwrap();
        let message = signing_message(
            req.method.as_str(),
            &path_and_query(&req.url),
            header("x-cloto-kernel-timestamp"),
            token,
            nonce,
        );
        let sig = ed25519::Signature::from_base64(header("x-cloto-kernel-signature")).unwrap();
        ed25519::verify(key, &key_id(), &message, &sig)
    }

    #[tokio::test]
    async fn bind_registers_the_key_it_signs_with_and_stores_what_the_hub_answered() {
        let hub = MockServer::start().await;
        let dir = temp_dir("bind");
        // The fingerprint is only known once the key exists; create it first so
        // the double can answer with it, as the hub would.
        let public = load_or_create_key(&dir).unwrap().public_key();
        Mock::given(method("POST"))
            .and(path("/api/access/bind"))
            .and(header_exists("x-cloto-kernel-signature"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token_id": "T1",
                "connector_ids": ["acme-panel"],
                "fingerprint": fingerprint(&public),
                "expires_at": "2026-12-19 00:00:00",
            })))
            .expect(1)
            .mount(&hub)
            .await;

        let status = bind(&dir, &hub.uri(), &token()).await.expect("bound");
        assert_eq!(status.token_id, "T1");
        assert_eq!(status.connector_ids, vec!["acme-panel".to_string()]);
        assert_eq!(status.expires_at, at("2026-12-19T00:00:00Z"));
        assert_eq!(status.hub_origin, hub.uri());

        let req = &hub.received_requests().await.unwrap()[0];
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body["public_key"], public.to_base64());
        assert!(
            hub_verifies(req, &public, None),
            "signed with the registered key"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn bind_refuses_a_publisher_token_without_calling_the_hub() {
        let hub = MockServer::start().await;
        let dir = temp_dir("bind-publisher");
        let err = bind(&dir, &hub.uri(), &format!("chub_{}", "ab".repeat(32)))
            .await
            .unwrap_err();
        assert!(matches!(err, AccessError::NotAnAccessToken), "{err}");
        assert!(hub.received_requests().await.unwrap().is_empty());
        assert!(status(&dir, Utc::now()).unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_hub_that_recorded_another_key_is_not_stored() {
        let hub = MockServer::start().await;
        let dir = temp_dir("bind-mismatch");
        Mock::given(method("POST"))
            .and(path("/api/access/bind"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token_id": "T1", "connector_ids": [], "fingerprint": "someone-else",
                "expires_at": "2026-12-19 00:00:00",
            })))
            .mount(&hub)
            .await;
        let err = bind(&dir, &hub.uri(), &token()).await.unwrap_err();
        assert!(matches!(err, AccessError::Mismatch(_)), "{err}");
        assert!(status(&dir, Utc::now()).unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn renew_signs_its_nonce_and_replaces_the_stored_token() {
        let hub = MockServer::start().await;
        let dir = temp_dir("renew");
        stored(&dir, &hub.uri(), Utc::now() + Duration::days(10));
        let public = load_key(&dir).unwrap().unwrap().public_key();
        let new_token = format!("{ACCESS_TOKEN_PREFIX}{}", "ef".repeat(32));
        Mock::given(method("POST"))
            .and(path("/api/access/renew"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token": new_token, "id": "T2", "connector_ids": ["acme-panel"],
                "bind_deadline": "2026-09-20 00:00:00", "expires_at": "2026-12-19 00:00:00",
            })))
            .expect(1)
            .mount(&hub)
            .await;

        let status = renew(&dir, &hub.uri()).await.expect("renewed");
        assert_eq!(status.token_id, "T2");
        assert_eq!(load_token(&dir).unwrap().unwrap().token, new_token);

        let req = &hub.received_requests().await.unwrap()[0];
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        let nonce = body["nonce"].as_str().unwrap();
        assert!(nonce.len() >= 16 && nonce.chars().all(|c| c.is_ascii_alphanumeric()));
        assert!(
            hub_verifies(req, &public, Some(nonce)),
            "the nonce is signed"
        );
        assert!(!hub_verifies(req, &public, None), "and not optional");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_expired_token_does_not_try_to_renew() {
        let hub = MockServer::start().await;
        let dir = temp_dir("renew-expired");
        stored(&dir, &hub.uri(), Utc::now() - Duration::seconds(1));
        let err = renew(&dir, &hub.uri()).await.unwrap_err();
        assert!(matches!(err, AccessError::Expired), "{err}");
        assert!(hub.received_requests().await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_refused_renewal_keeps_the_stored_token() {
        let hub = MockServer::start().await;
        let dir = temp_dir("renew-401");
        stored(&dir, &hub.uri(), Utc::now() + Duration::days(10));
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&hub)
            .await;
        let err = renew(&dir, &hub.uri()).await.unwrap_err();
        assert!(matches!(err, AccessError::Refused), "{err}");
        assert_eq!(load_token(&dir).unwrap().unwrap().token, token());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_token_bound_on_another_hub_is_not_sent_to_this_one() {
        let hub = MockServer::start().await;
        let dir = temp_dir("renew-other");
        stored(&dir, "https://hub.example", Utc::now() + Duration::days(10));
        let err = renew(&dir, &hub.uri()).await.unwrap_err();
        assert!(matches!(err, AccessError::OtherHub { .. }), "{err}");
        assert!(hub.received_requests().await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
