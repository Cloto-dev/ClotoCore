//! LM Studio-specific model probe.
//!
//! LM Studio exposes a beta endpoint `/api/v0/models` that enriches each entry
//! with runtime metadata — whether the model is currently loaded into memory,
//! the configured `max_context_length`, and the architecture name. This module
//! queries that endpoint (short timeout, optional) so the Dashboard can show
//! meaningful hints in the model dropdown without forcing the user to open
//! LM Studio to check.
//!
//! The probe is intentionally best-effort:
//!   - Only fires when the provider is plausibly LM Studio (local provider id
//!     or loopback host) to avoid hitting unrelated SaaS endpoints.
//!   - Short 2-second timeout, distinct from the provider's overall
//!     `timeout_secs`, so a hung local server never blocks the dropdown.
//!   - Returns `None` on any failure; callers continue with the plain
//!     `/v1/models` result.
//!
//! Results are cached in [`ProbeCache`] for 10 seconds to avoid hammering
//! LM Studio when the dropdown is opened repeatedly.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Deserialize;

/// Per-model runtime info returned by LM Studio's `/api/v0/models`.
///
/// `max_context_length` is the model's native maximum from its GGUF metadata;
/// `loaded_context_length` is the **actual** `n_ctx` the running instance is
/// configured for — LM Studio only populates it when `state == "loaded"`.
/// They often differ: a model might advertise `max=262_144` but be loaded at
/// `n_ctx=4096` (LM Studio's default). The loaded value is what constrains
/// real requests, so pre-flight validation and the dashboard's "Detect" button
/// must prefer it when available.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelInfo {
    pub loaded: bool,
    pub max_context_length: Option<i64>,
    pub loaded_context_length: Option<i64>,
    pub architecture: Option<String>,
}

/// LM Studio response shape (only the fields we care about).
#[derive(Debug, Deserialize)]
struct LmStudioModelsResponse {
    data: Vec<LmStudioModel>,
}

#[derive(Debug, Deserialize)]
struct LmStudioModel {
    id: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    max_context_length: Option<i64>,
    #[serde(default)]
    loaded_context_length: Option<i64>,
    #[serde(default)]
    arch: Option<String>,
}

/// Timeout for the LM Studio-specific probe. Kept deliberately short so a hung
/// local server never stalls the dropdown; the main `/v1/models` call uses the
/// provider's configured `timeout_secs`.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long to serve a probe result from cache before re-querying.
const CACHE_TTL: Duration = Duration::from_secs(10);

/// Extract the origin (scheme://host[:port]) from any provider api_url.
/// Returns `None` for malformed or non-http(s) inputs.
fn origin_of(api_url: &str) -> Option<String> {
    let url = reqwest::Url::parse(api_url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    Some(match url.port() {
        Some(p) => format!("{}://{}:{}", url.scheme(), url.host_str()?, p),
        None => format!("{}://{}", url.scheme(), url.host_str()?),
    })
}

/// True when the URL plausibly targets a local LM Studio / llama-server instance
/// (loopback host). Remote SaaS endpoints are never probed.
fn is_local_host(api_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(api_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    // Accept any loopback IP literal (IPv4 127.0.0.0/8 or IPv6 ::1).
    // url may return the IPv6 literal bracketed (`[::1]`) — strip before parsing.
    let stripped = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(addr) = stripped.parse::<std::net::IpAddr>() {
        return addr.is_loopback();
    }
    false
}

/// Decide whether it's worth calling `/api/v0/models` for a given provider.
#[must_use]
pub fn should_probe(provider_id: &str, api_url: &str) -> bool {
    provider_id == "local" || is_local_host(api_url)
}

/// Query LM Studio's `/api/v0/models` once. Returns `None` on any failure —
/// timeout, network error, non-2xx status, unparseable JSON — so the caller
/// can gracefully fall back to the plain `/v1/models` result.
async fn probe_once(api_url: &str, client: &reqwest::Client) -> Option<HashMap<String, ModelInfo>> {
    let origin = origin_of(api_url)?;
    let probe_url = format!("{}/api/v0/models", origin);

    let response = client
        .get(&probe_url)
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let parsed: LmStudioModelsResponse = response.json().await.ok()?;
    let mut out = HashMap::with_capacity(parsed.data.len());
    for m in parsed.data {
        out.insert(
            m.id,
            ModelInfo {
                loaded: m.state.as_deref() == Some("loaded"),
                max_context_length: m.max_context_length,
                loaded_context_length: m.loaded_context_length,
                architecture: m.arch,
            },
        );
    }
    Some(out)
}

type CachedEntry = (Instant, HashMap<String, ModelInfo>);

/// 10-second TTL cache of probe results keyed by provider id.
/// Cheap to share across handler invocations via `Arc`.
#[derive(Default, Clone)]
pub struct ProbeCache {
    inner: Arc<Mutex<HashMap<String, CachedEntry>>>,
}

impl ProbeCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Retrieve a cached probe if fresh, or run a new one and cache it.
    /// Returns `None` if the probe fails and there is no valid cached entry.
    pub async fn get_or_probe(
        &self,
        provider_id: &str,
        api_url: &str,
        client: &reqwest::Client,
    ) -> Option<HashMap<String, ModelInfo>> {
        // Fast path: fresh cache hit. The guard is released before any await.
        if let Ok(guard) = self.inner.lock() {
            if let Some((ts, data)) = guard.get(provider_id) {
                if ts.elapsed() < CACHE_TTL {
                    return Some(data.clone());
                }
            }
        }

        let fresh = probe_once(api_url, client).await?;
        if let Ok(mut guard) = self.inner.lock() {
            guard.insert(provider_id.to_string(), (Instant::now(), fresh.clone()));
        }
        Some(fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_extracts_scheme_host_port() {
        assert_eq!(
            origin_of("http://localhost:1234/v1/chat/completions").as_deref(),
            Some("http://localhost:1234")
        );
        assert_eq!(
            origin_of("https://api.anthropic.com/v1/messages").as_deref(),
            Some("https://api.anthropic.com")
        );
    }

    #[test]
    fn origin_rejects_non_http_schemes() {
        assert!(origin_of("file:///etc/passwd").is_none());
        assert!(origin_of("not-a-url").is_none());
    }

    #[test]
    fn local_host_detection() {
        assert!(is_local_host("http://localhost:1234/v1/chat/completions"));
        assert!(is_local_host("http://127.0.0.1:8080/anything"));
        assert!(is_local_host("http://[::1]:1234/x"));
        assert!(!is_local_host("https://api.openai.com/v1"));
    }

    #[test]
    fn should_probe_local_provider_id() {
        assert!(should_probe("local", "http://anything/"));
    }

    #[test]
    fn should_probe_loopback_host() {
        assert!(should_probe("custom", "http://127.0.0.1:9999/v1/chat"));
    }

    #[test]
    fn should_not_probe_remote_host() {
        assert!(!should_probe("claude", "https://api.anthropic.com/v1/messages"));
    }

    #[test]
    fn parses_lmstudio_response() {
        // Fixture mirroring the real LM Studio /api/v0/models payload:
        //   - loaded model: both max_context_length (GGUF native) and
        //     loaded_context_length (actual n_ctx) set, often to different values.
        //   - not-loaded model: max_context_length only; loaded_context_length absent.
        //   - degraded shape: state=loaded but no ctx fields at all (resilient parse).
        let body = r#"{
            "object": "list",
            "data": [
                {"id":"qwen/qwen3.5-9b","state":"loaded","max_context_length":262144,"loaded_context_length":4096,"arch":"qwen3"},
                {"id":"qwen/qwen3.5-27b","state":"not-loaded","max_context_length":32768,"arch":"qwen3"},
                {"id":"no-ctx","state":"loaded"}
            ]
        }"#;
        let parsed: LmStudioModelsResponse = serde_json::from_str(body).unwrap();
        let mut map: HashMap<String, ModelInfo> = HashMap::new();
        for m in parsed.data {
            map.insert(
                m.id,
                ModelInfo {
                    loaded: m.state.as_deref() == Some("loaded"),
                    max_context_length: m.max_context_length,
                    loaded_context_length: m.loaded_context_length,
                    architecture: m.arch,
                },
            );
        }
        assert!(map["qwen/qwen3.5-9b"].loaded);
        assert_eq!(map["qwen/qwen3.5-9b"].max_context_length, Some(262_144));
        assert_eq!(map["qwen/qwen3.5-9b"].loaded_context_length, Some(4096));
        assert_eq!(
            map["qwen/qwen3.5-9b"].architecture.as_deref(),
            Some("qwen3")
        );
        assert!(!map["qwen/qwen3.5-27b"].loaded);
        assert_eq!(map["qwen/qwen3.5-27b"].max_context_length, Some(32768));
        assert_eq!(map["qwen/qwen3.5-27b"].loaded_context_length, None);
        assert!(map["no-ctx"].loaded);
        assert_eq!(map["no-ctx"].max_context_length, None);
        assert_eq!(map["no-ctx"].loaded_context_length, None);
    }
}
