//! Retry policy implementation

use std::time::Duration;

use rand::Rng;
use serde::{Deserialize, Serialize};

/// Configuration for activity retries
///
/// Supports exponential backoff with jitter to avoid thundering herd.
///
/// # Example
///
/// ```
/// use everruns_durable::RetryPolicy;
/// use std::time::Duration;
///
/// let policy = RetryPolicy::exponential()
///     .with_max_attempts(5)
///     .with_initial_interval(Duration::from_secs(1))
///     .with_max_interval(Duration::from_secs(60));
///
/// // First retry after ~1 second
/// // Second retry after ~2 seconds
/// // Third retry after ~4 seconds
/// // etc.
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetryPolicy {
    /// Maximum number of attempts (including initial)
    pub max_attempts: u32,

    /// Initial delay before first retry
    #[serde(with = "duration_millis")]
    pub initial_interval: Duration,

    /// Maximum delay between retries
    #[serde(with = "duration_millis")]
    pub max_interval: Duration,

    /// Backoff multiplier (e.g., 2.0 for exponential)
    pub backoff_coefficient: f64,

    /// Jitter factor (0.0-1.0) to add randomness
    ///
    /// A value of 0.1 means ±10% randomness.
    pub jitter: f64,

    /// Error types that should NOT be retried
    #[serde(default)]
    pub non_retryable_errors: Vec<String>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::exponential()
    }
}

impl RetryPolicy {
    /// Create an exponential backoff retry policy with sensible defaults
    ///
    /// - 5 max attempts
    /// - 1 second initial interval
    /// - 60 second max interval
    /// - 2x backoff coefficient
    /// - 10% jitter
    pub fn exponential() -> Self {
        Self {
            max_attempts: 5,
            initial_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(60),
            backoff_coefficient: 2.0,
            jitter: 0.1,
            non_retryable_errors: vec![],
        }
    }

    /// Create a policy that never retries
    pub fn no_retry() -> Self {
        Self {
            max_attempts: 1,
            initial_interval: Duration::ZERO,
            max_interval: Duration::ZERO,
            backoff_coefficient: 1.0,
            jitter: 0.0,
            non_retryable_errors: vec![],
        }
    }

    /// Create a policy with fixed intervals (no backoff)
    pub fn fixed(interval: Duration, max_attempts: u32) -> Self {
        Self {
            max_attempts,
            initial_interval: interval,
            max_interval: interval,
            backoff_coefficient: 1.0,
            jitter: 0.0,
            non_retryable_errors: vec![],
        }
    }

    /// Set the maximum number of attempts
    pub fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    /// Set the initial retry interval
    pub fn with_initial_interval(mut self, interval: Duration) -> Self {
        self.initial_interval = interval;
        self
    }

    /// Set the maximum retry interval
    pub fn with_max_interval(mut self, interval: Duration) -> Self {
        self.max_interval = interval;
        self
    }

    /// Set the backoff coefficient
    pub fn with_backoff_coefficient(mut self, coefficient: f64) -> Self {
        self.backoff_coefficient = coefficient;
        self
    }

    /// Set the jitter factor (0.0-1.0)
    pub fn with_jitter(mut self, jitter: f64) -> Self {
        self.jitter = jitter.clamp(0.0, 1.0);
        self
    }

    /// Add a non-retryable error type
    pub fn with_non_retryable_error(mut self, error_type: impl Into<String>) -> Self {
        self.non_retryable_errors.push(error_type.into());
        self
    }

    /// Calculate delay for a given attempt number (1-based)
    ///
    /// Returns the duration to wait before the retry.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt <= 1 {
            return Duration::ZERO;
        }

        let retry_num = attempt - 1; // First retry is after attempt 1
        let base = self.initial_interval.as_secs_f64()
            * self.backoff_coefficient.powi(retry_num as i32 - 1);
        let capped = base.min(self.max_interval.as_secs_f64());

        // Apply jitter
        let jittered = if self.jitter > 0.0 {
            let mut rng = rand::thread_rng();
            let jitter_range = capped * self.jitter;
            let jitter_offset = rng.gen_range(-jitter_range..jitter_range);
            (capped + jitter_offset).max(0.0)
        } else {
            capped
        };

        Duration::from_secs_f64(jittered)
    }

    /// Check if an error type should be retried
    pub fn should_retry(&self, error_type: Option<&str>) -> bool {
        if let Some(error_type) = error_type {
            !self.non_retryable_errors.contains(&error_type.to_string())
        } else {
            true
        }
    }

    /// Check if there are remaining attempts
    pub fn has_attempts_remaining(&self, current_attempt: u32) -> bool {
        current_attempt < self.max_attempts
    }
}

/// Serde support for Duration as milliseconds
mod duration_millis {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_millis().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_defaults() {
        let policy = RetryPolicy::exponential();
        assert_eq!(policy.max_attempts, 5);
        assert_eq!(policy.initial_interval, Duration::from_secs(1));
        assert_eq!(policy.backoff_coefficient, 2.0);
    }

    #[test]
    fn test_no_retry() {
        let policy = RetryPolicy::no_retry();
        assert_eq!(policy.max_attempts, 1);
        assert!(!policy.has_attempts_remaining(1));
    }

    #[test]
    fn test_fixed_interval() {
        let policy = RetryPolicy::fixed(Duration::from_secs(5), 3);

        // All delays should be roughly 5 seconds (no jitter)
        let delay1 = policy.delay_for_attempt(2);
        let delay2 = policy.delay_for_attempt(3);

        assert_eq!(delay1, Duration::from_secs(5));
        assert_eq!(delay2, Duration::from_secs(5));
    }

    #[test]
    fn test_delay_for_attempt() {
        let policy = RetryPolicy::exponential()
            .with_jitter(0.0); // Disable jitter for predictable tests

        // Attempt 1 (initial) has no delay
        assert_eq!(policy.delay_for_attempt(1), Duration::ZERO);

        // Attempt 2 (first retry) = 1 second
        assert_eq!(policy.delay_for_attempt(2), Duration::from_secs(1));

        // Attempt 3 (second retry) = 2 seconds
        assert_eq!(policy.delay_for_attempt(3), Duration::from_secs(2));

        // Attempt 4 (third retry) = 4 seconds
        assert_eq!(policy.delay_for_attempt(4), Duration::from_secs(4));
    }

    #[test]
    fn test_max_interval_cap() {
        let policy = RetryPolicy::exponential()
            .with_max_interval(Duration::from_secs(5))
            .with_jitter(0.0);

        // Should be capped at 5 seconds
        let delay = policy.delay_for_attempt(10);
        assert_eq!(delay, Duration::from_secs(5));
    }

    #[test]
    fn test_non_retryable_errors() {
        let policy = RetryPolicy::exponential()
            .with_non_retryable_error("INVALID_INPUT")
            .with_non_retryable_error("NOT_FOUND");

        assert!(!policy.should_retry(Some("INVALID_INPUT")));
        assert!(!policy.should_retry(Some("NOT_FOUND")));
        assert!(policy.should_retry(Some("TIMEOUT")));
        assert!(policy.should_retry(None));
    }

    #[test]
    fn test_has_attempts_remaining() {
        let policy = RetryPolicy::exponential().with_max_attempts(3);

        assert!(policy.has_attempts_remaining(1));
        assert!(policy.has_attempts_remaining(2));
        assert!(!policy.has_attempts_remaining(3));
    }

    #[test]
    fn test_serialization() {
        let policy = RetryPolicy::exponential()
            .with_max_attempts(10)
            .with_non_retryable_error("TEST");

        let json = serde_json::to_string(&policy).unwrap();
        let parsed: RetryPolicy = serde_json::from_str(&json).unwrap();

        assert_eq!(policy, parsed);
    }
}
