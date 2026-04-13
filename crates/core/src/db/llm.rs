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
}

fn default_auth_type() -> String {
    "bearer".to_string()
}

pub async fn list_llm_providers(pool: &SqlitePool) -> anyhow::Result<Vec<LlmProviderRow>> {
    let rows = db_timeout(sqlx::query_as::<_, LlmProviderRow>(
        "SELECT id, display_name, api_url, api_key, model_id, timeout_secs, enabled, created_at, auth_type FROM llm_providers ORDER BY id"
    ).fetch_all(pool)).await?;
    Ok(rows)
}

pub async fn get_llm_provider(pool: &SqlitePool, id: &str) -> anyhow::Result<LlmProviderRow> {
    let row = db_timeout(sqlx::query_as::<_, LlmProviderRow>(
        "SELECT id, display_name, api_url, api_key, model_id, timeout_secs, enabled, created_at, auth_type FROM llm_providers WHERE id = ?"
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
