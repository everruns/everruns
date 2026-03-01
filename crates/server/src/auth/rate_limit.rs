// TM-AUTH-001: Per-IP rate limiting for authentication endpoints.
// Decision: Use governor crate with in-memory state (per-instance).
// Decision: Different limits for login (strict), register (strict), refresh (relaxed).
// Decision: Keyed by client IP extracted from X-Forwarded-For or socket addr.

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use governor::{Quota, RateLimiter, clock::DefaultClock, state::keyed::DashMapStateStore};
use std::{net::IpAddr, num::NonZeroU32, sync::Arc};

type KeyedLimiter = RateLimiter<IpAddr, DashMapStateStore<IpAddr>, DefaultClock>;

/// Rate limiter for auth endpoints, shared via Arc.
#[derive(Clone)]
pub struct AuthRateLimiter {
    /// Login: 10 requests per minute per IP
    login: Arc<KeyedLimiter>,
    /// Register: 5 requests per minute per IP
    register: Arc<KeyedLimiter>,
    /// Refresh: 30 requests per minute per IP
    refresh: Arc<KeyedLimiter>,
}

impl Default for AuthRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthRateLimiter {
    pub fn new() -> Self {
        Self {
            login: Arc::new(RateLimiter::keyed(Quota::per_minute(
                NonZeroU32::new(10).unwrap(),
            ))),
            register: Arc::new(RateLimiter::keyed(Quota::per_minute(
                NonZeroU32::new(5).unwrap(),
            ))),
            refresh: Arc::new(RateLimiter::keyed(Quota::per_minute(
                NonZeroU32::new(30).unwrap(),
            ))),
        }
    }

    /// Check login rate limit. Returns Err(429) if exceeded.
    pub fn check_login(&self, ip: IpAddr) -> Result<(), RateLimitError> {
        self.check(&self.login, ip)
    }

    /// Check register rate limit. Returns Err(429) if exceeded.
    pub fn check_register(&self, ip: IpAddr) -> Result<(), RateLimitError> {
        self.check(&self.register, ip)
    }

    /// Check refresh rate limit. Returns Err(429) if exceeded.
    pub fn check_refresh(&self, ip: IpAddr) -> Result<(), RateLimitError> {
        self.check(&self.refresh, ip)
    }

    fn check(&self, limiter: &KeyedLimiter, ip: IpAddr) -> Result<(), RateLimitError> {
        match limiter.check_key(&ip) {
            Ok(_) => Ok(()),
            Err(_) => {
                tracing::warn!(ip = %ip, "Rate limit exceeded on auth endpoint");
                Err(RateLimitError)
            }
        }
    }
}

/// Small error type to avoid large Result<(), Response> on the stack.
pub struct RateLimitError;

impl From<RateLimitError> for Response {
    fn from(_: RateLimitError) -> Self {
        (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "60")],
            axum::Json(serde_json::json!({
                "error": "Too many requests. Please try again later."
            })),
        )
            .into_response()
    }
}

/// Extract client IP from request. Checks X-Forwarded-For first, falls back to
/// X-Real-IP, then ConnectInfo peer addr, then loopback.
pub fn extract_client_ip(req: &Request<Body>) -> IpAddr {
    // X-Forwarded-For: client, proxy1, proxy2 — take the first (leftmost)
    if let Some(forwarded) = req.headers().get("x-forwarded-for")
        && let Ok(val) = forwarded.to_str()
        && let Some(first) = val.split(',').next()
        && let Ok(ip) = first.trim().parse::<IpAddr>()
    {
        return ip;
    }

    // X-Real-IP (single IP, set by reverse proxy)
    if let Some(real_ip) = req.headers().get("x-real-ip")
        && let Ok(val) = real_ip.to_str()
        && let Ok(ip) = val.trim().parse::<IpAddr>()
    {
        return ip;
    }

    // ConnectInfo from axum (socket peer address)
    if let Some(connect_info) = req.extensions().get::<ConnectInfo<std::net::SocketAddr>>() {
        return connect_info.0.ip();
    }

    // Fallback: loopback (shouldn't happen in production)
    IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_login_rate_limit_allows_initial_requests() {
        let limiter = AuthRateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        assert!(limiter.check_login(ip).is_ok());
    }

    #[test]
    fn test_login_rate_limit_blocks_after_burst() {
        let limiter = AuthRateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        for _ in 0..10 {
            let _ = limiter.check_login(ip);
        }
        assert!(limiter.check_login(ip).is_err());
    }

    #[test]
    fn test_register_rate_limit_stricter_than_login() {
        let limiter = AuthRateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        for _ in 0..5 {
            let _ = limiter.check_register(ip);
        }
        assert!(
            limiter.check_register(ip).is_err(),
            "Register should be blocked after 5 requests"
        );
    }

    #[test]
    fn test_different_ips_have_separate_limits() {
        let limiter = AuthRateLimiter::new();
        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 3));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 4));
        for _ in 0..5 {
            let _ = limiter.check_register(ip1);
        }
        assert!(limiter.check_register(ip1).is_err());
        assert!(limiter.check_register(ip2).is_ok());
    }

    #[test]
    fn test_extract_ip_from_x_forwarded_for() {
        let req = Request::builder()
            .header("x-forwarded-for", "203.0.113.50, 70.41.3.18")
            .body(Body::empty())
            .unwrap();
        let ip = extract_client_ip(&req);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 50)));
    }

    #[test]
    fn test_extract_ip_from_x_real_ip() {
        let req = Request::builder()
            .header("x-real-ip", "198.51.100.25")
            .body(Body::empty())
            .unwrap();
        let ip = extract_client_ip(&req);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(198, 51, 100, 25)));
    }

    #[test]
    fn test_extract_ip_fallback_to_loopback() {
        let req = Request::builder().body(Body::empty()).unwrap();
        let ip = extract_client_ip(&req);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
}
