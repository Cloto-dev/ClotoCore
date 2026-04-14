use axum::{extract::State, Json};
use std::{sync::Arc, time::Duration};

use crate::{AppError, AppResult, AppState};

use super::{check_auth, ok_data, spawn_admin_audit};

/// Maximum allowed length for `model_id` (characters after trimming).
const MODEL_ID_MAX_LEN: usize = 200;

/// HTTP timeout for calls from the admin API to upstream LLM providers' model-list endpoints.
const MODELS_FETCH_TIMEOUT_SECS: u64 = 15;

/// GET /api/llm/providers
pub async fn list_llm_providers(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let providers = crate::db::list_llm_providers(&state.pool)
        .await
        .map_err(AppError::Internal)?;
    // Mask API keys in response
    let masked: Vec<serde_json::Value> = providers
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "display_name": p.display_name,
                "api_url": p.api_url,
                "has_key": !p.api_key.is_empty(),
                "model_id": p.model_id,
                "timeout_secs": p.timeout_secs,
                "enabled": p.enabled,
                "context_length": p.context_length,
            })
        })
        .collect();
    ok_data(serde_json::json!({ "providers": masked }))
}

/// POST /api/llm/providers/:id/key
pub async fn set_llm_provider_key(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let api_key = payload["api_key"]
        .as_str()
        .ok_or_else(|| AppError::Validation("api_key is required".into()))?;
    crate::db::set_llm_provider_key(&state.pool, &provider_id, api_key)
        .await
        .map_err(|e| AppError::Validation(e.to_string()))?;
    tracing::info!(provider = %provider_id, "LLM provider API key updated");
    ok_data(serde_json::json!({}))
}

/// POST /api/llm/providers/:id/model
///
/// Updates the `model_id` for a provider, recording the change in
/// `llm_provider_model_history`. For `mind.ollama`, also relays the change to
/// the running MCP server's `switch_model` tool so the active model updates
/// without a kernel restart.
pub async fn set_llm_provider_model(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let raw = payload["model_id"]
        .as_str()
        .ok_or_else(|| AppError::Validation("model_id is required".into()))?;
    let model_id = raw.trim();

    if model_id.is_empty() {
        return Err(AppError::Validation("model_id must not be empty".into()));
    }
    if model_id.len() > MODEL_ID_MAX_LEN {
        return Err(AppError::Validation(format!(
            "model_id exceeds max length {}",
            MODEL_ID_MAX_LEN
        )));
    }
    if model_id.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "model_id must not contain control characters or newlines".into(),
        ));
    }

    let old_model = crate::db::set_llm_provider_model(&state.pool, &provider_id, model_id)
        .await
        .map_err(|e| AppError::Validation(e.to_string()))?;

    // mind.ollama reads OLLAMA_MODEL only at startup, so relay the change to
    // its switch_model tool. Failure here is non-fatal (DB is already updated;
    // post-connect sync will catch up on the next (re)start).
    if provider_id == "ollama" {
        let mcp_mgr = state.mcp_manager.clone();
        let model_owned = model_id.to_string();
        tokio::spawn(async move {
            match mcp_mgr
                .call_server_tool(
                    "mind.ollama",
                    "switch_model",
                    serde_json::json!({ "model": model_owned }),
                )
                .await
            {
                Ok(_) => tracing::info!(model = %model_owned, "mind.ollama switch_model relayed"),
                Err(e) => tracing::warn!(
                    error = %e,
                    "mind.ollama switch_model relay failed (DB updated; next connect will resync)"
                ),
            }
        });
    }

    spawn_admin_audit(
        state.pool.clone(),
        "LLM_PROVIDER_MODEL_UPDATED",
        provider_id.clone(),
        format!("Model changed from '{}' to '{}'", old_model, model_id),
        None,
        Some(serde_json::json!({ "old_model_id": old_model, "new_model_id": model_id })),
        None,
    );

    tracing::info!(provider = %provider_id, model = %model_id, "LLM provider model updated");
    ok_data(serde_json::json!({}))
}

/// Derive the model-list URL for a given provider.
///
/// Providers expose their model catalog at different paths:
///   - OpenAI-compatible (DeepSeek, Cerebras, Groq, LM Studio/local): strip the trailing
///     `/chat/completions` segment from the configured chat URL and append `/models`.
///   - Anthropic (Claude): `/v1/messages` → `/v1/models` (same parent).
///   - Ollama (native): `/api/chat` → `/api/tags`.
///
/// Rejects non-http(s) schemes so this function cannot be tricked into issuing
/// file:// or other unexpected requests from an admin-set DB value.
fn derive_models_url(api_url: &str, provider_id: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(api_url).map_err(|e| format!("invalid api_url: {}", e))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("unsupported api_url scheme: {}", url.scheme()));
    }
    let mut out = url.clone();
    out.set_query(None);
    out.set_fragment(None);

    // Ollama uses a native (non-OpenAI-compat) catalog endpoint.
    if provider_id == "ollama" {
        out.set_path("/api/tags");
        return Ok(out.to_string());
    }

    // OpenAI-compat and Anthropic: derive the parent path, strip any trailing
    // `/chat` segment (the leaf is `completions` or `messages`), then append `/models`.
    let path = url.path();
    let parent = path.rfind('/').map_or("", |i| &path[..i]);
    let stripped = parent.strip_suffix("/chat").unwrap_or(parent);
    // If the URL has no parent path (e.g. DeepSeek's `/chat/completions`), produce `/models`
    // rather than forcing a `/v1/` prefix the admin didn't configure.
    let new_path = if stripped.is_empty() {
        "/models".to_string()
    } else {
        format!("{}/models", stripped)
    };
    out.set_path(&new_path);
    Ok(out.to_string())
}

/// Static fallback model list for Claude when `/v1/models` is unavailable
/// (e.g., no API key configured or Anthropic auth failure). Keeps the dashboard
/// dropdown usable offline. Update when Anthropic releases a new family.
fn claude_static_models() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({ "id": "claude-sonnet-4-6", "name": "Claude Sonnet 4.6" }),
        serde_json::json!({ "id": "claude-opus-4-6", "name": "Claude Opus 4.6" }),
        serde_json::json!({ "id": "claude-haiku-4-5-20251001", "name": "Claude Haiku 4.5" }),
    ]
}

/// GET /api/llm/providers/:id/models
///
/// Fetches the provider's model catalog for the Dashboard dropdown.
/// Always returns `{models: [...], error_code?: string, error?: string}` with HTTP 200
/// so the frontend can gracefully fall back to manual entry on upstream failures
/// (no-key, unreachable, auth_failed). Never surfaces the provider's API key in errors.
#[allow(clippy::too_many_lines)]
pub async fn list_provider_models(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let provider = crate::db::get_llm_provider(&state.pool, &provider_id)
        .await
        .map_err(|_| AppError::NotFound(format!("LLM provider '{}' not found", provider_id)))?;

    let models_url =
        derive_models_url(&provider.api_url, &provider.id).map_err(AppError::Validation)?;

    // SaaS providers that require an API key will reject the call; surface a static
    // fallback for Claude (curated list) and a clean error code otherwise.
    let needs_key = provider.id != "ollama" && provider.id != "local";
    if needs_key && provider.api_key.is_empty() {
        if provider.id == "claude" {
            return ok_data(serde_json::json!({
                "models": claude_static_models(),
                "error_code": "static_fallback",
            }));
        }
        return ok_data(serde_json::json!({
            "models": [],
            "error_code": "no_api_key",
        }));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(MODELS_FETCH_TIMEOUT_SECS))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    let mut req = client.get(&models_url);
    if !provider.api_key.is_empty() {
        if provider.auth_type == "x-api-key" {
            req = req
                .header("x-api-key", &provider.api_key)
                .header("anthropic-version", "2023-06-01");
        } else {
            req = req.header("Authorization", format!("Bearer {}", provider.api_key));
        }
    }

    let response = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                provider = %provider_id,
                "list_provider_models: upstream connection failed: {}",
                e
            );
            if provider.id == "claude" {
                return ok_data(serde_json::json!({
                    "models": claude_static_models(),
                    "error_code": "static_fallback",
                }));
            }
            return ok_data(serde_json::json!({
                "models": [],
                "error_code": "unreachable",
            }));
        }
    };

    let status = response.status();
    if !status.is_success() {
        let code = match status.as_u16() {
            401 | 403 => "auth_failed",
            404 => "model_list_unavailable",
            _ => "provider_error",
        };
        tracing::warn!(
            provider = %provider_id,
            status = %status,
            "list_provider_models: upstream returned non-success"
        );
        if provider.id == "claude" && matches!(status.as_u16(), 401 | 403 | 404) {
            return ok_data(serde_json::json!({
                "models": claude_static_models(),
                "error_code": "static_fallback",
            }));
        }
        return ok_data(serde_json::json!({
            "models": [],
            "error_code": code,
        }));
    }

    let body: serde_json::Value = match response.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                provider = %provider_id,
                "list_provider_models: upstream returned unparseable body: {}",
                e
            );
            return ok_data(serde_json::json!({
                "models": [],
                "error_code": "parse_error",
            }));
        }
    };

    let mut models: Vec<serde_json::Value> = if provider.id == "ollama" {
        // Ollama: {"models":[{"name":"qwen3.5:9b", ...}]}
        body.get("models")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| {
                        m.get("name")
                            .and_then(|n| n.as_str())
                            .map(|name| serde_json::json!({ "id": name }))
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        // OpenAI-compat: {"data":[{"id": "...", "display_name"?: "..."}]}
        body.get("data")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| {
                        let id = m.get("id").and_then(|v| v.as_str())?;
                        let display_name = m.get("display_name").and_then(|v| v.as_str());
                        Some(match display_name {
                            Some(dn) => serde_json::json!({ "id": id, "name": dn }),
                            None => serde_json::json!({ "id": id }),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    // For local-ish providers, best-effort enrichment from LM Studio's `/api/v0/models`.
    // Failures are silent; non-LM-Studio backends (llama.cpp, vLLM) simply skip this.
    if crate::managers::provider_probe::should_probe(&provider.id, &provider.api_url) {
        if let Some(info_map) = state
            .provider_probe_cache
            .get_or_probe(&provider.id, &provider.api_url, &client)
            .await
        {
            for m in &mut models {
                let Some(id) = m.get("id").and_then(|v| v.as_str()).map(String::from) else {
                    continue;
                };
                let Some(info) = info_map.get(&id) else {
                    continue;
                };
                let Some(obj) = m.as_object_mut() else {
                    continue;
                };
                obj.insert("loaded".into(), serde_json::Value::Bool(info.loaded));
                if let Some(ctx) = info.max_context_length {
                    obj.insert("max_context_length".into(), serde_json::json!(ctx));
                }
                // LM Studio only reports loaded_context_length when state=loaded.
                // This is the authoritative n_ctx for pre-flight and the Detect button.
                if let Some(ctx) = info.loaded_context_length {
                    obj.insert("loaded_context_length".into(), serde_json::json!(ctx));
                }
                if let Some(arch) = &info.architecture {
                    obj.insert("architecture".into(), serde_json::json!(arch));
                }
            }
        }
    }

    ok_data(serde_json::json!({ "models": models }))
}

/// Scrub sensitive substrings (api_key, URL userinfo) from any upstream-derived
/// error message before returning it to the Dashboard. reqwest's Display impl
/// on a `Url` intentionally masks basic-auth credentials already, but upstream
/// error bodies sometimes echo the key back verbatim.
fn redact_secrets(s: &str, api_key: &str) -> String {
    if api_key.is_empty() {
        s.to_string()
    } else {
        s.replace(api_key, "[REDACTED]")
    }
}

/// POST /api/llm/providers/:id/test
///
/// End-to-end connectivity + auth check for a single provider. The Dashboard
/// shows the result as a colored pill ("green/yellow/red") so users can
/// diagnose "Local LLM → chat fails" in a single click rather than waiting
/// until they send a message.
///
/// Returns `{ status, latency_ms, reachable, auth_ok, model_list, models_count, error? }`
/// where `status` is one of:
///   - `ok`                        — reachable + authenticated + returned a model list
///   - `auth_failed`               — reached the endpoint, got 401/403
///   - `model_list_unavailable`    — reached, authenticated, but no model catalog
///   - `unreachable`               — connection / DNS / timeout failure
pub async fn test_provider_connection(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let provider = crate::db::get_llm_provider(&state.pool, &provider_id)
        .await
        .map_err(|_| AppError::NotFound(format!("LLM provider '{}' not found", provider_id)))?;

    let models_url = derive_models_url(&provider.api_url, &provider.id)
        .map_err(AppError::Validation)?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(MODELS_FETCH_TIMEOUT_SECS))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

    let mut req = client.get(&models_url);
    if !provider.api_key.is_empty() {
        if provider.auth_type == "x-api-key" {
            req = req
                .header("x-api-key", &provider.api_key)
                .header("anthropic-version", "2023-06-01");
        } else {
            req = req.header("Authorization", format!("Bearer {}", provider.api_key));
        }
    }

    let start = std::time::Instant::now();
    let response = req.send().await;
    let latency_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

    let (status_label, reachable, auth_ok, model_list, models_count, error_msg): (
        &str,
        bool,
        bool,
        bool,
        Option<usize>,
        Option<String>,
    ) = match response {
        Err(e) => (
            "unreachable",
            false,
            false,
            false,
            None,
            Some(redact_secrets(&e.to_string(), &provider.api_key)),
        ),
        Ok(r) => {
            let code = r.status().as_u16();
            match code {
                200..=299 => {
                    // Count entries so the UI can show "4 models available" as a success signal.
                    let count = match r.json::<serde_json::Value>().await {
                        Ok(body) => {
                            // Ollama: {"models":[...]}, OpenAI: {"data":[...]}
                            let arr = body
                                .get("models")
                                .or_else(|| body.get("data"))
                                .and_then(|v| v.as_array());
                            arr.map(std::vec::Vec::len)
                        }
                        Err(_) => None,
                    };
                    ("ok", true, true, count.is_some(), count, None)
                }
                401 | 403 => (
                    "auth_failed",
                    true,
                    false,
                    false,
                    None,
                    Some(format!("HTTP {}", code)),
                ),
                _ => (
                    "model_list_unavailable",
                    true,
                    true,
                    false,
                    None,
                    Some(format!("HTTP {}", code)),
                ),
            }
        }
    };

    ok_data(serde_json::json!({
        "status": status_label,
        "latency_ms": latency_ms,
        "reachable": reachable,
        "auth_ok": auth_ok,
        "model_list": model_list,
        "models_count": models_count,
        "error": error_msg,
    }))
}

/// POST /api/llm/providers/:id/context-length
///
/// Sets or clears the provider's context window hint. Accepts
/// `{ "context_length": number | null }`. Stored value is used by the kernel's
/// pre-flight budget check to reject oversized requests before they reach
/// the provider (and to surface a localized hint instead of a 400 from
/// upstream). Audit-logged like other LLM provider mutations.
pub async fn set_llm_provider_context_length(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let raw = payload.get("context_length");
    let new_value: Option<i64> = match raw {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => {
            let n = v
                .as_i64()
                .ok_or_else(|| AppError::Validation("context_length must be a number or null".into()))?;
            if n <= 0 {
                return Err(AppError::Validation(
                    "context_length must be positive".into(),
                ));
            }
            // 8M tokens is well beyond any shipping model; reject absurd values so the
            // pre-flight check stays meaningful.
            if n > 8_000_000 {
                return Err(AppError::Validation(
                    "context_length exceeds sane upper bound".into(),
                ));
            }
            Some(n)
        }
    };

    let old_value = crate::db::set_llm_provider_context_length(&state.pool, &provider_id, new_value)
        .await
        .map_err(|e| AppError::Validation(e.to_string()))?;

    spawn_admin_audit(
        state.pool.clone(),
        "LLM_PROVIDER_CONTEXT_LENGTH_UPDATED",
        provider_id.clone(),
        format!(
            "Context length changed from {} to {}",
            old_value.map_or_else(|| "null".to_string(), |n| n.to_string()),
            new_value.map_or_else(|| "null".to_string(), |n| n.to_string()),
        ),
        None,
        Some(serde_json::json!({
            "old_context_length": old_value,
            "new_context_length": new_value,
        })),
        None,
    );

    tracing::info!(
        provider = %provider_id,
        new_value = ?new_value,
        "LLM provider context_length updated"
    );
    ok_data(serde_json::json!({}))
}

/// DELETE /api/llm/providers/:id/key
pub async fn delete_llm_provider_key(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    crate::db::delete_llm_provider_key(&state.pool, &provider_id)
        .await
        .map_err(AppError::Internal)?;
    tracing::info!(provider = %provider_id, "LLM provider API key deleted");
    ok_data(serde_json::json!({}))
}

#[cfg(test)]
mod tests {
    use super::derive_models_url;

    #[test]
    fn derives_openai_compat_v1_base() {
        // local (LM Studio), cerebras, groq-adjacent all share the /v1/chat/completions form.
        assert_eq!(
            derive_models_url("http://localhost:1234/v1/chat/completions", "local").unwrap(),
            "http://localhost:1234/v1/models"
        );
        assert_eq!(
            derive_models_url("https://api.cerebras.ai/v1/chat/completions", "cerebras").unwrap(),
            "https://api.cerebras.ai/v1/models"
        );
    }

    #[test]
    fn derives_groq_openai_prefixed_base() {
        assert_eq!(
            derive_models_url("https://api.groq.com/openai/v1/chat/completions", "groq").unwrap(),
            "https://api.groq.com/openai/v1/models"
        );
    }

    #[test]
    fn derives_deepseek_non_v1_base() {
        // DeepSeek's seed URL has no /v1 prefix: /chat/completions → /models.
        assert_eq!(
            derive_models_url("https://api.deepseek.com/chat/completions", "deepseek").unwrap(),
            "https://api.deepseek.com/models"
        );
    }

    #[test]
    fn derives_anthropic_messages_base() {
        assert_eq!(
            derive_models_url("https://api.anthropic.com/v1/messages", "claude").unwrap(),
            "https://api.anthropic.com/v1/models"
        );
    }

    #[test]
    fn derives_ollama_native_tags_endpoint() {
        assert_eq!(
            derive_models_url("http://localhost:11434/api/chat", "ollama").unwrap(),
            "http://localhost:11434/api/tags"
        );
    }

    #[test]
    fn idempotent_on_v1_models() {
        // If admin already set api_url to /v1/models (unusual but harmless), we still produce
        // a valid /v1/models URL — parent of /v1/models is /v1, append /models back.
        assert_eq!(
            derive_models_url("http://localhost:1234/v1/models", "local").unwrap(),
            "http://localhost:1234/v1/models"
        );
    }

    #[test]
    fn trims_query_and_fragment() {
        assert_eq!(
            derive_models_url(
                "http://localhost:1234/v1/chat/completions?token=x#frag",
                "local"
            )
            .unwrap(),
            "http://localhost:1234/v1/models"
        );
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert!(derive_models_url("file:///etc/passwd", "local").is_err());
        assert!(derive_models_url("ftp://example.com/chat", "local").is_err());
    }

    #[test]
    fn rejects_malformed_url() {
        assert!(derive_models_url("not-a-url", "local").is_err());
    }
}
