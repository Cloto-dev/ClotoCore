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

struct ProxyState {
    pool: SqlitePool,
    http_client: reqwest::Client,
}

/// Spawn the internal LLM proxy on `127.0.0.1:{port}`.
///
/// Mind MCP servers send requests to this proxy with an `X-LLM-Provider` header
/// indicating which provider to route to. The proxy looks up the API key from
/// the database and forwards the request with proper authentication.
pub fn spawn_llm_proxy(pool: SqlitePool, port: u16, shutdown: Arc<Notify>) {
    let state = Arc::new(ProxyState {
        pool,
        http_client: reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .expect("Failed to create HTTP client"),
    });

    let app = Router::new()
        .route("/v1/chat/completions", post(proxy_handler))
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

async fn proxy_handler(
    State(state): State<Arc<ProxyState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    // Determine provider from header or body
    let provider_id = headers
        .get("X-LLM-Provider")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| body.get("provider").and_then(|v| v.as_str()).map(String::from));

    let provider_id = match provider_id {
        Some(id) => id,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": { "message": "Missing X-LLM-Provider header or 'provider' field" }
                })),
            );
        }
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

    // Add API key if configured
    if !provider.api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", provider.api_key));
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
                            "LLM provider returned error"
                        );
                        (
                            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                            Json(resp_body),
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
                    "error": { "message": format!("Failed to reach provider '{}': {}", provider_id, e) }
                })),
            )
        }
    }
}
