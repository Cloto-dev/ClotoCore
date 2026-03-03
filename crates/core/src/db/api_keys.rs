use sqlx::SqlitePool;

use super::DB_TIMEOUT_SECS;

// ============================================================
// Revoked API Keys
// ============================================================

/// Compute a deterministic fingerprint of a key for revocation storage.
/// Uses DefaultHasher with a fixed salt (not crypto-grade, but sufficient
/// for revocation purposes on a local LAN-only dashboard).
#[must_use]
pub fn hash_api_key(key: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::fmt::Write as _;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    key.len().hash(&mut h);
    b"cloto-revoke-salt-2026".hash(&mut h);
    let val = h.finish();
    let mut out = String::new();
    write!(out, "{:016x}{:016x}", val, val ^ 0xdead_beef_cafe_babe).unwrap();
    out
}

pub async fn revoke_api_key(pool: &SqlitePool, key: &str) -> anyhow::Result<()> {
    let key_hash = hash_api_key(key);
    let now = chrono::Utc::now().timestamp_millis();
    tokio::time::timeout(std::time::Duration::from_secs(DB_TIMEOUT_SECS), async {
        sqlx::query("INSERT OR IGNORE INTO revoked_keys (key_hash, revoked_at) VALUES (?, ?)")
            .bind(&key_hash)
            .bind(now)
            .execute(pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to revoke API key: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout revoking API key"))?
}

pub async fn load_revoked_key_hashes(pool: &SqlitePool) -> anyhow::Result<Vec<String>> {
    tokio::time::timeout(std::time::Duration::from_secs(DB_TIMEOUT_SECS), async {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT key_hash FROM revoked_keys")
            .fetch_all(pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load revoked key hashes: {}", e))?;
        Ok(rows.into_iter().map(|(h,)| h).collect())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout loading revoked keys"))?
}

pub async fn is_api_key_revoked(pool: &SqlitePool, key: &str) -> anyhow::Result<bool> {
    let key_hash = hash_api_key(key);
    tokio::time::timeout(std::time::Duration::from_secs(DB_TIMEOUT_SECS), async {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT key_hash FROM revoked_keys WHERE key_hash = ?")
                .bind(&key_hash)
                .fetch_optional(pool)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to check revoked keys: {}", e))?;
        Ok(row.is_some())
    })
    .await
    .map_err(|_| anyhow::anyhow!("Database timeout checking revoked keys"))?
}
