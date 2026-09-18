//! Rows the panel write gate reads: what an install recorded about a connector
//! tree, and which panels an operator has consented to let write.
//!
//! The decisions are made in `handlers::modules`; this module only stores and
//! returns rows. Both tables are described in
//! `migrations/20260918120000_add_panel_write_gate.sql`.

use serde::Serialize;
use sqlx::SqlitePool;

use super::db_timeout;

/// What a marketplace install recorded about the tree it placed.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct InstallReceipt {
    pub connector_dir: String,
    pub trust_level: String,
    pub seal: Option<String>,
    pub version: String,
    pub installed_at: String,
}

/// Record (or replace) the receipt for one installed tree.
///
/// Replaced rather than kept: a reinstall places a new tree with a new seal,
/// and the old receipt describes files that are no longer there.
pub async fn upsert_install_receipt(
    pool: &SqlitePool,
    connector_dir: &str,
    trust_level: &str,
    seal: Option<&str>,
    version: &str,
) -> anyhow::Result<()> {
    let query_future = sqlx::query(
        "INSERT INTO connector_install_receipts \
         (connector_dir, trust_level, seal, version, installed_at) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(connector_dir) DO UPDATE SET \
         trust_level = excluded.trust_level, seal = excluded.seal, \
         version = excluded.version, installed_at = excluded.installed_at",
    )
    .bind(connector_dir)
    .bind(trust_level)
    .bind(seal)
    .bind(version)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool);
    db_timeout(query_future).await?;
    Ok(())
}

pub async fn get_install_receipt(
    pool: &SqlitePool,
    connector_dir: &str,
) -> anyhow::Result<Option<InstallReceipt>> {
    let query_future = sqlx::query_as::<_, InstallReceipt>(
        "SELECT connector_dir, trust_level, seal, version, installed_at \
         FROM connector_install_receipts WHERE connector_dir = ?",
    )
    .bind(connector_dir)
    .fetch_optional(pool);
    db_timeout(query_future).await
}

pub async fn delete_install_receipt(pool: &SqlitePool, connector_dir: &str) -> anyhow::Result<()> {
    let query_future =
        sqlx::query("DELETE FROM connector_install_receipts WHERE connector_dir = ?")
            .bind(connector_dir)
            .execute(pool);
    db_timeout(query_future).await?;
    Ok(())
}

/// An operator's consent to one panel's declared writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct PanelWriteConsent {
    pub panel_id: String,
    pub writes_digest: String,
    pub connector_version: String,
    pub granted_at: String,
    pub granted_by: String,
}

pub async fn put_panel_write_consent(
    pool: &SqlitePool,
    panel_id: &str,
    writes_digest: &str,
    connector_version: &str,
    granted_by: &str,
) -> anyhow::Result<()> {
    let query_future = sqlx::query(
        "INSERT INTO panel_write_consents \
         (panel_id, writes_digest, connector_version, granted_at, granted_by) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(panel_id) DO UPDATE SET \
         writes_digest = excluded.writes_digest, connector_version = excluded.connector_version, \
         granted_at = excluded.granted_at, granted_by = excluded.granted_by",
    )
    .bind(panel_id)
    .bind(writes_digest)
    .bind(connector_version)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(granted_by)
    .execute(pool);
    db_timeout(query_future).await?;
    Ok(())
}

pub async fn get_panel_write_consent(
    pool: &SqlitePool,
    panel_id: &str,
) -> anyhow::Result<Option<PanelWriteConsent>> {
    let query_future = sqlx::query_as::<_, PanelWriteConsent>(
        "SELECT panel_id, writes_digest, connector_version, granted_at, granted_by \
         FROM panel_write_consents WHERE panel_id = ?",
    )
    .bind(panel_id)
    .fetch_optional(pool);
    db_timeout(query_future).await
}

pub async fn list_panel_write_consents(
    pool: &SqlitePool,
) -> anyhow::Result<Vec<PanelWriteConsent>> {
    let query_future = sqlx::query_as::<_, PanelWriteConsent>(
        "SELECT panel_id, writes_digest, connector_version, granted_at, granted_by \
         FROM panel_write_consents ORDER BY panel_id",
    )
    .fetch_all(pool);
    db_timeout(query_future).await
}

/// Returns whether a row was removed, so revoking a consent that was never
/// given can be answered as such rather than as a success.
pub async fn delete_panel_write_consent(pool: &SqlitePool, panel_id: &str) -> anyhow::Result<bool> {
    let query_future = sqlx::query("DELETE FROM panel_write_consents WHERE panel_id = ?")
        .bind(panel_id)
        .execute(pool);
    Ok(db_timeout(query_future).await?.rows_affected() > 0)
}
