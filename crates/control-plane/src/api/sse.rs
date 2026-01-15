// Shared SSE (Server-Sent Events) utilities
//
// Provides unified backoff configuration for all SSE endpoints.
// Different use cases have different latency requirements:
// - Real-time (sessions): Fast updates for interactive UX
// - Monitoring (durable): Relaxed updates for dashboards

use std::time::Duration;

/// SSE stream configuration with backoff parameters
#[derive(Debug, Clone, Copy)]
pub struct SseStreamConfig {
    /// Minimum backoff when polling for new events (ms)
    pub min_backoff_ms: u64,
    /// Maximum backoff when no new events (ms)
    pub max_backoff_ms: u64,
}

impl SseStreamConfig {
    /// Fast polling for real-time session events
    /// Min: 100ms, Max: 500ms
    pub fn realtime() -> Self {
        Self {
            min_backoff_ms: 100,
            max_backoff_ms: 500,
        }
    }

    /// Relaxed polling for monitoring dashboards
    /// Min: 1000ms, Max: 20000ms
    pub fn monitoring() -> Self {
        Self {
            min_backoff_ms: 1000,
            max_backoff_ms: 20000,
        }
    }

    /// Get minimum backoff as Duration
    pub fn min_backoff(&self) -> Duration {
        Duration::from_millis(self.min_backoff_ms)
    }

    /// Calculate next backoff (exponential, capped at max)
    pub fn next_backoff(&self, current_ms: u64) -> u64 {
        (current_ms * 2).min(self.max_backoff_ms)
    }
}

impl Default for SseStreamConfig {
    fn default() -> Self {
        Self::realtime()
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
    }

    #[test]
    fn test_monitoring_config() {
        let config = SseStreamConfig::monitoring();
        assert_eq!(config.min_backoff_ms, 1000);
        assert_eq!(config.max_backoff_ms, 20000);
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
}
