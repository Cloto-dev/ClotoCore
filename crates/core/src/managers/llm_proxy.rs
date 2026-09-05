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
    if !token_matches(presented, &state.token) {
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
        warn_untrusted_caller(presented.is_some());
    }

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
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::init_db(&pool, "sqlite::memory:", None)
            .await
            .unwrap();
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
        (port, shutdown)
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
}
