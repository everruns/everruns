//! Circuit breaker configuration

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Circuit breaker states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    /// Normal operation - all calls allowed
    Closed,

    /// Failure threshold exceeded - all calls rejected
    Open,

    /// Testing if service recovered - limited calls allowed
    HalfOpen,
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "closed"),
            Self::Open => write!(f, "open"),
            Self::HalfOpen => write!(f, "half_open"),
        }
    }
}

/// Circuit breaker configuration
///
/// Circuit breakers protect external services from cascading failures.
/// When failures exceed a threshold, the circuit "opens" and requests
/// fail fast without calling the service.
///
/// # State Machine
///
/// ```text
/// ┌─────────┐  failure threshold  ┌─────────┐  reset timeout  ┌──────────┐
/// │ Closed  │ ─────────────────► │  Open   │ ──────────────► │ HalfOpen │
/// └─────────┘                     └─────────┘                 └──────────┘
///      ▲                                                            │
///      │                                                            │
///      │              success threshold                             │
///      └────────────────────────────────────────────────────────────┘
/// ```
///
/// # Example
///
/// ```
/// use everruns_durable::CircuitBreakerConfig;
/// use std::time::Duration;
///
/// let config = CircuitBreakerConfig::default()
///     .with_failure_threshold(5)
///     .with_reset_timeout(Duration::from_secs(30));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CircuitBreakerConfig {
    /// Number of failures required to open the circuit
    pub failure_threshold: u32,

    /// Number of successes required to close the circuit (in half-open state)
    pub success_threshold: u32,

    /// Time to wait before transitioning from open to half-open
    #[serde(with = "duration_millis")]
    pub reset_timeout: Duration,

    /// Sliding window size for failure counting
    #[serde(with = "duration_millis")]
    pub window_size: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 2,
            reset_timeout: Duration::from_secs(30),
            window_size: Duration::from_secs(60),
        }
    }
}

impl CircuitBreakerConfig {
    /// Create a new circuit breaker configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the failure threshold to open the circuit
    pub fn with_failure_threshold(mut self, threshold: u32) -> Self {
        self.failure_threshold = threshold;
        self
    }

    /// Set the success threshold to close the circuit
    pub fn with_success_threshold(mut self, threshold: u32) -> Self {
        self.success_threshold = threshold;
        self
    }

    /// Set the reset timeout (time before trying again after opening)
    pub fn with_reset_timeout(mut self, timeout: Duration) -> Self {
        self.reset_timeout = timeout;
        self
    }

    /// Set the sliding window size for failure counting
    pub fn with_window_size(mut self, window: Duration) -> Self {
        self.window_size = window;
        self
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
    fn test_default_config() {
        let config = CircuitBreakerConfig::default();
        assert_eq!(config.failure_threshold, 5);
        assert_eq!(config.success_threshold, 2);
        assert_eq!(config.reset_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_config_builder() {
        let config = CircuitBreakerConfig::new()
            .with_failure_threshold(10)
            .with_success_threshold(3)
            .with_reset_timeout(Duration::from_secs(60));

        assert_eq!(config.failure_threshold, 10);
        assert_eq!(config.success_threshold, 3);
        assert_eq!(config.reset_timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_circuit_state_display() {
        assert_eq!(CircuitState::Closed.to_string(), "closed");
        assert_eq!(CircuitState::Open.to_string(), "open");
        assert_eq!(CircuitState::HalfOpen.to_string(), "half_open");
    }

    #[test]
    fn test_serialization() {
        let config = CircuitBreakerConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: CircuitBreakerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, parsed);
    }
}
