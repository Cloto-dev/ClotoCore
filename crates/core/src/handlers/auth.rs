//! Browser sign-in and sign-out.
//!
//! Two routes over [`crate::managers::browser_session`]: one that hands a
//! browser a cookie, one that takes it back. The store holds the credential;
//! these decide who may have one.
//!
//! # Why minting is an admin route
//!
//! A session grants every admin permission (the operator's ruling), so the only
//! caller that may create one is a caller that already holds them. That is what
//! `admin_routes` means, and it is the whole authorisation rule here — there is
//! no password, no user table, and nothing in this module that could become one
//! by accident.
//!
//! It also means this route alone does not yet make a browser keyless: something
//! has to present the key once. What removes that step is a *second* minting
//! route whose identity comes from an edge that already authenticated the human
//! (`Cf-Access-Assertion`, verified — not merely read). That route is deliberately
//! not here yet: it needs proof the request came through the edge, and a header
//! read on a loopback listener is not proof, because every local process can
//! reach a loopback listener and set a header. Adding it before that verification
//! exists would turn any process on the host into an admin.

use std::sync::Arc;

use axum::{extract::State, http::HeaderMap, response::IntoResponse, Json};
use chrono::Duration;
use serde_json::json;

use crate::managers::browser_session::{
    self, clear_cookie_value, set_cookie_value, DEFAULT_TTL_MINUTES,
};
use crate::AppState;

/// Whether the session cookie is marked `Secure`.
///
/// Default on: the deployment this exists for terminates TLS at the edge, so the
/// browser's view of the connection is https and `Secure` costs nothing. Set
/// `CLOTO_SESSION_COOKIE_SECURE=0` only to sign in over plain http — a browser
/// will not send a `Secure` cookie to `http://`, so the flag has to be able to
/// come off, and the failure it prevents (a session that silently never
/// authenticates) is easier to mistake for a server bug than for a cookie
/// attribute.
///
/// Read here rather than parsed into `AppConfig` for the same reason
/// `unauthenticated_http_allowed` is: it is a deployment switch consulted at the
/// one place that acts on it, not a value the rest of the kernel reasons about.
fn cookie_secure() -> bool {
    cookie_secure_from(std::env::var("CLOTO_SESSION_COOKIE_SECURE").ok().as_deref())
}

/// The decision itself, separated from where the value came from.
///
/// Split out so the tests are a function of their argument rather than of the
/// process environment: env vars are global and the suite runs in parallel, so a
/// test that sets one is testing the whole binary's mood, not this rule.
fn cookie_secure_from(raw: Option<&str>) -> bool {
    !matches!(raw, Some(v) if v == "0" || v.eq_ignore_ascii_case("false"))
}

/// **Route:** `POST /api/auth/session` (admin auth required)
///
/// Mints a browser session and returns it as a `Set-Cookie`.
///
/// The token is **not** in the response body. It is `HttpOnly` precisely so that
/// script cannot read it, and echoing it in JSON would hand back what the
/// attribute was there to withhold — the SPA would then have somewhere to store
/// it, and a place to store it is a place to leak it from.
pub async fn create_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // A browser that already holds a live session is re-signing-in; the old one
    // is dropped rather than left resolvable, so a cookie replaced on screen is
    // also replaced in the store.
    if let Some(previous) = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(browser_session::token_from_cookie_header)
    {
        state.browser_sessions.revoke(previous).await;
    }

    let ttl = Duration::minutes(DEFAULT_TTL_MINUTES);
    let token = state.browser_sessions.mint("admin-key", ttl).await;
    let expires_at = chrono::Utc::now() + ttl;

    tracing::info!(
        ttl_minutes = DEFAULT_TTL_MINUTES,
        "🔓 Browser session minted"
    );

    (
        [(
            axum::http::header::SET_COOKIE,
            set_cookie_value(&token, ttl, cookie_secure()),
        )],
        Json(json!({
            "data": {
                "expires_at": expires_at.to_rfc3339(),
                "ttl_minutes": DEFAULT_TTL_MINUTES,
            }
        })),
    )
}

/// **Route:** `DELETE /api/auth/session` (any admin credential, including the
/// session being ended)
///
/// Ends the session the request arrived with and clears the cookie.
///
/// Reachable *with the cookie* on purpose: a browser that wants to stop being
/// admin holds nothing else to prove it may. Clearing is unconditional — a
/// request that carried no session still gets the expiring cookie back, because
/// "you were not signed in" and "you are now signed out" should not be
/// distinguishable to something that just wants to be sure.
pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ended = match headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(browser_session::token_from_cookie_header)
    {
        Some(token) => state.browser_sessions.revoke(token).await,
        None => false,
    };

    if ended {
        tracing::info!("🔒 Browser session ended");
    }

    (
        [(
            axum::http::header::SET_COOKIE,
            clear_cookie_value(cookie_secure()),
        )],
        Json(json!({ "data": { "ended": ended } })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default has to be `true`: a deployment that set nothing should get
    /// the attribute, not lose it.
    #[test]
    fn the_cookie_is_secure_when_nothing_says_otherwise() {
        assert!(cookie_secure_from(None));
    }

    #[test]
    fn only_an_explicit_falsehood_takes_secure_off() {
        for value in ["0", "false", "FALSE", "False"] {
            assert!(
                !cookie_secure_from(Some(value)),
                "{value} should disable Secure"
            );
        }
    }

    #[test]
    fn nothing_else_reads_as_no() {
        // A security attribute must not come off by accident. An empty value, a
        // typo, or a word that merely sounds negative all leave it on.
        for value in ["1", "true", "", " ", "no", "off", "nope", "0 ", "falsey"] {
            assert!(
                cookie_secure_from(Some(value)),
                "{value:?} should leave Secure on"
            );
        }
    }
}
