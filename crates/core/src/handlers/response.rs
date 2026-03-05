use crate::{AppError, AppResult};
use axum::Json;
use serde::Serialize;

/// Wrap data in `{ "data": <value> }` envelope.
/// For handlers returning `AppResult<Json<serde_json::Value>>`.
pub fn ok_data(data: impl Serialize) -> AppResult<Json<serde_json::Value>> {
    let value = serde_json::to_value(data)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Serialization error: {}", e)))?;
    Ok(Json(serde_json::json!({ "data": value })))
}

/// Non-Result variant for public endpoints (health, version).
pub fn json_data(data: impl Serialize) -> Json<serde_json::Value> {
    let value = serde_json::to_value(data).unwrap_or(serde_json::Value::Null);
    Json(serde_json::json!({ "data": value }))
}
