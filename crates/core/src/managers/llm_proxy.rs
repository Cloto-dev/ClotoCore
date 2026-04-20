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
//! - The `127.0.0.1` binding is the security boundary — external access is impossible.
//! - Upstream LLM providers enforce their own rate limits (429 → structured error).
//!
//! See: Code Quality Audit H-4/H-5 (2026-03-22) — closed as By Design.

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
use tokio::sync::Notify;
use tracing::{debug, error, info, warn};

use crate::db;

/// OpenAI-compatible chat completions endpoint path.
const LLM_PROXY_ENDPOINT: &str = "/v1/chat/completions";

/// Required API version header for Anthropic requests (used when auth_type = "x-api-key").
const ANTHROPIC_API_VERSION: &str = "2023-06-01";

struct ProxyState {
    pool: SqlitePool,
    http_client: reqwest::Client,
}

/// Spawn the internal LLM proxy on `127.0.0.1:{port}`.
///
/// Mind MCP servers send requests to this proxy with an `X-LLM-Provider` header
/// indicating which provider to route to. The proxy looks up the API key from
/// the database and forwards the request with proper authentication.
///
/// Returns a oneshot receiver that resolves to `Ok(())` when the proxy binds
/// successfully, or `Err(message)` on failure.
pub fn spawn_llm_proxy(
    pool: SqlitePool,
    port: u16,
    timeout_secs: u64,
    shutdown: Arc<Notify>,
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
    let state = Arc::new(ProxyState { pool, http_client });

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
                shutdown.notified().await;
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

#[allow(clippy::too_many_lines)]
async fn proxy_handler(
    State(state): State<Arc<ProxyState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
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
                        warn!(
                            provider = %provider_id,
                            status = %status,
                            body = %resp_body,
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
                            format!("{}: {}", msg, upstream_detail)
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
