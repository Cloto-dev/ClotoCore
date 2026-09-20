//! Operator routes for the hub access token: set (bind), status, renew, forget.
//!
//! Design: `docs/HUB_ACCESS_DESIGN.md`. The token and the key it is bound to
//! are handled in [`crate::managers::hub_access`]; this is the HTTP surface
//! over it, and the notices that tell the operator when to renew.
//!
//! **Operator only.** Every route here requires the admin credential and is
//! refused outright to a request carrying an agent token, even alongside the
//! admin key: renewing or replacing the kernel's hub credential is not
//! something an agent may do on anyone's behalf. The panel write gate refuses
//! these routes as well (`panel_writes::DENIED_WRITE_PREFIXES`), so a panel
//! cannot declare them.

use std::sync::Arc;

use axum::{extract::State, http::HeaderMap, Json};
use chrono::{DateTime, Utc};
use cloto_shared::McpLogLevel;

use super::{check_auth, ok_data};
use crate::db::{NotificationItem, NotificationKind};
use crate::managers::hub_access::{self, AccessError, AccessStatus, ExpiryStage};
use crate::{AppError, AppResult, AppState};

/// Every notice this module raises has an id under this prefix, so renewing
/// or forgetting the token can settle all of them at once.
pub(crate) const NOTICE_PREFIX: &str = "hub-access:";

/// Where in the dashboard the operator acts on these notices. Carried in the
/// notice's metadata as `link`, an in-app path the bell offers to open.
// HARDCODED(dashboard/src/components/SettingsView.tsx::sectionFromQuery): the
// dashboard owns its routes; the kernel names one so a notice can lead to it.
pub(crate) const SETTINGS_LINK: &str = "/settings?section=security#hub-access";

/// Refuse a request that carries an agent token. Checked before the admin
/// key, so the answer does not depend on whether the key was also sent.
fn refuse_agent(headers: &HeaderMap) -> AppResult<()> {
    if headers.contains_key(crate::managers::agent_token::AGENT_TOKEN_HEADER) {
        return Err(AppError::Cloto(cloto_shared::ClotoError::PermissionDenied(
            cloto_shared::Permission::AdminAccess,
        )));
    }
    Ok(())
}

fn operator_only(state: &AppState, headers: &HeaderMap) -> AppResult<()> {
    refuse_agent(headers)?;
    check_auth(state, headers)
}

fn require_hub(hub_base: Option<String>) -> AppResult<String> {
    hub_base.ok_or_else(|| AppError::Conflict(AccessError::NoHub.to_string()))
}

fn to_app_error(e: AccessError) -> AppError {
    match e {
        AccessError::NotAnAccessToken => AppError::Validation(e.to_string()),
        AccessError::NoToken => AppError::NotFound(e.to_string()),
        AccessError::Other(inner) => AppError::Internal(inner),
        AccessError::NoHub
        | AccessError::OtherHub { .. }
        | AccessError::Expired
        | AccessError::Refused
        | AccessError::Hub(_)
        | AccessError::Mismatch(_) => AppError::Conflict(e.to_string()),
    }
}

/// The token changed: the catalog view it produced is no longer the right one,
/// and every notice about the old token is settled.
async fn token_changed(state: &AppState, decision: &str) {
    {
        let mut cache = state.marketplace_cache.write().await;
        cache.data = None;
        cache.fetched_at = None;
    }
    if let Err(e) =
        crate::db::resolve_notifications_with_prefix(&state.pool, NOTICE_PREFIX, decision).await
    {
        tracing::warn!(error = %e, "failed to settle hub access notices");
    }
}

fn status_body(status: Option<AccessStatus>) -> serde_json::Value {
    serde_json::json!({ "token": status })
}

/// GET /api/hub-access — what the stored token opens and when it expires.
/// Never the token itself.
pub async fn get_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    operator_only(&state, &headers)?;
    let status = hub_access::status(&state.data_dir, Utc::now()).map_err(AppError::Internal)?;
    ok_data(status_body(status))
}

#[derive(Debug, serde::Deserialize)]
pub struct SetTokenBody {
    pub token: String,
}

/// POST /api/hub-access/token — bind a token issued on the hub to this kernel
/// and keep it. Replaces a token already stored.
pub async fn set_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SetTokenBody>,
) -> AppResult<Json<serde_json::Value>> {
    operator_only(&state, &headers)?;
    set_token_on(&state, super::marketplace::hub_base(), &body.token).await
}

async fn set_token_on(
    state: &AppState,
    hub_base: Option<String>,
    token: &str,
) -> AppResult<Json<serde_json::Value>> {
    // A malformed token is refused before the hub is even looked up, so the
    // answer does not depend on how this kernel's catalog is configured.
    if !hub_access::is_access_token(token) {
        return Err(to_app_error(AccessError::NotAnAccessToken));
    }
    let base = require_hub(hub_base)?;
    let status = hub_access::bind(&state.data_dir, &base, token)
        .await
        .map_err(to_app_error)?;
    token_changed(state, "replaced").await;
    tracing::info!(
        token_id = %status.token_id,
        fingerprint = %status.fingerprint,
        connectors = status.connector_ids.len(),
        "hub access token bound"
    );
    ok_data(status_body(Some(status)))
}

/// POST /api/hub-access/renew — exchange the stored token for a new one. The
/// hub revokes the old token as it issues the new one.
pub async fn renew(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    operator_only(&state, &headers)?;
    renew_on(&state, super::marketplace::hub_base()).await
}

async fn renew_on(
    state: &AppState,
    hub_base: Option<String>,
) -> AppResult<Json<serde_json::Value>> {
    if hub_access::status(&state.data_dir, Utc::now())
        .map_err(AppError::Internal)?
        .is_none()
    {
        return Err(to_app_error(AccessError::NoToken));
    }
    let base = require_hub(hub_base)?;
    match hub_access::renew(&state.data_dir, &base).await {
        Ok(status) => {
            token_changed(state, "renewed").await;
            tracing::info!(token_id = %status.token_id, "hub access token renewed");
            ok_data(status_body(Some(status)))
        }
        Err(AccessError::Refused) => {
            report_refused(state, Utc::now()).await;
            Err(to_app_error(AccessError::Refused))
        }
        Err(e) => Err(to_app_error(e)),
    }
}

/// DELETE /api/hub-access/token — forget the token. The kernel's key stays,
/// and installed connectors keep working.
pub async fn forget_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    operator_only(&state, &headers)?;
    let removed = hub_access::forget(&state.data_dir)
        .await
        .map_err(AppError::Internal)?;
    if removed {
        token_changed(&state, "forgotten").await;
    }
    ok_data(serde_json::json!({ "removed": removed }))
}

// ─── Notices ────────────────────────────────────────────────────────────────

fn day(now: DateTime<Utc>) -> String {
    now.format("%Y-%m-%d").to_string()
}

/// The notice owed at `now`, if any: one per token per day, from the moment
/// the renewal window opens (design §7).
pub(crate) fn expiry_notice(status: &AccessStatus, now: DateTime<Utc>) -> Option<NotificationItem> {
    // The date goes into `params` in RFC 3339 and nowhere else: the reader
    // knows the locale, the kernel does not, so `%Y-%m-%d` here would be one
    // spelling imposed on every language. The English `body` keeps it only as
    // the fallback for a reader that cannot use the key.
    let (severity, title, body, key, params) =
        match hub_access::expiry_stage(status.expires_at, now) {
            ExpiryStage::Valid => return None,
            ExpiryStage::ExpiresSoon => (
                McpLogLevel::Warning,
                "The hub access token expires soon",
                format!(
                    "It expires on {}. Renew it in Settings → Security to keep receiving \
                 updates for {} restricted connector(s). Installed connectors are not affected.",
                    status.expires_at.format("%Y-%m-%d"),
                    status.connector_ids.len()
                ),
                "hub_access.expires_soon",
                serde_json::json!({
                    "expires_at": status.expires_at.to_rfc3339(),
                    "n": status.connector_ids.len(),
                }),
            ),
            ExpiryStage::Expired => (
                McpLogLevel::Error,
                "The hub access token has expired",
                "Restricted connectors can no longer be updated. Installed ones keep working. \
             An expired token cannot be renewed; ask for a new one to be issued on the hub \
             and set it in Settings → Security."
                    .to_string(),
                "hub_access.expired",
                serde_json::json!({}),
            ),
        };
    Some(
        NotificationItem::new(
            format!("{NOTICE_PREFIX}expiry:{}:{}", status.token_id, day(now)),
            NotificationKind::Notice,
            severity,
            title,
        )
        .body(body)
        .message(key, params)
        .metadata(serde_json::json!({
            "token_id": status.token_id,
            "expires_at": status.expires_at,
            "action": "renew",
            "link": SETTINGS_LINK,
        })),
    )
}

/// Raise today's expiry notice if one is owed, settling the ones from earlier
/// days so the bell carries one, not one per day.
pub async fn check_expiry(state: &AppState, now: DateTime<Utc>) {
    let status = match hub_access::status(&state.data_dir, now) {
        Ok(Some(status)) => status,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(error = %e, "hub access token is unreadable");
            return;
        }
    };
    let Some(item) = expiry_notice(&status, now) else {
        return;
    };
    raise_daily(state, item, &format!("{NOTICE_PREFIX}expiry:")).await;
}

/// The hub answered 401 to this kernel's token: it is expired, revoked, or
/// bound elsewhere. A notice, not an install failure (design §6).
pub async fn report_refused(state: &AppState, now: DateTime<Utc>) {
    let token_id = hub_access::status(&state.data_dir, now)
        .ok()
        .flatten()
        .map_or_else(|| "unknown".to_string(), |s| s.token_id);
    let item = NotificationItem::new(
        format!("{NOTICE_PREFIX}refused:{token_id}:{}", day(now)),
        NotificationKind::Notice,
        McpLogLevel::Error,
        "The hub refused this kernel's access token",
    )
    .body(
        "Restricted connectors are hidden from the catalog and cannot be updated until a \
         valid token is set in Settings → Security. Installed ones keep working.",
    )
    .message("hub_access.refused", serde_json::json!({}))
    .metadata(serde_json::json!({ "token_id": token_id, "link": SETTINGS_LINK }));
    raise_daily(state, item, &format!("{NOTICE_PREFIX}refused:")).await;
}

async fn raise_daily(state: &AppState, item: NotificationItem, family: &str) {
    let item_id = item.item_id.clone();
    match crate::db::record_notification_once(&state.pool, item).await {
        Ok(true) => {
            if let Err(e) = crate::db::resolve_notifications_with_prefix_except(
                &state.pool,
                family,
                &item_id,
                "superseded",
            )
            .await
            {
                tracing::warn!(error = %e, "failed to settle earlier hub access notices");
            }
        }
        Ok(false) => {}
        Err(e) => tracing::warn!(item_id = %item_id, error = %e, "failed to record notice"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn status(expires_at: &str) -> AccessStatus {
        AccessStatus {
            token_prefix: "chubr_abcd…".into(),
            token_id: "T1".into(),
            connector_ids: vec!["acme-panel".into()],
            fingerprint: "fp".into(),
            expires_at: at(expires_at),
            hub_origin: "https://hub.example".into(),
            stage: ExpiryStage::Valid,
        }
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn no_notice_before_thirty_days_one_per_day_after() {
        let s = status("2026-12-19T00:00:00Z");
        assert!(expiry_notice(&s, at("2026-11-18T23:59:59Z")).is_none());
        let first = expiry_notice(&s, at("2026-11-19T00:00:00Z")).expect("window open");
        let later = expiry_notice(&s, at("2026-11-19T20:00:00Z")).unwrap();
        let next_day = expiry_notice(&s, at("2026-11-20T00:00:00Z")).unwrap();
        assert_eq!(first.item_id, later.item_id, "same day, same notice");
        assert_ne!(first.item_id, next_day.item_id, "re-raised the next day");
        assert!(first.item_id.starts_with(NOTICE_PREFIX));
        assert_eq!(first.severity, McpLogLevel::Warning);
        assert_eq!(
            first.metadata.as_ref().unwrap()["link"],
            SETTINGS_LINK,
            "the notice leads to where it can be acted on"
        );
        let expired = expiry_notice(&s, at("2026-12-19T00:00:00Z")).unwrap();
        assert_eq!(expired.severity, McpLogLevel::Error);
    }

    /// Each stage carries its own key, and the date the reader has to spell
    /// travels in `params` unformatted.
    ///
    /// The two stages are checked against each other, not only against a
    /// literal: a producer that keyed both the same way would still read as
    /// "keyed" to a grep, and the bell would then show the expiring wording to
    /// someone whose token already expired.
    #[test]
    fn each_expiry_stage_carries_its_own_key_and_an_unformatted_date() {
        let s = status("2026-12-19T00:00:00Z");
        let soon = expiry_notice(&s, at("2026-11-19T00:00:00Z")).unwrap();
        let expired = expiry_notice(&s, at("2026-12-19T00:00:00Z")).unwrap();

        let message = |item: &NotificationItem| item.metadata.as_ref().unwrap()["message"].clone();
        let soon_msg = message(&soon);
        let expired_msg = message(&expired);

        assert_eq!(soon_msg["key"], "hub_access.expires_soon");
        assert_eq!(expired_msg["key"], "hub_access.expired");
        assert_ne!(soon_msg["key"], expired_msg["key"]);

        assert_eq!(
            soon_msg["params"]["expires_at"], "2026-12-19T00:00:00+00:00",
            "the date is handed over in RFC 3339 for the reader to format"
        );
        assert_eq!(soon_msg["params"]["n"], 1);

        // The English text stays put as the fallback, and `.metadata()` runs
        // after `.message()` at this site — so this also holds that the later
        // call did not drop the key.
        assert_eq!(soon.title, "The hub access token expires soon");
        assert_eq!(
            soon.metadata.as_ref().unwrap()["link"],
            SETTINGS_LINK,
            "the other metadata is still there beside the message"
        );
    }

    #[tokio::test]
    async fn every_route_refuses_an_agent_token_even_with_the_admin_key() {
        let state = crate::test_utils::create_test_app_state(Some("admin-key".into())).await;
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", HeaderValue::from_static("admin-key"));
        headers.insert(
            crate::managers::agent_token::AGENT_TOKEN_HEADER,
            HeaderValue::from_static("anything"),
        );
        let forbidden = |r: AppResult<Json<serde_json::Value>>| {
            matches!(
                r,
                Err(AppError::Cloto(cloto_shared::ClotoError::PermissionDenied(
                    _
                )))
            )
        };
        assert!(forbidden(
            get_status(State(state.clone()), headers.clone()).await
        ));
        assert!(forbidden(
            set_token(
                State(state.clone()),
                headers.clone(),
                Json(SetTokenBody {
                    token: "chubr_x".into()
                })
            )
            .await
        ));
        assert!(forbidden(
            renew(State(state.clone()), headers.clone()).await
        ));
        assert!(forbidden(
            forget_token(State(state.clone()), headers.clone()).await
        ));

        // The same request without the agent token is the operator's.
        headers.remove(crate::managers::agent_token::AGENT_TOKEN_HEADER);
        let Ok(ok) = get_status(State(state), headers).await else {
            panic!("the operator is let through");
        };
        assert_eq!(ok.0["data"]["token"], serde_json::Value::Null);
    }

    /// Both answers are decided before the hub is looked up, so they hold
    /// however this kernel's catalog is configured, with no network.
    #[tokio::test]
    async fn a_malformed_token_and_a_missing_one_are_answered_without_a_hub() {
        let dir = std::env::temp_dir().join(format!(
            "cloto-hub-access-handler-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let state =
            crate::test_utils::create_test_app_state_in(dir.clone(), Some("admin-key".into()))
                .await;
        for bad in ["not-an-access-token", "chub_abcdef", "chubr_", ""] {
            let r = set_token_on(&state, None, bad).await;
            assert!(
                matches!(r, Err(AppError::Validation(_))),
                "{bad:?} must be a 400"
            );
        }
        let r = renew_on(&state, None).await;
        assert!(
            matches!(r, Err(AppError::NotFound(_))),
            "nothing to renew is a 404"
        );
        // With no hub configured, a well-formed token is a conflict, not a 400.
        let r = set_token_on(&state, None, &format!("chubr_{}", "ab".repeat(32))).await;
        assert!(matches!(r, Err(AppError::Conflict(_))));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The router is built inline at boot, so no handler test can show the
    /// routes are mounted. This reads the registration itself.
    #[test]
    fn the_access_routes_are_registered() {
        let source = include_str!("../lib.rs");
        for needle in [
            "get(handlers::hub_access::get_status)",
            "post(handlers::hub_access::set_token)",
            ".delete(handlers::hub_access::forget_token)",
            "post(handlers::hub_access::renew)",
            "handlers::hub_access::check_expiry(",
        ] {
            assert!(source.contains(needle), "lib.rs must register {needle}");
        }
    }
}
