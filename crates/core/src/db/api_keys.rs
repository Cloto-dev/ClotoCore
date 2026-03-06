use sqlx::SqlitePool;

use super::db_timeout;

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
    db_timeout(
        sqlx::query("INSERT OR IGNORE INTO revoked_keys (key_hash, revoked_at) VALUES (?, ?)")
            .bind(&key_hash)
            .bind(now)
            .execute(pool),
    )
    .await?;
    Ok(())
}

pub async fn load_revoked_key_hashes(pool: &SqlitePool) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> =
        db_timeout(sqlx::query_as("SELECT key_hash FROM revoked_keys").fetch_all(pool)).await?;
    Ok(rows.into_iter().map(|(h,)| h).collect())
}

/// Delete revoked key entries older than `ttl_days` and return the remaining hashes.
pub async fn cleanup_revoked_keys(pool: &SqlitePool, ttl_days: i64) -> anyhow::Result<Vec<String>> {
    let cutoff_ms = (chrono::Utc::now() - chrono::Duration::days(ttl_days)).timestamp_millis();
    db_timeout(
        sqlx::query("DELETE FROM revoked_keys WHERE revoked_at < ?")
            .bind(cutoff_ms)
            .execute(pool),
    )
    .await?;
    load_revoked_key_hashes(pool).await
}

pub async fn is_api_key_revoked(pool: &SqlitePool, key: &str) -> anyhow::Result<bool> {
    let key_hash = hash_api_key(key);
    let row: Option<(String,)> = db_timeout(
        sqlx::query_as("SELECT key_hash FROM revoked_keys WHERE key_hash = ?")
            .bind(&key_hash)
            .fetch_optional(pool),
    )
    .await?;
    Ok(row.is_some())
}
