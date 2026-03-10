use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::db_timeout;

/// Permission request entry for human-in-the-loop workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub request_id: String,
    pub created_at: DateTime<Utc>,
    pub plugin_id: String,
    pub permission_type: String,
    pub target_resource: Option<String>,
    pub justification: String,
    pub status: String,
    pub approved_by: Option<String>,
    pub approved_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub metadata: Option<serde_json::Value>,
}

/// Create a new permission request
pub async fn create_permission_request(
    pool: &SqlitePool,
    request: PermissionRequest,
) -> anyhow::Result<()> {
    let created_at = request.created_at.to_rfc3339();
    let expires_at = request.expires_at.map(|dt| dt.to_rfc3339());
    let metadata_str = request.metadata.map(|v| v.to_string());

    // Bug #7: Add timeout to prevent indefinite hangs on database locks
    let query_future = sqlx::query(
        "INSERT INTO permission_requests (request_id, created_at, plugin_id, permission_type, target_resource, justification, status, expires_at, metadata)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&request.request_id)
    .bind(&created_at)
    .bind(&request.plugin_id)
    .bind(&request.permission_type)
    .bind(&request.target_resource)
    .bind(&request.justification)
    .bind(&request.status)
    .bind(&expires_at)
    .bind(&metadata_str)
    .execute(pool);

    db_timeout(query_future).await?;

    Ok(())
}

/// Query pending permission requests
pub async fn get_pending_permission_requests(
    pool: &SqlitePool,
) -> anyhow::Result<Vec<PermissionRequest>> {
    // Bug #7: Add timeout to prevent indefinite hangs on database locks
    #[allow(clippy::type_complexity)]
    let query_future = sqlx::query_as::<_, (String, String, String, String, Option<String>, String, String, Option<String>, Option<String>, Option<String>, Option<String>)>(
            "SELECT request_id, created_at, plugin_id, permission_type, target_resource, justification, status, approved_by, approved_at, expires_at, metadata
             FROM permission_requests
             WHERE status = 'pending'
             ORDER BY created_at DESC"
        )
        .fetch_all(pool);

    let rows = db_timeout(query_future).await?;

    let mut requests = Vec::new();
    for (
        request_id,
        created_at,
        plugin_id,
        permission_type,
        target_resource,
        justification,
        status,
        approved_by,
        approved_at,
        expires_at,
        metadata,
    ) in rows
    {
        requests.push(PermissionRequest {
            request_id,
            created_at: DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&Utc),
            plugin_id,
            permission_type,
            target_resource,
            justification,
            status,
            approved_by,
            approved_at: approved_at.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            expires_at: expires_at.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            metadata: metadata.and_then(|s| serde_json::from_str(&s).ok()),
        });
    }

    Ok(requests)
}

/// Get a single permission request by ID (for post-approval processing).
pub async fn get_permission_request(
    pool: &SqlitePool,
    request_id: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let query_future = sqlx::query_as::<_, (String, String)>(
        "SELECT plugin_id, permission_type FROM permission_requests WHERE request_id = ?",
    )
    .bind(request_id)
    .fetch_optional(pool);

    Ok(db_timeout(query_future).await?)
}

/// Update permission request status (approve/deny)
/// Only transitions from 'pending' status are allowed to prevent double-approval
pub async fn update_permission_request(
    pool: &SqlitePool,
    request_id: &str,
    status: &str,
    approved_by: &str,
) -> anyhow::Result<()> {
    // Whitelist allowed status transitions
    if !["approved", "denied"].contains(&status) {
        return Err(anyhow::anyhow!(
            "Invalid status value: '{}'. Must be 'approved' or 'denied'",
            status
        ));
    }

    let approved_at = Utc::now().to_rfc3339();

    // Bug #7: Add timeout to prevent indefinite hangs on database locks
    let query_future = sqlx::query(
        "UPDATE permission_requests
         SET status = ?, approved_by = ?, approved_at = ?
         WHERE request_id = ? AND status = 'pending'",
    )
    .bind(status)
    .bind(approved_by)
    .bind(&approved_at)
    .bind(request_id)
    .execute(pool);

    let result = db_timeout(query_future).await?;

    if result.rows_affected() == 0 {
        return Err(anyhow::anyhow!(
            "Permission request '{}' not found or already processed",
            request_id
        ));
    }

    Ok(())
}

/// Check if a specific permission is already approved for a plugin/server.
/// Returns true if an approved, non-expired permission exists.
pub async fn is_permission_approved(
    pool: &SqlitePool,
    plugin_id: &str,
    permission_type: &str,
) -> anyhow::Result<bool> {
    let query_future = sqlx::query_scalar::<_, i32>(
        "SELECT COUNT(*) FROM permission_requests
         WHERE plugin_id = ? AND permission_type = ? AND status IN ('approved', 'auto-approved')
           AND (expires_at IS NULL OR expires_at > datetime('now'))",
    )
    .bind(plugin_id)
    .bind(permission_type)
    .fetch_one(pool);

    let count = db_timeout(query_future).await?;

    Ok(count > 0)
}
