// Per-IP rate limiting for authentication endpoints
// SECURITY[TM-AUTH-001]: Prevents brute-force attacks on login/register endpoints
// Decision: In-memory sliding window; suitable for single-process deployments.
// For multi-process, switch to Redis-backed rate limiting.

use axum::{
    extract::ConnectInfo,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

/// Per-IP rate limiter using sliding window.
#[derive(Clone)]
pub struct RateLimiter {
    state: Arc<Mutex<RateLimiterState>>,
    max_requests: u32,
    window: Duration,
}

struct RateLimiterState {
    /// Map from IP string to list of request timestamps
    requests: HashMap<String, Vec<Instant>>,
    /// Last time we cleaned up stale entries
    last_cleanup: Instant,
}

impl RateLimiter {
    /// Create a new rate limiter.
    /// `max_requests` per `window` duration, per IP.
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(RateLimiterState {
                requests: HashMap::new(),
                last_cleanup: Instant::now(),
            })),
            max_requests,
            window,
        }
    }

    /// Check if request from `ip` is allowed.
    /// Returns Ok(()) if allowed, Err(retry_after_secs) if rate limited.
    pub fn check(&self, ip: &str) -> Result<(), u64> {
        let now = Instant::now();
        let mut state = self.state.lock();

        // Periodic cleanup of stale entries (every 60s)
        if now.duration_since(state.last_cleanup) > Duration::from_secs(60) {
            state.requests.retain(|_, timestamps| {
                timestamps
                    .last()
                    .is_some_and(|t| now.duration_since(*t) < self.window)
            });
            state.last_cleanup = now;
        }

        let timestamps = state.requests.entry(ip.to_string()).or_default();

        // Remove expired timestamps
        timestamps.retain(|t| now.duration_since(*t) < self.window);

        if timestamps.len() >= self.max_requests as usize {
            // Calculate retry-after from oldest request in window
            let oldest = timestamps[0];
            let retry_after = self.window.as_secs()
                - now
                    .duration_since(oldest)
                    .as_secs()
                    .min(self.window.as_secs());
            return Err(retry_after.max(1));
        }

        timestamps.push(now);
        Ok(())
    }
}

/// Extract client IP from request, checking X-Forwarded-For first.
pub fn extract_client_ip(headers: &HeaderMap, addr: Option<&ConnectInfo<SocketAddr>>) -> String {
    // Check X-Forwarded-For header (first IP is the original client)
    if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(first_ip) = forwarded.split(',').next()
    {
        let ip = first_ip.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }

    // Fall back to direct connection address
    addr.map(|a| a.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Rate limit exceeded response: 429 Too Many Requests.
pub fn rate_limit_response(retry_after: u64) -> Response {
    let body = serde_json::json!({
        "error": format!("Too many requests. Try again in {} seconds.", retry_after)
    });
    (
        StatusCode::TOO_MANY_REQUESTS,
        [("retry-after", retry_after.to_string())],
        axum::Json(body),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_allows_under_limit() {
        let limiter = RateLimiter::new(5, Duration::from_secs(60));
        for _ in 0..5 {
            assert!(limiter.check("1.2.3.4").is_ok());
        }
    }

    #[test]
    fn test_rate_limiter_blocks_over_limit() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
        for _ in 0..3 {
            assert!(limiter.check("1.2.3.4").is_ok());
        }
        let result = limiter.check("1.2.3.4");
        assert!(result.is_err());
        let retry_after = result.unwrap_err();
        assert!(retry_after >= 1);
    }

    #[test]
    fn test_rate_limiter_separate_ips() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.check("1.1.1.1").is_ok());
        assert!(limiter.check("1.1.1.1").is_ok());
        assert!(limiter.check("1.1.1.1").is_err());
        // Different IP should still be allowed
        assert!(limiter.check("2.2.2.2").is_ok());
    }

    #[test]
    fn test_extract_client_ip_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "203.0.113.50, 70.41.3.18".parse().unwrap(),
        );
        let ip = extract_client_ip(&headers, None);
        assert_eq!(ip, "203.0.113.50");
    }

    #[test]
    fn test_extract_client_ip_direct() {
        let headers = HeaderMap::new();
        let addr = ConnectInfo("127.0.0.1:12345".parse().unwrap());
        let ip = extract_client_ip(&headers, Some(&addr));
        assert_eq!(ip, "127.0.0.1");
    }

    #[test]
    fn test_extract_client_ip_fallback() {
        let headers = HeaderMap::new();
        let ip = extract_client_ip(&headers, None);
        assert_eq!(ip, "unknown");
    }
}
