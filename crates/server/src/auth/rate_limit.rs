// TM-AUTH-001: Per-IP rate limiting for authentication endpoints.
// Decision: Dual backend — in-memory (governor) for single-instance/dev, Valkey for distributed.
// Decision: When VALKEY_URL is set, use Valkey sliding-window counter (Lua script, atomic).
//   When not set, fall back to governor in-memory (per-instance, same as before).
// Decision: Different limits for login (strict), register (strict), refresh (relaxed).
// Decision: Keyed by client IP extracted from X-Forwarded-For or socket addr.
// Decision: On Valkey errors, fail open (allow request) and log — availability over strictness.

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use governor::{Quota, RateLimiter, clock::DefaultClock, state::keyed::DashMapStateStore};
use std::{net::IpAddr, num::NonZeroU32, sync::Arc};

use crate::valkey::ValkeyClient;

type KeyedLimiter = RateLimiter<IpAddr, DashMapStateStore<IpAddr>, DefaultClock>;

/// Rate limiter for auth endpoints, shared via Arc.
///
/// Supports two backends:
/// - **In-memory** (governor): per-instance, used when `VALKEY_URL` is not set.
/// - **Valkey**: distributed sliding-window counter, used when `VALKEY_URL` is set.
#[derive(Clone)]
pub struct AuthRateLimiter {
    backend: RateLimitBackend,
}

#[derive(Clone)]
enum RateLimitBackend {
    /// Per-instance rate limiting (governor crate, DashMap state)
    InMemory {
        login: Arc<KeyedLimiter>,
        register: Arc<KeyedLimiter>,
        refresh: Arc<KeyedLimiter>,
    },
    /// Distributed rate limiting (Valkey sliding-window counter)
    Valkey(ValkeyClient),
}

impl Default for AuthRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Rate limits per endpoint (requests per minute)
const LOGIN_LIMIT: u32 = 10;
const REGISTER_LIMIT: u32 = 5;
const REFRESH_LIMIT: u32 = 30;
const WINDOW_SECS: u64 = 60;

impl AuthRateLimiter {
    /// Create an in-memory rate limiter (per-instance, no external deps).
    pub fn new() -> Self {
        Self {
            backend: RateLimitBackend::InMemory {
                login: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(LOGIN_LIMIT).unwrap(),
                ))),
                register: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(REGISTER_LIMIT).unwrap(),
                ))),
                refresh: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(REFRESH_LIMIT).unwrap(),
                ))),
            },
        }
    }

    /// Create a distributed rate limiter backed by Valkey.
    pub fn with_valkey(client: ValkeyClient) -> Self {
        Self {
            backend: RateLimitBackend::Valkey(client),
        }
    }

    /// Check login rate limit. Returns Err(429) if exceeded.
    pub async fn check_login(&self, ip: IpAddr) -> Result<(), RateLimitError> {
        self.check("login", LOGIN_LIMIT, ip).await
    }

    /// Check register rate limit. Returns Err(429) if exceeded.
    pub async fn check_register(&self, ip: IpAddr) -> Result<(), RateLimitError> {
        self.check("register", REGISTER_LIMIT, ip).await
    }

    /// Check refresh rate limit. Returns Err(429) if exceeded.
    pub async fn check_refresh(&self, ip: IpAddr) -> Result<(), RateLimitError> {
        self.check("refresh", REFRESH_LIMIT, ip).await
    }

    async fn check(&self, endpoint: &str, limit: u32, ip: IpAddr) -> Result<(), RateLimitError> {
        match &self.backend {
            RateLimitBackend::InMemory {
                login,
                register,
                refresh,
            } => {
                let limiter = match endpoint {
                    "login" => login,
                    "register" => register,
                    "refresh" => refresh,
                    _ => return Ok(()),
                };
                match limiter.check_key(&ip) {
                    Ok(_) => Ok(()),
                    Err(_) => {
                        tracing::warn!(ip = %ip, endpoint, "Rate limit exceeded (in-memory)");
                        Err(RateLimitError)
                    }
                }
            }
            RateLimitBackend::Valkey(client) => {
                let key = format!("rl:auth:{endpoint}:{ip}");
                match client.check_rate_limit(&key, limit, WINDOW_SECS).await {
                    Ok(_remaining) => Ok(()),
                    Err(()) => {
                        tracing::warn!(ip = %ip, endpoint, "Rate limit exceeded (valkey)");
                        Err(RateLimitError)
                    }
                }
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

// ============================================================================
// Generic API rate limiting middleware
// ============================================================================

/// Global per-IP rate limiter for all API endpoints.
///
/// Applied as Axum middleware. Uses governor in-memory backend.
/// Configurable via `RATE_LIMIT_API_REQUESTS_PER_MINUTE` env var (default: 120).
#[derive(Clone)]
pub struct ApiRateLimiter {
    limiter: Arc<KeyedLimiter>,
}

impl ApiRateLimiter {
    pub fn from_env() -> Self {
        let rpm: u32 = everruns_config::env_or("RATE_LIMIT_API_REQUESTS_PER_MINUTE", 1200);
        let rpm = rpm.max(1); // ensure nonzero
        Self {
            limiter: Arc::new(RateLimiter::keyed(Quota::per_minute(
                NonZeroU32::new(rpm).unwrap(),
            ))),
        }
    }

    /// Check if rate limiting is disabled (set to 0 via env var).
    pub fn is_disabled() -> bool {
        std::env::var("RATE_LIMIT_API_REQUESTS_PER_MINUTE")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            == Some(0)
    }
}

/// Axum middleware function for global API rate limiting.
pub async fn api_rate_limit_middleware(
    limiter: ApiRateLimiter,
    req: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let ip = extract_client_ip(&req);
    match limiter.limiter.check_key(&ip) {
        Ok(_) => next.run(req).await,
        Err(_) => {
            tracing::debug!(ip = %ip, "API rate limit exceeded");
            RateLimitError.into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[tokio::test]
    async fn test_login_rate_limit_allows_initial_requests() {
        let limiter = AuthRateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        assert!(limiter.check_login(ip).await.is_ok());
    }

    #[tokio::test]
    async fn test_login_rate_limit_blocks_after_burst() {
        let limiter = AuthRateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        for _ in 0..10 {
            let _ = limiter.check_login(ip).await;
        }
        assert!(limiter.check_login(ip).await.is_err());
    }

    #[tokio::test]
    async fn test_register_rate_limit_stricter_than_login() {
        let limiter = AuthRateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        for _ in 0..5 {
            let _ = limiter.check_register(ip).await;
        }
        assert!(
            limiter.check_register(ip).await.is_err(),
            "Register should be blocked after 5 requests"
        );
    }

    #[tokio::test]
    async fn test_different_ips_have_separate_limits() {
        let limiter = AuthRateLimiter::new();
        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 3));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 4));
        for _ in 0..5 {
            let _ = limiter.check_register(ip1).await;
        }
        assert!(limiter.check_register(ip1).await.is_err());
        assert!(limiter.check_register(ip2).await.is_ok());
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

    // ============================================
    // API rate limiter tests
    // ============================================

    #[test]
    fn test_api_rate_limiter_allows_initial_requests() {
        let limiter = ApiRateLimiter::from_env();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
        assert!(limiter.limiter.check_key(&ip).is_ok());
    }

    #[test]
    fn test_api_rate_limiter_blocks_after_burst() {
        // Set a low limit for testing
        let limiter = ApiRateLimiter {
            limiter: Arc::new(RateLimiter::keyed(Quota::per_minute(
                NonZeroU32::new(5).unwrap(),
            ))),
        };
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 101));
        for _ in 0..5 {
            let _ = limiter.limiter.check_key(&ip);
        }
        assert!(
            limiter.limiter.check_key(&ip).is_err(),
            "Should block after exceeding limit"
        );
    }

    #[test]
    fn test_api_rate_limiter_separate_ips() {
        let limiter = ApiRateLimiter {
            limiter: Arc::new(RateLimiter::keyed(Quota::per_minute(
                NonZeroU32::new(2).unwrap(),
            ))),
        };
        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 10));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 11));
        for _ in 0..2 {
            let _ = limiter.limiter.check_key(&ip1);
        }
        assert!(limiter.limiter.check_key(&ip1).is_err());
        assert!(limiter.limiter.check_key(&ip2).is_ok());
    }
}
