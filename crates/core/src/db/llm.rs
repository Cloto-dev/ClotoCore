use sqlx::SqlitePool;

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
}

pub async fn list_llm_providers(pool: &SqlitePool) -> anyhow::Result<Vec<LlmProviderRow>> {
    let rows = sqlx::query_as::<_, LlmProviderRow>(
        "SELECT id, display_name, api_url, api_key, model_id, timeout_secs, enabled, created_at FROM llm_providers ORDER BY id"
    ).fetch_all(pool).await?;
    Ok(rows)
}

pub async fn get_llm_provider(pool: &SqlitePool, id: &str) -> anyhow::Result<LlmProviderRow> {
    let row = sqlx::query_as::<_, LlmProviderRow>(
        "SELECT id, display_name, api_url, api_key, model_id, timeout_secs, enabled, created_at FROM llm_providers WHERE id = ?"
    ).bind(id).fetch_optional(pool).await?;
    row.ok_or_else(|| anyhow::anyhow!("LLM provider '{}' not found", id))
}

pub async fn set_llm_provider_key(
    pool: &SqlitePool,
    id: &str,
    api_key: &str,
) -> anyhow::Result<()> {
    let result = sqlx::query("UPDATE llm_providers SET api_key = ? WHERE id = ?")
        .bind(api_key)
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(anyhow::anyhow!("LLM provider '{}' not found", id));
    }
    Ok(())
}

pub async fn delete_llm_provider_key(pool: &SqlitePool, id: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE llm_providers SET api_key = '' WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Sync API keys from environment variables into the llm_providers table.
///
/// For each known provider, checks for `{PREFIX}_API_KEY` in the environment.
/// Only updates rows where the current api_key is empty (never overwrites
/// keys that were set via the Dashboard UI or API).
pub async fn sync_env_api_keys(pool: &SqlitePool) {
    let mappings: &[(&str, &str)] = &[
        ("deepseek", "DEEPSEEK_API_KEY"),
        ("cerebras", "CEREBRAS_API_KEY"),
        ("claude", "CLAUDE_API_KEY"),
        ("ollama", "OLLAMA_API_KEY"),
    ];

    for &(provider_id, env_var) in mappings {
        if let Ok(key) = std::env::var(env_var) {
            if key.is_empty() {
                continue;
            }
            let result =
                sqlx::query("UPDATE llm_providers SET api_key = ? WHERE id = ? AND api_key = ''")
                    .bind(&key)
                    .bind(provider_id)
                    .execute(pool)
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
