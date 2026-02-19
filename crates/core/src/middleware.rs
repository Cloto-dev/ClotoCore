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
    /// Create a new rate limiter.
    /// - `per_second`: token replenish rate per second
    /// - `burst`: maximum burst capacity
    #[must_use] 
    pub fn new(per_second: u32, burst: u32) -> Self {
        // M-03: Prevent panic on zero values by falling back to 1
        let per_second = NonZeroU32::new(per_second).unwrap_or(NonZeroU32::MIN);
        let burst = NonZeroU32::new(burst).unwrap_or(NonZeroU32::MIN);
        let quota = Quota::per_second(per_second)
            .allow_burst(burst);

        Self {
            limiters: DashMap::new(),
            quota,
        }
    }

    /// Check if the given IP is allowed to proceed.
    /// Returns `true` if allowed, `false` if rate-limited.
    #[must_use] 
    pub fn check(&self, ip: IpAddr) -> bool {
        let mut entry = self
            .limiters
            .entry(ip)
            .or_insert_with(|| (Arc::new(GovernorRateLimiter::direct(self.quota)), std::time::Instant::now()));
        // Bug #11: Update timestamp BEFORE check to prevent race condition
        entry.1 = std::time::Instant::now();
        entry.0.check().is_ok()
    }

    /// Remove idle entries to prevent memory growth.
    /// M-04: Uses timestamp-based staleness instead of consuming tokens
    pub fn cleanup(&self) {
        let idle_threshold = std::time::Duration::from_secs(600); // 10 minutes
        self.limiters.retain(|_, (_, last_seen)| {
            last_seen.elapsed() < idle_threshold
        });
    }

    /// Number of tracked IPs (useful for metrics)
    #[must_use] 
    pub fn tracked_ips(&self) -> usize {
        self.limiters.len()
    }
}

/// Axum middleware: rejects requests with 429 when rate limit is exceeded.
pub async fn rate_limit_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<crate::AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if !state.rate_limiter.check(addr.ip()) {
        tracing::warn!(ip = %addr.ip(), "Rate limit exceeded");
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    Ok(next.run(request).await)
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

        limiter.check(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));
        assert_eq!(limiter.tracked_ips(), 1);

        limiter.check(IpAddr::V4(Ipv4Addr::new(2, 2, 2, 2)));
        assert_eq!(limiter.tracked_ips(), 2);

        // Same IP should not increase count
        limiter.check(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));
        assert_eq!(limiter.tracked_ips(), 2);
    }
}
