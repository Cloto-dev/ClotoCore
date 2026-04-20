use sqlx::SqlitePool;

use super::db_timeout;

// ── LLM Provider Registry (MGP §13.4 llm_completion) ──

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
    /// Override for reasoning/thinking mode: "auto" (default), "on", "off".
    /// Injected by `augment_mind_env` as `{PREFIX}_REASONING_PREFILL=true/false`
    /// when the value is "on"/"off"; "auto" leaves env untouched so the
    /// server-side heuristic can decide.
    #[serde(default = "default_reasoning_prefill")]
    pub reasoning_prefill: String,
}

fn default_auth_type() -> String {
    "bearer".to_string()
}

fn default_reasoning_prefill() -> String {
    "auto".to_string()
}

pub async fn list_llm_providers(pool: &SqlitePool) -> anyhow::Result<Vec<LlmProviderRow>> {
    let rows = db_timeout(sqlx::query_as::<_, LlmProviderRow>(
        "SELECT id, display_name, api_url, api_key, model_id, timeout_secs, enabled, created_at, auth_type, context_length, reasoning_prefill FROM llm_providers ORDER BY id"
    ).fetch_all(pool)).await?;
    Ok(rows)
}

pub async fn get_llm_provider(pool: &SqlitePool, id: &str) -> anyhow::Result<LlmProviderRow> {
    let row = db_timeout(sqlx::query_as::<_, LlmProviderRow>(
        "SELECT id, display_name, api_url, api_key, model_id, timeout_secs, enabled, created_at, auth_type, context_length, reasoning_prefill FROM llm_providers WHERE id = ?"
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

/// Update the provider's reasoning/thinking override.
/// Valid values: "auto" (default), "on", "off". Returns the previous value for audit logs.
pub async fn set_llm_provider_reasoning_prefill(
    pool: &SqlitePool,
    id: &str,
    value: &str,
) -> anyhow::Result<String> {
    if !matches!(value, "auto" | "on" | "off") {
        return Err(anyhow::anyhow!(
            "reasoning_prefill must be 'auto', 'on', or 'off' (got '{}')",
            value
        ));
    }
    let old: String = db_timeout(
        sqlx::query_scalar("SELECT reasoning_prefill FROM llm_providers WHERE id = ?")
            .bind(id)
            .fetch_optional(pool),
    )
    .await?
    .ok_or_else(|| anyhow::anyhow!("LLM provider '{}' not found", id))?;

    let result = db_timeout(
        sqlx::query("UPDATE llm_providers SET reasoning_prefill = ? WHERE id = ?")
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
