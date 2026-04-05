//! Kernel health check API — scan and repair endpoints.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;

use super::ok_data;
use crate::db;
use crate::{AppError, AppResult, AppState};

#[derive(Deserialize)]
pub struct ScanQuery {
    /// Force a fresh scan instead of returning cached result.
    pub fresh: Option<bool>,
}

/// GET /api/health/scan — Run a quick health scan (or return cached result).
pub async fn scan_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<ScanQuery>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    let fresh = params.fresh.unwrap_or(false);

    // Return cached result if available and not forced fresh
    if !fresh {
        let cached = state.last_health_report.read().await;
        if let Some(ref report) = *cached {
            return ok_data(serde_json::to_value(report).map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to serialize health report: {e}"))
            })?);
        }
    }

    // Run fresh scan
    let report = db::health::run_quick_scan(&state.pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Health scan failed: {e}")))?;

    // Cache the result
    {
        let mut cached = state.last_health_report.write().await;
        *cached = Some(report.clone());
    }

    ok_data(serde_json::to_value(&report).map_err(|e| {
        AppError::Internal(anyhow::anyhow!("Failed to serialize health report: {e}"))
    })?)
}

/// POST /api/health/repair — Run standard repair on detected issues.
pub async fn repair_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    let report = db::health::run_standard_repair(&state.pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Health repair failed: {e}")))?;

    // Invalidate cached health report so next scan reflects repairs
    {
        let mut cached = state.last_health_report.write().await;
        *cached = None;
    }

    ok_data(serde_json::to_value(&report).map_err(|e| {
        AppError::Internal(anyhow::anyhow!("Failed to serialize repair report: {e}"))
    })?)
}
