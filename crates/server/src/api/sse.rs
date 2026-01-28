// Shared SSE (Server-Sent Events) utilities
//
// Provides unified backoff configuration for all SSE endpoints.
// Different use cases have different latency requirements:
// - Real-time (sessions): Fast updates for interactive UX
// - Monitoring (durable): Relaxed updates for dashboards
//
// Connection Cycling:
// SSE connections are gracefully closed after max_connection_duration to prevent
// stale connections through proxies/load balancers. Before closing, a "disconnecting"
// event is sent so clients can reconnect immediately with since_id to resume.
//
// SSE Retry Hints:
// The SSE `retry:` field hints to clients how long to wait before reconnecting.
// We use the current backoff value as the retry hint, so clients reconnect faster
// when the stream is active (low backoff) and slower when idle (high backoff).

use std::time::Duration;

/// SSE stream configuration with backoff and connection cycling parameters
#[derive(Debug, Clone, Copy)]
pub struct SseStreamConfig {
    /// Minimum backoff when polling for new events (ms)
    pub min_backoff_ms: u64,
    /// Maximum backoff when no new events (ms)
    pub max_backoff_ms: u64,
    /// Maximum connection duration before graceful close (seconds)
    /// Connection cycling prevents stale connections through proxies
    pub max_connection_secs: u64,
    /// Retry hint for immediate reconnect after disconnecting event (ms)
    pub disconnect_retry_ms: u64,
}

impl SseStreamConfig {
    /// Fast polling for real-time session events
    /// Min: 100ms, Max: 500ms, Connection cycle: 5 minutes
    pub fn realtime() -> Self {
        Self {
            min_backoff_ms: 100,
            max_backoff_ms: 500,
            max_connection_secs: 300, // 5 minutes
            disconnect_retry_ms: 100, // Fast reconnect
        }
    }

    /// Relaxed polling for monitoring dashboards
    /// Min: 1000ms, Max: 20000ms, Connection cycle: 10 minutes
    pub fn monitoring() -> Self {
        Self {
            min_backoff_ms: 1000,
            max_backoff_ms: 20000,
            max_connection_secs: 600,  // 10 minutes
            disconnect_retry_ms: 1000, // 1 second reconnect
        }
    }

    /// Get minimum backoff as Duration
    pub fn min_backoff(&self) -> Duration {
        Duration::from_millis(self.min_backoff_ms)
    }

    /// Get maximum connection duration
    pub fn max_connection_duration(&self) -> Duration {
        Duration::from_secs(self.max_connection_secs)
    }

    /// Calculate next backoff (exponential, capped at max)
    pub fn next_backoff(&self, current_ms: u64) -> u64 {
        (current_ms * 2).min(self.max_backoff_ms)
    }

    /// Get retry hint duration based on current backoff
    /// Used in SSE `retry:` field to hint reconnection timing
    pub fn retry_hint(&self, current_backoff_ms: u64) -> Duration {
        Duration::from_millis(current_backoff_ms)
    }

    /// Get retry hint for disconnecting event (fast reconnect)
    pub fn disconnect_retry(&self) -> Duration {
        Duration::from_millis(self.disconnect_retry_ms)
    }
}

impl Default for SseStreamConfig {
    fn default() -> Self {
        Self::realtime()
    }
}

/// Reason for SSE stream disconnection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    /// Connection cycling - normal graceful close after max duration
    ConnectionCycle,
    /// Server shutdown
    Shutdown,
}

impl DisconnectReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            DisconnectReason::ConnectionCycle => "connection_cycle",
            DisconnectReason::Shutdown => "shutdown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_realtime_config() {
        let config = SseStreamConfig::realtime();
        assert_eq!(config.min_backoff_ms, 100);
        assert_eq!(config.max_backoff_ms, 500);
        assert_eq!(config.max_connection_secs, 300);
        assert_eq!(config.disconnect_retry_ms, 100);
    }

    #[test]
    fn test_monitoring_config() {
        let config = SseStreamConfig::monitoring();
        assert_eq!(config.min_backoff_ms, 1000);
        assert_eq!(config.max_backoff_ms, 20000);
        assert_eq!(config.max_connection_secs, 600);
        assert_eq!(config.disconnect_retry_ms, 1000);
    }

    #[test]
    fn test_next_backoff_doubles() {
        let config = SseStreamConfig::realtime();
        assert_eq!(config.next_backoff(100), 200);
        assert_eq!(config.next_backoff(200), 400);
    }

    #[test]
    fn test_next_backoff_caps_at_max() {
        let config = SseStreamConfig::realtime();
        assert_eq!(config.next_backoff(400), 500); // Capped at max
        assert_eq!(config.next_backoff(500), 500); // Already at max
    }

    #[test]
    fn test_monitoring_backoff_caps() {
        let config = SseStreamConfig::monitoring();
        assert_eq!(config.next_backoff(10000), 20000);
        assert_eq!(config.next_backoff(15000), 20000); // Capped
    }

    #[test]
    fn test_min_backoff_duration() {
        let config = SseStreamConfig::realtime();
        assert_eq!(config.min_backoff(), Duration::from_millis(100));

        let config = SseStreamConfig::monitoring();
        assert_eq!(config.min_backoff(), Duration::from_millis(1000));
    }

    #[test]
    fn test_default_is_realtime() {
        let config = SseStreamConfig::default();
        assert_eq!(config.min_backoff_ms, 100);
        assert_eq!(config.max_backoff_ms, 500);
        assert_eq!(config.max_connection_secs, 300);
    }

    #[test]
    fn test_config_is_copy() {
        let config = SseStreamConfig::realtime();
        let config2 = config; // Copy
        assert_eq!(config.min_backoff_ms, config2.min_backoff_ms);
        assert_eq!(config.max_connection_secs, config2.max_connection_secs);
    }

    #[test]
    fn test_max_connection_duration() {
        let config = SseStreamConfig::realtime();
        assert_eq!(config.max_connection_duration(), Duration::from_secs(300));

        let config = SseStreamConfig::monitoring();
        assert_eq!(config.max_connection_duration(), Duration::from_secs(600));
    }

    #[test]
    fn test_retry_hint() {
        let config = SseStreamConfig::realtime();
        assert_eq!(config.retry_hint(100), Duration::from_millis(100));
        assert_eq!(config.retry_hint(500), Duration::from_millis(500));
    }

    #[test]
    fn test_disconnect_retry() {
        let config = SseStreamConfig::realtime();
        assert_eq!(config.disconnect_retry(), Duration::from_millis(100));

        let config = SseStreamConfig::monitoring();
        assert_eq!(config.disconnect_retry(), Duration::from_millis(1000));
    }

    #[test]
    fn test_disconnect_reason() {
        assert_eq!(
            DisconnectReason::ConnectionCycle.as_str(),
            "connection_cycle"
        );
        assert_eq!(DisconnectReason::Shutdown.as_str(), "shutdown");
    }

    #[test]
    fn test_exponential_backoff_sequence_realtime() {
        let config = SseStreamConfig::realtime();
        let mut backoff = config.min_backoff_ms;

        // Sequence: 100 -> 200 -> 400 -> 500 (capped)
        assert_eq!(backoff, 100);
        backoff = config.next_backoff(backoff);
        assert_eq!(backoff, 200);
        backoff = config.next_backoff(backoff);
        assert_eq!(backoff, 400);
        backoff = config.next_backoff(backoff);
        assert_eq!(backoff, 500); // Capped
        backoff = config.next_backoff(backoff);
        assert_eq!(backoff, 500); // Stays at max
    }

    #[test]
    fn test_exponential_backoff_sequence_monitoring() {
        let config = SseStreamConfig::monitoring();
        let mut backoff = config.min_backoff_ms;

        // Sequence: 1000 -> 2000 -> 4000 -> 8000 -> 16000 -> 20000 (capped)
        assert_eq!(backoff, 1000);
        backoff = config.next_backoff(backoff);
        assert_eq!(backoff, 2000);
        backoff = config.next_backoff(backoff);
        assert_eq!(backoff, 4000);
        backoff = config.next_backoff(backoff);
        assert_eq!(backoff, 8000);
        backoff = config.next_backoff(backoff);
        assert_eq!(backoff, 16000);
        backoff = config.next_backoff(backoff);
        assert_eq!(backoff, 20000); // Capped
        backoff = config.next_backoff(backoff);
        assert_eq!(backoff, 20000); // Stays at max
    }
}
