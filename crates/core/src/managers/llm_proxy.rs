//! Internal LLM Proxy — Centralizes API key management (MGP §13.4 llm_completion).
//!
//! Mind MCP servers call this proxy instead of LLM provider APIs directly.
//! The proxy adds the appropriate Authorization header from the `llm_providers` table.
//! This ensures API keys are never exposed to MCP server subprocesses.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
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

/// Provider ID that triggers Anthropic-specific authentication.
const ANTHROPIC_PROVIDER_ID: &str = "claude";

/// Required API version header for Anthropic requests.
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
pub fn spawn_llm_proxy(pool: SqlitePool, port: u16, timeout_secs: u64, shutdown: Arc<Notify>) {
    let http_client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            error!("Failed to create LLM proxy HTTP client: {}", e);
            return;
        }
    };
    let state = Arc::new(ProxyState {
        pool,
        http_client,
    });

    let app = Router::new()
        .route(LLM_PROXY_ENDPOINT, post(proxy_handler))
        .with_state(state);

    tokio::spawn(async move {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        info!("LLM Proxy started on http://{}", addr);

        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                error!("Failed to bind LLM proxy on port {}: {}", port, e);
                return;
            }
        };

        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown.notified().await;
                info!("LLM Proxy shutting down");
            })
            .await
            .ok();
    });
}

#[allow(clippy::too_many_lines)]
async fn proxy_handler(
    State(state): State<Arc<ProxyState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
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
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": { "message": "Missing X-LLM-Provider header or 'provider' field" }
            })),
        );
    };

    // Look up provider config
    let provider = match db::get_llm_provider(&state.pool, &provider_id).await {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": { "message": format!("Provider '{}' not found: {}", provider_id, e) }
                })),
            );
        }
    };

    if !provider.enabled {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": { "message": format!("Provider '{}' is disabled", provider_id) }
            })),
        );
    }

    // Strip the 'provider' field from body before forwarding
    let mut forward_body = body.clone();
    if let Some(obj) = forward_body.as_object_mut() {
        obj.remove("provider");
    }

    // Build the forwarded request
    let mut req = state
        .http_client
        .post(&provider.api_url)
        .header("Content-Type", "application/json")
        .timeout(Duration::from_secs(provider.timeout_secs as u64));

    // Add API key if configured (provider-specific auth format)
    if !provider.api_key.is_empty() {
        if provider_id == ANTHROPIC_PROVIDER_ID {
            // Anthropic uses x-api-key header + required version header
            req = req.header("x-api-key", &provider.api_key);
            req = req.header("anthropic-version", ANTHROPIC_API_VERSION);
        } else {
            req = req.header("Authorization", format!("Bearer {}", provider.api_key));
        }
    }

    debug!(
        provider = %provider_id,
        url = %provider.api_url,
        "Proxying LLM request"
    );

    // Forward the request
    match req.json(&forward_body).send().await {
        Ok(response) => {
            let status = response.status();
            match response.json::<Value>().await {
                Ok(resp_body) => {
                    if status.is_success() {
                        (StatusCode::OK, Json(resp_body))
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
                        // Include upstream error detail so MCP servers can surface it
                        let upstream_detail = resp_body
                            .get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str())
                            .unwrap_or("");
                        let full_msg = if upstream_detail.is_empty() {
                            msg
                        } else {
                            format!("{}: {}", msg, upstream_detail)
                        };
                        (
                            StatusCode::from_u16(status.as_u16())
                                .unwrap_or(StatusCode::BAD_GATEWAY),
                            Json(serde_json::json!({
                                "error": { "message": full_msg, "code": code }
                            })),
                        )
                    }
                }
                Err(e) => {
                    error!(provider = %provider_id, error = %e, "Failed to parse provider response");
                    (
                        StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({
                            "error": { "message": format!("Failed to parse provider response: {}", e) }
                        })),
                    )
                }
            }
        }
        Err(e) => {
            error!(provider = %provider_id, error = %e, "Failed to reach LLM provider");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": {
                        "message": format!("Cannot connect to provider '{}'. Ensure the service is running.", provider_id),
                        "code": "connection_failed"
                    }
                })),
            )
        }
    }
}
