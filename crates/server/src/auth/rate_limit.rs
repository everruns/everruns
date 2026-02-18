// THREAT[TM-AUTH-001]: Per-IP rate limiting on authentication endpoints
//
// Sliding window rate limiter using in-memory state. Limits are:
// - 10 attempts per minute on login/register/refresh
// - Returns 429 Too Many Requests with Retry-After header when exceeded
//
// Decision: In-memory over Redis — sufficient for single-node deployment.
// Stale entries are cleaned up lazily on each check.

use axum::{
    extract::{ConnectInfo, Request},
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Configuration for auth rate limiting.
#[derive(Debug, Clone)]
pub struct AuthRateLimitConfig {
    /// Maximum requests per window
    pub max_requests: usize,
    /// Window duration
    pub window: Duration,
}

impl Default for AuthRateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 10,
            window: Duration::from_secs(60),
        }
    }
}

impl AuthRateLimitConfig {
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            max_requests: std::env::var("AUTH_RATE_LIMIT_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(defaults.max_requests),
            window: std::env::var("AUTH_RATE_LIMIT_WINDOW_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs)
                .unwrap_or(defaults.window),
        }
    }
}

/// Per-IP sliding window entry.
#[derive(Debug)]
struct WindowEntry {
    /// Timestamps of requests within the current window
    timestamps: Vec<Instant>,
}

/// In-memory rate limiter state shared across requests.
#[derive(Debug, Clone)]
pub struct AuthRateLimiter {
    config: AuthRateLimitConfig,
    state: Arc<Mutex<HashMap<IpAddr, WindowEntry>>>,
}

impl AuthRateLimiter {
    pub fn new(config: AuthRateLimitConfig) -> Self {
        tracing::info!(
            max_requests = config.max_requests,
            window_secs = config.window.as_secs(),
            "Auth rate limiter initialized"
        );
        Self {
            config,
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Check if a request from `ip` is allowed. Returns remaining attempts or 0 if blocked.
    fn check(&self, ip: IpAddr) -> RateLimitResult {
        let now = Instant::now();
        let cutoff = now - self.config.window;

        let mut state = self.state.lock().unwrap();

        let entry = state.entry(ip).or_insert_with(|| WindowEntry {
            timestamps: Vec::new(),
        });

        // Evict expired timestamps
        entry.timestamps.retain(|t| *t > cutoff);

        if entry.timestamps.len() >= self.config.max_requests {
            // Calculate retry-after from oldest timestamp in window
            let oldest = entry.timestamps.first().unwrap();
            let retry_after = self.config.window - now.duration_since(*oldest);
            RateLimitResult::Limited {
                retry_after_secs: retry_after.as_secs().max(1),
            }
        } else {
            entry.timestamps.push(now);
            let remaining = self.config.max_requests - entry.timestamps.len();
            RateLimitResult::Allowed { remaining }
        }
    }
}

enum RateLimitResult {
    Allowed { remaining: usize },
    Limited { retry_after_secs: u64 },
}

/// Axum middleware that enforces auth rate limiting per IP.
pub async fn auth_rate_limit_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    // Extract rate limiter from request extensions
    let limiter = request.extensions().get::<AuthRateLimiter>().cloned();

    let Some(limiter) = limiter else {
        // No rate limiter configured — pass through
        return next.run(request).await;
    };

    let ip = addr.ip();

    match limiter.check(ip) {
        RateLimitResult::Allowed { remaining } => {
            let mut response = next.run(request).await;
            // Add rate limit headers
            response.headers_mut().insert(
                "X-RateLimit-Remaining",
                HeaderValue::from_str(&remaining.to_string()).unwrap(),
            );
            response
        }
        RateLimitResult::Limited { retry_after_secs } => {
            tracing::warn!(
                ip = %ip,
                retry_after = retry_after_secs,
                "Auth rate limit exceeded"
            );
            let mut response = (
                StatusCode::TOO_MANY_REQUESTS,
                "Too many authentication attempts. Please try again later.",
            )
                .into_response();
            response.headers_mut().insert(
                header::RETRY_AFTER,
                HeaderValue::from_str(&retry_after_secs.to_string()).unwrap(),
            );
            response
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_allows_under_limit() {
        let limiter = AuthRateLimiter::new(AuthRateLimitConfig {
            max_requests: 3,
            window: Duration::from_secs(60),
        });

        let ip: IpAddr = "127.0.0.1".parse().unwrap();

        assert!(matches!(
            limiter.check(ip),
            RateLimitResult::Allowed { remaining: 2 }
        ));
        assert!(matches!(
            limiter.check(ip),
            RateLimitResult::Allowed { remaining: 1 }
        ));
        assert!(matches!(
            limiter.check(ip),
            RateLimitResult::Allowed { remaining: 0 }
        ));
    }

    #[test]
    fn test_rate_limiter_blocks_over_limit() {
        let limiter = AuthRateLimiter::new(AuthRateLimitConfig {
            max_requests: 2,
            window: Duration::from_secs(60),
        });

        let ip: IpAddr = "127.0.0.1".parse().unwrap();

        assert!(matches!(limiter.check(ip), RateLimitResult::Allowed { .. }));
        assert!(matches!(limiter.check(ip), RateLimitResult::Allowed { .. }));
        assert!(matches!(limiter.check(ip), RateLimitResult::Limited { .. }));
    }

    #[test]
    fn test_rate_limiter_different_ips_independent() {
        let limiter = AuthRateLimiter::new(AuthRateLimitConfig {
            max_requests: 1,
            window: Duration::from_secs(60),
        });

        let ip1: IpAddr = "1.1.1.1".parse().unwrap();
        let ip2: IpAddr = "2.2.2.2".parse().unwrap();

        assert!(matches!(
            limiter.check(ip1),
            RateLimitResult::Allowed { .. }
        ));
        assert!(matches!(
            limiter.check(ip1),
            RateLimitResult::Limited { .. }
        ));
        // Different IP should still be allowed
        assert!(matches!(
            limiter.check(ip2),
            RateLimitResult::Allowed { .. }
        ));
    }

    #[test]
    fn test_default_config() {
        let config = AuthRateLimitConfig::default();
        assert_eq!(config.max_requests, 10);
        assert_eq!(config.window, Duration::from_secs(60));
    }
}
