//! Internal LLM Proxy — Centralizes API key management (MGP §13.4 llm_completion).
//!
//! Mind MCP servers call this proxy instead of LLM provider APIs directly.
//! The proxy adds the appropriate Authorization header from the `llm_providers` table.
//! This ensures API keys are never exposed to MCP server subprocesses.
//!
//! ## Design Decision: Separate Port (By Design, not a vulnerability)
//!
//! This proxy intentionally runs on a **separate port** (default 8082) without
//! X-API-Key authentication. This is required by P5 (Strict Permission Isolation):
//!
//! - MCP servers are kernel-spawned child processes that must NOT hold admin API keys.
//! - Merging into the `/api` router (port 8081) would require sharing admin credentials
//!   with MCP servers, which is strictly worse for security.
//! - Upstream LLM providers enforce their own rate limits (429 → structured error).
//!
//! ## Proxy token (the proxy's own authenticator)
//!
//! Not holding the *admin* key is not the same as holding nothing. The kernel
//! generates a per-boot `llm_proxy_token` (`AppConfig`) and hands it to every
//! MCP child it spawns as `CLOTO_LLM_PROXY_TOKEN`; callers present it as
//! `Authorization: Bearer <token>` or `X-Proxy-Token: <token>`, and the
//! comparison here is constant-time. The token is scoped to this proxy only —
//! it grants no kernel API access — so P5 is preserved.
//!
//! Enforcement is staged. `llm_proxy_require_token`
//! (`CLOTO_LLM_PROXY_REQUIRE_TOKEN=1`) makes a missing/wrong token a `401`;
//! with it off — the current default — the request is served and a
//! rate-limited warning is logged instead, so connectors that do not yet send
//! the header keep working. The default flips once the engine library ships
//! the header.
//!
//! ## The bind address is NOT a security boundary
//!
//! This proxy binds `127.0.0.1` (hard-coded, unlike the kernel API, which honours
//! `BIND_ADDRESS`). A loopback bind narrows who can reach the process; it does not
//! make the process unreachable. Any tunnel or reverse proxy that forwards to
//! loopback — cloudflared, ngrok, an nginx `proxy_pass`, a container port
//! publish — puts this port on the far side of that forwarder. A sibling service
//! in this project was publicly reachable for 13 days on exactly that path while
//! its own comments described it as loopback-only.
//!
//! So: reaching this port means calling an upstream LLM provider with someone
//! else's stored API key, and the bind address is not what prevents that.
//!
//! ## Status of the By Design closure
//!
//! Code Quality Audit H-4/H-5 (2026-03-22) closed the missing authentication as
//! By Design, and one of the four stated grounds was the loopback claim corrected
//! above. Removing it leaves the P5 argument and the upstream rate limits — and
//! both of those justify *not sharing admin credentials*, which is a different
//! claim from *this proxy needs no authentication of its own*. The proxy token
//! above answers that second claim; until enforcement is the default, the gap is
//! narrowed rather than closed, because an unauthenticated request is still
//! served (loudly). Do not read this module as settled.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::Value;
use sqlx::SqlitePool;
use tracing::{debug, error, info, warn};

use crate::db;
use crate::shutdown::ShutdownSignal;

/// OpenAI-compatible chat completions endpoint path.
const LLM_PROXY_ENDPOINT: &str = "/v1/chat/completions";

/// Required API version header for Anthropic requests (used when auth_type = "x-api-key").
const ANTHROPIC_API_VERSION: &str = "2023-06-01";

/// Header the proxy token may be presented on, besides `Authorization: Bearer`.
const PROXY_TOKEN_HEADER: &str = "X-Proxy-Token";

/// Minimum gap between two "caller sent no valid proxy token" warnings. Without
/// it a single unauthenticated connector floods the log at request rate.
const TOKEN_WARN_INTERVAL: Duration = Duration::from_secs(60);

/// When the last untrusted-caller warning was emitted (process-wide).
static LAST_TOKEN_WARN: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Where this installation's record of proxy callers lives.
///
/// `plugin_data` rather than a column of its own: this mechanism decides a
/// DEFAULT, and a default-chooser is the kind of thing that gets retired once
/// every shipped connector sends the token. A schema outlives the mechanism
/// that asked for it; a key in an existing store does not. Plugins reach this
/// table only through `ScopedDataStore`, which binds them to their own id, so
/// nothing outside the kernel writes under this one.
const KERNEL_STORE_ID: &str = "cloto.kernel";
const TOKEN_EVIDENCE_KEY: &str = "llm_proxy.token_evidence";

/// What this installation has learned about its own callers.
///
/// Two facts, not a count: the decision below only asks whether each has ever
/// happened, and a counter would make every request a write.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TokenEvidence {
    /// The proxy has served at least one request.
    #[serde(default)]
    pub served: bool,
    /// At least one of them carried no valid token.
    #[serde(default)]
    pub untrusted: bool,
}

/// Whether enforcement should default to ON for an installation with this record.
///
/// **Positive evidence, not absent negative evidence.** "Never saw an untrusted
/// call" is true of every installation that has not served one yet — including
/// one whose connectors all predate the token and simply have not been used.
/// Reading that as "safe to require the token" would break exactly the people
/// this staging exists to protect, and it would do it on their first restart,
/// which is why a boot-scoped window cannot answer this question either. So the
/// answer requires the proxy to have served something AND all of it to have
/// been authenticated.
///
/// An installation that never uses the proxy therefore never auto-enables. That
/// is the intended outcome: there is nothing to protect there, and nothing that
/// would prove it safe.
#[must_use]
pub fn enforcement_default(evidence: TokenEvidence) -> bool {
    evidence.served && !evidence.untrusted
}

/// Read this installation's record. A missing or unreadable row is "no
/// evidence", which `enforcement_default` treats as "do not enable" — the
/// failure direction that cannot break a caller.
pub async fn load_token_evidence(pool: &SqlitePool) -> TokenEvidence {
    use cloto_shared::PluginDataStore as _;
    let store = crate::db::SqliteDataStore::new(pool.clone());
    match store.get_json(KERNEL_STORE_ID, TOKEN_EVIDENCE_KEY).await {
        Ok(Some(v)) => serde_json::from_value(v).unwrap_or_default(),
        _ => TokenEvidence::default(),
    }
}

/// Forget what the proxy has seen, because the connector set changed.
///
/// Called when a connector is re-vendored: the record describes callers that no
/// longer exist on disk, and carrying it forward would either hold enforcement
/// off for a fault that was just fixed, or turn it on for a connector nobody has
/// heard from yet. The installation earns the answer again from the new set.
pub async fn clear_token_evidence(pool: &SqlitePool) {
    PERSISTED_SERVED.store(false, std::sync::atomic::Ordering::Relaxed);
    PERSISTED_UNTRUSTED.store(false, std::sync::atomic::Ordering::Relaxed);
    store_token_evidence(pool, TokenEvidence::default()).await;
}

/// Write the record. Failures are logged and swallowed: a proxy request must
/// not fail because bookkeeping did, and the cost of a lost write is that the
/// installation has to earn the answer again.
async fn store_token_evidence(pool: &SqlitePool, evidence: TokenEvidence) {
    use cloto_shared::PluginDataStore as _;
    let store = crate::db::SqliteDataStore::new(pool.clone());
    if let Err(e) = store
        .set_json(
            KERNEL_STORE_ID,
            TOKEN_EVIDENCE_KEY,
            serde_json::to_value(evidence).unwrap_or(serde_json::Value::Null),
        )
        .await
    {
        tracing::debug!("could not write the LLM proxy token evidence: {e}");
    }
}

/// What the record becomes after serving one request. Split out so the rule is
/// testable without the process-wide flags below, which every test in this
/// binary shares.
#[must_use]
fn evidence_after(prev: TokenEvidence, untrusted: bool) -> TokenEvidence {
    TokenEvidence {
        served: true,
        untrusted: prev.untrusted || untrusted,
    }
}

/// Facts already on disk, so the steady state costs an atomic load rather than
/// a write per request. Both are conservative when wrong: a lost write means
/// the installation has to earn the answer again, never that it claims one it
/// did not earn.
static PERSISTED_SERVED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PERSISTED_UNTRUSTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Record one served request. Writes only when it says something new.
async fn record_evidence(pool: &SqlitePool, untrusted: bool) {
    use std::sync::atomic::Ordering::Relaxed;
    let new_served = !PERSISTED_SERVED.swap(true, Relaxed);
    let new_untrusted = untrusted && !PERSISTED_UNTRUSTED.swap(true, Relaxed);
    if !new_served && !new_untrusted {
        return;
    }
    let evidence = evidence_after(load_token_evidence(pool).await, untrusted);
    store_token_evidence(pool, evidence).await;
}

/// Largest number of distinct connectors named in [`UntrustedCallers::connectors`].
///
/// The set is bounded because it is reported to the operator and held for the
/// life of the process. Every id in it is a row key the proxy has already
/// resolved in the database, so the bound is not there to contain untrusted
/// input — it is there so a kernel serving many engines still answers with a
/// list someone can read and act on. `served` remains exact.
const MAX_NAMED_CALLERS: usize = 16;

/// A request the proxy served without a valid token, because enforcement was off.
#[derive(Debug, Clone)]
pub struct UntrustedCallers {
    /// How many such requests this kernel has served since it started.
    pub served: u64,
    /// When the most recent one arrived.
    pub last_seen: std::time::SystemTime,
    /// Which connectors were behind them, as far as the proxy could tell.
    ///
    /// Named from the resolved provider row, whose key is the connector's
    /// marketplace id, and only AFTER that row was found — so an id here is
    /// one this kernel knows, not a string a caller chose. A request that
    /// never got that far (no provider header, or an unknown provider) is
    /// still counted in `served` and simply not named, which is why this set
    /// is a lower bound on `served` and never a substitute for it.
    pub connectors: std::collections::BTreeSet<String>,
}

/// Requests served without a valid proxy token, or `None` when there were none.
///
/// The proxy already decides this on every request; keeping the answer turns a
/// rate-limited log line into state that can be read back. Making the token
/// mandatory can then be decided on what this kernel has actually seen, rather
/// than on elapsed time — a connector that never sends the header does not
/// start sending it because a deadline passed.
static UNTRUSTED_CALLERS: std::sync::Mutex<Option<UntrustedCallers>> = std::sync::Mutex::new(None);

/// What this kernel has served without a valid token since it started.
///
/// `None` means every proxy request carried a valid token — or that none
/// arrived at all. The two are deliberately not distinguished: both say
/// "nothing seen here that requiring the token would break".
#[must_use]
pub fn untrusted_callers() -> Option<UntrustedCallers> {
    UNTRUSTED_CALLERS
        .lock()
        .ok()
        .and_then(|seen| seen.as_ref().cloned())
}

/// Record one request served without a valid token.
///
/// Unlike the warning, this is not rate-limited: the count is the point.
fn record_untrusted_caller() {
    let Ok(mut seen) = UNTRUSTED_CALLERS.lock() else {
        return;
    };
    let now = std::time::SystemTime::now();
    *seen = Some(match seen.take() {
        Some(prev) => UntrustedCallers {
            served: prev.served.saturating_add(1),
            last_seen: now,
            connectors: prev.connectors,
        },
        None => UntrustedCallers {
            served: 1,
            last_seen: now,
            connectors: std::collections::BTreeSet::new(),
        },
    });
}

/// Name the connector behind a request that was served without a valid token.
///
/// Called only after the provider row resolved, and only for a request the
/// count above already recorded — so this never creates an entry on its own. A
/// request that is counted but never named is the honest outcome for one that
/// failed before the proxy knew who was calling.
fn record_untrusted_provider(provider_id: &str) {
    let Ok(mut seen) = UNTRUSTED_CALLERS.lock() else {
        return;
    };
    let Some(entry) = seen.as_mut() else {
        return;
    };
    if entry.connectors.len() >= MAX_NAMED_CALLERS && !entry.connectors.contains(provider_id) {
        return;
    }
    entry.connectors.insert(provider_id.to_string());
}

/// Drop one connector from the "needs updating" list, because it was replaced.
///
/// The set is a worklist, and a worklist that only grows is not one: after the
/// operator updates a connector, leaving it listed asks them to do it again.
/// Losing the entry costs nothing that matters — if the replacement still calls
/// without a token, its next call puts it straight back, which is a truer
/// answer than the one we would have kept. `served` is deliberately untouched:
/// it records what this kernel has served, and that does not stop being true.
pub fn forget_untrusted_provider(provider_id: &str) {
    if let Ok(mut seen) = UNTRUSTED_CALLERS.lock() {
        if let Some(entry) = seen.as_mut() {
            entry.connectors.remove(provider_id);
        }
    }
}

struct ProxyState {
    pool: SqlitePool,
    http_client: reqwest::Client,
    /// Expected proxy token (see module doc). Empty means no caller can match,
    /// which is the fail-closed direction.
    token: String,
    /// Whether a token mismatch is fatal (401) or merely logged.
    require_token: bool,
}

/// Spawn the internal LLM proxy on `127.0.0.1:{port}`.
///
/// Mind MCP servers send requests to this proxy with an `X-LLM-Provider` header
/// indicating which provider to route to. The proxy looks up the API key from
/// the database and forwards the request with proper authentication.
///
/// `token` is the per-boot secret callers must present (module doc §Proxy
/// token); `require_token` decides whether a mismatch is a 401 or a warning.
///
/// Returns a oneshot receiver that resolves to `Ok(())` when the proxy binds
/// successfully, or `Err(message)` on failure.
pub fn spawn_llm_proxy(
    pool: SqlitePool,
    port: u16,
    token: String,
    require_token: bool,
    timeout_secs: u64,
    shutdown: ShutdownSignal,
) -> tokio::sync::oneshot::Receiver<Result<(), String>> {
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();

    let http_client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            let msg = format!("Failed to create LLM proxy HTTP client: {}", e);
            error!("{}", msg);
            let _ = ready_tx.send(Err(msg));
            return ready_rx;
        }
    };
    let state = Arc::new(ProxyState {
        pool,
        http_client,
        token,
        require_token,
    });

    let app = Router::new()
        .route(LLM_PROXY_ENDPOINT, post(proxy_handler))
        .with_state(state);

    tokio::spawn(async move {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));

        let listener = match bind_llm_proxy(addr).await {
            Ok(l) => l,
            Err(e) => {
                let msg = format!("Failed to bind LLM proxy on port {}: {}", port, e);
                error!("{}", msg);
                let _ = ready_tx.send(Err(msg));
                return;
            }
        };
        info!("LLM Proxy listening on http://{}", addr);
        let _ = ready_tx.send(Ok(()));

        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown.raised().await;
                info!("LLM Proxy shutting down");
            })
            .await
            .ok();
    });

    ready_rx
}

/// Bind with retry to handle port conflicts during `tauri dev` restarts.
async fn bind_llm_proxy(addr: SocketAddr) -> std::io::Result<tokio::net::TcpListener> {
    const MAX_RETRIES: u32 = 5;
    const DELAY: Duration = Duration::from_secs(2);
    for attempt in 0..=MAX_RETRIES {
        let socket = tokio::net::TcpSocket::new_v4()?;
        socket.set_reuseaddr(true)?;
        match socket.bind(addr) {
            Ok(()) => match socket.listen(1024) {
                Ok(listener) => return Ok(listener),
                Err(e) if attempt < MAX_RETRIES => {
                    tracing::warn!(
                        "LLM proxy port {} listen failed (attempt {}/{}): {}",
                        addr.port(),
                        attempt + 1,
                        MAX_RETRIES,
                        e
                    );
                    tokio::time::sleep(DELAY).await;
                }
                Err(e) => return Err(e),
            },
            Err(e) if attempt < MAX_RETRIES => {
                tracing::warn!(
                    "LLM proxy port {} bind failed (attempt {}/{}): {}",
                    addr.port(),
                    attempt + 1,
                    MAX_RETRIES,
                    e
                );
                tokio::time::sleep(DELAY).await;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

/// Build a JSON error response with a uniform envelope.
fn json_error(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

/// Extract the proxy token the caller presented, if any.
///
/// `Authorization: Bearer <t>` wins over `X-Proxy-Token: <t>` so a caller that
/// sets both cannot get a second guess. The `Bearer` scheme is matched
/// case-insensitively (RFC 7235 §2.1 auth-scheme is case-insensitive).
fn token_presented(headers: &HeaderMap) -> Option<&str> {
    if let Some(value) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        const BEARER: &str = "bearer ";
        if value
            .get(..BEARER.len())
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case(BEARER))
        {
            return Some(value[BEARER.len()..].trim());
        }
    }
    headers
        .get(PROXY_TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
}

/// Constant-time comparison of the presented token against the expected one.
///
/// An absent token is a mismatch, never a pass — the caller decides what a
/// mismatch costs, but it is never silently treated as authenticated here.
fn token_matches(presented: Option<&str>, expected: &str) -> bool {
    use subtle::ConstantTimeEq;
    match presented {
        // `ct_eq` on slices of different lengths returns 0 without leaking
        // which byte diverged.
        Some(p) => p.as_bytes().ct_eq(expected.as_bytes()).into(),
        None => false,
    }
}

/// Log an unauthenticated caller at most once per [`TOKEN_WARN_INTERVAL`].
fn warn_untrusted_caller(had_token: bool) {
    let Ok(mut last) = LAST_TOKEN_WARN.lock() else {
        return;
    };
    let now = std::time::Instant::now();
    let due = match *last {
        Some(previous) => now.duration_since(previous) >= TOKEN_WARN_INTERVAL,
        None => true,
    };
    if !due {
        return;
    }
    *last = Some(now);
    drop(last);
    warn!(
        had_token,
        "LLM proxy request carried {} proxy token — served anyway because \
         CLOTO_LLM_PROXY_REQUIRE_TOKEN is off. Callers must send \
         `Authorization: Bearer $CLOTO_LLM_PROXY_TOKEN` (or X-Proxy-Token); \
         enforcement will become the default and unauthenticated calls will \
         then be rejected with 401.",
        if had_token { "an invalid" } else { "no" }
    );
}

#[allow(clippy::too_many_lines)]
async fn proxy_handler(
    State(state): State<Arc<ProxyState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    // Authenticate BEFORE the provider lookup: everything past this point can
    // spend stored provider credit, and the DB read itself is work done on
    // behalf of a caller we have not identified yet.
    let presented = token_presented(&headers);
    // Whether this request is one the operator would have to fix before the
    // token can be required. Kept so the connector behind it can be named once
    // the provider row resolves, a few lines down: naming it from the header
    // here would report a string the caller chose.
    let untrusted = !token_matches(presented, &state.token);
    if untrusted {
        if state.require_token {
            return json_error(
                StatusCode::UNAUTHORIZED,
                serde_json::json!({
                    "error": {
                        "message": "Missing or invalid proxy token. Send `Authorization: Bearer $CLOTO_LLM_PROXY_TOKEN` or `X-Proxy-Token`.",
                        "code": "proxy_unauthorized"
                    }
                }),
            );
        }
        record_untrusted_caller();
        warn_untrusted_caller(presented.is_some());
    }
    // Persisted, unlike the in-memory count: the question this answers is
    // "has this INSTALLATION ever served an unauthenticated caller", and an
    // answer that resets every restart would say "no" to every kernel on its
    // first request of the day.
    record_evidence(&state.pool, untrusted).await;

    // Determine provider from header or body
    let provider_id = headers
        .get("X-LLM-Provider")
        .and_then(|v| v.to_str().ok())
        .map(std::string::ToString::to_string)
        .or_else(|| {
            body.get("provider")
                .and_then(|v| v.as_str())
                .map(String::from)
        });

    let Some(provider_id) = provider_id else {
        return json_error(
            StatusCode::BAD_REQUEST,
            serde_json::json!({
                "error": { "message": "Missing X-LLM-Provider header or 'provider' field" }
            }),
        );
    };

    // Look up provider config
    let provider = match db::get_llm_provider(&state.pool, &provider_id).await {
        Ok(p) => p,
        Err(e) => {
            return json_error(
                StatusCode::NOT_FOUND,
                serde_json::json!({
                    "error": { "message": format!("Provider '{}' not found: {}", provider_id, e) }
                }),
            );
        }
    };

    if untrusted {
        record_untrusted_provider(&provider_id);
    }

    if !provider.enabled {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({
                "error": { "message": format!("Provider '{}' is disabled", provider_id) }
            }),
        );
    }

    // Strip the 'provider' field from body before forwarding.
    // Also override `model` with the DB-configured `provider.model_id` — the
    // DB is the authority for model selection (ADR 2026-04-13). Empty
    // model_id means "not configured yet"; let the original body.model
    // pass through so the upstream provider returns a meaningful error.
    let mut forward_body = body.clone();
    if let Some(obj) = forward_body.as_object_mut() {
        obj.remove("provider");
        if !provider.model_id.is_empty() {
            obj.insert(
                "model".to_string(),
                serde_json::Value::String(provider.model_id.clone()),
            );
        }
    }

    // Phase C: when the MCP server requested `stream: true`, pass the SSE
    // body through untouched instead of buffering + JSON-parsing it. Both the
    // flag check and the passthrough are pure transport — no reasoning about
    // provider shape — so it works across OpenAI-compatible, Anthropic, and
    // llama.cpp upstreams uniformly.
    let streaming_requested = forward_body
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // Build the forwarded request.
    //
    // For non-streaming requests, `provider.timeout_secs` caps the whole
    // call. For streaming requests we deliberately omit the reqwest
    // `.timeout(...)` because it applies to the entire response (including
    // body reading): a 120 s cap would otherwise truncate long LLM
    // generations mid-flight and the server-side handler would return the
    // partial text as if it were complete.
    //
    // The safety nets for streaming are:
    //   * mcp_client.rs::call_tool_streaming — per-request total cap and
    //     per-chunk idle cap (Phase B, bug-351)
    //   * The upstream's own timeout (LM Studio / OpenAI / Anthropic all
    //     enforce server-side generation limits)
    //   * call_llm_api_streaming — raises on upstream closing without the
    //     [DONE] sentinel so truncation is surfaced to the agent
    let mut req = state
        .http_client
        .post(&provider.api_url)
        .header("Content-Type", "application/json");
    if !streaming_requested {
        req = req.timeout(Duration::from_secs(provider.timeout_secs as u64));
    }

    // Add API key if configured (auth_type driven — no hard-coded provider IDs)
    if !provider.api_key.is_empty() {
        if provider.auth_type == "x-api-key" {
            req = req.header("x-api-key", &provider.api_key);
            req = req.header("anthropic-version", ANTHROPIC_API_VERSION);
        } else {
            // Default: Bearer token (OpenAI-compatible)
            req = req.header("Authorization", format!("Bearer {}", provider.api_key));
        }
    }

    debug!(
        provider = %provider_id,
        url = %provider.api_url,
        streaming = %streaming_requested,
        "Proxying LLM request"
    );

    // Forward the request
    match req.json(&forward_body).send().await {
        Ok(response) => {
            let status = response.status();

            if streaming_requested && status.is_success() {
                // Streaming pass-through: lift the upstream byte stream into
                // an Axum response body with the original content-type header.
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("text/event-stream")
                    .to_string();
                let stream = response.bytes_stream();
                return match Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", content_type)
                    .header("cache-control", "no-cache")
                    .body(Body::from_stream(stream))
                {
                    Ok(resp) => resp,
                    Err(e) => {
                        error!(provider = %provider_id, error = %e, "Failed to build streaming response");
                        json_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            serde_json::json!({
                                "error": { "message": "Failed to build streaming response", "code": "internal" }
                            }),
                        )
                    }
                };
            }

            match response.json::<Value>().await {
                Ok(resp_body) => {
                    if status.is_success() {
                        json_error(StatusCode::OK, resp_body)
                    } else {
                        // bug-464: this path faces MCP subprocesses, whose whole
                        // point is never seeing the key — scrub any echoed key
                        // material out of the logged body and the returned
                        // detail before it leaves the proxy.
                        let redact =
                            |s: &str| crate::handlers::llm::redact_secrets(s, &provider.api_key);
                        warn!(
                            provider = %provider_id,
                            status = %status,
                            body = %redact(&resp_body.to_string()),
                            "LLM provider returned error"
                        );
                        // Translate HTTP status into user-friendly error with code
                        let (msg, code) = match status.as_u16() {
                            401 | 403 => (
                                format!(
                                    "API key authentication failed for provider '{}'",
                                    provider_id
                                ),
                                "auth_failed",
                            ),
                            429 => (
                                format!("Rate limit exceeded for provider '{}'", provider_id),
                                "rate_limited",
                            ),
                            500..=599 => (
                                format!(
                                    "Provider '{}' returned a server error ({})",
                                    provider_id,
                                    status.as_u16()
                                ),
                                "provider_error",
                            ),
                            _ => (
                                format!(
                                    "Provider '{}' returned an error ({})",
                                    provider_id,
                                    status.as_u16()
                                ),
                                "unknown",
                            ),
                        };
                        // Include upstream error detail so MCP servers can surface it.
                        // Providers use several shapes — try in order of specificity:
                        //   OpenAI-style:   {error: {message: "..."}}
                        //   LM Studio:      {error: "..."}          (plain string)
                        //   Anthropic:      {error: {type, message}} (same as OpenAI)
                        //   Fallback:       {message: "..."}
                        let upstream_detail = resp_body
                            .get("error")
                            .and_then(|e| e.get("message").and_then(|m| m.as_str()))
                            .or_else(|| resp_body.get("error").and_then(|e| e.as_str()))
                            .or_else(|| resp_body.get("message").and_then(|m| m.as_str()))
                            .unwrap_or("");
                        let full_msg = if upstream_detail.is_empty() {
                            msg
                        } else {
                            format!("{}: {}", msg, redact(upstream_detail))
                        };
                        json_error(
                            StatusCode::from_u16(status.as_u16())
                                .unwrap_or(StatusCode::BAD_GATEWAY),
                            serde_json::json!({
                                "error": { "message": full_msg, "code": code }
                            }),
                        )
                    }
                }
                Err(e) => {
                    error!(provider = %provider_id, error = %e, "Failed to parse provider response");
                    json_error(
                        StatusCode::BAD_GATEWAY,
                        serde_json::json!({
                            "error": { "message": format!("Failed to parse provider response: {}", e) }
                        }),
                    )
                }
            }
        }
        Err(e) => {
            error!(provider = %provider_id, error = %e, "Failed to reach LLM provider");
            json_error(
                StatusCode::BAD_GATEWAY,
                serde_json::json!({
                    "error": {
                        "message": format!("Cannot connect to provider '{}'. Ensure the service is running.", provider_id),
                        "code": "connection_failed"
                    }
                }),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with(name: &'static str, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(name, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn token_presented_reads_bearer_authorization() {
        let headers = headers_with("authorization", "Bearer abc123");
        assert_eq!(token_presented(&headers), Some("abc123"));
        // The auth-scheme is case-insensitive (RFC 7235 §2.1).
        let headers = headers_with("authorization", "bearer abc123");
        assert_eq!(token_presented(&headers), Some("abc123"));
    }

    #[test]
    fn token_presented_reads_x_proxy_token() {
        let headers = headers_with(PROXY_TOKEN_HEADER, "abc123");
        assert_eq!(token_presented(&headers), Some("abc123"));
    }

    #[test]
    fn token_presented_is_none_when_absent() {
        assert_eq!(token_presented(&HeaderMap::new()), None);
        // A non-Bearer Authorization scheme is not a proxy token either.
        let headers = headers_with("authorization", "Basic dXNlcjpwdw==");
        assert_eq!(token_presented(&headers), None);
    }

    #[test]
    fn token_matches_only_on_exact_token() {
        assert!(token_matches(Some("abc123"), "abc123"));
        assert!(!token_matches(Some("abc124"), "abc123"));
        // A prefix must not pass: ct_eq is length-aware.
        assert!(!token_matches(Some("abc"), "abc123"));
        assert!(!token_matches(None, "abc123"));
        // Fail-closed when no token was configured at all.
        assert!(!token_matches(None, ""));
    }

    /// Reserve a port, then release it so the proxy can bind it.
    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// Start a proxy on a free port and wait for the bind to succeed.
    async fn spawn_test_proxy(token: &str, require_token: bool) -> (u16, ShutdownSignal) {
        let (port, shutdown, _pool) = spawn_test_proxy_with_pool(token, require_token).await;
        (port, shutdown)
    }

    /// The same, handing back the pool so a test can give the proxy a provider
    /// row to resolve — which is the only way to reach the code that names a
    /// caller, since naming happens after the lookup and never from the header.
    async fn spawn_test_proxy_with_pool(
        token: &str,
        require_token: bool,
    ) -> (u16, ShutdownSignal, SqlitePool) {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", None)
            .await
            .unwrap();
        let handed_back = pool.clone();
        let port = free_port();
        let shutdown = ShutdownSignal::new();
        let ready = spawn_llm_proxy(
            pool,
            port,
            token.to_string(),
            require_token,
            5,
            shutdown.clone(),
        );
        ready
            .await
            .expect("proxy task dropped the ready channel")
            .expect("proxy failed to bind");
        (port, shutdown, handed_back)
    }

    /// POST naming a provider, optionally presenting `header: token`.
    async fn post_to_provider(
        port: u16,
        provider: &str,
        auth: Option<(&str, &str)>,
    ) -> reqwest::Response {
        let url = format!("http://127.0.0.1:{port}{LLM_PROXY_ENDPOINT}");
        let mut req = reqwest::Client::new()
            .post(url)
            .header("X-LLM-Provider", provider)
            .json(&serde_json::json!({}));
        if let Some((name, value)) = auth {
            req = req.header(name, value);
        }
        req.send().await.expect("request to local proxy failed")
    }

    /// Register a provider that resolves and then fails to connect upstream.
    /// The forward is irrelevant here — everything under test happens before it.
    async fn register_dead_provider(pool: &SqlitePool, id: &str) {
        crate::db::upsert_llm_provider_meta(
            pool,
            id,
            id,
            "http://127.0.0.1:1/v1/chat/completions",
            "bearer",
            "model-under-test",
            1,
            None,
        )
        .await
        .expect("provider row must be inserted");
    }

    /// POST an empty body, optionally presenting `header: token`.
    async fn post_empty(port: u16, auth: Option<(&str, &str)>) -> reqwest::Response {
        let url = format!("http://127.0.0.1:{port}{LLM_PROXY_ENDPOINT}");
        let mut req = reqwest::Client::new()
            .post(url)
            .json(&serde_json::json!({}));
        if let Some((name, value)) = auth {
            req = req.header(name, value);
        }
        req.send().await.expect("request to local proxy failed")
    }

    /// With enforcement on, an unauthenticated call is rejected before the
    /// provider lookup, and an authenticated one gets through to it.
    #[tokio::test]
    async fn require_token_rejects_unauthenticated_callers() {
        const TOKEN: &str = "proxy-token-under-test";
        let (port, shutdown) = spawn_test_proxy(TOKEN, true).await;

        // (a) No token at all → 401 with the standard error envelope.
        let resp = post_empty(port, None).await;
        assert_eq!(resp.status().as_u16(), 401);
        let body: Value = resp.json().await.unwrap();
        assert!(
            body["error"]["message"].is_string(),
            "expected the {{error:{{message}}}} envelope, got {body}"
        );

        // A wrong token is no better than none.
        let resp = post_empty(port, Some(("Authorization", "Bearer wrong-token"))).await;
        assert_eq!(resp.status().as_u16(), 401);

        // (b) Correct token → auth passes, and the request fails later, on the
        // missing provider (400). That 400 is what proves the token check ran
        // *before* the provider lookup and accepted this caller.
        let auth = format!("Bearer {TOKEN}");
        let resp = post_empty(port, Some(("Authorization", &auth))).await;
        assert_eq!(resp.status().as_u16(), 400);

        // The same token on X-Proxy-Token is equally accepted.
        let resp = post_empty(port, Some((PROXY_TOKEN_HEADER, TOKEN))).await;
        assert_eq!(resp.status().as_u16(), 400);

        shutdown.raise();
    }

    /// With enforcement off (the rollout default), the same unauthenticated
    /// call is served: it reaches the provider lookup and fails there (400),
    /// not at the auth check (401).
    #[tokio::test]
    async fn without_require_token_unauthenticated_callers_proceed() {
        let (port, shutdown) = spawn_test_proxy("proxy-token-under-test", false).await;

        let resp = post_empty(port, None).await;
        assert_eq!(resp.status().as_u16(), 400);

        shutdown.raise();
    }

    /// The counter is process-wide and other tests in this binary also serve
    /// unauthenticated calls, so assert the change this call causes rather than
    /// an absolute value — `served` only ever grows.
    #[test]
    fn recording_an_untrusted_caller_increases_the_count() {
        let before = untrusted_callers().map_or(0, |seen| seen.served);
        record_untrusted_caller();
        let after = untrusted_callers().expect("a recorded caller must be visible");
        assert!(
            after.served > before,
            "expected the count to grow past {before}, got {}",
            after.served
        );
    }

    // ── The rule that decides the default (Task: stage 3) ──

    /// The arm the staging exists for: an installation that has heard from a
    /// connector with no token must not start rejecting it.
    #[test]
    fn an_installation_that_saw_an_untrusted_call_does_not_auto_enable() {
        assert!(!enforcement_default(TokenEvidence {
            served: true,
            untrusted: true
        }));
    }

    /// The other arm: everything it has served carried the token, so requiring
    /// it breaks nothing that has spoken.
    #[test]
    fn an_installation_that_only_saw_authenticated_calls_auto_enables() {
        assert!(enforcement_default(TokenEvidence {
            served: true,
            untrusted: false
        }));
    }

    /// The trap this design exists to avoid. "Never saw an untrusted call" is
    /// also true of an installation that has served nothing at all — every
    /// kernel, on every first request after a restart, and every kernel whose
    /// connectors predate the token but have not been used yet. Silence is not
    /// a clean record.
    #[test]
    fn silence_is_not_evidence_of_safety() {
        assert!(!enforcement_default(TokenEvidence::default()));
        assert!(!enforcement_default(TokenEvidence {
            served: false,
            untrusted: false
        }));
    }

    #[test]
    fn one_untrusted_call_is_remembered_and_a_later_clean_one_does_not_erase_it() {
        let after_bad = evidence_after(TokenEvidence::default(), true);
        assert_eq!(
            after_bad,
            TokenEvidence {
                served: true,
                untrusted: true
            }
        );
        // A connector that sends the token does not vouch for the one that does
        // not. Only replacing a connector clears the record.
        let after_good = evidence_after(after_bad, false);
        assert!(
            after_good.untrusted,
            "a clean call must not clear the fault"
        );
        assert!(!enforcement_default(after_good));
    }

    #[tokio::test]
    async fn the_record_survives_a_restart_and_a_reinstall_clears_it() {
        // Persistence is the whole point: an in-memory record answers "no
        // untrusted calls" to every freshly started kernel.
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", None)
            .await
            .unwrap();

        assert_eq!(
            load_token_evidence(&pool).await,
            TokenEvidence::default(),
            "a fresh installation knows nothing"
        );

        store_token_evidence(
            &pool,
            TokenEvidence {
                served: true,
                untrusted: true,
            },
        )
        .await;
        let reloaded = load_token_evidence(&pool).await;
        assert!(
            reloaded.served && reloaded.untrusted,
            "the row must persist"
        );
        assert!(!enforcement_default(reloaded));

        clear_token_evidence(&pool).await;
        assert_eq!(
            load_token_evidence(&pool).await,
            TokenEvidence::default(),
            "re-vendoring a connector must reset what the record describes"
        );
        // And the reset lands on "do not enable", not on "enable" — the
        // installation earns the answer again from the new connector set.
        assert!(!enforcement_default(load_token_evidence(&pool).await));
    }

    /// A worklist that never shrinks stops being one. The count is a different
    /// claim — what this kernel served — and updating a connector does not
    /// unserve those requests, so it must survive.
    #[test]
    fn updating_a_connector_takes_it_off_the_list_without_erasing_the_count() {
        const PROVIDER: &str = "engine-that-was-updated";
        record_untrusted_caller();
        record_untrusted_provider(PROVIDER);
        let before = untrusted_callers().expect("a recorded caller must be visible");
        assert!(
            before.connectors.iter().any(|id| id == PROVIDER),
            "precondition: the connector must be listed before it is forgotten"
        );

        forget_untrusted_provider(PROVIDER);

        let after = untrusted_callers().expect("forgetting a name must not drop the record");
        assert!(
            !after.connectors.iter().any(|id| id == PROVIDER),
            "an updated connector must leave the list: {:?}",
            after.connectors
        );
        // Not equality: the counter is process-wide and other tests in this
        // binary add to it. What must hold is that forgetting a name never
        // takes the count down with it.
        assert!(
            after.served >= before.served,
            "the count of served requests must not shrink when a name is dropped"
        );
    }

    /// The count says something is wrong; the name says what to update. Without
    /// it the operator has to guess which connector to reinstall, and the
    /// marketplace cannot tell them either: it compares version strings, and a
    /// connector whose content moved under the same version still reads as
    /// current there.
    #[tokio::test]
    async fn an_untrusted_call_names_the_connector_behind_it() {
        const TOKEN: &str = "proxy-token-naming";
        const PROVIDER: &str = "engine-that-sent-no-token";
        let (port, shutdown, pool) = spawn_test_proxy_with_pool(TOKEN, false).await;
        register_dead_provider(&pool, PROVIDER).await;

        let _ = post_to_provider(port, PROVIDER, None).await;

        let seen = untrusted_callers().expect("the served call must be visible");
        assert!(
            seen.connectors.iter().any(|id| id == PROVIDER),
            "the connector that called without a token must be named: {:?}",
            seen.connectors
        );
        shutdown.raise();
    }

    /// The set is the list of connectors to update, so a connector that is
    /// already sending the token must never appear in it — being told to update
    /// something that is current is how a report loses its reader.
    #[tokio::test]
    async fn an_authenticated_call_names_nobody() {
        const TOKEN: &str = "proxy-token-authenticated";
        const PROVIDER: &str = "engine-that-sent-its-token";
        let (port, shutdown, pool) = spawn_test_proxy_with_pool(TOKEN, false).await;
        register_dead_provider(&pool, PROVIDER).await;

        let _ = post_to_provider(
            port,
            PROVIDER,
            Some(("Authorization", &format!("Bearer {TOKEN}"))),
        )
        .await;

        let named = untrusted_callers()
            .map(|seen| seen.connectors)
            .unwrap_or_default();
        assert!(
            !named.iter().any(|id| id == PROVIDER),
            "an authenticated caller was listed as needing an update: {named:?}"
        );
        shutdown.raise();
    }

    /// With enforcement off, a call with no token is served *and* observed —
    /// that observation is what later decides whether requiring the token is
    /// safe here, so losing it would be silent.
    #[tokio::test]
    async fn serving_without_a_token_is_observed() {
        let (port, shutdown) = spawn_test_proxy("proxy-token-under-test", false).await;

        let before = untrusted_callers().map_or(0, |seen| seen.served);
        let resp = post_empty(port, None).await;
        assert_eq!(resp.status().as_u16(), 400);
        let after = untrusted_callers().expect("the served call must be visible");
        assert!(
            after.served > before,
            "expected the count to grow past {before}, got {}",
            after.served
        );

        shutdown.raise();
    }
}
