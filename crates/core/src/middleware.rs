use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use dashmap::DashMap;
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter as GovernorRateLimiter,
};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::Arc;

pub type IpLimiter = GovernorRateLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// IP-based rate limiter using token bucket algorithm (via governor)
pub struct RateLimiter {
    // M-04: Store last-seen timestamp alongside limiter for side-effect-free cleanup
    limiters: DashMap<IpAddr, (Arc<IpLimiter>, std::time::Instant)>,
    quota: Quota,
}

impl RateLimiter {
    /// Create a new rate limiter with per-second quota.
    /// - `per_second`: token replenish rate per second
    /// - `burst`: maximum burst capacity
    #[must_use]
    pub fn new(per_second: u32, burst: u32) -> Self {
        // M-03: Prevent panic on zero values by falling back to 1
        let per_second = NonZeroU32::new(per_second).unwrap_or(NonZeroU32::MIN);
        let burst = NonZeroU32::new(burst).unwrap_or(NonZeroU32::MIN);
        let quota = Quota::per_second(per_second).allow_burst(burst);

        Self {
            limiters: DashMap::new(),
            quota,
        }
    }

    /// Create a new rate limiter with per-minute quota.
    /// Suitable for heavy operations (e.g. marketplace install).
    #[must_use]
    pub fn per_minute(per_minute: u32, burst: u32) -> Self {
        let per_minute = NonZeroU32::new(per_minute).unwrap_or(NonZeroU32::MIN);
        let burst = NonZeroU32::new(burst).unwrap_or(NonZeroU32::MIN);
        let quota = Quota::per_minute(per_minute).allow_burst(burst);

        Self {
            limiters: DashMap::new(),
            quota,
        }
    }

    /// Check if the given IP is allowed to proceed.
    /// Returns `true` if allowed, `false` if rate-limited.
    #[must_use]
    pub fn check(&self, ip: IpAddr) -> bool {
        let mut entry = self.limiters.entry(ip).or_insert_with(|| {
            (
                Arc::new(GovernorRateLimiter::direct(self.quota)),
                std::time::Instant::now(),
            )
        });
        // Bug #11: Update timestamp BEFORE check to prevent race condition
        entry.1 = std::time::Instant::now();
        entry.0.check().is_ok()
    }

    /// Remove idle entries to prevent memory growth.
    /// M-04: Uses timestamp-based staleness instead of consuming tokens
    pub fn cleanup(&self) {
        let idle_threshold = std::time::Duration::from_mins(10); // 10 minutes
        self.limiters
            .retain(|_, (_, last_seen)| last_seen.elapsed() < idle_threshold);
    }

    /// Number of tracked IPs (useful for metrics)
    #[must_use]
    pub fn tracked_ips(&self) -> usize {
        self.limiters.len()
    }
}

/// Routes under `/api` that answer without the admin key.
///
/// Everything else under `/api` is authenticated by [`auth_middleware`].
/// Keep this list short and deliberate: each entry is a promise that the
/// response carries nothing an unauthenticated caller should not see.
pub const PUBLIC_API_PATHS: &[&str] = &[
    "/system/version",
    "/system/health",
    "/setup/status",
    "/setup/progress",
    "/marketplace/progress",
];

/// Whether a request path (with or without the `/api` prefix) is public.
#[must_use]
pub fn is_public_api_path(path: &str) -> bool {
    let path = path.strip_prefix("/api").unwrap_or(path);
    PUBLIC_API_PATHS.contains(&path)
}

/// Routes that accept a second credential this layer cannot evaluate, and so
/// decide for themselves who is calling.
///
/// `POST /api/mcp/call` is the only one. It takes the admin key — the
/// coordinator credential, which runs as `Caller::System` — *and* an agent
/// token, which names one agent and keeps the per-agent capability gate. Only
/// the handler can resolve a token, so the layer that knows about the first
/// credential must not be the one that refuses the second.
///
/// This is not an exemption. A request reaches the handler through here only by
/// presenting a token header, and an unresolvable token is refused there —
/// fail-closed, never retried as admin. A request with no credential at all is
/// still stopped by this layer, exactly as before.
pub const AGENT_TOKEN_API_PATHS: &[&str] = &["/mcp/call"];

/// Whether this request is one the handler authenticates itself: a path on
/// [`AGENT_TOKEN_API_PATHS`], carrying an agent token to be resolved there.
#[must_use]
pub fn defers_auth_to_handler(path: &str, headers: &axum::http::HeaderMap) -> bool {
    let path = path.strip_prefix("/api").unwrap_or(path);
    AGENT_TOKEN_API_PATHS.contains(&path)
        && headers.contains_key(crate::managers::agent_token::AGENT_TOKEN_HEADER)
}

/// Axum middleware: every `/api` route requires the admin key unless it is
/// on [`PUBLIC_API_PATHS`] or defers the decision to its handler
/// ([`AGENT_TOKEN_API_PATHS`]).
///
/// The key is accepted in `X-API-Key` or as `?token=` (browser-initiated
/// loads such as `EventSource` and `<img src>` cannot set headers). A
/// rejection carries the same JSON error envelope as a handler-level denial.
/// Handlers keep their own `check_auth` calls; this layer is what makes a
/// route that forgets one still safe.
pub async fn auth_middleware(
    State(state): State<Arc<crate::AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if is_public_api_path(request.uri().path()) {
        return next.run(request).await;
    }
    if defers_auth_to_handler(request.uri().path(), request.headers()) {
        return next.run(request).await;
    }
    let mut request = request;
    if authenticate_browser_session(&state, &mut request).await {
        return next.run(request).await;
    }
    let query: HashMap<String, String> = axum::extract::Query::try_from_uri(request.uri())
        .map(|q: axum::extract::Query<HashMap<String, String>>| q.0)
        .unwrap_or_default();
    if let Err(e) = crate::handlers::check_auth_with_query(&state, request.headers(), &query) {
        return axum::response::IntoResponse::into_response(e);
    }
    next.run(request).await
}

/// Authenticate a request by its browser-session cookie, and if it holds one,
/// rewrite it so the rest of the kernel sees an ordinary keyed request.
///
/// Returns whether the session authenticated it.
///
/// # Why it rewrites rather than sets a flag of its own
///
/// This layer is not the only thing that checks: handlers keep 97 `check_auth`
/// calls of their own, and those read the header and nothing else. A new
/// credential that the layer alone understood would pass here and then be
/// refused one frame later, by a route that looked authenticated from the
/// outside — the failure mode is a 403 that no log explains. Nor is a private
/// marker header the answer: it would be a second thing that grants admin, and
/// forgeable by anyone the layer forgot to strip it from.
///
/// Substituting the live key is neither. Downstream sees the credential it
/// already knows, every existing check keeps its meaning (including the
/// revocation check — a session standing on a revoked key dies with it), and
/// nothing new is trusted: the value the caller sent is replaced, never read.
async fn authenticate_browser_session(state: &Arc<crate::AppState>, request: &mut Request) -> bool {
    use crate::managers::browser_session;

    let Some(token) = request
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(browser_session::token_from_cookie_header)
        .map(str::to_owned)
    else {
        return false;
    };
    if state.browser_sessions.resolve(&token).await.is_none() {
        return false;
    }
    // A session was authorised by a route that already held the admin key, so
    // the key is the credential it stands for. No key configured means no
    // session could have been minted, and letting the request through with
    // nothing attached would be the one case where this helper granted more
    // than it verified — so it declines and the normal path refuses it.
    let Ok(Some(key)) = state.admin_api_key.read().map(|g| (*g).clone()) else {
        return false;
    };
    let Ok(value) = axum::http::HeaderValue::from_str(&key) else {
        return false;
    };
    request
        .headers_mut()
        .insert(crate::handlers::ADMIN_API_KEY_HEADER, value);
    true
}

/// Axum middleware: rejects requests with 429 when rate limit is exceeded.
///
/// bug-427: the 429 must carry the same `{"error":{...}}` JSON envelope as
/// every `AppError` response — a bare status code breaks clients that parse
/// the uniform error body.
pub async fn rate_limit_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<crate::AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if !state.rate_limiter.check(addr.ip()) {
        tracing::warn!(ip = %addr.ip(), "Rate limit exceeded");
        let body = axum::Json(serde_json::json!({
            "error": {
                "type": "RateLimited",
                "message": "Too many requests; retry later"
            }
        }));
        return axum::response::IntoResponse::into_response((StatusCode::TOO_MANY_REQUESTS, body));
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_allows_within_burst() {
        let limiter = RateLimiter::new(1, 10);
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

        for i in 0..10 {
            assert!(limiter.check(ip), "Request {} should be allowed", i);
        }
    }

    #[test]
    fn test_blocks_after_burst_exhausted() {
        let limiter = RateLimiter::new(1, 5);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

        // Exhaust burst
        for _ in 0..5 {
            assert!(limiter.check(ip));
        }

        // Next request should be blocked
        assert!(!limiter.check(ip), "Should be rate-limited after burst");
    }

    #[test]
    fn test_different_ips_are_independent() {
        let limiter = RateLimiter::new(1, 3);
        let ip_a = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));
        let ip_b = IpAddr::V4(Ipv4Addr::new(2, 2, 2, 2));

        // Exhaust IP A
        for _ in 0..3 {
            assert!(limiter.check(ip_a));
        }
        assert!(!limiter.check(ip_a));

        // IP B should still be allowed
        assert!(limiter.check(ip_b), "Different IP should not be affected");
    }

    #[tokio::test]
    async fn test_refills_after_wait() {
        let limiter = RateLimiter::new(10, 5);
        let ip = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));

        // Exhaust burst
        for _ in 0..5 {
            assert!(limiter.check(ip));
        }
        assert!(!limiter.check(ip));

        // Wait for refill (10/s = 1 token per 100ms)
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        assert!(limiter.check(ip), "Should allow after refill");
    }

    #[test]
    fn test_tracked_ips_count() {
        let limiter = RateLimiter::new(1, 10);

        assert_eq!(limiter.tracked_ips(), 0);

        let _ = limiter.check(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));
        assert_eq!(limiter.tracked_ips(), 1);

        let _ = limiter.check(IpAddr::V4(Ipv4Addr::new(2, 2, 2, 2)));
        assert_eq!(limiter.tracked_ips(), 2);

        // Same IP should not increase count
        let _ = limiter.check(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));
        assert_eq!(limiter.tracked_ips(), 2);
    }

    /// Both halves of the deferral matter, and only one of them is visible in a
    /// response. Dropping the path check turns an agent token into a key to the
    /// whole admin surface — a router test catches that. Dropping the *header*
    /// check does not change any status code, because the handler refuses a
    /// credential-less call with the same 403 the layer would have: the two are
    /// indistinguishable from outside. So the header half is asserted here, at
    /// the level where the difference exists.
    #[test]
    fn a_credential_less_request_never_defers_even_on_the_deferring_path() {
        let empty = axum::http::HeaderMap::new();
        assert!(
            !defers_auth_to_handler("/api/mcp/call", &empty),
            "with no token to resolve there is nothing for the handler to decide"
        );

        let mut with_token = axum::http::HeaderMap::new();
        with_token.insert(
            crate::managers::agent_token::AGENT_TOKEN_HEADER,
            axum::http::HeaderValue::from_static("t"),
        );
        assert!(defers_auth_to_handler("/api/mcp/call", &with_token));
        assert!(
            !defers_auth_to_handler("/api/agents", &with_token),
            "the deferral is scoped to the route that can resolve a token"
        );
    }
}

#[cfg(test)]
mod browser_session_layer_tests {
    use super::*;
    use crate::managers::browser_session::SessionStore;

    fn request_with_cookie(raw: &str) -> Request {
        Request::builder()
            .uri("/api/agents")
            .header(axum::http::header::COOKIE, raw)
            .body(axum::body::Body::empty())
            .expect("build request")
    }

    /// The one case where this helper could grant more than it verified: a store
    /// entry exists, but there is no admin credential for it to stand for. It
    /// has to decline rather than let the request through with nothing attached.
    ///
    /// Asserted here rather than through the router because the fall-through it
    /// would otherwise be measured against is `CLOTO_DEBUG_SKIP_AUTH`-sensitive,
    /// and that variable is process-global — a sibling test setting it decides
    /// the answer.
    #[tokio::test]
    async fn a_session_grants_nothing_when_no_admin_key_is_configured() {
        let state = crate::test_utils::create_test_app_state(None).await;
        let token = state.browser_sessions.mint_default("operator").await;
        let mut request = request_with_cookie(&format!("cloto_session={token}"));
        assert!(!authenticate_browser_session(&state, &mut request).await);
        assert!(
            request
                .headers()
                .get(crate::handlers::ADMIN_API_KEY_HEADER)
                .is_none(),
            "declining must also mean attaching nothing"
        );
    }

    #[tokio::test]
    async fn a_resolved_session_attaches_the_live_key() {
        let state = crate::test_utils::create_test_app_state(Some("live-key".into())).await;
        let token = state.browser_sessions.mint_default("operator").await;
        let mut request = request_with_cookie(&format!("cloto_session={token}"));
        assert!(authenticate_browser_session(&state, &mut request).await);
        assert_eq!(
            request
                .headers()
                .get(crate::handlers::ADMIN_API_KEY_HEADER)
                .and_then(|v| v.to_str().ok()),
            Some("live-key")
        );
    }

    /// What the caller sent in the header is replaced, never read. Otherwise a
    /// session cookie plus a guessed header would be a way to have the guess
    /// evaluated.
    #[tokio::test]
    async fn the_callers_own_key_header_is_replaced_not_trusted() {
        let state = crate::test_utils::create_test_app_state(Some("live-key".into())).await;
        let token = state.browser_sessions.mint_default("operator").await;
        let mut request = Request::builder()
            .uri("/api/agents")
            .header(axum::http::header::COOKIE, format!("cloto_session={token}"))
            .header(crate::handlers::ADMIN_API_KEY_HEADER, "attacker-supplied")
            .body(axum::body::Body::empty())
            .expect("build request");
        assert!(authenticate_browser_session(&state, &mut request).await);
        assert_eq!(
            request
                .headers()
                .get(crate::handlers::ADMIN_API_KEY_HEADER)
                .and_then(|v| v.to_str().ok()),
            Some("live-key")
        );
    }

    #[tokio::test]
    async fn a_request_without_a_cookie_is_left_untouched() {
        let state = crate::test_utils::create_test_app_state(Some("live-key".into())).await;
        let mut request = Request::builder()
            .uri("/api/agents")
            .body(axum::body::Body::empty())
            .expect("build request");
        assert!(!authenticate_browser_session(&state, &mut request).await);
        assert!(request
            .headers()
            .get(crate::handlers::ADMIN_API_KEY_HEADER)
            .is_none());
    }

    /// The store is consulted, not merely the cookie's shape.
    #[tokio::test]
    async fn a_cookie_from_a_different_store_does_not_resolve() {
        let state = crate::test_utils::create_test_app_state(Some("live-key".into())).await;
        let elsewhere = SessionStore::new();
        let token = elsewhere.mint_default("operator").await;
        let mut request = request_with_cookie(&format!("cloto_session={token}"));
        assert!(!authenticate_browser_session(&state, &mut request).await);
    }
}
