// LLM Rate Limit Retry Logic
//
// Provider-specific retry handling for transient API errors (429, 408, 409, 5xx).
// Separate from durable execution RetryPolicy - this handles transient API errors.
//
// Aligns with official SDK behavior:
// - Anthropic SDK: https://github.com/anthropics/anthropic-sdk-python
// - OpenAI SDK: https://github.com/openai/openai-python
//
// Provider-specific headers:
// - Anthropic: retry-after, retry-after-ms, anthropic-ratelimit-*
// - OpenAI: retry-after, retry-after-ms, x-ratelimit-*
//
// Design: exponential backoff with 25% jitter, respecting provider retry-after hints.
// Defaults match official SDKs: 2 retries, 1s initial, 60s max, 2x multiplier.

use std::time::Duration;

/// Maximum retry-after value to honor (seconds).
/// Matches official SDK behavior - if server says wait longer, use backoff instead.
const MAX_RETRY_AFTER_SECS: u64 = 60;

/// Configuration for LLM rate limit retry behavior.
///
/// Defaults match official Anthropic/OpenAI SDK behavior:
/// - max_retries: 2
/// - initial_backoff: 1 second
/// - max_backoff: 60 seconds
/// - backoff_multiplier: 2.0
/// - jitter_factor: 0.25 (±25%)
#[derive(Debug, Clone)]
pub struct LlmRetryConfig {
    /// Maximum number of retry attempts (0 = no retries)
    pub max_retries: u32,
    /// Initial backoff duration (before exponential increase)
    pub initial_backoff: Duration,
    /// Maximum backoff duration (cap for exponential growth)
    pub max_backoff: Duration,
    /// Backoff multiplier (typically 2.0 for exponential)
    pub backoff_multiplier: f64,
    /// Jitter factor (0.0-1.0, adds randomness to avoid thundering herd)
    /// Official SDKs use 0.25 (±25%)
    pub jitter_factor: f64,
}

impl Default for LlmRetryConfig {
    fn default() -> Self {
        // Matches official Anthropic/OpenAI SDK defaults
        Self {
            max_retries: 2,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            jitter_factor: 0.25,
        }
    }
}

impl LlmRetryConfig {
    /// Create a config with no retries (fail immediately on rate limit)
    pub fn no_retry() -> Self {
        Self {
            max_retries: 0,
            ..Default::default()
        }
    }

    /// Create a config with aggressive retry settings (more retries, longer waits)
    pub fn aggressive() -> Self {
        Self {
            max_retries: 5,
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(120),
            backoff_multiplier: 2.0,
            jitter_factor: 0.25,
        }
    }

    /// Calculate backoff duration for a given attempt number (0-indexed)
    pub fn calculate_backoff(&self, attempt: u32) -> Duration {
        let base_backoff =
            self.initial_backoff.as_secs_f64() * self.backoff_multiplier.powi(attempt as i32);
        let capped_backoff = base_backoff.min(self.max_backoff.as_secs_f64());

        // Add jitter (±jitter_factor around the base)
        // Official SDKs use: sleep_seconds * (1 - 0.25 * random()) where random is 0-1
        // This gives range [0.75, 1.0] * base
        let jitter = if self.jitter_factor > 0.0 {
            let jitter_range = capped_backoff * self.jitter_factor;
            // Simple deterministic jitter based on attempt number
            // In production, use rand crate for true randomness
            let jitter_offset = (attempt as f64 * 0.37).fract() * 2.0 - 1.0; // -1 to 1
            jitter_range * jitter_offset
        } else {
            0.0
        };

        Duration::from_secs_f64((capped_backoff + jitter).max(0.0))
    }
}

/// Rate limit information extracted from provider response headers
#[derive(Debug, Clone, Default)]
pub struct RateLimitInfo {
    /// Retry-After header value (seconds to wait)
    pub retry_after_secs: Option<u64>,
    /// Requests remaining before limit
    pub requests_remaining: Option<u32>,
    /// Tokens remaining before limit
    pub tokens_remaining: Option<u32>,
    /// Time until request limit resets
    pub requests_reset: Option<String>,
    /// Time until token limit resets
    pub tokens_reset: Option<String>,
    /// Provider-specific limit type that was hit
    pub limit_type: Option<RateLimitType>,
}

/// Type of rate limit that was exceeded
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitType {
    /// Requests per minute/hour
    Requests,
    /// Input tokens per minute
    InputTokens,
    /// Output tokens per minute
    OutputTokens,
    /// Total tokens per minute
    TotalTokens,
    /// Unknown or unspecified
    Unknown,
}

impl RateLimitInfo {
    /// Get the recommended wait duration, preferring retry-after if available.
    /// Caps retry-after at MAX_RETRY_AFTER_SECS (60s) like official SDKs.
    pub fn recommended_wait(&self, config: &LlmRetryConfig, attempt: u32) -> Duration {
        if let Some(retry_after) = self.retry_after_secs {
            // Provider told us how long to wait
            // Cap at 60s like official SDKs - if longer, use backoff instead
            if retry_after > 0 && retry_after <= MAX_RETRY_AFTER_SECS {
                return Duration::from_secs(retry_after);
            }
        }
        // Fall back to exponential backoff
        config.calculate_backoff(attempt)
    }

    /// Parse rate limit info from Anthropic response headers
    pub fn from_anthropic_headers(headers: &reqwest::header::HeaderMap) -> Self {
        let mut info = Self::default();

        // Try non-standard retry-after-ms header first (milliseconds)
        // Used by some providers for sub-second precision
        if let Some(val) = headers.get("retry-after-ms")
            && let Ok(s) = val.to_str()
            && let Ok(ms) = s.parse::<u64>()
        {
            // Convert ms to seconds (round up)
            info.retry_after_secs = Some(ms.div_ceil(1000));
        }

        // retry-after header (standard, seconds)
        if info.retry_after_secs.is_none()
            && let Some(val) = headers.get("retry-after")
            && let Ok(s) = val.to_str()
        {
            info.retry_after_secs = s.parse().ok();
        }

        // anthropic-ratelimit-requests-remaining
        if let Some(val) = headers.get("anthropic-ratelimit-requests-remaining")
            && let Ok(s) = val.to_str()
        {
            info.requests_remaining = s.parse().ok();
        }

        // anthropic-ratelimit-tokens-remaining
        if let Some(val) = headers.get("anthropic-ratelimit-tokens-remaining")
            && let Ok(s) = val.to_str()
        {
            info.tokens_remaining = s.parse().ok();
        }

        // anthropic-ratelimit-requests-reset
        if let Some(val) = headers.get("anthropic-ratelimit-requests-reset")
            && let Ok(s) = val.to_str()
        {
            info.requests_reset = Some(s.to_string());
        }

        // anthropic-ratelimit-tokens-reset
        if let Some(val) = headers.get("anthropic-ratelimit-tokens-reset")
            && let Ok(s) = val.to_str()
        {
            info.tokens_reset = Some(s.to_string());
        }

        // Determine limit type from remaining values
        if info.requests_remaining == Some(0) {
            info.limit_type = Some(RateLimitType::Requests);
        } else if info.tokens_remaining == Some(0) {
            info.limit_type = Some(RateLimitType::InputTokens);
        }

        info
    }

    /// Parse rate limit info from OpenAI response headers
    pub fn from_openai_headers(headers: &reqwest::header::HeaderMap) -> Self {
        let mut info = Self::default();

        // Try non-standard retry-after-ms header first (milliseconds)
        if let Some(val) = headers.get("retry-after-ms")
            && let Ok(s) = val.to_str()
            && let Ok(ms) = s.parse::<u64>()
        {
            // Convert ms to seconds (round up)
            info.retry_after_secs = Some(ms.div_ceil(1000));
        }

        // retry-after header (standard, seconds)
        if info.retry_after_secs.is_none()
            && let Some(val) = headers.get("retry-after")
            && let Ok(s) = val.to_str()
        {
            info.retry_after_secs = s.parse().ok();
        }

        // x-ratelimit-remaining-requests
        if let Some(val) = headers.get("x-ratelimit-remaining-requests")
            && let Ok(s) = val.to_str()
        {
            info.requests_remaining = s.parse().ok();
        }

        // x-ratelimit-remaining-tokens
        if let Some(val) = headers.get("x-ratelimit-remaining-tokens")
            && let Ok(s) = val.to_str()
        {
            // OpenAI sometimes returns -1 for unlimited
            let val: i64 = s.parse().unwrap_or(-1);
            if val >= 0 {
                info.tokens_remaining = Some(val as u32);
            }
        }

        // x-ratelimit-reset-requests (e.g., "1s", "6m0s")
        if let Some(val) = headers.get("x-ratelimit-reset-requests")
            && let Ok(s) = val.to_str()
        {
            info.requests_reset = Some(s.to_string());
            // Try to parse as seconds for retry-after fallback
            if info.retry_after_secs.is_none() {
                info.retry_after_secs = parse_duration_string(s);
            }
        }

        // x-ratelimit-reset-tokens
        if let Some(val) = headers.get("x-ratelimit-reset-tokens")
            && let Ok(s) = val.to_str()
        {
            info.tokens_reset = Some(s.to_string());
        }

        // Determine limit type
        if info.requests_remaining == Some(0) {
            info.limit_type = Some(RateLimitType::Requests);
        } else if info.tokens_remaining == Some(0) {
            info.limit_type = Some(RateLimitType::TotalTokens);
        }

        info
    }
}

/// Parse duration strings like "1s", "6m0s", "1h30m"
fn parse_duration_string(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let mut total_secs: u64 = 0;
    let mut current_num = String::new();

    for c in s.chars() {
        if c.is_ascii_digit() {
            current_num.push(c);
        } else {
            let num: u64 = current_num.parse().ok()?;
            current_num.clear();

            match c {
                'h' => total_secs += num * 3600,
                'm' => total_secs += num * 60,
                's' => total_secs += num,
                _ => return None,
            }
        }
    }

    if total_secs > 0 {
        Some(total_secs)
    } else {
        None
    }
}

/// Metadata about retry attempts for observability
#[derive(Debug, Clone, Default)]
pub struct RetryMetadata {
    /// Number of retry attempts made (0 = succeeded on first try)
    pub attempts: u32,
    /// Total time spent waiting between retries
    pub total_retry_wait: Duration,
    /// Rate limit info from the last 429 response (if any)
    pub last_rate_limit_info: Option<RateLimitInfo>,
}

impl RetryMetadata {
    /// Check if any retries were made
    pub fn had_retries(&self) -> bool {
        self.attempts > 0
    }

    /// Create metadata for a successful first attempt
    pub fn first_attempt_success() -> Self {
        Self::default()
    }

    /// Record a retry attempt
    pub fn record_retry(
        &mut self,
        wait_duration: Duration,
        rate_limit_info: Option<RateLimitInfo>,
    ) {
        self.attempts += 1;
        self.total_retry_wait += wait_duration;
        if rate_limit_info.is_some() {
            self.last_rate_limit_info = rate_limit_info;
        }
    }
}

/// Check if an HTTP status code is a rate limit error (429)
pub fn is_rate_limit_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

/// Check if an error is a transient error that should be retried.
///
/// Matches official SDK behavior - retries on:
/// - 408 Request Timeout
/// - 409 Conflict (lock timeout)
/// - 429 Too Many Requests (rate limit)
/// - 5xx Server errors (except 501 Not Implemented)
pub fn is_transient_error(status: reqwest::StatusCode) -> bool {
    // 408 Request Timeout
    if status == reqwest::StatusCode::REQUEST_TIMEOUT {
        return true;
    }
    // 409 Conflict (often lock timeout in APIs)
    if status == reqwest::StatusCode::CONFLICT {
        return true;
    }
    // 429 Too Many Requests
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return true;
    }
    // 5xx Server errors (except 501 Not Implemented)
    if status.is_server_error() && status != reqwest::StatusCode::NOT_IMPLEMENTED {
        return true;
    }
    false
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_matches_official_sdks() {
        // Defaults should match official Anthropic/OpenAI SDK behavior
        let config = LlmRetryConfig::default();
        assert_eq!(config.max_retries, 2); // SDK default is 2
        assert_eq!(config.initial_backoff, Duration::from_secs(1));
        assert_eq!(config.max_backoff, Duration::from_secs(60));
        assert_eq!(config.backoff_multiplier, 2.0);
        assert!((config.jitter_factor - 0.25).abs() < 0.001); // SDK uses ±25%
    }

    #[test]
    fn test_calculate_backoff_exponential() {
        let config = LlmRetryConfig {
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            jitter_factor: 0.0, // No jitter for predictable test
            ..Default::default()
        };

        // attempt 0: 1s * 2^0 = 1s
        assert_eq!(config.calculate_backoff(0), Duration::from_secs(1));
        // attempt 1: 1s * 2^1 = 2s
        assert_eq!(config.calculate_backoff(1), Duration::from_secs(2));
        // attempt 2: 1s * 2^2 = 4s
        assert_eq!(config.calculate_backoff(2), Duration::from_secs(4));
        // attempt 3: 1s * 2^3 = 8s
        assert_eq!(config.calculate_backoff(3), Duration::from_secs(8));
    }

    #[test]
    fn test_calculate_backoff_capped() {
        let config = LlmRetryConfig {
            initial_backoff: Duration::from_secs(10),
            max_backoff: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            jitter_factor: 0.0,
            ..Default::default()
        };

        // attempt 0: 10s
        assert_eq!(config.calculate_backoff(0), Duration::from_secs(10));
        // attempt 1: 20s
        assert_eq!(config.calculate_backoff(1), Duration::from_secs(20));
        // attempt 2: 40s -> capped to 30s
        assert_eq!(config.calculate_backoff(2), Duration::from_secs(30));
        // attempt 3: 80s -> capped to 30s
        assert_eq!(config.calculate_backoff(3), Duration::from_secs(30));
    }

    #[test]
    fn test_parse_duration_string() {
        assert_eq!(parse_duration_string("1s"), Some(1));
        assert_eq!(parse_duration_string("30s"), Some(30));
        assert_eq!(parse_duration_string("1m"), Some(60));
        assert_eq!(parse_duration_string("6m0s"), Some(360));
        assert_eq!(parse_duration_string("1h"), Some(3600));
        assert_eq!(parse_duration_string("1h30m"), Some(5400));
        assert_eq!(parse_duration_string("1h30m45s"), Some(5445));
        assert_eq!(parse_duration_string(""), None);
        assert_eq!(parse_duration_string("invalid"), None);
    }

    #[test]
    fn test_rate_limit_info_recommended_wait_with_retry_after() {
        let config = LlmRetryConfig::default();
        let info = RateLimitInfo {
            retry_after_secs: Some(10),
            ..Default::default()
        };

        // Should use retry-after, not exponential backoff
        assert_eq!(info.recommended_wait(&config, 0), Duration::from_secs(10));
        assert_eq!(info.recommended_wait(&config, 5), Duration::from_secs(10));
    }

    #[test]
    fn test_rate_limit_info_recommended_wait_capped_at_60s() {
        // Like official SDKs, if retry-after > 60s, use backoff instead
        let config = LlmRetryConfig {
            jitter_factor: 0.0, // No jitter for predictable test
            ..Default::default()
        };
        let info = RateLimitInfo {
            retry_after_secs: Some(120), // 2 minutes - too long
            ..Default::default()
        };

        // Should fall back to exponential backoff, not use 120s
        assert_eq!(info.recommended_wait(&config, 0), Duration::from_secs(1));
    }

    #[test]
    fn test_rate_limit_info_recommended_wait_fallback() {
        let config = LlmRetryConfig {
            initial_backoff: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            jitter_factor: 0.0,
            ..Default::default()
        };
        let info = RateLimitInfo::default(); // No retry-after

        // Should use exponential backoff
        assert_eq!(info.recommended_wait(&config, 0), Duration::from_secs(1));
        assert_eq!(info.recommended_wait(&config, 1), Duration::from_secs(2));
    }

    #[test]
    fn test_retry_metadata_record() {
        let mut meta = RetryMetadata::default();
        assert!(!meta.had_retries());
        assert_eq!(meta.attempts, 0);

        meta.record_retry(Duration::from_secs(1), None);
        assert!(meta.had_retries());
        assert_eq!(meta.attempts, 1);
        assert_eq!(meta.total_retry_wait, Duration::from_secs(1));

        meta.record_retry(Duration::from_secs(2), None);
        assert_eq!(meta.attempts, 2);
        assert_eq!(meta.total_retry_wait, Duration::from_secs(3));
    }

    #[test]
    fn test_is_transient_error_matches_official_sdks() {
        // Official SDKs retry on: 408, 409, 429, 5xx (except 501)
        assert!(is_transient_error(reqwest::StatusCode::REQUEST_TIMEOUT)); // 408
        assert!(is_transient_error(reqwest::StatusCode::CONFLICT)); // 409
        assert!(is_transient_error(reqwest::StatusCode::TOO_MANY_REQUESTS)); // 429
        assert!(is_transient_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        )); // 500
        assert!(is_transient_error(reqwest::StatusCode::BAD_GATEWAY)); // 502
        assert!(is_transient_error(reqwest::StatusCode::SERVICE_UNAVAILABLE)); // 503
        assert!(is_transient_error(reqwest::StatusCode::GATEWAY_TIMEOUT)); // 504

        // Not transient
        assert!(!is_transient_error(reqwest::StatusCode::OK));
        assert!(!is_transient_error(reqwest::StatusCode::BAD_REQUEST)); // 400
        assert!(!is_transient_error(reqwest::StatusCode::UNAUTHORIZED)); // 401
        assert!(!is_transient_error(reqwest::StatusCode::FORBIDDEN)); // 403
        assert!(!is_transient_error(reqwest::StatusCode::NOT_FOUND)); // 404
        assert!(!is_transient_error(reqwest::StatusCode::NOT_IMPLEMENTED)); // 501
    }

    #[test]
    fn test_max_retry_after_constant() {
        // Verify the constant matches SDK behavior
        assert_eq!(MAX_RETRY_AFTER_SECS, 60);
    }
}
