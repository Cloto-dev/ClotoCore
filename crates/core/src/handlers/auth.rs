//! Browser sign-in and sign-out.
//!
//! Three routes over [`crate::managers::browser_session`]: two that hand a
//! browser a cookie, one that takes it back. The store holds the credential;
//! these decide who may have one.
//!
//! # The two ways to be given a session
//!
//! **With the admin key.** A session grants every admin permission (the
//! operator's ruling), so a caller that already holds them may create one. That
//! is `POST /api/auth/session`, and it is the whole authorisation rule there —
//! no password, no user table, and nothing that could become one by accident.
//! It leaves the browser having presented the key once.
//!
//! **With a verified Access assertion.** That last step is what
//! `POST /api/auth/session/access` removes: identity comes from the edge that
//! already authenticated the human, proved by a signature a local process
//! cannot forge. The verification lives in
//! [`crate::managers::access_assertion`], which also explains why reading the
//! header would not do.
//!
//! # Why a refusal here says so little
//!
//! Both minting routes answer a failure with the same bare 403 that every other
//! denied `/api` request gets. The verifier knows precisely which condition was
//! missed, and the log says so; the response does not, because an error that
//! names the failing check is a description of how to search for one that passes.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use chrono::Duration;
use serde_json::json;

use crate::managers::access_assertion::ACCESS_ASSERTION_HEADER;
use crate::managers::browser_session::{
    self, clear_cookie_value, set_cookie_value, DEFAULT_TTL_MINUTES,
};
use crate::{AppError, AppState};

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
    issue_session(&state, &headers, "admin-key").await
}

/// **Route:** `POST /api/auth/session/access` (no admin key; a verified
/// `Cf-Access-Jwt-Assertion` instead)
///
/// Mints a browser session for the person Cloudflare Access authenticated.
///
/// This is the route that makes a browser keyless. It is reachable without the
/// admin key — [`crate::middleware::ACCESS_ASSERTION_API_PATHS`] is what lets it
/// past the layer — so *everything* that decides whether the caller may have a
/// session happens here, and it is all
/// [`crate::managers::access_assertion::AccessVerifier::verify`]: the peer must
/// be loopback (i.e. the tunnel), and the assertion must carry a signature from
/// the team's published keys naming this application and an unexpired person.
///
/// The session it produces is indistinguishable from one minted with the key,
/// which is the ruling: an operator who reached us through Access holds every
/// admin permission. What differs is the label, so a session's provenance stays
/// legible in diagnostics.
pub async fn create_session_from_access(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, AppError> {
    let assertion = headers
        .get(ACCESS_ASSERTION_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();

    let identity = match state
        .access_verifier
        .verify(assertion, peer.ip(), chrono::Utc::now())
        .await
    {
        Ok(identity) => identity,
        Err(reason) => {
            // The reason belongs in the log and nowhere else — see the module
            // docs. `peer` is included because "refused as not-from-the-edge"
            // is the one failure whose cause is the connection, not the token.
            tracing::warn!(%reason, peer = %peer.ip(), "🚫 Access sign-in refused");
            return Err(denied());
        }
    };

    tracing::info!(
        subject = %identity.subject,
        "🔓 Access sign-in accepted"
    );

    Ok(issue_session(&state, &headers, &identity.session_label())
        .await
        .into_response())
}

/// The refusal every failed sign-in gets: the same 403 envelope as any other
/// denied admin request.
fn denied() -> AppError {
    AppError::Cloto(cloto_shared::ClotoError::PermissionDenied(
        cloto_shared::Permission::AdminAccess,
    ))
}

/// Mint a session for `label` and hand it back as a `Set-Cookie`.
///
/// Shared by both minting routes so that *how* a session is installed cannot
/// drift between them — only *who may have one* differs, and that decision has
/// already been made by the time this is called.
async fn issue_session(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    label: &str,
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
    let token = state.browser_sessions.mint(label, ttl).await;
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
    use crate::managers::access_assertion::{
        test_support::{
            claims_json_at, config, good_token_now, header_json, key_a, published_keys, sign,
        },
        AccessVerifier,
    };
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::net::SocketAddr;
    use tower::ServiceExt;

    /// The router as `lib.rs` mounts it: the Access route behind the same auth
    /// layer as everything else. Built here rather than asserted about in
    /// isolation because the question this file has to answer is whether a
    /// browser holding *no key at all* can get a session — and only the layer
    /// and the handler together answer it.
    async fn router_with_access(enabled: bool) -> (Arc<AppState>, axum::Router) {
        let verifier = if enabled {
            AccessVerifier::with_keys(config(), published_keys())
        } else {
            AccessVerifier::new(None)
        };
        let state = crate::test_utils::create_test_app_state_with_access(
            std::path::PathBuf::from("data"),
            Some("test-key".to_string()),
            Arc::new(verifier),
        )
        .await;
        let api = axum::Router::new()
            .route(
                crate::middleware::ACCESS_SESSION_PATH,
                axum::routing::post(create_session_from_access),
            )
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::middleware::auth_middleware,
            ))
            .with_state(state.clone());
        (state, axum::Router::new().nest("/api", api))
    }

    /// A sign-in attempt carrying `assertion` (when `Some`) from `peer`, and
    /// never an admin key — the credential this route exists to do without.
    fn sign_in(assertion: Option<&str>, peer: &str) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("/api{}", crate::middleware::ACCESS_SESSION_PATH));
        if let Some(assertion) = assertion {
            builder = builder.header(ACCESS_ASSERTION_HEADER, assertion);
        }
        let mut request = builder.body(Body::empty()).expect("build request");
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            peer.parse::<SocketAddr>().expect("peer address"),
        ));
        request
    }

    fn session_cookie(response: &axum::response::Response) -> Option<String> {
        response
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    #[tokio::test]
    async fn a_verified_assertion_signs_a_browser_in_without_any_key() {
        let (state, app) = router_with_access(true).await;
        let response = app
            .oneshot(sign_in(Some(&good_token_now()), "127.0.0.1:54321"))
            .await
            .expect("router responds");

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "a browser behind Access holds no admin key and must not need one"
        );
        let cookie = session_cookie(&response).expect("a session cookie");
        let token = crate::managers::browser_session::token_from_cookie_header(&cookie)
            .expect("the cookie carries a session");
        assert_eq!(
            state.browser_sessions.resolve(token).await.as_deref(),
            Some("access:operator@example.com"),
            "the session is labelled with the person the edge authenticated"
        );
    }

    #[tokio::test]
    async fn a_valid_assertion_from_outside_the_tunnel_signs_nobody_in() {
        // The forgery this route has to survive: a local process that obtained a
        // real assertion but is not reaching us through cloudflared.
        let (state, app) = router_with_access(true).await;
        let response = app
            .oneshot(sign_in(Some(&good_token_now()), "192.168.0.10:54321"))
            .await
            .expect("router responds");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(session_cookie(&response).is_none(), "nothing was installed");
        assert_eq!(state.browser_sessions.live_count().await, 0);
    }

    #[tokio::test]
    async fn a_forged_assertion_signs_nobody_in() {
        let (state, app) = router_with_access(true).await;
        // Signed by the right key, but claiming another team: a token that is
        // real somewhere and worthless here.
        let mut claims = claims_json_at(chrono::Utc::now());
        claims["iss"] = serde_json::json!("https://attacker.cloudflareaccess.com");
        let token = sign(&key_a(), &header_json(), &claims.to_string());

        let response = app
            .oneshot(sign_in(Some(&token), "127.0.0.1:54321"))
            .await
            .expect("router responds");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(session_cookie(&response).is_none());
        assert_eq!(state.browser_sessions.live_count().await, 0);
    }

    #[tokio::test]
    async fn a_deployment_that_is_not_behind_access_signs_nobody_in() {
        let (state, app) = router_with_access(false).await;
        let response = app
            .oneshot(sign_in(Some(&good_token_now()), "127.0.0.1:54321"))
            .await
            .expect("router responds");

        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "an unconfigured verifier must refuse, not fall back to trusting the header"
        );
        assert_eq!(state.browser_sessions.live_count().await, 0);
    }

    #[tokio::test]
    async fn the_route_still_needs_the_key_when_no_assertion_is_offered() {
        // Without the header the exemption does not apply, so this is an
        // ordinary unauthenticated /api request and the layer refuses it.
        let (state, app) = router_with_access(true).await;
        let response = app
            .oneshot(sign_in(None, "127.0.0.1:54321"))
            .await
            .expect("router responds");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(state.browser_sessions.live_count().await, 0);
    }

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
