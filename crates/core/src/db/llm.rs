use sqlx::SqlitePool;

use super::db_timeout;

// ── LLM Provider Registry (MGP §13.4 llm_completion) ──

/// Per-provider quirks declared as data, so the kernel does not need to
/// hard-code provider-specific branches (ARCHITECTURE.md §1.1 Core Minimalism).
///
/// Decoded lazily from the JSON stored in `llm_providers.quirks`. Absent or
/// invalid JSON yields `ProviderQuirks::default()`, i.e. "standard
/// OpenAI-compatible provider, key required".
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ProviderQuirks {
    /// Provider does not require an API key (Ollama, local LM Studio).
    #[serde(default)]
    pub no_api_key: bool,
    /// Native models-list path (absolute URL path, e.g. "/api/tags").
    /// Overrides the OpenAI-compat `.../models` derivation when set.
    #[serde(default)]
    pub models_endpoint_path: Option<String>,
    /// MCP tool name on `mind.<provider_id>` to relay a live model switch,
    /// for providers whose mind server binds the model name at startup.
    #[serde(default)]
    pub switch_model_tool: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct LlmProviderRow {
    pub id: String,
    pub display_name: String,
    pub api_url: String,
    pub api_key: String,
    pub model_id: String,
    pub timeout_secs: i32,
    pub enabled: bool,
    pub created_at: String,
    /// Authentication type: "bearer" (default) or "x-api-key" (Anthropic-style).
    #[serde(default = "default_auth_type")]
    pub auth_type: String,
    /// Provider's context window in tokens. `None` = unknown; pre-flight validation is skipped.
    /// Populated manually via Dashboard or auto-detected from LM Studio probes.
    #[serde(default)]
    pub context_length: Option<i64>,
    /// User-facing thinking mode toggle: "auto" (default), "on", "off".
    ///
    /// Semantic is the USER INTENT, not the internal mechanism:
    ///   * `"on"`   — thinking is allowed (no prefill applied)
    ///   * `"off"`  — thinking is suppressed via an assistant `<think></think>`
    ///     prefill on the outbound request
    ///   * `"auto"` — server-side heuristic on the model id decides
    ///
    /// `augment_mind_env` translates this into the internal
    /// `{PREFIX}_REASONING_PREFILL=true/false` env var that the Python MGP
    /// servers already consume. The env var keeps its old name because it
    /// describes the mechanism ("apply the anti-think prefill"); only the
    /// user-facing column/field was renamed.
    #[serde(default = "default_thinking_mode")]
    pub thinking_mode: String,
    /// Raw JSON payload from the `quirks` column. Parse via
    /// [`LlmProviderRow::quirks_parsed`] to get a [`ProviderQuirks`] with
    /// defaults filled in.
    #[serde(default)]
    pub quirks: Option<String>,
}

impl LlmProviderRow {
    /// Decode the `quirks` column into a [`ProviderQuirks`]. Missing or
    /// malformed JSON falls back to [`ProviderQuirks::default`] so callers
    /// can rely on `.no_api_key`, `.models_endpoint_path`, etc. unconditionally.
    #[must_use]
    pub fn quirks_parsed(&self) -> ProviderQuirks {
        self.quirks
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }
}

fn default_auth_type() -> String {
    "bearer".to_string()
}

fn default_thinking_mode() -> String {
    "auto".to_string()
}

pub async fn list_llm_providers(pool: &SqlitePool) -> anyhow::Result<Vec<LlmProviderRow>> {
    let rows = db_timeout(sqlx::query_as::<_, LlmProviderRow>(
        "SELECT id, display_name, api_url, api_key, model_id, timeout_secs, enabled, created_at, auth_type, context_length, thinking_mode, quirks FROM llm_providers ORDER BY id"
    ).fetch_all(pool)).await?;
    Ok(rows)
}

pub async fn get_llm_provider(pool: &SqlitePool, id: &str) -> anyhow::Result<LlmProviderRow> {
    let row = db_timeout(sqlx::query_as::<_, LlmProviderRow>(
        "SELECT id, display_name, api_url, api_key, model_id, timeout_secs, enabled, created_at, auth_type, context_length, thinking_mode, quirks FROM llm_providers WHERE id = ?"
    ).bind(id).fetch_optional(pool)).await?;
    row.ok_or_else(|| anyhow::anyhow!("LLM provider '{}' not found", id))
}

pub async fn set_llm_provider_key(
    pool: &SqlitePool,
    id: &str,
    api_key: &str,
) -> anyhow::Result<()> {
    let result = db_timeout(
        sqlx::query("UPDATE llm_providers SET api_key = ? WHERE id = ?")
            .bind(api_key)
            .bind(id)
            .execute(pool),
    )
    .await?;
    if result.rows_affected() == 0 {
        return Err(anyhow::anyhow!("LLM provider '{}' not found", id));
    }
    Ok(())
}

/// Update a provider's model_id, recording the old value in the history table.
/// Returns the previous model_id so callers can include it in audit logs.
pub async fn set_llm_provider_model(
    pool: &SqlitePool,
    id: &str,
    model_id: &str,
) -> anyhow::Result<String> {
    let old_model: String = db_timeout(
        sqlx::query_scalar("SELECT model_id FROM llm_providers WHERE id = ?")
            .bind(id)
            .fetch_optional(pool),
    )
    .await?
    .ok_or_else(|| anyhow::anyhow!("LLM provider '{}' not found", id))?;

    let result = db_timeout(
        sqlx::query("UPDATE llm_providers SET model_id = ? WHERE id = ?")
            .bind(model_id)
            .bind(id)
            .execute(pool),
    )
    .await?;
    if result.rows_affected() == 0 {
        return Err(anyhow::anyhow!("LLM provider '{}' not found", id));
    }

    db_timeout(
        sqlx::query(
            "INSERT INTO llm_provider_model_history (provider_id, old_model_id, new_model_id) \
             VALUES (?, ?, ?)",
        )
        .bind(id)
        .bind(&old_model)
        .bind(model_id)
        .execute(pool),
    )
    .await?;

    Ok(old_model)
}

/// Update the provider's context_length. Pass `None` to clear a previously set value.
/// Returns the previous value so callers can populate audit logs.
pub async fn set_llm_provider_context_length(
    pool: &SqlitePool,
    id: &str,
    context_length: Option<i64>,
) -> anyhow::Result<Option<i64>> {
    let old: Option<i64> = db_timeout(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT context_length FROM llm_providers WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(pool),
    )
    .await?
    .flatten();

    let result = db_timeout(
        sqlx::query("UPDATE llm_providers SET context_length = ? WHERE id = ?")
            .bind(context_length)
            .bind(id)
            .execute(pool),
    )
    .await?;
    if result.rows_affected() == 0 {
        return Err(anyhow::anyhow!("LLM provider '{}' not found", id));
    }
    Ok(old)
}

/// Update the provider's thinking mode.
/// Valid values: "auto" (default), "on", "off". Returns the previous value for audit logs.
pub async fn set_llm_provider_thinking_mode(
    pool: &SqlitePool,
    id: &str,
    value: &str,
) -> anyhow::Result<String> {
    if !matches!(value, "auto" | "on" | "off") {
        return Err(anyhow::anyhow!(
            "thinking_mode must be 'auto', 'on', or 'off' (got '{}')",
            value
        ));
    }
    let old: String = db_timeout(
        sqlx::query_scalar("SELECT thinking_mode FROM llm_providers WHERE id = ?")
            .bind(id)
            .fetch_optional(pool),
    )
    .await?
    .ok_or_else(|| anyhow::anyhow!("LLM provider '{}' not found", id))?;

    let result = db_timeout(
        sqlx::query("UPDATE llm_providers SET thinking_mode = ? WHERE id = ?")
            .bind(value)
            .bind(id)
            .execute(pool),
    )
    .await?;
    if result.rows_affected() == 0 {
        return Err(anyhow::anyhow!("LLM provider '{}' not found", id));
    }
    Ok(old)
}

/// Fill in `context_length` from a detected value only when the user has not already
/// set one. Returns `true` when the row was actually updated (so callers can log it).
pub async fn maybe_autofill_context_length(
    pool: &SqlitePool,
    id: &str,
    detected: i64,
) -> anyhow::Result<bool> {
    let result = db_timeout(
        sqlx::query(
            "UPDATE llm_providers SET context_length = ? \
             WHERE id = ? AND context_length IS NULL",
        )
        .bind(detected)
        .bind(id)
        .execute(pool),
    )
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_llm_provider_key(pool: &SqlitePool, id: &str) -> anyhow::Result<()> {
    db_timeout(
        sqlx::query("UPDATE llm_providers SET api_key = '' WHERE id = ?")
            .bind(id)
            .execute(pool),
    )
    .await?;
    Ok(())
}

/// Sync API keys from environment variables into the llm_providers table.
///
/// For each provider mapping, checks for the env var in the environment.
/// Only updates rows where the current api_key is empty (never overwrites
/// keys that were set via the Dashboard UI or API).
///
/// `mappings` is a slice of (provider_id, env_var_name) pairs, loaded from AppConfig.
pub async fn sync_env_api_keys(pool: &SqlitePool, mappings: &[(String, String)]) {
    let mappings: Vec<(&str, &str)> = mappings
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();

    for (provider_id, env_var) in &mappings {
        if let Ok(key) = std::env::var(env_var) {
            if key.is_empty() {
                continue;
            }
            let result = db_timeout(
                sqlx::query("UPDATE llm_providers SET api_key = ? WHERE id = ? AND api_key = ''")
                    .bind(&key)
                    .bind(provider_id)
                    .execute(pool),
            )
            .await;

            match result {
                Ok(r) if r.rows_affected() > 0 => {
                    tracing::info!(
                        provider = %provider_id,
                        "Synced API key from env var {}",
                        env_var,
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        provider = %provider_id,
                        error = %e,
                        "Failed to sync API key from env",
                    );
                }
                _ => {}
            }
        }
    }
}
