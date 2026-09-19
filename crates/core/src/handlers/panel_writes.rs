//! The panel write gate: which connector panels may change kernel state, through
//! which routes, and where that is enforced.
//!
//! Design and reasoning: `docs/PANEL_WRITE_GATE_DESIGN.md`. In short, a panel
//! may write only when all of these hold, and the kernel asks every one of them
//! again on every write:
//!
//! * it was shipped by a connector installed from the marketplace, whose install
//!   receipt records trust `standard` or above and a tree seal;
//! * that seal still verifies against the installed tree, and the panel is
//!   served from inside that tree;
//! * the route is one the panel declared in `writes`, exactly;
//! * the operator consented to that exact list for that connector version.
//!
//! The browser keeps its own check (`dashboard/src/lib/moduleBridge.ts`) as a
//! first filter, but the rule is here so that it holds even if the dashboard
//! gets it wrong, and so that the audit row is written by what did the write.
//!
//! The trust level is read from the install receipt, not from the server
//! handle's `NegotiatedMgp::trust_level`. That value comes from the handshake
//! and never sees the seal downgrade, so a connector could unlock writes by
//! declaring `core` about itself. A connector that ships only panels has no
//! handle at all.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::Path as StdPath;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    body::{Body, Bytes},
    extract::{ConnectInfo, Path, State},
    http::{header, HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use tracing::warn;

use super::modules::FoundModule;
use super::{check_auth, ok_data};
use crate::managers::mcp_mgp::TrustLevel;
use crate::{AppError, AppResult, AppState};

/// Methods a panel may declare in `writes`.
// HARDCODED(docs/PANEL_WRITE_GATE_DESIGN.md §4.2): the design admits these two
// and names DELETE and PUT as out of scope for this version.
pub const WRITE_METHODS: &[&str] = &["POST", "PATCH"];

/// Lowest install-time trust level that may write.
// HARDCODED(docs/PANEL_WRITE_GATE_DESIGN.md §3): the threshold the design
// settled on; it is a policy of this gate, not a value another component owns.
const MINIMUM_TRUST: TrustLevel = TrustLevel::Standard;

/// Largest request body a panel may send through the relay.
// HARDCODED(docs/PANEL_WRITE_GATE_DESIGN.md §4.4): check 4.
const MAX_WRITE_BODY_BYTES: usize = 64 * 1024;

/// Writes one panel may make inside [`RATE_WINDOW`].
// HARDCODED(docs/PANEL_WRITE_GATE_DESIGN.md §4.4): check 5; §7 records that
// the number is a starting value, not a measured one.
const WRITES_PER_WINDOW: usize = 30;
const RATE_WINDOW: Duration = Duration::from_secs(60);

/// Route prefixes a panel may never declare in `writes`, whatever else holds.
/// A prefix covers itself and everything below it, segment-wise:
/// `/api/modules` refuses `/api/modules/x/write`, not `/api/modulesx`.
///
/// - `/api/modules` — a write routed back into this gate would let one
///   consent stand in for another panel's.
/// - `/api/hub-access` — the kernel's hub credential. Renewing or replacing it
///   is the operator's act alone (`docs/HUB_ACCESS_DESIGN.md` §4); relaying it
///   for a panel would let a panel act as the operator toward the hub.
// HARDCODED(docs/HUB_ACCESS_DESIGN.md §4): the list is this gate's policy; each
// entry names routes this kernel mounts in lib.rs.
pub const DENIED_WRITE_PREFIXES: &[&str] = &["/api/modules", "/api/hub-access"];

fn is_denied(path: &str) -> bool {
    DENIED_WRITE_PREFIXES.iter().any(|prefix| {
        path == *prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// Who is recorded as acting. The kernel has one operator — anything that
/// authenticates to `/api` is it — so there is no finer identity to record.
const OPERATOR: &str = "operator";

/// State the gate keeps for the life of the process.
#[derive(Default)]
pub struct PanelWriteState {
    /// The kernel's own router, set once it is built, so an admitted write is
    /// dispatched in-process through the same layers as any other request.
    router: OnceLock<Router>,
    /// Recent write times per panel, for the rate cap.
    recent: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl PanelWriteState {
    /// Called once, after the router is built. A second call is ignored: the
    /// router a running kernel dispatches through does not change.
    pub fn install_router(&self, router: Router) {
        let _ = self.router.set(router);
    }

    /// Count one write against the panel's window, or refuse it. A poisoned
    /// lock refuses: the cap is a safety limit, so it fails closed.
    fn admit(&self, panel_id: &str, now: Instant) -> bool {
        let Ok(mut recent) = self.recent.lock() else {
            return false;
        };
        let times = recent.entry(panel_id.to_string()).or_default();
        while times
            .front()
            .is_some_and(|t| now.duration_since(*t) >= RATE_WINDOW)
        {
            times.pop_front();
        }
        if times.len() >= WRITES_PER_WINDOW {
            return false;
        }
        times.push_back(now);
        true
    }
}

// ─── Declarations ───────────────────────────────────────────────────────────

/// Check a panel's `writes` list; the error names the first bad entry and why.
pub(crate) fn validate_writes(writes: &[String]) -> Result<(), String> {
    for entry in writes {
        parse_write(entry).map_err(|why| format!("{entry:?}: {why}"))?;
    }
    Ok(())
}

/// Split one declaration into method and path, refusing anything that is not
/// one exact route this gate is willing to relay.
fn parse_write(entry: &str) -> Result<(&str, &str), &'static str> {
    let (method, path) = entry
        .split_once(' ')
        .ok_or("is not written as \"<METHOD> <path>\"")?;
    if !WRITE_METHODS.contains(&method) {
        return Err("uses a method other than POST or PATCH");
    }
    if !path.starts_with("/api/") {
        return Err("is not an /api/ path");
    }
    if is_denied(path) {
        return Err("targets a route no panel may write to");
    }
    // Exact means exact: no pattern, no query, and every segment a real one.
    // `.` and `..` are excluded because they read as one place and resolve to
    // another; an empty segment is a doubled or trailing slash.
    if path.contains(['*', '?', '#', ' ', '\\', '%']) {
        return Err("is not one exact path");
    }
    if path
        .split('/')
        .skip(1)
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err("is not one exact path");
    }
    Ok((method, path))
}

/// What a consent is given to: the declared list, order and repeats ignored.
pub(crate) fn writes_digest(writes: &[String]) -> String {
    let mut entries: Vec<&str> = writes.iter().map(String::as_str).collect();
    entries.sort_unstable();
    entries.dedup();
    hex::encode(Sha256::digest(entries.join("\n").as_bytes()))
}

// ─── Eligibility ────────────────────────────────────────────────────────────

/// What an eligible panel was found to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Eligible {
    pub trust_level: TrustLevel,
    pub connector_version: String,
}

fn trust_name(level: TrustLevel) -> String {
    serde_json::to_value(level)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{level:?}"))
}

/// Whether this panel may write at all, answered from the install receipt and
/// the tree on disk. The error is a sentence the operator is shown, completing
/// "This panel cannot write because …".
pub(crate) async fn evaluate_eligibility(
    pool: &sqlx::SqlitePool,
    data_dir: &StdPath,
    servers_root: &StdPath,
    module: &FoundModule,
) -> Result<Eligible, String> {
    let Some(connector) = module.connector.as_deref() else {
        return Err("it was placed by hand, not installed with a connector".into());
    };
    if module.manifest.writes.is_empty() {
        return Err("it declares no writes".into());
    }

    let receipt = crate::db::get_install_receipt(pool, connector)
        .await
        .map_err(|e| {
            warn!("panel {}: install receipt unreadable: {e}", module.id);
            "its install record could not be read".to_string()
        })?
        .ok_or("its connector has no install record; reinstall it from the marketplace")?;

    let trust = TrustLevel::from_str_lossy(&receipt.trust_level);
    if trust < MINIMUM_TRUST {
        return Err(format!(
            "its connector's trust level is {} and writing needs {} or above",
            trust_name(trust),
            trust_name(MINIMUM_TRUST)
        ));
    }

    let seal = receipt
        .seal
        .filter(|s| crate::managers::tree_seal::is_tree_seal(s))
        .ok_or("its connector was installed without a seal")?;

    // The seal covers the installed tree; a panel served from anywhere else
    // would be verified against files it is not made of.
    let install_root = servers_root.join(connector);
    let (Ok(canonical_install), Ok(canonical_panel)) =
        (install_root.canonicalize(), module.root.canonicalize())
    else {
        return Err("its files could not be located".into());
    };
    if !canonical_panel.starts_with(&canonical_install) {
        return Err("it is not served from the tree its install record describes".into());
    }

    let key = mgp_seal::load_or_generate_seal_key(data_dir).map_err(|e| {
        warn!("panel {}: seal key unavailable: {e}", module.id);
        "its seal could not be checked".to_string()
    })?;
    match crate::managers::tree_seal::verify_tree_seal(&install_root, &seal, &key) {
        Ok(true) => {}
        Ok(false) => return Err("its files no longer match the seal recorded at install".into()),
        // Unreadable evidence is not a pass (same rule as the spawn path).
        Err(e) => {
            warn!("panel {}: tree seal not evaluable: {e}", module.id);
            return Err("its seal could not be checked".into());
        }
    }

    Ok(Eligible {
        trust_level: trust,
        connector_version: receipt.version,
    })
}

fn consent_holds(
    consent: &crate::db::PanelWriteConsent,
    digest: &str,
    connector_version: &str,
) -> bool {
    consent.writes_digest == digest && consent.connector_version == connector_version
}

/// The module the gate is asked about, resolved from the roots the gate
/// verifies against — not from wherever the listing happens to look — so the
/// panel checked and the tree sealed are the same files.
fn find_panel(state: &AppState, id: &str) -> Option<FoundModule> {
    let servers_root = super::marketplace::resolve_servers_dir(state);
    super::modules::find_module_in(
        &state.data_dir.join(super::modules::MODULES_DIR),
        Some(&servers_root),
        id,
    )
}

// ─── Audit and refusals ─────────────────────────────────────────────────────

async fn audit(
    state: &AppState,
    event_type: &str,
    panel_id: &str,
    result: &str,
    reason: &str,
    metadata: serde_json::Value,
) {
    let entry = crate::db::AuditLogEntry {
        timestamp: chrono::Utc::now(),
        event_type: event_type.to_string(),
        actor_id: Some(OPERATOR.to_string()),
        target_id: Some(panel_id.to_string()),
        permission: None,
        result: result.to_string(),
        reason: reason.to_string(),
        metadata: Some(metadata),
        trace_id: None,
    };
    if let Err(e) = crate::db::write_audit_log(&state.pool, entry).await {
        warn!("panel {panel_id}: audit write failed for {event_type}: {e}");
    }
}

/// 403 with the reason in the kernel's usual error shape, so the panel can
/// show the operator why.
fn forbidden(message: String) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({
            "error": { "type": "PanelWriteDenied", "message": message }
        })),
    )
        .into_response()
}

async fn deny(
    state: &AppState,
    panel_id: &str,
    reason: String,
    metadata: serde_json::Value,
) -> Response {
    audit(
        state,
        "PANEL_WRITE_DENIED",
        panel_id,
        "denied",
        &reason,
        metadata,
    )
    .await;
    forbidden(format!("This panel cannot write: {reason}"))
}

// ─── Routes ─────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ConsentView {
    granted_at: String,
    /// False when the declared list or the connector version changed since.
    valid: bool,
}

#[derive(Serialize)]
struct WriteAccess {
    panel_id: String,
    writes: Vec<String>,
    eligible: bool,
    /// Why not, when not eligible.
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trust_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    consent: Option<ConsentView>,
}

/// GET /api/modules/:id/write-access — whether a panel may write, and whether
/// the operator has consented to what it declares.
///
/// **Route:** `GET /api/modules/{id}/write-access`
pub async fn get_write_access(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    let module =
        find_panel(&state, &id).ok_or_else(|| AppError::NotFound("Module not found".into()))?;
    let servers_root = super::marketplace::resolve_servers_dir(&state);
    let eligibility =
        evaluate_eligibility(&state.pool, &state.data_dir, &servers_root, &module).await;
    let digest = writes_digest(&module.manifest.writes);
    let consent = crate::db::get_panel_write_consent(&state.pool, &id).await?;

    let (eligible, reason, trust_level, version) = match &eligibility {
        Ok(e) => (
            true,
            None,
            Some(trust_name(e.trust_level)),
            Some(e.connector_version.as_str()),
        ),
        Err(why) => (false, Some(why.clone()), None, None),
    };
    ok_data(WriteAccess {
        panel_id: id,
        writes: module.manifest.writes.clone(),
        eligible,
        reason,
        trust_level,
        consent: consent.map(|c| ConsentView {
            valid: version.is_some_and(|v| consent_holds(&c, &digest, v)),
            granted_at: c.granted_at,
        }),
    })
}

/// PUT /api/modules/:id/write-consent — consent to the panel's declared writes
/// as they are now.
///
/// **Route:** `PUT /api/modules/{id}/write-consent`
///
/// Refused for a panel that is not eligible: a consent that nothing could honour
/// would sit in the list looking like a grant.
///
/// The body is optional. `{"note": "..."}` is written into the audit row, so a
/// consent given by a script (a release tool re-consenting after it shipped a
/// new version) can be told apart from one given on the consent sheet: both
/// hold the same admin credential, so the actor alone cannot say which it was.
/// The note is what the caller declares, not something the kernel verified.
pub async fn put_write_consent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Response, AppError> {
    check_auth(&state, &headers)?;
    let note = consent_note(&body)?;
    let module =
        find_panel(&state, &id).ok_or_else(|| AppError::NotFound("Module not found".into()))?;
    let servers_root = super::marketplace::resolve_servers_dir(&state);
    let eligible =
        match evaluate_eligibility(&state.pool, &state.data_dir, &servers_root, &module).await {
            Ok(e) => e,
            Err(why) => return Ok(forbidden(format!("This panel cannot write: {why}"))),
        };
    let digest = writes_digest(&module.manifest.writes);
    crate::db::put_panel_write_consent(
        &state.pool,
        &id,
        &digest,
        &eligible.connector_version,
        OPERATOR,
    )
    .await?;
    audit(
        &state,
        "PANEL_WRITE_CONSENT_GRANTED",
        &id,
        "granted",
        &match &note {
            Some(note) => format!("operator consented to the declared writes ({note})"),
            None => "operator consented to the declared writes".to_string(),
        },
        serde_json::json!({
            "writes": module.manifest.writes,
            "writes_digest": digest,
            "connector_version": eligible.connector_version,
            "trust_level": trust_name(eligible.trust_level),
            "note": note,
        }),
    )
    .await;
    Ok(ok_data(serde_json::json!({ "panel_id": id, "consented": true }))?.into_response())
}

/// Longest note a consent may carry, in characters. A label, not a document.
const CONSENT_NOTE_MAX_CHARS: usize = 200;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConsentRequest {
    #[serde(default)]
    note: Option<String>,
}

/// The optional note of a consent request. An empty body is no note (the
/// consent sheet sends none); anything else must be the documented shape.
fn consent_note(body: &[u8]) -> Result<Option<String>, AppError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    let req: ConsentRequest = serde_json::from_slice(body)
        .map_err(|e| AppError::Validation(format!("invalid consent request: {e}")))?;
    let Some(note) = req.note.map(|n| n.trim().to_string()) else {
        return Ok(None);
    };
    if note.is_empty() {
        return Ok(None);
    }
    if note.chars().count() > CONSENT_NOTE_MAX_CHARS {
        return Err(AppError::Validation(format!(
            "a consent note is at most {CONSENT_NOTE_MAX_CHARS} characters"
        )));
    }
    if note.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "a consent note is one line of text".to_string(),
        ));
    }
    Ok(Some(note))
}

/// DELETE /api/modules/:id/write-consent — revoke a panel's consent.
///
/// **Route:** `DELETE /api/modules/{id}/write-consent`
///
/// Works whether or not the panel is still installed, so a consent can always
/// be taken back.
pub async fn delete_write_consent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    if !crate::db::delete_panel_write_consent(&state.pool, &id).await? {
        return Err(AppError::NotFound(
            "No consent recorded for this panel".into(),
        ));
    }
    audit(
        &state,
        "PANEL_WRITE_CONSENT_REVOKED",
        &id,
        "revoked",
        "operator revoked the consent",
        serde_json::json!({}),
    )
    .await;
    ok_data(serde_json::json!({ "panel_id": id, "consented": false }))
}

/// GET /api/modules/write-consents — every recorded consent.
///
/// **Route:** `GET /api/modules/write-consents`
pub async fn list_write_consents(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    ok_data(crate::db::list_panel_write_consents(&state.pool).await?)
}

#[derive(Deserialize)]
struct WriteRequest {
    method: String,
    path: String,
    #[serde(default)]
    body: Option<serde_json::Value>,
}

/// POST /api/modules/:id/write — relay one write a panel asked for.
///
/// **Route:** `POST /api/modules/{id}/write`
///
/// Checks, in order: the panel is eligible; the route is declared exactly;
/// a consent to the current declarations exists; the body is JSON within the
/// size cap; the panel is under its rate cap. Every refusal is a 403 with the
/// reason and an audit row. An admitted write is dispatched in-process through
/// the kernel's own router carrying the caller's own headers, so the target
/// authenticates it as it would any request and the relay grants nothing the
/// caller does not already hold.
pub async fn relay_write(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
    raw: Bytes,
) -> Result<Response, AppError> {
    check_auth(&state, &headers)?;

    let request: WriteRequest = serde_json::from_slice(&raw)
        .map_err(|e| AppError::Validation(format!("invalid write request: {e}")))?;
    let asked = serde_json::json!({ "method": request.method, "path": request.path });

    // 1. Eligible — asked now, not remembered from when the panel loaded.
    let Some(module) = find_panel(&state, &id) else {
        return Ok(deny(&state, &id, "it is not an installed panel".into(), asked).await);
    };
    let servers_root = super::marketplace::resolve_servers_dir(&state);
    let eligible =
        match evaluate_eligibility(&state.pool, &state.data_dir, &servers_root, &module).await {
            Ok(e) => e,
            Err(why) => return Ok(deny(&state, &id, why, asked).await),
        };
    let digest = writes_digest(&module.manifest.writes);
    let metadata = serde_json::json!({
        "method": request.method,
        "path": request.path,
        "trust_level": trust_name(eligible.trust_level),
        "seal_verified": true,
        "writes_digest": digest,
    });

    // 2. Declared, exactly.
    let entry = format!("{} {}", request.method, request.path);
    if !module.manifest.writes.contains(&entry) {
        return Ok(deny(
            &state,
            &id,
            format!("{entry} is not one of the writes it declares"),
            metadata,
        )
        .await);
    }

    // 3. Consented to the declarations as they are now.
    let consent = crate::db::get_panel_write_consent(&state.pool, &id).await?;
    match consent {
        Some(c) if consent_holds(&c, &digest, &eligible.connector_version) => {}
        Some(_) => {
            return Ok(deny(
                &state,
                &id,
                "its declared writes or its version changed since the consent was given".into(),
                metadata,
            )
            .await)
        }
        None => {
            return Ok(deny(
                &state,
                &id,
                "the operator has not consented to its writes".into(),
                metadata,
            )
            .await)
        }
    }

    // 4. JSON within the cap.
    let body = match &request.body {
        Some(value) => serde_json::to_vec(value).map_err(|e| AppError::Internal(e.into()))?,
        None => Vec::new(),
    };
    if body.len() > MAX_WRITE_BODY_BYTES {
        return Ok(deny(
            &state,
            &id,
            format!("the request body is larger than {MAX_WRITE_BODY_BYTES} bytes"),
            metadata,
        )
        .await);
    }

    // 5. Under the rate cap.
    if !state.panel_writes.admit(&id, Instant::now()) {
        return Ok(deny(
            &state,
            &id,
            format!("it made more than {WRITES_PER_WINDOW} writes in the last minute"),
            metadata,
        )
        .await);
    }

    let Some(router) = state.panel_writes.router.get() else {
        return Err(AppError::Internal(anyhow::anyhow!(
            "panel write relay has no router"
        )));
    };

    let mut builder = Request::builder()
        .method(request.method.as_str())
        .uri(request.path.as_str());
    for (name, value) in &headers {
        // The framing headers describe the relay request, not this one.
        if name == header::CONTENT_LENGTH || name == header::CONTENT_TYPE || name == header::HOST {
            continue;
        }
        builder = builder.header(name, value);
    }
    if !body.is_empty() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let mut inner = builder
        .body(Body::from(body))
        .map_err(|e| AppError::Internal(e.into()))?;
    inner.extensions_mut().insert(ConnectInfo(addr));

    let response = router
        .clone()
        .oneshot(inner)
        .await
        .map_err(|e: std::convert::Infallible| AppError::Internal(anyhow::anyhow!("{e}")))?;

    let status = response.status();
    let mut recorded = metadata;
    recorded["status"] = serde_json::json!(status.as_u16());
    audit(
        &state,
        "PANEL_WRITE",
        &id,
        if status.is_success() {
            "success"
        } else {
            "failure"
        },
        &format!("{entry} returned {}", status.as_u16()),
        recorded,
    )
    .await;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn exact_post_and_patch_routes_are_accepted() {
        assert!(validate_writes(&w(&[
            "POST /api/chat/agent.manager/messages",
            "PATCH /api/chat/agent.manager/conversations/abc",
        ]))
        .is_ok());
    }

    #[test]
    fn declarations_that_are_not_one_exact_route_are_refused() {
        for bad in [
            "DELETE /api/chat/agent.manager/messages",
            "PUT /api/chat/agent.manager/messages",
            "GET /api/chat/agent.manager/messages",
            "post /api/chat/agent.manager/messages",
            "POST /api/chat/*",
            "POST /api/chat/agent.manager/messages?x=1",
            "POST /api/chat/../system/shutdown",
            "POST /api/chat/./messages",
            "POST /api/chat//messages",
            "POST /api/chat/messages/",
            "POST /api/chat/%2e%2e/system",
            "POST /system/shutdown",
            "POST /api/modules/other-console/write",
            "POST",
        ] {
            assert!(
                validate_writes(&w(&[bad])).is_err(),
                "{bad} should be refused"
            );
        }
    }

    #[test]
    fn every_denied_prefix_is_refused_and_only_on_a_segment_boundary() {
        for bad in [
            "POST /api/modules/other-console/write",
            "PATCH /api/modules/other-console/write-consent",
            "POST /api/hub-access/token",
            "POST /api/hub-access/renew",
            "PATCH /api/hub-access",
        ] {
            assert!(
                validate_writes(&w(&[bad])).is_err(),
                "{bad} should be refused"
            );
        }
        for fine in ["POST /api/hub-accessory/x", "POST /api/modulesx/y"] {
            assert!(
                validate_writes(&w(&[fine])).is_ok(),
                "{fine} is another route"
            );
        }
    }

    #[test]
    fn the_digest_ignores_order_and_repeats_but_not_content() {
        let a = writes_digest(&w(&["POST /api/a", "POST /api/b"]));
        assert_eq!(
            a,
            writes_digest(&w(&["POST /api/b", "POST /api/a", "POST /api/a"]))
        );
        assert_ne!(a, writes_digest(&w(&["POST /api/a"])));
        assert_ne!(a, writes_digest(&w(&["POST /api/a", "PATCH /api/b"])));
    }

    // ─── Fixtures: a real installed tree, a real seal, a real receipt ───────

    const KEY: &str = "test-admin-key";
    const CONNECTOR: &str = "ops-console";
    const PANEL: &str = "ops-console-console";
    const SEND: &str = "POST /api/chat/agent.manager/messages";

    fn temp_data_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "clotocore-panel-writes-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write the connector tree. `extra` lands in the manifest verbatim, so a
    /// test can make the connector claim things about itself.
    fn place_tree(data_dir: &StdPath, writes: &[&str], extra: &str) {
        let root = data_dir.join("mcp-servers").join(CONNECTOR);
        std::fs::create_dir_all(&root).unwrap();
        let manifest = serde_json::json!({
            "spec_version": 1,
            "connector_type": "ui_module",
            "ui": { "panels": [{ "id": "console", "name": "Console", "writes": writes }] }
        });
        let mut text = manifest.to_string();
        if !extra.is_empty() {
            text.insert_str(1, &format!("{extra},"));
        }
        std::fs::write(root.join("cloto-connector.json"), text).unwrap();
        std::fs::write(root.join("index.html"), "<p>console</p>").unwrap();
    }

    fn seal_tree(data_dir: &StdPath) -> String {
        let key = mgp_seal::load_or_generate_seal_key(data_dir).unwrap();
        crate::managers::tree_seal::compute_tree_seal(
            &data_dir.join("mcp-servers").join(CONNECTOR),
            &key,
        )
        .unwrap()
    }

    /// A connector installed the way the marketplace installs one: tree placed,
    /// sealed, receipt recorded.
    async fn installed(
        tag: &str,
        trust: &str,
        writes: &[&str],
    ) -> (Arc<AppState>, std::path::PathBuf) {
        let data_dir = temp_data_dir(tag);
        let state =
            crate::test_utils::create_test_app_state_in(data_dir.clone(), Some(KEY.into())).await;
        place_tree(&data_dir, writes, "");
        let seal = seal_tree(&data_dir);
        crate::db::upsert_install_receipt(&state.pool, CONNECTOR, trust, Some(&seal), "1.0.0")
            .await
            .unwrap();
        (state, data_dir)
    }

    async fn eligibility(state: &AppState) -> Result<Eligible, String> {
        let module = find_panel(state, PANEL).expect("panel is discovered");
        let servers_root = super::super::marketplace::resolve_servers_dir(state);
        evaluate_eligibility(&state.pool, &state.data_dir, &servers_root, &module).await
    }

    fn auth_headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-api-key", KEY.parse().unwrap());
        // Not a credential: a marker the target checks arrived, to show the
        // relay carries the caller's own headers rather than building its own.
        h.insert("x-caller-marker", "from-the-caller".parse().unwrap());
        h
    }

    /// Stand-in for the chat route: records what reached it.
    #[derive(Default, Clone)]
    struct Seen(Arc<Mutex<Vec<(Option<String>, Option<String>, String)>>>);

    fn target_router(seen: Seen) -> Router {
        Router::new().route(
            "/api/chat/agent.manager/messages",
            axum::routing::post(move |headers: HeaderMap, body: Bytes| {
                let seen = seen.clone();
                async move {
                    let key = headers
                        .get("x-api-key")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    let marker = headers
                        .get("x-caller-marker")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    seen.0.lock().unwrap().push((
                        key,
                        marker,
                        String::from_utf8_lossy(&body).into_owned(),
                    ));
                    (StatusCode::CREATED, "sent")
                }
            }),
        )
    }

    async fn send(state: &Arc<AppState>, method: &str, path: &str, text: &str) -> Response {
        let raw = serde_json::to_vec(&serde_json::json!({
            "method": method, "path": path, "body": { "content": text }
        }))
        .unwrap();
        relay_write(
            State(state.clone()),
            ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 9))),
            auth_headers(),
            Path(PANEL.to_string()),
            Bytes::from(raw),
        )
        .await
        .unwrap_or_else(|_| panic!("relay returned an error"))
    }

    async fn consent(state: &Arc<AppState>) -> StatusCode {
        consent_with(state, "").await
    }

    async fn consent_with(state: &Arc<AppState>, body: &str) -> StatusCode {
        put_write_consent(
            State(state.clone()),
            auth_headers(),
            Path(PANEL.to_string()),
            Bytes::from(body.to_string()),
        )
        .await
        .unwrap_or_else(|_| panic!("consent returned an error"))
        .status()
    }

    async fn audit_rows(state: &AppState, event_type: &str) -> Vec<(String, String, String)> {
        sqlx::query_as::<_, (String, String, String)>(
            "SELECT result, reason, COALESCE(metadata, '') FROM audit_logs WHERE event_type = ? ORDER BY id",
        )
        .bind(event_type)
        .fetch_all(&state.pool)
        .await
        .unwrap()
    }

    // ─── Eligibility ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_sealed_standard_connector_is_eligible() {
        let (state, dir) = installed("ok", "standard", &[SEND]).await;
        let e = eligibility(&state).await.expect("eligible");
        assert_eq!(e.trust_level, TrustLevel::Standard);
        assert_eq!(e.connector_version, "1.0.0");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_core_connector_is_eligible_and_experimental_or_untrusted_is_not() {
        let (state, dir) = installed("core", "core", &[SEND]).await;
        assert!(eligibility(&state).await.is_ok());
        for low in ["experimental", "untrusted", "nonsense"] {
            let seal = seal_tree(&dir);
            crate::db::upsert_install_receipt(&state.pool, CONNECTOR, low, Some(&seal), "1.0.0")
                .await
                .unwrap();
            let why = eligibility(&state).await.expect_err(low);
            assert!(why.contains("trust level"), "{low}: {why}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The trust level is the install record's, not anything the connector
    /// says about itself — the property that reading the handshake level
    /// would break.
    #[tokio::test]
    async fn a_connector_claiming_core_in_its_own_files_is_not_believed() {
        let data_dir = temp_data_dir("claim");
        let state =
            crate::test_utils::create_test_app_state_in(data_dir.clone(), Some(KEY.into())).await;
        place_tree(&data_dir, &[SEND], r#""trust_level":"core""#);
        let seal = seal_tree(&data_dir);
        crate::db::upsert_install_receipt(
            &state.pool,
            CONNECTOR,
            "experimental",
            Some(&seal),
            "1.0.0",
        )
        .await
        .unwrap();
        assert!(eligibility(&state).await.is_err());
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn no_seal_an_entry_point_seal_or_no_receipt_is_not_eligible() {
        let (state, dir) = installed("seal", "standard", &[SEND]).await;

        crate::db::upsert_install_receipt(&state.pool, CONNECTOR, "standard", None, "1.0.0")
            .await
            .unwrap();
        assert!(eligibility(&state)
            .await
            .unwrap_err()
            .contains("without a seal"));

        crate::db::upsert_install_receipt(
            &state.pool,
            CONNECTOR,
            "standard",
            Some("sha256:00"),
            "1.0.0",
        )
        .await
        .unwrap();
        assert!(eligibility(&state)
            .await
            .unwrap_err()
            .contains("without a seal"));

        crate::db::delete_install_receipt(&state.pool, CONNECTOR)
            .await
            .unwrap();
        assert!(eligibility(&state)
            .await
            .unwrap_err()
            .contains("no install record"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_tree_changed_after_install_is_not_eligible() {
        let (state, dir) = installed("tamper", "standard", &[SEND]).await;
        std::fs::write(
            dir.join("mcp-servers").join(CONNECTOR).join("index.html"),
            "<script>changed</script>",
        )
        .unwrap();
        assert!(eligibility(&state)
            .await
            .unwrap_err()
            .contains("no longer match"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The tree seal neither follows nor hashes a symlink, while manifest
    /// discovery reads `servers/<id>/` as a fixed path and does follow one. So
    /// a sealed tree can hold a link that serves a panel from files the seal
    /// never covered; the containment check is what refuses it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_panel_served_through_a_link_out_of_the_sealed_tree_cannot_write() {
        let data_dir = temp_data_dir("escape");
        let state =
            crate::test_utils::create_test_app_state_in(data_dir.clone(), Some(KEY.into())).await;
        let base = data_dir.join("mcp-servers").join(CONNECTOR);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("README"), "sealed").unwrap();

        let outside = data_dir.join("elsewhere");
        let nested = outside.join(CONNECTOR);
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            nested.join("cloto-connector.json"),
            serde_json::json!({
                "spec_version": 1,
                "connector_type": "ui_module",
                "ui": { "panels": [{ "id": "console", "name": "Console", "writes": [SEND] }] }
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(nested.join("index.html"), "unsealed").unwrap();
        std::os::unix::fs::symlink(&outside, base.join("servers")).unwrap();

        let seal = seal_tree(&data_dir);
        crate::db::upsert_install_receipt(&state.pool, CONNECTOR, "standard", Some(&seal), "1.0.0")
            .await
            .unwrap();

        let why = eligibility(&state)
            .await
            .expect_err("served from outside the seal");
        assert!(why.contains("not served from the tree"), "{why}");
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn a_module_placed_by_hand_cannot_write() {
        let data_dir = temp_data_dir("placed");
        let state =
            crate::test_utils::create_test_app_state_in(data_dir.clone(), Some(KEY.into())).await;
        let placed = data_dir.join("modules").join("handmade");
        std::fs::create_dir_all(&placed).unwrap();
        std::fs::write(placed.join("index.html"), "x").unwrap();

        // Declaring writes gets the module rejected outright ...
        std::fs::write(
            placed.join("module.json"),
            format!(r#"{{"name":"Handmade","writes":["{SEND}"]}}"#),
        )
        .unwrap();
        assert!(find_panel(&state, "handmade").is_none());

        // ... and without them it resolves, but is never eligible.
        std::fs::write(placed.join("module.json"), r#"{"name":"Handmade"}"#).unwrap();
        let module = find_panel(&state, "handmade").expect("resolves");
        let servers_root = super::super::marketplace::resolve_servers_dir(&state);
        let why = evaluate_eligibility(&state.pool, &state.data_dir, &servers_root, &module)
            .await
            .unwrap_err();
        assert!(why.contains("placed by hand"), "{why}");
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn a_panel_with_an_invalid_write_is_not_listed_as_usable() {
        let (state, dir) = installed(
            "invalid",
            "standard",
            &["DELETE /api/chat/agent.manager/messages"],
        )
        .await;
        assert!(find_panel(&state, PANEL).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    // ─── Consent ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn consent_is_refused_for_a_panel_that_cannot_write() {
        let (state, dir) = installed("consent-inel", "experimental", &[SEND]).await;
        assert_eq!(consent(&state).await, StatusCode::FORBIDDEN);
        assert!(crate::db::get_panel_write_consent(&state.pool, PANEL)
            .await
            .unwrap()
            .is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    // ─── Relay ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn an_admitted_write_reaches_the_target_with_the_callers_headers_and_is_audited() {
        let (state, dir) = installed("relay", "standard", &[SEND]).await;
        let seen = Seen::default();
        state
            .panel_writes
            .install_router(target_router(seen.clone()));
        assert_eq!(consent(&state).await, StatusCode::OK);

        let response = send(
            &state,
            "POST",
            "/api/chat/agent.manager/messages",
            "hello secret words",
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);

        let got = seen.0.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0.as_deref(), Some(KEY));
        assert_eq!(
            got[0].1.as_deref(),
            Some("from-the-caller"),
            "caller headers are carried"
        );
        assert!(got[0].2.contains("hello secret words"));

        let rows = audit_rows(&state, "PANEL_WRITE").await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "success");
        assert!(
            !rows[0].1.contains("secret words") && !rows[0].2.contains("secret words"),
            "no body in audit"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Stand-in for the renewal route: counts what reached it.
    fn renewal_router(hits: Arc<Mutex<usize>>) -> Router {
        Router::new().route(
            "/api/hub-access/renew",
            axum::routing::post(move || {
                let hits = hits.clone();
                async move {
                    *hits.lock().unwrap() += 1;
                    StatusCode::OK
                }
            }),
        )
    }

    #[tokio::test]
    async fn a_panel_that_declares_the_renewal_route_cannot_relay_it() {
        const RENEW: &str = "POST /api/hub-access/renew";
        let (state, dir) = installed("deny-renew", "standard", &[SEND, RENEW]).await;
        let hits = Arc::new(Mutex::new(0));
        state
            .panel_writes
            .install_router(renewal_router(hits.clone()));
        // Consent is attempted as an operator would. With the route denied the
        // panel is not usable, so this is refused; without the deny list it
        // succeeds and the write below would go through.
        let _ = put_write_consent(
            State(state.clone()),
            auth_headers(),
            Path(PANEL.to_string()),
            Bytes::new(),
        )
        .await;

        let response = send(&state, "POST", "/api/hub-access/renew", "").await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            *hits.lock().unwrap(),
            0,
            "the renewal route was never reached"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `docs/HUB_ACCESS_DESIGN.md` §6: a lapsed hub token stops updates, not an installed
    /// panel. Nothing in the gate reads the token; this holds that in place.
    #[tokio::test]
    async fn an_expired_hub_token_does_not_stop_an_installed_panel_writing() {
        let (state, dir) = installed("expired-token", "standard", &[SEND]).await;
        crate::managers::hub_access::store_expired_for_test(&state.data_dir, CONNECTOR);
        assert_eq!(
            crate::managers::hub_access::status(&state.data_dir, chrono::Utc::now())
                .unwrap()
                .unwrap()
                .stage,
            crate::managers::hub_access::ExpiryStage::Expired
        );
        let seen = Seen::default();
        state
            .panel_writes
            .install_router(target_router(seen.clone()));
        assert_eq!(consent(&state).await, StatusCode::OK);

        let response = send(&state, "POST", "/api/chat/agent.manager/messages", "hi").await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(seen.0.lock().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    async fn assert_denied(
        state: &Arc<AppState>,
        seen: &Seen,
        method: &str,
        path: &str,
        expect: &str,
    ) {
        let before = audit_rows(state, "PANEL_WRITE_DENIED").await.len();
        let response = send(state, method, path, "x").await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{expect}");
        let rows = audit_rows(state, "PANEL_WRITE_DENIED").await;
        assert_eq!(rows.len(), before + 1, "{expect}: one denial row");
        assert!(
            rows.last().unwrap().1.contains(expect),
            "{expect} in {:?}",
            rows.last()
        );
        assert!(
            seen.0.lock().unwrap().is_empty(),
            "{expect}: target not reached"
        );
    }

    #[tokio::test]
    async fn undeclared_routes_and_missing_consent_are_refused_before_the_target() {
        let (state, dir) = installed("deny", "standard", &[SEND]).await;
        let seen = Seen::default();
        state
            .panel_writes
            .install_router(target_router(seen.clone()));

        assert_denied(
            &state,
            &seen,
            "POST",
            "/api/chat/agent.manager/messages",
            "not consented",
        )
        .await;
        assert_eq!(consent(&state).await, StatusCode::OK);
        assert_denied(
            &state,
            &seen,
            "POST",
            "/api/chat/agent.other/messages",
            "not one of the writes",
        )
        .await;
        assert_denied(
            &state,
            &seen,
            "DELETE",
            "/api/chat/agent.manager/messages",
            "not one of the writes",
        )
        .await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_consent_lapses_when_the_declarations_or_the_version_change() {
        let (state, dir) = installed("lapse", "standard", &[SEND]).await;
        let seen = Seen::default();
        state
            .panel_writes
            .install_router(target_router(seen.clone()));
        assert_eq!(consent(&state).await, StatusCode::OK);

        // An update that adds a write, reinstalled and resealed.
        place_tree(
            &dir,
            &[SEND, "POST /api/chat/agent.manager/conversations"],
            "",
        );
        let seal = seal_tree(&dir);
        crate::db::upsert_install_receipt(&state.pool, CONNECTOR, "standard", Some(&seal), "1.0.0")
            .await
            .unwrap();
        assert_denied(
            &state,
            &seen,
            "POST",
            "/api/chat/agent.manager/messages",
            "changed since the consent",
        )
        .await;

        // Consent again, then only the version moves.
        assert_eq!(consent(&state).await, StatusCode::OK);
        crate::db::upsert_install_receipt(&state.pool, CONNECTOR, "standard", Some(&seal), "1.1.0")
            .await
            .unwrap();
        assert_denied(
            &state,
            &seen,
            "POST",
            "/api/chat/agent.manager/messages",
            "changed since the consent",
        )
        .await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn eligibility_is_asked_again_on_every_write() {
        let (state, dir) = installed("reask", "standard", &[SEND]).await;
        let seen = Seen::default();
        state
            .panel_writes
            .install_router(target_router(seen.clone()));
        assert_eq!(consent(&state).await, StatusCode::OK);
        std::fs::write(
            dir.join("mcp-servers").join(CONNECTOR).join("index.html"),
            "changed",
        )
        .unwrap();
        assert_denied(
            &state,
            &seen,
            "POST",
            "/api/chat/agent.manager/messages",
            "no longer match",
        )
        .await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_oversized_body_and_a_flood_are_refused() {
        let (state, dir) = installed("caps", "standard", &[SEND]).await;
        let seen = Seen::default();
        state
            .panel_writes
            .install_router(target_router(seen.clone()));
        assert_eq!(consent(&state).await, StatusCode::OK);

        let big = "a".repeat(MAX_WRITE_BODY_BYTES);
        let response = send(&state, "POST", "/api/chat/agent.manager/messages", &big).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(seen.0.lock().unwrap().is_empty());

        for _ in 0..WRITES_PER_WINDOW {
            assert_eq!(
                send(&state, "POST", "/api/chat/agent.manager/messages", "hi")
                    .await
                    .status(),
                StatusCode::CREATED
            );
        }
        let response = send(&state, "POST", "/api/chat/agent.manager/messages", "hi").await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(seen.0.lock().unwrap().len(), WRITES_PER_WINDOW);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn revoking_a_consent_stops_writes() {
        let (state, dir) = installed("revoke", "standard", &[SEND]).await;
        let seen = Seen::default();
        state
            .panel_writes
            .install_router(target_router(seen.clone()));
        assert_eq!(consent(&state).await, StatusCode::OK);
        let _ = delete_write_consent(
            State(state.clone()),
            auth_headers(),
            Path(PANEL.to_string()),
        )
        .await
        .unwrap_or_else(|_| panic!("revoke returned an error"));
        assert_denied(
            &state,
            &seen,
            "POST",
            "/api/chat/agent.manager/messages",
            "not consented",
        )
        .await;
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A consent from a script says so in the audit row; one from the sheet
    /// (no body) carries no note. Both still record the same actor.
    #[tokio::test]
    async fn a_consent_note_is_recorded_in_the_audit_row() {
        let (state, dir) = installed("note", "standard", &[SEND]).await;
        assert_eq!(consent(&state).await, StatusCode::OK);
        assert_eq!(
            consent_with(&state, r#"{"note":"  cil console ship-panel  "}"#).await,
            StatusCode::OK
        );
        let rows = audit_rows(&state, "PANEL_WRITE_CONSENT_GRANTED").await;
        assert_eq!(rows.len(), 2);
        let sheet: serde_json::Value = serde_json::from_str(&rows[0].2).unwrap();
        assert!(sheet["note"].is_null(), "{sheet}");
        assert_eq!(rows[0].1, "operator consented to the declared writes");
        let script: serde_json::Value = serde_json::from_str(&rows[1].2).unwrap();
        assert_eq!(script["note"], "cil console ship-panel");
        assert_eq!(
            rows[1].1,
            "operator consented to the declared writes (cil console ship-panel)"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_consent_note_must_be_a_short_single_line() {
        // `Some(note)` = accepted, `None` = refused (AppError has no Debug).
        let read = |body: &[u8]| consent_note(body).ok();
        assert_eq!(read(b""), Some(None));
        assert_eq!(read(b" \n"), Some(None));
        assert_eq!(read(b"{}"), Some(None));
        assert_eq!(read(br#"{"note":"   "}"#), Some(None));
        assert_eq!(read(br#"{"note":"ok"}"#), Some(Some("ok".to_string())));
        let at_cap = format!(r#"{{"note":"{}"}}"#, "あ".repeat(CONSENT_NOTE_MAX_CHARS));
        assert!(matches!(read(at_cap.as_bytes()), Some(Some(_))));
        let over = format!(r#"{{"note":"{}"}}"#, "a".repeat(CONSENT_NOTE_MAX_CHARS + 1));
        assert_eq!(read(over.as_bytes()), None);
        assert_eq!(read(br#"{"note":"a\nb"}"#), None);
        assert_eq!(read(br#"{"note":1}"#), None);
        assert_eq!(read(br#"{"by":"x"}"#), None);
        assert_eq!(read(b"not json"), None);
    }

    /// The router is built inline at boot, so no handler test can show the
    /// routes are mounted. This reads the registration itself.
    #[test]
    fn the_gate_routes_are_registered() {
        let source = include_str!("../lib.rs");
        for needle in [
            "get(handlers::panel_writes::list_write_consents)",
            "get(handlers::panel_writes::get_write_access)",
            "put(handlers::panel_writes::put_write_consent)",
            ".delete(handlers::panel_writes::delete_write_consent)",
            "post(handlers::panel_writes::relay_write)",
            "app_state.panel_writes.install_router(app.clone())",
        ] {
            assert!(source.contains(needle), "lib.rs must register {needle}");
        }
    }

    #[test]
    fn the_rate_cap_admits_up_to_the_window_then_refuses_and_recovers() {
        let gate = PanelWriteState::default();
        let t0 = Instant::now();
        for _ in 0..WRITES_PER_WINDOW {
            assert!(gate.admit("p", t0));
        }
        assert!(!gate.admit("p", t0), "one past the cap is refused");
        assert!(gate.admit("q", t0), "the cap is per panel");
        assert!(gate.admit("p", t0 + RATE_WINDOW), "the window rolls over");
    }
}
