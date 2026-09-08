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

#[cfg(feature = "http")]
use crate::error::AgentLoopError;
use rand::RngExt;
#[cfg(feature = "http")]
use std::future::Future;
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
    /// Maximum wall-clock time spent recovering after the first transient
    /// failure. The initial request is not charged against this budget.
    pub max_retry_elapsed: Duration,
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
            max_retry_elapsed: Duration::from_secs(30),
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
            max_retry_elapsed: Duration::from_secs(120),
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
            // EVE-635: real RNG so concurrent clients hitting the same attempt
            // number (e.g. a shared 429/503) do not retry in lockstep
            // (thundering herd). Range [-1, 1).
            let jitter_offset = rand::rng().random::<f64>() * 2.0 - 1.0;
            jitter_range * jitter_offset
        } else {
            0.0
        };

        Duration::from_secs_f64((capped_backoff + jitter).max(0.0))
    }
}

/// Reserve one retry wait inside the configured elapsed-time budget.
///
/// `started_at` is initialized on the first transient failure, so a slow but
/// successful initial request does not consume recovery time. Returning `None`
/// means the caller must surface the failure instead of starting another
/// attempt that cannot finish within the strict budget.
pub fn reserve_retry_wait(
    config: &LlmRetryConfig,
    started_at: &mut Option<tokio::time::Instant>,
    wait: Duration,
) -> Option<Duration> {
    let started = *started_at.get_or_insert_with(tokio::time::Instant::now);
    let remaining = config.max_retry_elapsed.checked_sub(started.elapsed())?;
    (wait < remaining).then_some(wait)
}

/// Remaining wall-clock retry budget after recovery has started.
pub fn remaining_retry_time(
    config: &LlmRetryConfig,
    started_at: Option<tokio::time::Instant>,
) -> Option<Duration> {
    started_at.and_then(|started| config.max_retry_elapsed.checked_sub(started.elapsed()))
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
    #[cfg(feature = "http")]
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

    /// Parse rate limit info from OpenAI-compatible response headers.
    #[cfg(feature = "http")]
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
            info.tokens_remaining = s.parse::<u32>().ok();
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
#[cfg(feature = "http")]
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

            let multiplier = match c {
                'h' => 3600,
                'm' => 60,
                's' => 1,
                _ => return None,
            };
            total_secs = total_secs.checked_add(num.checked_mul(multiplier)?)?;
        }
    }

    if total_secs > 0 && current_num.is_empty() {
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
    /// Wall-clock time consumed after automatic recovery began.
    pub total_retry_elapsed: Duration,
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

    /// Merge retries consumed by a nested request/reconnect layer.
    pub fn absorb(&mut self, other: RetryMetadata) {
        self.attempts = self.attempts.saturating_add(other.attempts);
        self.total_retry_wait = self.total_retry_wait.saturating_add(other.total_retry_wait);
        self.total_retry_elapsed = self
            .total_retry_elapsed
            .saturating_add(other.total_retry_elapsed);
        if other.last_rate_limit_info.is_some() {
            self.last_rate_limit_info = other.last_rate_limit_info;
        }
    }
}

/// Check if an HTTP status code is a rate limit error (429)
#[cfg(feature = "http")]
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
#[cfg(feature = "http")]
pub fn is_transient_error(status: reqwest::StatusCode) -> bool {
    is_transient_status(status.as_u16())
}

fn is_transient_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 429 | 500 | 502..=599)
}

/// Check if a `reqwest` error raised while *sending* a request is a transient
/// connection-level failure that is safe to retry.
///
/// These are errors where the request never produced an HTTP response: a
/// connect/TLS failure, a connect timeout, or a generic send failure on a
/// (possibly stale) pooled keep-alive connection — surfaced by reqwest as
/// `error sending request for url ...`. The official Anthropic/OpenAI SDKs
/// retry exactly these as `APIConnectionError`. Because the server produced no
/// response, the request had no effect, so retrying is safe.
///
/// This matters since EVE-635 introduced a process-wide shared connection pool:
/// a keep-alive connection the peer has already closed is only discovered when
/// the next request tries to reuse it, and that send fails here rather than
/// returning an HTTP status — so it must be retried alongside the 429/5xx path
/// (see `is_transient_error`).
#[cfg(feature = "http")]
pub fn is_transient_send_error(err: &reqwest::Error) -> bool {
    err.is_connect() || err.is_timeout() || err.is_request()
}

/// Build the user-facing message for a request that failed to send, noting how
/// many retries were exhausted so it mirrors the HTTP-status error path.
#[cfg(feature = "http")]
pub fn send_error_message(err: &reqwest::Error, attempts: u32) -> String {
    if attempts > 0 {
        format!("Failed to send request: {err} (after {attempts} retries)")
    } else {
        format!("Failed to send request: {err}")
    }
}

/// Check if an in-band provider error message looks transient and safe to retry.
///
/// This complements HTTP-status-based retry detection for streaming APIs that can
/// emit retryable provider failures inside an otherwise successful event stream.
pub fn is_transient_error_message(message: &str) -> bool {
    // A subscription/plan usage limit (e.g. Codex `usage_limit_reached`) surfaces
    // as a 429 ("too many requests") but does not recover within the retry
    // window — it resets hours later at `resets_at`. Treating it as transient
    // would waste retries and suppress the human-readable error message the
    // reason atom emits for terminal failures, so it is explicitly non-transient.
    if crate::user_facing_error::is_usage_limit_message(message) {
        return false;
    }

    let msg = message.trim().to_ascii_lowercase();

    // EVE-806: the runtime's stream-liveness watchdog aborts a stream that
    // produced no tokens within its window with "provider stream stall: no
    // tokens for Ns". A stall before any output is equivalent to a dropped
    // connection, so it is transient and safe for the shared bounded retry path
    // to recover.
    if msg.contains("provider stream stall") {
        return true;
    }

    [
        "server_error",
        "internal server error",
        "overloaded",
        "overloaded_error",
        "rate limit",
        "too many requests",
        "request timeout",
        "timed out",
        "service unavailable",
        "bad gateway",
        "gateway timeout",
        "temporarily unavailable",
    ]
    .iter()
    .any(|needle| msg.contains(needle))
}

/// Classify an in-band provider error using structured fields first.
///
/// Machine-readable provider codes are authoritative, followed by HTTP status.
/// Message matching is retained only for legacy drivers that cannot preserve
/// either field.
pub fn is_transient_stream_error(error: &crate::driver_registry::LlmStreamError) -> bool {
    if let Some(code) = error.code.as_deref()
        && let Some(kind) = crate::error::LlmErrorKind::from_provider_code(code)
    {
        return matches!(
            kind,
            crate::error::LlmErrorKind::RateLimited | crate::error::LlmErrorKind::Unavailable
        );
    }

    if let Some(status) = error.status {
        return is_transient_status(status);
    }

    is_transient_error_message(&error.message)
}

// ============================================================================
// Generic retry executor
// ============================================================================

/// Outcome of a failed *send* (no HTTP response was produced).
///
/// Lets the [`retry_request`] caller's send closure distinguish a hard error
/// that must propagate immediately (e.g. an auth-header failure) from a
/// `reqwest` transport error that should be classified for transient retry.
#[cfg(feature = "http")]
pub enum SendOutcome {
    /// A `reqwest` send error (connection/TLS/timeout). Classified via
    /// [`is_transient_send_error`] for retry.
    Send(reqwest::Error),
    /// A non-transport error that must abort the loop immediately (e.g. an
    /// `ProviderAuth` failure resolved per attempt).
    Fatal(AgentLoopError),
}

/// Decision returned by the per-attempt classifier when a response carries a
/// non-success HTTP status.
#[cfg(feature = "http")]
pub enum RetryDecision {
    /// Retry after the given wait duration, recording the supplied rate-limit
    /// info against the retry metadata.
    Retry {
        wait: Duration,
        rate_limit_info: Option<RateLimitInfo>,
    },
    /// Retry immediately without counting an attempt or sleeping (e.g.
    /// Anthropic's one-shot `max_tokens` fallback that mutates the request and
    /// re-sends). Use sparingly — the classifier is responsible for ensuring
    /// this cannot loop forever.
    RetryNow,
    /// Stop and return this terminal error to the caller.
    Terminal(AgentLoopError),
}

/// Run the shared LLM request retry loop.
///
/// Reproduces the loop every native streaming driver hand-rolled: send the
/// request, retry transient *send* failures with backoff, break on success,
/// and otherwise defer to a provider-specific `classify` closure for terminal
/// vs. retry decisions on a non-success HTTP status.
///
/// - `send` builds and sends the request fresh each attempt (so per-attempt
///   auth/header rebuilds are the caller's responsibility). It returns the
///   `reqwest::Response` on success, or a [`SendOutcome`] distinguishing a
///   transport error (retryable) from a fatal error (propagated immediately).
/// - `classify` is invoked with the failed response, the current attempt count,
///   and a `bool` indicating whether more retries remain; it consumes the
///   response body as needed and returns a [`RetryDecision`].
/// - `send_error` builds the terminal error when a send failure is not (or no
///   longer) retryable, given the `reqwest::Error` and the attempt count.
///
/// On success returns the `reqwest::Response` plus the accumulated
/// [`RetryMetadata`].
#[cfg(feature = "http")]
pub async fn retry_request<S, SFut, C, CFut, E>(
    config: &LlmRetryConfig,
    driver_name: &str,
    mut send: S,
    mut classify: C,
    send_error: E,
) -> Result<(reqwest::Response, RetryMetadata), AgentLoopError>
where
    S: FnMut() -> SFut,
    SFut: Future<Output = Result<reqwest::Response, SendOutcome>>,
    C: FnMut(reqwest::Response, u32, bool) -> CFut,
    CFut: Future<Output = RetryDecision>,
    E: Fn(&reqwest::Error, u32) -> AgentLoopError,
{
    let mut retry_metadata = RetryMetadata::default();
    let mut retry_started_at = None;

    let budget_exhausted = |metadata: &RetryMetadata| {
        AgentLoopError::llm_kind(
            crate::error::LlmErrorKind::Unavailable,
            format!(
                "{driver_name} retry time budget exhausted after {} retries over {:.1}s",
                metadata.attempts,
                config.max_retry_elapsed.as_secs_f64()
            ),
        )
        .with_retry_metadata(metadata)
    };

    let response = loop {
        let send_result = if retry_started_at.is_some() {
            // A late wakeup can exhaust the budget between reserving a wait and
            // sending again. None here means expired, not an initial request.
            let remaining = remaining_retry_time(config, retry_started_at).unwrap_or_default();
            if remaining.is_zero() {
                return Err(budget_exhausted(&retry_metadata));
            }
            match tokio::time::timeout(remaining, send()).await {
                Ok(result) => result,
                Err(_) => return Err(budget_exhausted(&retry_metadata)),
            }
        } else {
            send().await
        };
        let response = match send_result {
            Ok(response) => response,
            Err(SendOutcome::Fatal(err)) => return Err(err),
            Err(SendOutcome::Send(e)) => {
                // A send failure never produced an HTTP response, so it bypasses
                // the status-based retry below. Connection-level errors (incl. a
                // stale pooled keep-alive connection, EVE-635) are transient —
                // retry them with backoff, matching SDK `APIConnectionError`.
                if is_transient_send_error(&e) && retry_metadata.attempts < config.max_retries {
                    let proposed_wait = config.calculate_backoff(retry_metadata.attempts);
                    let Some(wait_duration) =
                        reserve_retry_wait(config, &mut retry_started_at, proposed_wait)
                    else {
                        return Err(send_error(&e, retry_metadata.attempts)
                            .with_retry_metadata(&retry_metadata));
                    };
                    tracing::warn!(
                        error = %e,
                        driver = driver_name,
                        attempt = retry_metadata.attempts + 1,
                        max_retries = config.max_retries,
                        wait_secs = wait_duration.as_secs_f64(),
                        "transient connection error sending request, retrying"
                    );
                    retry_metadata.record_retry(wait_duration, None);
                    tokio::time::sleep(wait_duration).await;
                    continue;
                }
                return Err(
                    send_error(&e, retry_metadata.attempts).with_retry_metadata(&retry_metadata)
                );
            }
        };

        let status = response.status();
        if status.is_success() {
            break response;
        }

        let can_retry = is_transient_error(status) && retry_metadata.attempts < config.max_retries;
        match classify(response, retry_metadata.attempts, can_retry).await {
            RetryDecision::Retry {
                wait,
                rate_limit_info,
            } => {
                let Some(wait) = reserve_retry_wait(config, &mut retry_started_at, wait) else {
                    return Err(budget_exhausted(&retry_metadata));
                };
                tracing::warn!(
                    status = %status,
                    driver = driver_name,
                    attempt = retry_metadata.attempts + 1,
                    max_retries = config.max_retries,
                    wait_secs = wait.as_secs_f64(),
                    "rate limit or transient error, retrying"
                );
                retry_metadata.record_retry(wait, rate_limit_info);
                tokio::time::sleep(wait).await;
                continue;
            }
            RetryDecision::RetryNow => continue,
            RetryDecision::Terminal(err) => {
                return Err(err.with_retry_metadata(&retry_metadata));
            }
        }
    };

    if retry_metadata.had_retries() {
        retry_metadata.total_retry_elapsed = retry_started_at
            .map(|started| started.elapsed())
            .unwrap_or_default();
        tracing::info!(
            driver = driver_name,
            attempts = retry_metadata.attempts,
            total_wait_secs = retry_metadata.total_retry_wait.as_secs_f64(),
            "request succeeded after retries"
        );
    }

    Ok((response, retry_metadata))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn default_retry_policy_stops_after_three_requests() {
        let config = LlmRetryConfig {
            jitter_factor: 0.0,
            ..Default::default()
        };
        let mut calls = 0;
        let error = retry_request(
            &config,
            "TestDriver",
            || {
                calls += 1;
                async { Ok(fake_response(429, "limited")) }
            },
            |response, attempt, can_retry| {
                assert_eq!(response.status().as_u16(), 429);
                assert_eq!(can_retry, attempt < 2);
                let wait = config.calculate_backoff(attempt);
                async move {
                    if can_retry {
                        RetryDecision::Retry {
                            wait,
                            rate_limit_info: None,
                        }
                    } else {
                        RetryDecision::Terminal(AgentLoopError::llm("exhausted"))
                    }
                }
            },
            |_, _| panic!("unexpected transport error"),
        )
        .await
        .unwrap_err();
        assert_eq!(calls, 3);
        let AgentLoopError::Llm(error) = error else {
            panic!("lost LLM variant")
        };
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({"kind":"other","message":"exhausted","retry_attempts":2,"retry_wait_ms":3000,"retry_handled":true})
        );
    }

    #[test]
    fn backoff_exponential_progression_respects_the_cap() {
        let defaults = LlmRetryConfig {
            jitter_factor: 0.0,
            ..Default::default()
        };
        let custom = LlmRetryConfig {
            initial_backoff: Duration::from_secs(10),
            max_backoff: Duration::from_secs(30),
            jitter_factor: 0.0,
            ..Default::default()
        };
        for (config, expected) in [
            (defaults, [1, 2, 4, 8, 16, 32, 60, 60]),
            (custom, [10, 20, 30, 30, 30, 30, 30, 30]),
        ] {
            for (attempt, seconds) in expected.into_iter().enumerate() {
                assert_eq!(
                    config.calculate_backoff(attempt as u32),
                    Duration::from_secs(seconds)
                );
            }
        }
    }

    /// EVE-635: with jitter enabled the backoff must use a real RNG, so repeated
    /// computations for the same attempt number diverge (no thundering herd).
    #[test]
    fn test_backoff_jitter_is_randomized() {
        let config = LlmRetryConfig {
            initial_backoff: Duration::from_secs(10),
            max_backoff: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            ..Default::default()
        };
        let samples: std::collections::HashSet<u128> = (0..20)
            .map(|_| config.calculate_backoff(1).as_nanos())
            .collect();
        assert!(
            samples.len() > 1,
            "jittered backoff should vary across calls, got {} distinct value(s)",
            samples.len()
        );
        // Jitter stays within ±25% of the 20s base for attempt 1.
        for _ in 0..50 {
            let secs = config.calculate_backoff(1).as_secs_f64();
            assert!(
                (15.0..=25.0).contains(&secs),
                "backoff {secs}s out of range"
            );
        }
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
    fn retry_after_literal_boundaries_fall_back_to_backoff() {
        let config = LlmRetryConfig {
            jitter_factor: 0.0,
            ..Default::default()
        };
        for (hint, expected) in [
            (None, 4),
            (Some(0), 4),
            (Some(1), 1),
            (Some(10), 10),
            (Some(59), 59),
            (Some(60), 60),
            (Some(61), 4),
            (Some(120), 4),
            (Some(u64::MAX), 4),
        ] {
            let info = RateLimitInfo {
                retry_after_secs: hint,
                ..Default::default()
            };
            assert_eq!(
                info.recommended_wait(&config, 2),
                Duration::from_secs(expected),
                "{hint:?}"
            );
        }
    }

    #[test]
    fn retry_metadata_accumulates_waits_and_preserves_latest_limit_info() {
        let mut meta = RetryMetadata::first_attempt_success();
        assert!(!meta.had_retries());
        meta.record_retry(
            Duration::from_secs(1),
            Some(RateLimitInfo {
                retry_after_secs: Some(7),
                ..Default::default()
            }),
        );
        meta.record_retry(Duration::from_secs(2), None);
        assert!(meta.had_retries());
        assert_eq!(meta.attempts, 2);
        assert_eq!(meta.total_retry_wait, Duration::from_secs(3));
        assert_eq!(
            meta.last_rate_limit_info.as_ref().unwrap().retry_after_secs,
            Some(7)
        );
        meta.absorb(RetryMetadata {
            attempts: 3,
            total_retry_wait: Duration::from_secs(4),
            total_retry_elapsed: Duration::from_secs(9),
            last_rate_limit_info: Some(RateLimitInfo {
                retry_after_secs: Some(11),
                ..Default::default()
            }),
        });
        assert_eq!(meta.attempts, 5);
        assert_eq!(meta.total_retry_wait, Duration::from_secs(7));
        assert_eq!(meta.total_retry_elapsed, Duration::from_secs(9));
        assert_eq!(
            meta.last_rate_limit_info.as_ref().unwrap().retry_after_secs,
            Some(11)
        );
        meta.absorb(RetryMetadata {
            attempts: u32::MAX,
            total_retry_wait: Duration::MAX,
            total_retry_elapsed: Duration::MAX,
            last_rate_limit_info: None,
        });
        assert_eq!(meta.attempts, u32::MAX);
        assert_eq!(meta.total_retry_wait, Duration::MAX);
        assert_eq!(meta.total_retry_elapsed, Duration::MAX);
        assert_eq!(
            meta.last_rate_limit_info.as_ref().unwrap().retry_after_secs,
            Some(11)
        );
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
    fn test_is_transient_error_message_detects_provider_server_errors() {
        assert!(is_transient_error_message(
            "server_error: An error occurred while processing your request."
        ));
        assert!(is_transient_error_message("Rate limit exceeded"));
        assert!(is_transient_error_message(
            "Service temporarily unavailable"
        ));
    }

    #[test]
    fn test_is_transient_error_message_rejects_non_retryable_messages() {
        assert!(!is_transient_error_message(
            "invalid_request_error: bad tool schema"
        ));
        assert!(!is_transient_error_message("Model not available: gpt-99"));
    }

    #[test]
    fn structured_stream_error_prefers_code_and_status_over_message() {
        use crate::driver_registry::LlmStreamError;

        assert!(is_transient_stream_error(&LlmStreamError::provider(
            Some("processing_error"),
            None,
            "An error occurred while processing your request.",
        )));
        assert!(is_transient_stream_error(&LlmStreamError::provider(
            None::<String>,
            Some(503),
            "opaque failure",
        )));
        assert!(!is_transient_stream_error(&LlmStreamError::provider(
            Some("invalid_request_error"),
            Some(503),
            "server unavailable",
        )));
        assert!(!is_transient_stream_error(&LlmStreamError::provider(
            Some("insufficient_quota"),
            Some(429),
            "rate limit",
        )));
    }

    #[test]
    fn test_provider_stream_stall_is_transient() {
        // EVE-806: the runtime stream-liveness watchdog message must be
        // classified transient so the shared bounded retry path recovers it.
        assert!(is_transient_error_message(
            "provider stream stall: no tokens for 120s"
        ));
        // Untyped stream error carrying the stall message routes through the
        // message fallback in is_transient_stream_error.
        use crate::driver_registry::LlmStreamError;
        assert!(is_transient_stream_error(&LlmStreamError::new(
            "provider stream stall: no tokens for 120s"
        )));
    }

    #[test]
    fn test_is_transient_error_message_treats_usage_limit_as_non_transient() {
        // Codex `usage_limit_reached` arrives as a 429 ("too many requests") but
        // resets hours later — retrying inside the backoff window is pointless
        // and would suppress the terminal user-facing message.
        assert!(!is_transient_error_message(
            "Codex API error (429 Too Many Requests): {\"error\":{\"type\":\"usage_limit_reached\",\"resets_at\":1783767823}}"
        ));
    }

    // ------------------------------------------------------------------------
    // retry_request executor tests (no live server; responses are synthesized)
    // ------------------------------------------------------------------------

    /// Build a `reqwest::Response` with a chosen status and body for testing.
    fn fake_response(status: u16, body: &str) -> reqwest::Response {
        let http_response = http::Response::builder()
            .status(status)
            .body(body.to_string())
            .unwrap();
        reqwest::Response::from(http_response)
    }

    /// Zero-backoff config so retries don't actually sleep in tests.
    fn fast_config(max_retries: u32) -> LlmRetryConfig {
        LlmRetryConfig {
            max_retries,
            initial_backoff: Duration::from_millis(0),
            max_backoff: Duration::from_millis(0),
            backoff_multiplier: 1.0,
            jitter_factor: 0.0,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_retry_request_success_first_try() {
        let config = fast_config(2);
        let (resp, meta) = retry_request(
            &config,
            "TestDriver",
            || async { Ok(fake_response(200, "ok")) },
            |_resp, _attempt, _can_retry| async {
                RetryDecision::Terminal(AgentLoopError::llm("unreachable"))
            },
            |e, attempts| AgentLoopError::llm(send_error_message(e, attempts)),
        )
        .await
        .expect("should succeed");
        assert_eq!(resp.status().as_u16(), 200);
        assert_eq!(resp.text().await.unwrap(), "ok");
        assert_eq!(
            (
                meta.attempts,
                meta.total_retry_wait,
                meta.total_retry_elapsed
            ),
            (0, Duration::ZERO, Duration::ZERO)
        );
        assert!(meta.last_rate_limit_info.is_none());
    }

    #[tokio::test]
    async fn test_retry_request_retries_then_succeeds() {
        let config = fast_config(3);
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let calls_send = calls.clone();
        let (resp, meta) = retry_request(
            &config,
            "TestDriver",
            move || {
                let calls = calls_send.clone();
                async move {
                    let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    // First two attempts return 429, third succeeds.
                    if n < 2 {
                        Ok(fake_response(429, "rate limited"))
                    } else {
                        Ok(fake_response(200, "ok"))
                    }
                }
            },
            |_resp, _attempt, can_retry| async move {
                assert!(can_retry, "429 within budget should be retryable");
                RetryDecision::Retry {
                    wait: Duration::from_millis(0),
                    rate_limit_info: None,
                }
            },
            |e, attempts| AgentLoopError::llm(send_error_message(e, attempts)),
        )
        .await
        .expect("should eventually succeed");
        assert_eq!(resp.status().as_u16(), 200);
        assert_eq!(resp.text().await.unwrap(), "ok");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert_eq!(meta.attempts, 2);
        assert_eq!(meta.total_retry_wait, Duration::ZERO);
    }

    #[tokio::test]
    async fn test_retry_request_terminal_decision_propagates() {
        let config = fast_config(2);
        let result = retry_request(
            &config,
            "TestDriver",
            || async { Ok(fake_response(400, "bad request")) },
            |_resp, _attempt, _can_retry| async {
                RetryDecision::Terminal(AgentLoopError::llm("classified terminal"))
            },
            |e, attempts| AgentLoopError::llm(send_error_message(e, attempts)),
        )
        .await;
        let err = result.expect_err("terminal decision should error");
        let AgentLoopError::Llm(error) = err else {
            panic!("lost terminal LLM variant")
        };
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({"kind":"other","message":"classified terminal","retry_attempts":0,"retry_wait_ms":0,"retry_handled":true})
        );
    }

    #[tokio::test]
    async fn test_retry_request_retry_now_does_not_count_attempt() {
        let config = fast_config(2);
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let calls_send = calls.clone();
        let (resp, meta) = retry_request(
            &config,
            "TestDriver",
            move || {
                let calls = calls_send.clone();
                async move {
                    let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if n == 0 {
                        Ok(fake_response(400, "max_tokens too large"))
                    } else {
                        Ok(fake_response(200, "ok"))
                    }
                }
            },
            {
                let mut used_fallback = false;
                move |_resp, _attempt, _can_retry| {
                    let do_fallback = !used_fallback;
                    used_fallback = true;
                    async move {
                        if do_fallback {
                            RetryDecision::RetryNow
                        } else {
                            RetryDecision::Terminal(AgentLoopError::llm("unreachable"))
                        }
                    }
                }
            },
            |e, attempts| AgentLoopError::llm(send_error_message(e, attempts)),
        )
        .await
        .expect("RetryNow then success");
        assert_eq!(resp.status().as_u16(), 200);
        assert_eq!(resp.text().await.unwrap(), "ok");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!((meta.attempts, meta.total_retry_wait), (0, Duration::ZERO));
    }

    #[tokio::test]
    async fn test_retry_request_send_error_exhausts() {
        // A send closure that always returns a transient send error must, after
        // exhausting retries, return the send_error-built terminal error.
        let config = fast_config(1);

        // Obtain a real transient reqwest::Error (connection refused).
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let make_err = || async {
            reqwest::Client::new()
                .get(format!("http://{addr}/"))
                .send()
                .await
                .expect_err("closed port")
        };

        let result = retry_request(
            &config,
            "TestDriver",
            move || async move { Err(SendOutcome::Send(make_err().await)) },
            |_resp, _attempt, _can_retry| async {
                RetryDecision::Terminal(AgentLoopError::llm("unreachable"))
            },
            |e, attempts| AgentLoopError::llm(send_error_message(e, attempts)),
        )
        .await;
        let err = result.expect_err("send errors should exhaust to terminal");
        // After 1 retry, message notes the retry count.
        assert!(err.to_string().contains("after 1 retries"), "got: {err}");
        assert_eq!(err.llm_retry_attempts(), 1);
        assert!(err.llm_retry_handled());
    }

    #[tokio::test]
    async fn test_retry_request_fatal_send_propagates_immediately() {
        let config = fast_config(3);
        let result = retry_request(
            &config,
            "TestDriver",
            || async { Err(SendOutcome::Fatal(AgentLoopError::config("auth failed"))) },
            |_resp, _attempt, _can_retry| async {
                RetryDecision::Terminal(AgentLoopError::llm("unreachable"))
            },
            |e, attempts| AgentLoopError::llm(send_error_message(e, attempts)),
        )
        .await;
        let err = result.expect_err("fatal send should propagate");
        assert!(matches!(err,AgentLoopError::Configuration(message) if message=="auth failed"));
    }
    #[test]
    fn reset_duration_rejects_overflow_and_unterminated_components() {
        for value in [
            "18446744073709551615h",
            "18446744073709551615m",
            "18446744073709551615s1s",
            "1s2",
            "1m30",
        ] {
            assert_eq!(parse_duration_string(value), None, "{value}");
        }
        assert_eq!(
            parse_duration_string("18446744073709551615s"),
            Some(u64::MAX)
        );
    }

    #[test]
    fn openai_remaining_tokens_rejects_out_of_range_values() {
        let mut valid = reqwest::header::HeaderMap::new();
        valid.insert(
            "x-ratelimit-remaining-tokens",
            "4294967295".parse().unwrap(),
        );
        assert_eq!(
            RateLimitInfo::from_openai_headers(&valid).tokens_remaining,
            Some(u32::MAX)
        );
        for value in ["4294967296", "9223372036854775807", "-1", "invalid"] {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert("x-ratelimit-remaining-tokens", value.parse().unwrap());
            let info = RateLimitInfo::from_openai_headers(&headers);
            assert_eq!(info.tokens_remaining, None, "{value}");
            assert_eq!(info.limit_type, None, "{value}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn elapsed_retry_budget_does_not_start_an_unbounded_request() {
        use std::sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        };
        let calls = Arc::new(AtomicU32::new(0));
        let sent = calls.clone();
        let task = tokio::spawn(async move {
            let config = LlmRetryConfig {
                max_retry_elapsed: Duration::from_secs(2),
                ..fast_config(3)
            };
            retry_request(
                &config,
                "TestDriver",
                move || {
                    let index = sent.fetch_add(1, Ordering::SeqCst);
                    async move {
                        if index == 0 {
                            Ok(fake_response(503, "retry"))
                        } else {
                            std::future::pending().await
                        }
                    }
                },
                |_, _, _| async {
                    RetryDecision::Retry {
                        wait: Duration::from_secs(1),
                        rate_limit_info: None,
                    }
                },
                |_, _| AgentLoopError::llm("transport"),
            )
            .await
        });
        tokio::task::yield_now().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        tokio::time::advance(Duration::from_secs(3)).await;
        tokio::task::yield_now().await;
        assert!(
            task.is_finished(),
            "retry deadline must finish even after a late wakeup"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "expired retry must not send again"
        );
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(
            error.llm_error_kind(),
            Some(crate::error::LlmErrorKind::Unavailable)
        );
        assert_eq!(error.llm_retry_attempts(), 1);
        assert!(error.llm_retry_handled());
    }
    #[test]
    fn provider_headers_preserve_complete_limits_and_retry_hint_precedence() {
        for (parse, names, token_class) in [
            (
                RateLimitInfo::from_anthropic_headers
                    as fn(&reqwest::header::HeaderMap) -> RateLimitInfo,
                [
                    "anthropic-ratelimit-requests-remaining",
                    "anthropic-ratelimit-tokens-remaining",
                    "anthropic-ratelimit-requests-reset",
                    "anthropic-ratelimit-tokens-reset",
                ],
                RateLimitType::InputTokens,
            ),
            (
                RateLimitInfo::from_openai_headers
                    as fn(&reqwest::header::HeaderMap) -> RateLimitInfo,
                [
                    "x-ratelimit-remaining-requests",
                    "x-ratelimit-remaining-tokens",
                    "x-ratelimit-reset-requests",
                    "x-ratelimit-reset-tokens",
                ],
                RateLimitType::TotalTokens,
            ),
        ] {
            let mut headers = reqwest::header::HeaderMap::new();
            for (name, value) in [
                ("retry-after-ms", "1001"),
                ("retry-after", "9"),
                (names[0], "0"),
                (names[1], "0"),
                (names[2], "6m0s"),
                (names[3], "1h"),
            ] {
                headers.insert(name, value.parse().unwrap());
            }
            let info = parse(&headers);
            assert_eq!(
                (
                    info.retry_after_secs,
                    info.requests_remaining,
                    info.tokens_remaining,
                    info.requests_reset.as_deref(),
                    info.tokens_reset.as_deref(),
                    info.limit_type
                ),
                (
                    Some(2),
                    Some(0),
                    Some(0),
                    Some("6m0s"),
                    Some("1h"),
                    Some(RateLimitType::Requests)
                )
            );
            headers.insert(names[0], "3".parse().unwrap());
            assert_eq!(parse(&headers).limit_type, Some(token_class));
            for (milliseconds, expected) in [
                ("0", 0),
                ("1", 1),
                ("999", 1),
                ("1000", 1),
                ("1001", 2),
                ("invalid", 9),
            ] {
                headers.insert("retry-after-ms", milliseconds.parse().unwrap());
                assert_eq!(
                    parse(&headers).retry_after_secs,
                    Some(expected),
                    "{milliseconds}"
                );
            }
            headers.remove("retry-after-ms");
            headers.remove("retry-after");
            assert_eq!(
                RateLimitInfo::from_openai_headers(&headers).retry_after_secs,
                if names[0].starts_with("x-") {
                    Some(360)
                } else {
                    None
                }
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn retry_wait_reservations_enforce_strict_remaining_time() {
        let config = LlmRetryConfig {
            max_retry_elapsed: Duration::from_secs(3),
            ..Default::default()
        };
        let mut started = None;
        assert_eq!(remaining_retry_time(&config, started), None);
        assert_eq!(
            reserve_retry_wait(&config, &mut started, Duration::from_secs(3)),
            None
        );
        assert_eq!(
            reserve_retry_wait(&config, &mut started, Duration::from_secs(2)),
            Some(Duration::from_secs(2))
        );
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(
            remaining_retry_time(&config, started),
            Some(Duration::from_secs(1))
        );
        assert_eq!(
            reserve_retry_wait(&config, &mut started, Duration::from_secs(1)),
            None
        );
        assert_eq!(
            reserve_retry_wait(&config, &mut started, Duration::from_millis(999)),
            Some(Duration::from_millis(999))
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(
            reserve_retry_wait(&config, &mut started, Duration::ZERO),
            None
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(remaining_retry_time(&config, started), None);
    }
}
