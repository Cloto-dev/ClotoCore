//! Advisory feed evaluation (DEFENDER_DESIGN.md §5) — canonical source 2.
//!
//! Known issues curated in `qa/issue-registry.json` are exported by the
//! release pipeline into an `advisories` block of the signed updater-feed
//! manifest (`scripts/generate-release-feed.py`). This module fetches that
//! manifest, verifies its minisign signature, and maps the advisories to the
//! installed version. Output is always a **recommendation** — the defender
//! never applies an update (HITL invariant, §8.4).

use serde::{Deserialize, Serialize};

use crate::db::health::{HealthCheck, HealthStatus};

pub const CHECK_NAME: &str = "known_issue_advisories";

/// Minisign public key for the updater feed, base64-encoded in the Tauri
/// convention. MUST stay in sync with `plugins.updater.pubkey` in
/// `dashboard/src-tauri/tauri.conf.json` and with the CI signing secret —
/// key rotation runbook: RELEASE_PIPELINE_DESIGN.md §4.5 (a 2026-03-08
/// rotation that updated only one side silently broke verification).
const FEED_PUBKEY_B64: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDY2RkQ3QzUxNzI4MTlERUMKUldUc25ZRnlVWHo5Wm92U3E3ZXdoM1BKZExuMm9jVitPdGlHYWNaVWxwL2FOenFzSEF4SGxMczcK";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Advisory {
    pub bug_id: String,
    #[serde(default)]
    pub severity: String,
    /// Version range this advisory applies to. Grammar: whitespace-separated
    /// comparators out of `>=A`, `>A`, `<=B`, `<B`, `=C` (e.g.
    /// `">=0.6.0 <0.6.7"`). Deliberately narrower than full semver ranges —
    /// see [`version_in_range`].
    pub affected: String,
    #[serde(default)]
    pub fixed_in: Option<String>,
    /// Name of a registry check that corroborates the issue locally. When
    /// that check is non-healthy the advisory is *manifesting*, not just
    /// possible.
    #[serde(default)]
    pub symptom_check: Option<String>,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Deserialize)]
struct FeedManifest {
    #[serde(default)]
    advisories: Vec<Advisory>,
    #[serde(default)]
    channels: std::collections::HashMap<String, String>,
}

fn feed_manifest_url() -> String {
    if let Ok(url) = std::env::var("CLOTO_UPDATE_FEED_URL") {
        if !url.trim().is_empty() {
            return url.trim().to_string();
        }
    }
    let repo =
        std::env::var("CLOTO_UPDATE_REPO").unwrap_or_else(|_| "Cloto-dev/ClotoCore".to_string());
    format!("https://github.com/{repo}/releases/download/updater-feed/manifest.json")
}

/// Verify a Tauri-convention signature (base64-encoded minisign signature
/// file) over `data` with a Tauri-convention public key (base64-encoded
/// minisign public-key file).
fn verify_minisign(data: &[u8], sig_b64: &str, pubkey_b64: &str) -> anyhow::Result<()> {
    use base64::Engine as _;
    let engine = base64::engine::general_purpose::STANDARD;
    let pubkey_text = String::from_utf8(engine.decode(pubkey_b64.trim())?)?;
    let public_key = minisign_verify::PublicKey::decode(&pubkey_text)
        .map_err(|e| anyhow::anyhow!("invalid feed public key: {e}"))?;
    let sig_text = String::from_utf8(engine.decode(sig_b64.trim())?)?;
    let signature = minisign_verify::Signature::decode(&sig_text)
        .map_err(|e| anyhow::anyhow!("invalid feed signature: {e}"))?;
    public_key
        .verify(data, &signature, true)
        .map_err(|e| anyhow::anyhow!("feed signature verification failed: {e}"))?;
    Ok(())
}

/// True when `version` falls inside the advisory `range`.
///
/// Comparison uses full semver precedence via [`semver::Version`] `Ord`
/// (pre-releases order below their final release), instead of
/// `semver::VersionReq` — a `VersionReq` like `<0.6.8` deliberately does NOT
/// match `0.6.8-beta.1` (cargo pre-release opt-in semantics), which would
/// silently under-report advisories on alpha/beta installs. We own both ends
/// of the grammar, so plain ordered comparison is the correct semantic here.
pub fn version_in_range(version: &semver::Version, range: &str) -> anyhow::Result<bool> {
    let mut any = false;
    for token in range.split_whitespace() {
        let token = token.trim_end_matches(',');
        if token.is_empty() {
            continue;
        }
        any = true;
        let (op, rest) = if let Some(r) = token.strip_prefix(">=") {
            (">=", r)
        } else if let Some(r) = token.strip_prefix("<=") {
            ("<=", r)
        } else if let Some(r) = token.strip_prefix('>') {
            (">", r)
        } else if let Some(r) = token.strip_prefix('<') {
            ("<", r)
        } else if let Some(r) = token.strip_prefix('=') {
            ("=", r)
        } else {
            ("=", token)
        };
        let bound = semver::Version::parse(rest.trim())
            .map_err(|e| anyhow::anyhow!("invalid version '{rest}' in range '{range}': {e}"))?;
        let holds = match op {
            ">=" => *version >= bound,
            "<=" => *version <= bound,
            ">" => *version > bound,
            "<" => *version < bound,
            _ => *version == bound,
        };
        if !holds {
            return Ok(false);
        }
    }
    if !any {
        anyhow::bail!("empty advisory range");
    }
    Ok(true)
}

/// One evaluated advisory, as surfaced in the check detail.
#[derive(Debug, Serialize)]
struct EvaluatedAdvisory {
    bug_id: String,
    severity: String,
    summary: String,
    fixed_in: Option<String>,
    /// True when the named symptom check fired locally.
    manifesting: bool,
}

/// Evaluate the advisory feed against the installed version. `prior` is the
/// list of already-evaluated registry checks, used to corroborate
/// `symptom_check` references. Never fails the scan: network unavailability
/// degrades to an explicit skip note (a broken *signature*, however, is
/// reported loudly — that is tampering or key drift, not absence).
pub async fn evaluate(prior: &[HealthCheck]) -> HealthCheck {
    let name = CHECK_NAME.to_string();
    let url = feed_manifest_url();

    let fetched = fetch_manifest(&url).await;
    let manifest = match fetched {
        Ok(manifest) => manifest,
        Err(FetchError::Unavailable(e)) => {
            return HealthCheck {
                name,
                status: HealthStatus::Healthy,
                message: format!("Advisory feed unavailable (offline?) — evaluation skipped: {e}"),
                repairable: false,
                detail: None,
            };
        }
        Err(FetchError::BadSignature(e)) => {
            return HealthCheck {
                name,
                status: HealthStatus::Error,
                message: format!(
                    "Advisory feed signature verification failed — the feed may be tampered \
                     with or the signing key has drifted: {e}"
                ),
                repairable: false,
                detail: None,
            };
        }
    };

    let current = match semver::Version::parse(env!("CARGO_PKG_VERSION")) {
        Ok(v) => v,
        Err(e) => {
            return HealthCheck {
                name,
                status: HealthStatus::Healthy,
                message: format!("Cannot parse own version: {e}"),
                repairable: false,
                detail: None,
            };
        }
    };

    let mut applicable = Vec::new();
    for advisory in &manifest.advisories {
        match version_in_range(&current, &advisory.affected) {
            Ok(true) => {
                let manifesting = advisory.symptom_check.as_ref().is_some_and(|check_name| {
                    prior
                        .iter()
                        .any(|c| &c.name == check_name && c.status != HealthStatus::Healthy)
                });
                applicable.push(EvaluatedAdvisory {
                    bug_id: advisory.bug_id.clone(),
                    severity: advisory.severity.clone(),
                    summary: advisory.summary.clone(),
                    fixed_in: advisory.fixed_in.clone(),
                    manifesting,
                });
            }
            Ok(false) => {}
            Err(e) => {
                tracing::warn!("Skipping advisory {}: {e}", advisory.bug_id);
            }
        }
    }

    if applicable.is_empty() {
        return HealthCheck {
            name,
            status: HealthStatus::Healthy,
            message: format!(
                "No known-issue advisories apply to v{} ({} advisories in feed)",
                current,
                manifest.advisories.len()
            ),
            repairable: false,
            detail: None,
        };
    }

    let manifesting = applicable.iter().filter(|a| a.manifesting).count();
    let fixes: Vec<&str> = applicable
        .iter()
        .filter_map(|a| a.fixed_in.as_deref())
        .collect();
    let recommendation = if fixes.is_empty() {
        String::new()
    } else {
        format!(" — update recommended (fixed in {})", fixes.join(", "))
    };
    HealthCheck {
        name,
        status: HealthStatus::Degraded,
        message: format!(
            "{} known issue(s) affect v{current} ({manifesting} manifesting \
             locally){recommendation}. The defender never updates automatically — run \
             `clotocore update` or use the dashboard.",
            applicable.len()
        ),
        repairable: false,
        detail: Some(serde_json::json!({
            "advisories": applicable,
            "current_channel_versions": manifest.channels,
        })),
    }
}

enum FetchError {
    /// Network/HTTP problems — expected offline, reported as a skip.
    Unavailable(String),
    /// The manifest arrived but its signature does not verify — loud.
    BadSignature(String),
}

async fn fetch_manifest(url: &str) -> Result<FeedManifest, FetchError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .user_agent(format!("ClotoCore/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| FetchError::Unavailable(e.to_string()))?;

    let get = |u: String| {
        let client = client.clone();
        async move {
            let resp = client
                .get(&u)
                .send()
                .await
                .map_err(|e| FetchError::Unavailable(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(FetchError::Unavailable(format!(
                    "{u}: HTTP {}",
                    resp.status()
                )));
            }
            resp.bytes()
                .await
                .map_err(|e| FetchError::Unavailable(e.to_string()))
        }
    };

    let manifest_bytes = get(url.to_string()).await?;
    let sig_bytes = get(format!("{url}.minisig")).await?;
    let sig_text = String::from_utf8(sig_bytes.to_vec())
        .map_err(|e| FetchError::Unavailable(e.to_string()))?;

    verify_minisign(&manifest_bytes, &sig_text, FEED_PUBKEY_B64)
        .map_err(|e| FetchError::BadSignature(e.to_string()))?;

    serde_json::from_slice::<FeedManifest>(&manifest_bytes)
        .map_err(|e| FetchError::Unavailable(format!("manifest parse: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> semver::Version {
        semver::Version::parse(s).unwrap()
    }

    #[test]
    fn range_matching_basic() {
        assert!(version_in_range(&v("0.6.5"), ">=0.6.0 <0.6.7").unwrap());
        assert!(!version_in_range(&v("0.6.7"), ">=0.6.0 <0.6.7").unwrap());
        assert!(!version_in_range(&v("0.5.9"), ">=0.6.0 <0.6.7").unwrap());
        assert!(version_in_range(&v("0.6.7"), "=0.6.7").unwrap());
        assert!(version_in_range(&v("0.6.7"), "0.6.7").unwrap());
        // comma-tolerant
        assert!(version_in_range(&v("0.6.5"), ">=0.6.0, <0.6.7").unwrap());
    }

    #[test]
    fn range_matching_includes_pre_releases() {
        // The exact pitfall VersionReq would introduce: a beta of 0.6.8 IS
        // below 0.6.8 and must be reported as affected by "<0.6.8".
        assert!(version_in_range(&v("0.6.8-beta.1"), "<0.6.8").unwrap());
        assert!(!version_in_range(&v("0.6.8"), "<0.6.8").unwrap());
        assert!(version_in_range(&v("0.6.8-alpha.2"), ">=0.6.0 <0.6.8").unwrap());
        // pre-release ordering: beta.1 < rc.1 < final
        assert!(version_in_range(&v("0.6.8-beta.1"), "<0.6.8-rc.1").unwrap());
    }

    #[test]
    fn range_rejects_garbage() {
        assert!(version_in_range(&v("0.6.5"), "").is_err());
        assert!(version_in_range(&v("0.6.5"), ">=banana").is_err());
    }

    #[test]
    fn manifest_advisories_deserialize() {
        let json = serde_json::json!({
            "schema_version": 1,
            "channels": { "current": "0.6.7" },
            "releases": [],
            "advisories": [{
                "bug_id": "bug-386",
                "severity": "critical",
                "affected": ">=0.6.0 <0.6.7",
                "fixed_in": "0.6.7",
                "symptom_check": "legacy_data_dir_drift",
                "summary": "legacy cloto-system install can break boot"
            }]
        });
        let manifest: FeedManifest = serde_json::from_value(json).unwrap();
        assert_eq!(manifest.advisories.len(), 1);
        assert_eq!(manifest.advisories[0].bug_id, "bug-386");
        assert_eq!(
            manifest.advisories[0].symptom_check.as_deref(),
            Some("legacy_data_dir_drift")
        );
    }

    #[test]
    fn manifest_without_advisories_block_is_fine() {
        let manifest: FeedManifest =
            serde_json::from_value(serde_json::json!({ "schema_version": 1 })).unwrap();
        assert!(manifest.advisories.is_empty());
    }

    #[test]
    fn verify_minisign_rejects_garbage() {
        assert!(verify_minisign(b"data", "not base64!!!", FEED_PUBKEY_B64).is_err());
        use base64::Engine as _;
        let fake_sig = base64::engine::general_purpose::STANDARD.encode(
            "untrusted comment: x\nRUTsnYFyUXz9Zm9ub25zZW5zZQ==\ntrusted comment: y\nAAAA\n",
        );
        assert!(verify_minisign(b"data", &fake_sig, FEED_PUBKEY_B64).is_err());
    }

    #[test]
    fn feed_pubkey_decodes() {
        use base64::Engine as _;
        let text = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(FEED_PUBKEY_B64)
                .unwrap(),
        )
        .unwrap();
        assert!(
            minisign_verify::PublicKey::decode(&text).is_ok(),
            "embedded feed pubkey must be a valid minisign public key"
        );
    }
}
