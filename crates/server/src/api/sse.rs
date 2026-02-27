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
//
// Connection Limits (EVE-8 / TM-DOS-003):
// SseConnectionTracker enforces global, per-session, and per-org limits to prevent
// resource exhaustion from unbounded SSE connections. Each SSE stream holds a Tokio task
// and polls the database, so limiting connections is critical for system stability.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// SSE stream configuration with backoff, connection cycling, and heartbeat parameters.
///
/// Connection cycling duration is configurable via environment variables to
/// accommodate different proxy configurations:
/// - `SSE_REALTIME_CYCLE_SECS` (default: 300) — session event streams
/// - `SSE_MONITORING_CYCLE_SECS` (default: 600) — durable monitoring streams
/// - `SSE_HEARTBEAT_INTERVAL_SECS` (default: 30) — heartbeat comment interval
///
/// Heartbeat comments (`: heartbeat\n\n`) are sent periodically on all SSE streams
/// to allow clients to detect stale/half-open connections. The interval MUST be
/// less than the client read timeout (SDK default: 60s) with safety margin.
/// At 30s, this gives a 2x safety factor.
///
/// When running behind proxies with HTTP/1.1 backends (no h2c), increase the
/// cycling interval to reduce reconnection frequency and avoid exhausting
/// SDK retry budgets. See `specs/events.md` for protocol details.
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
    /// Interval between heartbeat SSE comments (seconds).
    /// Heartbeats are SSE comments (`: heartbeat\n\n`) invisible to event parsers
    /// but reset the client's TCP read timer to detect stale connections.
    pub heartbeat_interval_secs: u64,
}

impl SseStreamConfig {
    /// Fast polling for real-time session events.
    /// Min: 100ms, Max: 500ms, Connection cycle: 5 minutes (configurable via SSE_REALTIME_CYCLE_SECS)
    pub fn realtime() -> Self {
        let cycle_secs = std::env::var("SSE_REALTIME_CYCLE_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300u64);
        Self {
            min_backoff_ms: 100,
            max_backoff_ms: 500,
            max_connection_secs: cycle_secs,
            disconnect_retry_ms: 100, // Fast reconnect
            heartbeat_interval_secs: heartbeat_interval_from_env(),
        }
    }

    /// Relaxed polling for monitoring dashboards.
    /// Min: 1000ms, Max: 20000ms, Connection cycle: 10 minutes (configurable via SSE_MONITORING_CYCLE_SECS)
    pub fn monitoring() -> Self {
        let cycle_secs = std::env::var("SSE_MONITORING_CYCLE_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(600u64);
        Self {
            min_backoff_ms: 1000,
            max_backoff_ms: 20000,
            max_connection_secs: cycle_secs,
            disconnect_retry_ms: 1000, // 1 second reconnect
            heartbeat_interval_secs: heartbeat_interval_from_env(),
        }
    }

    /// Get minimum backoff as Duration
    pub fn min_backoff(&self) -> Duration {
        Duration::from_millis(self.min_backoff_ms)
    }

    /// Get maximum connection duration (fixed, no jitter).
    pub fn max_connection_duration(&self) -> Duration {
        Duration::from_secs(self.max_connection_secs)
    }

    /// Get jittered max connection duration to prevent thundering herd reconnections.
    ///
    /// When many SSE connections start around the same time, they'd all cycle at
    /// exactly max_connection_secs, creating a reconnection storm. Adding ±20% jitter
    /// spreads disconnects over a 2-minute window (for 5-min base = 4:00–6:00).
    pub fn jittered_max_connection_duration(&self) -> Duration {
        use rand::Rng;
        let base = self.max_connection_secs as f64;
        let jitter_range = base * 0.2;
        let jitter = rand::rng().random_range(-jitter_range..jitter_range);
        Duration::from_secs_f64(base + jitter)
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

    /// Get heartbeat interval for SSE keepalive comments.
    /// Used to configure axum's `KeepAlive::interval()`.
    pub fn heartbeat_interval(&self) -> Duration {
        Duration::from_secs(self.heartbeat_interval_secs)
    }
}

/// Default heartbeat interval (30 seconds).
/// Must be less than SDK read timeout (60s) with safety margin.
/// 30s gives a 2x safety factor.
const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// Read heartbeat interval from `SSE_HEARTBEAT_INTERVAL_SECS` env var, falling back to default.
fn heartbeat_interval_from_env() -> u64 {
    std::env::var("SSE_HEARTBEAT_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_HEARTBEAT_INTERVAL_SECS)
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
    /// Error during data fetching - client should retry
    Error,
}

impl DisconnectReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            DisconnectReason::ConnectionCycle => "connection_cycle",
            DisconnectReason::Shutdown => "shutdown",
            DisconnectReason::Error => "error",
        }
    }
}

// ============================================================
// SSE Connection Limits (EVE-8 / TM-DOS-003)
// ============================================================

/// Configuration for SSE connection limits.
#[derive(Debug, Clone)]
pub struct SseConnectionLimits {
    /// Maximum total SSE connections across all users/sessions
    pub global_max: usize,
    /// Maximum SSE connections per session
    pub per_session_max: usize,
    /// Maximum SSE connections per organization (user)
    pub per_org_max: usize,
}

impl Default for SseConnectionLimits {
    fn default() -> Self {
        Self {
            global_max: 10_000,
            per_session_max: 5,
            per_org_max: 1_000,
        }
    }
}

impl SseConnectionLimits {
    /// Load limits from environment variables with sensible defaults.
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            global_max: std::env::var("SSE_GLOBAL_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(defaults.global_max),
            per_session_max: std::env::var("SSE_PER_SESSION_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(defaults.per_session_max),
            per_org_max: std::env::var("SSE_PER_ORG_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(defaults.per_org_max),
        }
    }
}

/// Reason why an SSE connection was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseConnectionRejection {
    GlobalLimitReached,
    SessionLimitReached,
    OrgLimitReached,
}

impl std::fmt::Display for SseConnectionRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GlobalLimitReached => write!(f, "global SSE connection limit reached"),
            Self::SessionLimitReached => write!(f, "per-session SSE connection limit reached"),
            Self::OrgLimitReached => write!(f, "per-organization SSE connection limit reached"),
        }
    }
}

/// Tracks active SSE connections and enforces limits.
///
/// Uses a RAII guard pattern: `try_acquire()` returns an `SseConnectionGuard`
/// that automatically decrements counters when the SSE stream is dropped.
#[derive(Debug)]
pub struct SseConnectionTracker {
    limits: SseConnectionLimits,
    global_count: AtomicUsize,
    /// org_id -> active connection count
    per_org: Mutex<HashMap<i64, usize>>,
    /// session_id (UUID) -> active connection count
    per_session: Mutex<HashMap<uuid::Uuid, usize>>,
}

impl SseConnectionTracker {
    pub fn new(limits: SseConnectionLimits) -> Self {
        tracing::info!(
            global_max = limits.global_max,
            per_session_max = limits.per_session_max,
            per_org_max = limits.per_org_max,
            "SSE connection tracker initialized"
        );
        Self {
            limits,
            global_count: AtomicUsize::new(0),
            per_org: Mutex::new(HashMap::new()),
            per_session: Mutex::new(HashMap::new()),
        }
    }

    /// Try to acquire an SSE connection slot.
    /// Returns a guard on success that auto-releases on drop, or a rejection reason.
    pub fn try_acquire(
        self: &Arc<Self>,
        org_id: i64,
        session_id: uuid::Uuid,
    ) -> Result<SseConnectionGuard, SseConnectionRejection> {
        // Check global limit
        let prev = self.global_count.fetch_add(1, Ordering::SeqCst);
        if prev >= self.limits.global_max {
            self.global_count.fetch_sub(1, Ordering::SeqCst);
            return Err(SseConnectionRejection::GlobalLimitReached);
        }

        // Check per-org limit
        {
            let mut per_org = self.per_org.lock().unwrap();
            let count = per_org.entry(org_id).or_insert(0);
            if *count >= self.limits.per_org_max {
                self.global_count.fetch_sub(1, Ordering::SeqCst);
                return Err(SseConnectionRejection::OrgLimitReached);
            }
            *count += 1;
        }

        // Check per-session limit
        {
            let mut per_session = self.per_session.lock().unwrap();
            let count = per_session.entry(session_id).or_insert(0);
            if *count >= self.limits.per_session_max {
                // Undo org increment
                let mut per_org = self.per_org.lock().unwrap();
                if let Some(c) = per_org.get_mut(&org_id) {
                    *c = c.saturating_sub(1);
                    if *c == 0 {
                        per_org.remove(&org_id);
                    }
                }
                self.global_count.fetch_sub(1, Ordering::SeqCst);
                return Err(SseConnectionRejection::SessionLimitReached);
            }
            *count += 1;
        }

        tracing::debug!(
            org_id,
            %session_id,
            global = self.global_count.load(Ordering::Relaxed),
            "SSE connection acquired"
        );

        Ok(SseConnectionGuard {
            tracker: Arc::clone(self),
            org_id,
            session_id,
        })
    }

    /// Get current global connection count (for metrics/health).
    pub fn global_count(&self) -> usize {
        self.global_count.load(Ordering::Relaxed)
    }

    fn release(&self, org_id: i64, session_id: uuid::Uuid) {
        self.global_count.fetch_sub(1, Ordering::SeqCst);

        {
            let mut per_org = self.per_org.lock().unwrap();
            if let Some(c) = per_org.get_mut(&org_id) {
                *c = c.saturating_sub(1);
                if *c == 0 {
                    per_org.remove(&org_id);
                }
            }
        }

        {
            let mut per_session = self.per_session.lock().unwrap();
            if let Some(c) = per_session.get_mut(&session_id) {
                *c = c.saturating_sub(1);
                if *c == 0 {
                    per_session.remove(&session_id);
                }
            }
        }

        tracing::debug!(
            org_id,
            %session_id,
            global = self.global_count.load(Ordering::Relaxed),
            "SSE connection released"
        );
    }
}

/// RAII guard that releases the SSE connection slot when the stream is dropped.
#[derive(Debug)]
pub struct SseConnectionGuard {
    tracker: Arc<SseConnectionTracker>,
    org_id: i64,
    session_id: uuid::Uuid,
}

impl Drop for SseConnectionGuard {
    fn drop(&mut self) {
        self.tracker.release(self.org_id, self.session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_realtime_config() {
        unsafe {
            std::env::remove_var("SSE_REALTIME_CYCLE_SECS");
            std::env::remove_var("SSE_HEARTBEAT_INTERVAL_SECS");
        }
        let config = SseStreamConfig::realtime();
        assert_eq!(config.min_backoff_ms, 100);
        assert_eq!(config.max_backoff_ms, 500);
        assert_eq!(config.max_connection_secs, 300);
        assert_eq!(config.disconnect_retry_ms, 100);
        assert_eq!(config.heartbeat_interval_secs, 30);
    }

    #[test]
    fn test_monitoring_config() {
        unsafe {
            std::env::remove_var("SSE_MONITORING_CYCLE_SECS");
            std::env::remove_var("SSE_HEARTBEAT_INTERVAL_SECS");
        }
        let config = SseStreamConfig::monitoring();
        assert_eq!(config.min_backoff_ms, 1000);
        assert_eq!(config.max_backoff_ms, 20000);
        assert_eq!(config.max_connection_secs, 600);
        assert_eq!(config.disconnect_retry_ms, 1000);
        assert_eq!(config.heartbeat_interval_secs, 30);
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
        unsafe {
            std::env::remove_var("SSE_REALTIME_CYCLE_SECS");
        }
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
        // Clear env overrides that might leak from parallel tests
        unsafe {
            std::env::remove_var("SSE_REALTIME_CYCLE_SECS");
            std::env::remove_var("SSE_MONITORING_CYCLE_SECS");
        }
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
    fn test_jittered_max_connection_duration_within_bounds() {
        let config = SseStreamConfig::realtime();
        // 300s base, ±20% → 240–360s
        for _ in 0..100 {
            let duration = config.jittered_max_connection_duration();
            assert!(
                duration >= Duration::from_secs(240) && duration <= Duration::from_secs(360),
                "jittered duration {:?} out of ±20% range",
                duration
            );
        }
    }

    #[test]
    fn test_jittered_durations_have_variance() {
        let config = SseStreamConfig::realtime();
        let durations: Vec<Duration> = (0..50)
            .map(|_| config.jittered_max_connection_duration())
            .collect();
        let min = durations.iter().min().unwrap();
        let max = durations.iter().max().unwrap();
        // With 50 samples, we should see at least 10 seconds of spread
        assert!(
            *max - *min > Duration::from_secs(10),
            "expected variance in jittered durations, got min={:?} max={:?}",
            min,
            max
        );
    }

    #[test]
    fn test_disconnect_reason() {
        assert_eq!(
            DisconnectReason::ConnectionCycle.as_str(),
            "connection_cycle"
        );
        assert_eq!(DisconnectReason::Shutdown.as_str(), "shutdown");
        assert_eq!(DisconnectReason::Error.as_str(), "error");
    }

    #[test]
    fn test_disconnect_reason_error_can_be_used_in_json() {
        // Test that the Error variant produces valid JSON
        let reason = DisconnectReason::Error;
        let json = format!(r#"{{"reason":"{}","retry_ms":1000}}"#, reason.as_str());
        assert!(json.contains("\"reason\":\"error\""));
        assert!(json.contains("\"retry_ms\":1000"));

        // Verify it's valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["reason"], "error");
    }

    #[test]
    fn test_disconnect_reason_equality() {
        assert_eq!(DisconnectReason::Error, DisconnectReason::Error);
        assert_ne!(DisconnectReason::Error, DisconnectReason::ConnectionCycle);
        assert_ne!(DisconnectReason::Error, DisconnectReason::Shutdown);
    }

    #[test]
    fn test_disconnect_reason_copy() {
        // Verify DisconnectReason implements Copy
        let reason = DisconnectReason::Error;
        let reason_copy = reason;
        assert_eq!(reason, reason_copy);
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

    // ============================================
    // Connection Tracker Tests
    // ============================================

    #[test]
    fn test_connection_tracker_basic_acquire_release() {
        let tracker = Arc::new(SseConnectionTracker::new(SseConnectionLimits::default()));
        let session = uuid::Uuid::new_v4();

        let guard = tracker.try_acquire(1, session).unwrap();
        assert_eq!(tracker.global_count(), 1);

        drop(guard);
        assert_eq!(tracker.global_count(), 0);
    }

    #[test]
    fn test_connection_tracker_per_session_limit() {
        let limits = SseConnectionLimits {
            global_max: 100,
            per_session_max: 2,
            per_org_max: 50,
        };
        let tracker = Arc::new(SseConnectionTracker::new(limits));
        let session = uuid::Uuid::new_v4();

        let _g1 = tracker.try_acquire(1, session).unwrap();
        let _g2 = tracker.try_acquire(1, session).unwrap();

        // Third should fail
        let result = tracker.try_acquire(1, session);
        assert_eq!(
            result.unwrap_err(),
            SseConnectionRejection::SessionLimitReached
        );

        // Global should not have incremented for the failed attempt
        assert_eq!(tracker.global_count(), 2);
    }

    #[test]
    fn test_connection_tracker_per_org_limit() {
        let limits = SseConnectionLimits {
            global_max: 100,
            per_session_max: 5,
            per_org_max: 2,
        };
        let tracker = Arc::new(SseConnectionTracker::new(limits));

        let _g1 = tracker.try_acquire(1, uuid::Uuid::new_v4()).unwrap();
        let _g2 = tracker.try_acquire(1, uuid::Uuid::new_v4()).unwrap();

        // Third for same org should fail
        let result = tracker.try_acquire(1, uuid::Uuid::new_v4());
        assert_eq!(result.unwrap_err(), SseConnectionRejection::OrgLimitReached);
        assert_eq!(tracker.global_count(), 2);

        // Different org should work
        let _g3 = tracker.try_acquire(2, uuid::Uuid::new_v4()).unwrap();
        assert_eq!(tracker.global_count(), 3);
    }

    #[test]
    fn test_connection_tracker_global_limit() {
        let limits = SseConnectionLimits {
            global_max: 2,
            per_session_max: 5,
            per_org_max: 50,
        };
        let tracker = Arc::new(SseConnectionTracker::new(limits));

        let _g1 = tracker.try_acquire(1, uuid::Uuid::new_v4()).unwrap();
        let _g2 = tracker.try_acquire(2, uuid::Uuid::new_v4()).unwrap();

        let result = tracker.try_acquire(3, uuid::Uuid::new_v4());
        assert_eq!(
            result.unwrap_err(),
            SseConnectionRejection::GlobalLimitReached
        );
        assert_eq!(tracker.global_count(), 2);
    }

    #[test]
    fn test_connection_tracker_release_allows_new_connections() {
        let limits = SseConnectionLimits {
            global_max: 1,
            per_session_max: 1,
            per_org_max: 1,
        };
        let tracker = Arc::new(SseConnectionTracker::new(limits));
        let session = uuid::Uuid::new_v4();

        let guard = tracker.try_acquire(1, session).unwrap();
        assert!(tracker.try_acquire(1, session).is_err());

        drop(guard);
        // Should succeed after release
        let _g2 = tracker.try_acquire(1, session).unwrap();
        assert_eq!(tracker.global_count(), 1);
    }

    #[test]
    fn test_connection_limits_defaults() {
        let limits = SseConnectionLimits::default();
        assert_eq!(limits.global_max, 10_000);
        assert_eq!(limits.per_session_max, 5);
        assert_eq!(limits.per_org_max, 1_000);
    }

    #[test]
    fn test_connection_rejection_display() {
        assert_eq!(
            SseConnectionRejection::GlobalLimitReached.to_string(),
            "global SSE connection limit reached"
        );
        assert_eq!(
            SseConnectionRejection::SessionLimitReached.to_string(),
            "per-session SSE connection limit reached"
        );
        assert_eq!(
            SseConnectionRejection::OrgLimitReached.to_string(),
            "per-organization SSE connection limit reached"
        );
    }

    // NOTE: Env var tests are inherently racy with parallel tests.
    // These test the override mechanism; we accept that parallel test
    // interference may cause occasional failures in isolation.
    // Run with --test-threads=1 if needed for determinism.
    #[test]
    fn test_realtime_cycle_secs_env_override() {
        unsafe {
            std::env::set_var("SSE_REALTIME_CYCLE_SECS", "900");
        }
        let config = SseStreamConfig::realtime();
        unsafe {
            std::env::remove_var("SSE_REALTIME_CYCLE_SECS");
        }
        assert_eq!(config.max_connection_secs, 900); // 15 minutes
        assert_eq!(config.min_backoff_ms, 100); // unchanged
    }

    #[test]
    fn test_monitoring_cycle_secs_env_override() {
        unsafe {
            std::env::set_var("SSE_MONITORING_CYCLE_SECS", "1800");
        }
        let config = SseStreamConfig::monitoring();
        unsafe {
            std::env::remove_var("SSE_MONITORING_CYCLE_SECS");
        }
        assert_eq!(config.max_connection_secs, 1800); // 30 minutes
        assert_eq!(config.min_backoff_ms, 1000); // unchanged
    }

    #[test]
    fn test_cycle_secs_env_invalid_fallback() {
        unsafe {
            std::env::set_var("SSE_REALTIME_CYCLE_SECS", "not_a_number");
        }
        let config = SseStreamConfig::realtime();
        unsafe {
            std::env::remove_var("SSE_REALTIME_CYCLE_SECS");
        }
        assert_eq!(config.max_connection_secs, 300); // falls back to default
    }

    // ============================================
    // Heartbeat Tests
    // ============================================

    #[test]
    fn test_heartbeat_interval_default() {
        unsafe {
            std::env::remove_var("SSE_HEARTBEAT_INTERVAL_SECS");
        }
        let config = SseStreamConfig::realtime();
        assert_eq!(config.heartbeat_interval_secs, 30);
        assert_eq!(config.heartbeat_interval(), Duration::from_secs(30));

        let config = SseStreamConfig::monitoring();
        assert_eq!(config.heartbeat_interval_secs, 30);
        assert_eq!(config.heartbeat_interval(), Duration::from_secs(30));
    }

    #[test]
    fn test_heartbeat_interval_env_override() {
        unsafe {
            std::env::set_var("SSE_HEARTBEAT_INTERVAL_SECS", "15");
        }
        let config = SseStreamConfig::realtime();
        unsafe {
            std::env::remove_var("SSE_HEARTBEAT_INTERVAL_SECS");
        }
        assert_eq!(config.heartbeat_interval_secs, 15);
        assert_eq!(config.heartbeat_interval(), Duration::from_secs(15));
    }

    #[test]
    fn test_heartbeat_interval_env_invalid_fallback() {
        unsafe {
            std::env::set_var("SSE_HEARTBEAT_INTERVAL_SECS", "not_a_number");
        }
        let config = SseStreamConfig::realtime();
        unsafe {
            std::env::remove_var("SSE_HEARTBEAT_INTERVAL_SECS");
        }
        assert_eq!(config.heartbeat_interval_secs, 30); // falls back to default
    }

    #[test]
    fn test_heartbeat_interval_less_than_sdk_read_timeout() {
        // SDK read timeout is 60s. Heartbeat must be less than that.
        unsafe {
            std::env::remove_var("SSE_HEARTBEAT_INTERVAL_SECS");
        }
        let config = SseStreamConfig::realtime();
        let sdk_read_timeout = Duration::from_secs(60);
        assert!(
            config.heartbeat_interval() < sdk_read_timeout,
            "heartbeat interval {:?} must be < SDK read timeout {:?}",
            config.heartbeat_interval(),
            sdk_read_timeout
        );
    }
}
