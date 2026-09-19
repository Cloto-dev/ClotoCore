//! Choosing an agent's model and reasoning effort within a range set elsewhere.
//!
//! An engine that reads per-agent settings keeps them under the agent's
//! metadata key named after the engine (`metadata["cli_agent"]` for the
//! `cli_agent` engine), as a JSON object. An operator — or a panel acting for
//! one — may change only two fields of that object, `model` and `effort`, and
//! only to values listed in the same object's `allowed_models` and
//! `allowed_efforts`. The range is written by whoever owns the desired state
//! (an organisation ledger, reviewed as code); this route is the everyday
//! selection inside it. Widening the range is not possible from here.
//!
//! Why a route of its own rather than `POST /api/agents/{id}`: that one replaces
//! name, engine and the whole metadata map. A panel given it could rename the
//! agent or point it at another engine, and a panel write is pinned to exact
//! routes, so the only way to give a panel "change the model" is a route that
//! can do nothing else.
//!
//! The write is a compare-and-set on the binding as it was read, so a
//! concurrent change (the ledger being re-applied, another tab) is reported as a
//! conflict instead of being silently overwritten.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use tracing::warn;

use super::{check_auth, ok_data};
use crate::{AppError, AppResult, AppState};

/// Longest note a selection may carry, in characters (same bound as a panel
/// write consent note).
const NOTE_MAX_CHARS: usize = 200;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionRequest {
    pub model: String,
    pub effort: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// The previous and the new selection, as recorded.
#[derive(Debug, PartialEq, Eq)]
pub struct Selection {
    pub binding: String,
    pub previous_model: String,
    pub previous_effort: String,
}

/// Apply a selection to one binding (the JSON object stored under the engine's
/// metadata key). Pure: the whole decision is here, so it is tested without a
/// database.
pub fn apply_selection(binding: &str, model: &str, effort: &str) -> Result<Selection, String> {
    let mut object: serde_json::Map<String, serde_json::Value> = serde_json::from_str(binding)
        .map_err(|_| "the agent's engine settings are not a JSON object".to_string())?;
    let list = |key: &str| -> Result<Vec<String>, String> {
        match object.get(key) {
            Some(serde_json::Value::Array(items)) if !items.is_empty() => items
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| format!("`{key}` must list strings"))
                })
                .collect(),
            _ => Err(format!(
                "the agent has no `{key}` range, so nothing can be chosen here; its range is set with the organisation ledger"
            )),
        }
    };
    let models = list("allowed_models")?;
    let efforts = list("allowed_efforts")?;
    if !models.iter().any(|m| m == model) {
        return Err(format!(
            "model {model:?} is outside this agent's range ({})",
            models.join(", ")
        ));
    }
    if !efforts.iter().any(|e| e == effort) {
        return Err(format!(
            "effort {effort:?} is outside this agent's range ({})",
            efforts.join(", ")
        ));
    }
    let previous = |key: &str| {
        object
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let previous_model = previous("model");
    let previous_effort = previous("effort");
    object.insert("model".into(), model.into());
    object.insert("effort".into(), effort.into());
    Ok(Selection {
        binding: serde_json::Value::Object(object).to_string(),
        previous_model,
        previous_effort,
    })
}

/// Reasoning effort words from least to most (the words the `cli_agent`
/// engine accepts). Used only to find the bottom of a range.
const EFFORT_ORDER: [&str; 6] = ["low", "medium", "high", "xhigh", "max", "ultra"];

/// The engine settings for a side run the kernel makes on an agent's behalf —
/// a conversation title, an episode summary — derived from the agent's own
/// binding.
///
/// Without them the engine falls back to the harness's own default model,
/// which is whatever the harness ships with rather than anything the agent's
/// range allows (on a host with no harness config that was the most expensive
/// model the account can run, once per title and several times per archived
/// episode). So the side run keeps the agent's model and harness, and takes the
/// lowest effort the range allows: a title does not need the reasoning a
/// manager's own work does. The working directory is left out on purpose — it
/// holds the agent's brief, which a harness reads as instructions and which has
/// nothing to do with naming a conversation — and so is the subagent policy.
///
/// `None` when the binding is not a JSON object; the caller then sends no
/// settings, as before.
pub fn side_run_binding(binding: &str) -> Option<String> {
    let object: serde_json::Map<String, serde_json::Value> = serde_json::from_str(binding).ok()?;
    let mut side = serde_json::Map::new();
    for key in ["harness", "model", "allowed_models", "allowed_efforts"] {
        if let Some(value) = object.get(key) {
            side.insert(key.into(), value.clone());
        }
    }
    let rank = |effort: &str| {
        EFFORT_ORDER
            .iter()
            .position(|known| *known == effort)
            .unwrap_or(usize::MAX)
    };
    let floor = object
        .get("allowed_efforts")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .min_by_key(|effort| rank(effort))
        })
        .or_else(|| object.get("effort").and_then(serde_json::Value::as_str));
    if let Some(effort) = floor {
        side.insert("effort".into(), effort.into());
    }
    Some(serde_json::Value::Object(side).to_string())
}

/// Compare-and-set: replace the binding only if it still reads `old`. False
/// when something else changed it after it was read.
pub async fn write_if_unchanged(
    pool: &sqlx::SqlitePool,
    agent_id: &str,
    engine_id: &str,
    old: &str,
    new: &str,
) -> Result<bool, sqlx::Error> {
    let path = format!("$.\"{engine_id}\"");
    let written = sqlx::query(
        "UPDATE agents SET metadata = json_set(metadata, ?1, ?2) \
         WHERE id = ?3 AND json_extract(metadata, ?1) = ?4",
    )
    .bind(&path)
    .bind(new)
    .bind(agent_id)
    .bind(old)
    .execute(pool)
    .await?;
    Ok(written.rows_affected() == 1)
}

fn checked_note(note: Option<String>) -> Result<Option<String>, AppError> {
    let Some(note) = note.map(|n| n.trim().to_string()) else {
        return Ok(None);
    };
    if note.is_empty() {
        return Ok(None);
    }
    if note.chars().count() > NOTE_MAX_CHARS || note.chars().any(char::is_control) {
        return Err(AppError::Validation(format!(
            "a note is one line of at most {NOTE_MAX_CHARS} characters"
        )));
    }
    Ok(Some(note))
}

/// POST /api/agents/:id/engine-selection — choose the model and reasoning
/// effort an agent runs on, within the range its engine settings carry.
///
/// **Route:** `POST /api/agents/{id}/engine-selection`
///
/// Body: `{"model": "...", "effort": "...", "note": "..."}` (note optional,
/// written to the audit row). 400 outside the range or without one, 404 for an
/// unknown agent, 409 when the settings changed between read and write.
pub async fn select_engine(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Json(req): Json<SelectionRequest>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let note = checked_note(req.note)?;

    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT default_engine_id, json_extract(metadata, '$.\"' || default_engine_id || '\"') \
         FROM agents WHERE id = ?",
    )
    .bind(&agent_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| AppError::Internal(e.into()))?;
    let Some((engine_id, binding)) = row else {
        return Err(AppError::NotFound(format!("Agent {agent_id} not found")));
    };
    let Some(binding) = binding.filter(|b| !b.trim().is_empty()) else {
        return Err(AppError::Validation(format!(
            "agent {agent_id} has no settings for its engine {engine_id:?}, so nothing can be chosen here"
        )));
    };
    let selection =
        apply_selection(&binding, &req.model, &req.effort).map_err(AppError::Validation)?;

    if !write_if_unchanged(
        &state.pool,
        &agent_id,
        &engine_id,
        &binding,
        &selection.binding,
    )
    .await
    .map_err(|e| AppError::Internal(e.into()))?
    {
        return Err(AppError::Conflict(format!(
            "agent {agent_id}'s engine settings changed while this was being applied; read them again"
        )));
    }

    let entry = crate::db::AuditLogEntry {
        timestamp: chrono::Utc::now(),
        event_type: "AGENT_ENGINE_SELECTED".to_string(),
        actor_id: Some("operator".to_string()),
        target_id: Some(agent_id.clone()),
        permission: None,
        result: "changed".to_string(),
        reason: match &note {
            Some(note) => format!(
                "{} / {} -> {} / {} ({note})",
                selection.previous_model, selection.previous_effort, req.model, req.effort
            ),
            None => format!(
                "{} / {} -> {} / {}",
                selection.previous_model, selection.previous_effort, req.model, req.effort
            ),
        },
        metadata: Some(serde_json::json!({
            "engine": engine_id,
            "from": { "model": selection.previous_model, "effort": selection.previous_effort },
            "to": { "model": req.model, "effort": req.effort },
            "note": note,
        })),
        trace_id: None,
    };
    if let Err(e) = crate::db::write_audit_log(&state.pool, entry).await {
        warn!("agent {agent_id}: audit write failed for an engine selection: {e}");
    }

    ok_data(serde_json::json!({
        "agent_id": agent_id,
        "model": req.model,
        "effort": req.effort,
        "previous": { "model": selection.previous_model, "effort": selection.previous_effort },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use std::collections::HashMap;

    const RANGED: &str = r#"{"harness":"codex","cwd":"/w","model":"gpt-5.6-sol","effort":"xhigh","allowed_models":["gpt-5.6-sol","gpt-5.6-luna"],"allowed_efforts":["high","xhigh","max"]}"#;

    fn field(binding: &str, key: &str) -> serde_json::Value {
        serde_json::from_str::<serde_json::Value>(binding).unwrap()[key].clone()
    }

    #[test]
    fn a_side_run_keeps_the_model_and_takes_the_bottom_of_the_range() {
        let side = side_run_binding(RANGED).expect("a JSON object");
        assert_eq!(field(&side, "model"), "gpt-5.6-sol");
        assert_eq!(field(&side, "harness"), "codex");
        assert_eq!(field(&side, "effort"), "high");
        // The range travels too, so the engine's own range check still applies.
        assert_eq!(
            field(&side, "allowed_efforts"),
            serde_json::json!(["high", "xhigh", "max"])
        );
        // The brief's directory and the subagent policy do not.
        assert_eq!(field(&side, "cwd"), serde_json::Value::Null);
    }

    #[test]
    fn the_bottom_of_the_range_is_by_effort_not_by_list_order() {
        let binding = r#"{"model":"m","effort":"max","allowed_efforts":["xhigh","max","medium"],"subagent_model":"s","subagent_effort_min":"high"}"#;
        let side = side_run_binding(binding).expect("a JSON object");
        assert_eq!(field(&side, "effort"), "medium");
        assert_eq!(field(&side, "subagent_model"), serde_json::Value::Null);
        assert_eq!(field(&side, "subagent_effort_min"), serde_json::Value::Null);
    }

    #[test]
    fn without_a_range_a_side_run_keeps_the_agents_effort() {
        let side = side_run_binding(r#"{"model":"m","effort":"low","cwd":"/w"}"#).expect("object");
        assert_eq!(field(&side, "effort"), "low");
        assert_eq!(field(&side, "model"), "m");
        assert_eq!(side_run_binding("not json"), None);
    }

    #[test]
    fn a_selection_inside_the_range_changes_only_model_and_effort() {
        let s = apply_selection(RANGED, "gpt-5.6-luna", "high").unwrap();
        assert_eq!(field(&s.binding, "model"), "gpt-5.6-luna");
        assert_eq!(field(&s.binding, "effort"), "high");
        assert_eq!(field(&s.binding, "harness"), "codex");
        assert_eq!(field(&s.binding, "cwd"), "/w");
        assert_eq!(
            field(&s.binding, "allowed_models"),
            field(RANGED, "allowed_models")
        );
        assert_eq!(
            (s.previous_model.as_str(), s.previous_effort.as_str()),
            ("gpt-5.6-sol", "xhigh")
        );
    }

    #[test]
    fn a_selection_outside_the_range_is_refused() {
        let e = apply_selection(RANGED, "claude-opus-5", "high").unwrap_err();
        assert!(e.contains("model \"claude-opus-5\" is outside"), "{e}");
        let e = apply_selection(RANGED, "gpt-5.6-luna", "medium").unwrap_err();
        assert!(e.contains("effort \"medium\" is outside"), "{e}");
    }

    #[test]
    fn an_empty_range_is_reported_as_no_range_not_as_outside_it() {
        // The operator's next step differs: an empty list is set in the ledger.
        let e = apply_selection(
            r#"{"allowed_models":[],"allowed_efforts":["high"]}"#,
            "m",
            "high",
        )
        .unwrap_err();
        assert!(e.contains("has no `allowed_models` range"), "{e}");
    }

    #[test]
    fn without_a_range_nothing_can_be_chosen() {
        for binding in [
            r#"{"model":"m","effort":"high"}"#,
            r#"{"allowed_models":[],"allowed_efforts":["high"]}"#,
            r#"{"allowed_models":["m"],"allowed_efforts":"high"}"#,
            r#"{"allowed_models":["m",1],"allowed_efforts":["high"]}"#,
            "not json",
            "[]",
        ] {
            assert!(apply_selection(binding, "m", "high").is_err(), "{binding}");
        }
    }

    async fn state_with(binding: Option<&str>) -> (Arc<AppState>, String) {
        let state = crate::test_utils::create_test_app_state(Some("k".into())).await;
        let mut meta = HashMap::new();
        if let Some(b) = binding {
            meta.insert("cli_agent".to_string(), b.to_string());
        }
        meta.insert("preferred_memory".to_string(), "cpersona".to_string());
        let id = state
            .agent_manager
            .create_agent("Selector", "d", "cli_agent", meta, vec![], None)
            .await
            .unwrap();
        (state, id)
    }

    fn headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-api-key", "k".parse().unwrap());
        h
    }

    async fn select(state: &Arc<AppState>, id: &str, model: &str, effort: &str) -> StatusCode {
        let req = SelectionRequest {
            model: model.into(),
            effort: effort.into(),
            note: Some("from the console".into()),
        };
        match select_engine(
            State(state.clone()),
            headers(),
            Path(id.to_string()),
            Json(req),
        )
        .await
        {
            Ok(r) => r.into_response().status(),
            Err(e) => e.into_response().status(),
        }
    }

    async fn metadata(state: &AppState, id: &str) -> HashMap<String, String> {
        let raw: String = sqlx::query_scalar("SELECT metadata FROM agents WHERE id = ?")
            .bind(id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
        serde_json::from_str(&raw).unwrap()
    }

    #[tokio::test]
    async fn the_route_writes_the_selection_keeps_other_metadata_and_audits() {
        let (state, id) = state_with(Some(RANGED)).await;
        assert_eq!(
            select(&state, &id, "gpt-5.6-luna", "max").await,
            StatusCode::OK
        );
        let meta = metadata(&state, &id).await;
        assert_eq!(meta["preferred_memory"], "cpersona");
        assert_eq!(field(&meta["cli_agent"], "model"), "gpt-5.6-luna");
        assert_eq!(field(&meta["cli_agent"], "effort"), "max");
        let (reason, meta_json): (String, String) = sqlx::query_as(
            "SELECT reason, metadata FROM audit_logs WHERE event_type = 'AGENT_ENGINE_SELECTED'",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(
            reason,
            "gpt-5.6-sol / xhigh -> gpt-5.6-luna / max (from the console)"
        );
        let audit: serde_json::Value = serde_json::from_str(&meta_json).unwrap();
        assert_eq!(audit["from"]["model"], "gpt-5.6-sol");
        assert_eq!(audit["to"]["effort"], "max");
    }

    #[tokio::test]
    async fn the_route_refuses_outside_the_range_and_writes_nothing() {
        let (state, id) = state_with(Some(RANGED)).await;
        assert_eq!(
            select(&state, &id, "gpt-5.6-luna", "low").await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(metadata(&state, &id).await["cli_agent"], RANGED);
    }

    #[tokio::test]
    async fn the_route_answers_404_and_400_for_a_missing_agent_or_binding() {
        let (state, _) = state_with(None).await;
        assert_eq!(
            select(&state, "agent.nobody", "m", "high").await,
            StatusCode::NOT_FOUND
        );
        let (state, id) = state_with(None).await;
        assert_eq!(
            select(&state, &id, "m", "high").await,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn a_binding_changed_after_it_was_read_is_a_conflict() {
        let (state, id) = state_with(Some(RANGED)).await;
        let stale = RANGED.replace("\"xhigh\",\"allowed", "\"high\",\"allowed");
        assert!(
            !write_if_unchanged(&state.pool, &id, "cli_agent", &stale, "{}")
                .await
                .unwrap()
        );
        assert_eq!(metadata(&state, &id).await["cli_agent"], RANGED);
        assert!(
            write_if_unchanged(&state.pool, &id, "cli_agent", RANGED, r#"{"x":1}"#)
                .await
                .unwrap()
        );
        let meta = metadata(&state, &id).await;
        assert_eq!(meta["cli_agent"], r#"{"x":1}"#);
        assert_eq!(meta["preferred_memory"], "cpersona");
    }

    /// The router is built inline at boot; this reads the registration itself.
    #[test]
    fn the_route_is_registered() {
        let source = include_str!("../lib.rs");
        assert!(source.contains("post(handlers::engine_selection::select_engine)"));
    }
}
