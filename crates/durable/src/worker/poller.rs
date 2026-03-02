//! Task polling with exponential backoff and jitter
//!
//! Implements efficient task claiming with adaptive polling intervals.
//! Jitter prevents thundering herd when multiple workers wake up simultaneously.

use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tracing::{debug, instrument, trace};

use crate::persistence::{ClaimedTask, StoreError, WorkflowEventStore};

/// Polling configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PollerConfig {
    /// Minimum poll interval (when tasks are available)
    #[serde(with = "duration_millis")]
    pub min_interval: Duration,

    /// Maximum poll interval (when idle)
    #[serde(with = "duration_millis")]
    pub max_interval: Duration,

    /// Backoff multiplier when no tasks found
    pub backoff_multiplier: f64,

    /// Maximum tasks to claim per poll
    pub batch_size: usize,

    /// Jitter factor (0.0 – 1.0). Random jitter up to this fraction of the
    /// current interval is added to each wait, preventing thundering herd.
    /// Default: 0.2 (20% jitter). Set to 0.0 to disable.
    pub jitter_factor: f64,
}

impl Default for PollerConfig {
    fn default() -> Self {
        Self {
            min_interval: Duration::from_millis(100),
            max_interval: Duration::from_secs(5),
            backoff_multiplier: 1.5,
            batch_size: 10,
            jitter_factor: 0.2,
        }
    }
}

impl PollerConfig {
    /// Create a new poller configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set minimum poll interval
    pub fn with_min_interval(mut self, interval: Duration) -> Self {
        self.min_interval = interval;
        self
    }

    /// Set maximum poll interval
    pub fn with_max_interval(mut self, interval: Duration) -> Self {
        self.max_interval = interval;
        self
    }

    /// Set backoff multiplier
    pub fn with_backoff_multiplier(mut self, multiplier: f64) -> Self {
        self.backoff_multiplier = multiplier.max(1.0);
        self
    }

    /// Set batch size
    pub fn with_batch_size(mut self, size: usize) -> Self {
        self.batch_size = size.max(1);
        self
    }

    /// Set jitter factor (0.0 – 1.0)
    pub fn with_jitter_factor(mut self, factor: f64) -> Self {
        self.jitter_factor = factor.clamp(0.0, 1.0);
        self
    }
}

/// Task poller with adaptive backoff
///
/// Polls for tasks with exponential backoff when idle and resets to
/// minimum interval when tasks are found.
pub struct TaskPoller {
    store: Arc<dyn WorkflowEventStore>,
    worker_id: String,
    activity_types: Vec<String>,
    config: PollerConfig,
    current_interval: Duration,
    shutdown_rx: watch::Receiver<bool>,
}

impl TaskPoller {
    /// Create a new task poller
    pub fn new(
        store: Arc<dyn WorkflowEventStore>,
        worker_id: String,
        activity_types: Vec<String>,
        config: PollerConfig,
        shutdown_rx: watch::Receiver<bool>,
    ) -> Self {
        Self {
            store,
            worker_id,
            activity_types,
            config: config.clone(),
            current_interval: config.min_interval,
            shutdown_rx,
        }
    }

    /// Poll for available tasks
    ///
    /// Returns claimed tasks and updates internal backoff state.
    #[instrument(skip(self), fields(worker_id = %self.worker_id))]
    pub async fn poll(&mut self, max_tasks: usize) -> Result<Vec<ClaimedTask>, PollerError> {
        // Check for shutdown
        if *self.shutdown_rx.borrow() {
            debug!("Poller shutdown requested");
            return Ok(vec![]);
        }

        let batch_size = max_tasks.min(self.config.batch_size);

        let tasks = self
            .store
            .claim_task(&self.worker_id, &self.activity_types, batch_size)
            .await
            .map_err(PollerError::Store)?;

        if tasks.is_empty() {
            // No tasks, increase backoff
            self.increase_backoff();
            trace!(
                interval_ms = self.current_interval.as_millis(),
                "No tasks found, backing off"
            );
        } else {
            // Found tasks, reset to minimum interval
            self.reset_backoff();
            debug!(count = tasks.len(), "Claimed tasks");
        }

        Ok(tasks)
    }

    /// Wait for the current backoff interval plus random jitter.
    ///
    /// Jitter is `[0, jitter_factor * current_interval]` and prevents
    /// thundering herd when multiple workers have identical poll cadences.
    /// Returns early if shutdown is signaled.
    pub async fn wait(&mut self) -> bool {
        let sleep_duration = jittered_interval(self.current_interval, self.config.jitter_factor);

        let mut shutdown_rx = self.shutdown_rx.clone();
        tokio::select! {
            _ = tokio::time::sleep(sleep_duration) => false,
            _ = shutdown_rx.changed() => {
                debug!("Shutdown signal received during wait");
                true
            }
        }
    }

    /// Get the current poll interval
    pub fn current_interval(&self) -> Duration {
        self.current_interval
    }

    /// Check if shutdown has been requested
    pub fn is_shutdown(&self) -> bool {
        *self.shutdown_rx.borrow()
    }

    /// Reset backoff to minimum interval
    fn reset_backoff(&mut self) {
        self.current_interval = self.config.min_interval;
    }

    /// Increase backoff interval
    fn increase_backoff(&mut self) {
        let new_interval = Duration::from_secs_f64(
            self.current_interval.as_secs_f64() * self.config.backoff_multiplier,
        );
        self.current_interval = new_interval.min(self.config.max_interval);
    }
}

/// Compute a jittered wait duration given a base interval and jitter factor.
/// Returns a duration in `[base, base * (1 + jitter_factor)]`.
pub(crate) fn jittered_interval(base: Duration, jitter_factor: f64) -> Duration {
    if jitter_factor <= 0.0 {
        return base;
    }
    let max_jitter = base.as_secs_f64() * jitter_factor;
    let jitter = rand::rng().random_range(0.0..max_jitter);
    base + Duration::from_secs_f64(jitter)
}

/// Poller errors
#[derive(Debug, thiserror::Error)]
pub enum PollerError {
    /// Store error
    #[error("store error: {0}")]
    Store(#[from] StoreError),

    /// Worker shutdown
    #[error("worker is shutting down")]
    Shutdown,
}

/// Adaptive poll interval calculator
///
/// Calculates optimal poll intervals based on queue depth and worker capacity.
pub struct AdaptivePoller {
    config: PollerConfig,
    recent_task_counts: Vec<usize>,
    window_size: usize,
}

impl AdaptivePoller {
    /// Create a new adaptive poller
    pub fn new(config: PollerConfig) -> Self {
        Self {
            config,
            recent_task_counts: Vec::with_capacity(10),
            window_size: 10,
        }
    }

    /// Record the result of a poll
    pub fn record_poll(&mut self, tasks_found: usize) {
        if self.recent_task_counts.len() >= self.window_size {
            self.recent_task_counts.remove(0);
        }
        self.recent_task_counts.push(tasks_found);
    }

    /// Calculate the optimal poll interval
    pub fn optimal_interval(&self) -> Duration {
        if self.recent_task_counts.is_empty() {
            return self.config.min_interval;
        }

        let avg_tasks: f64 = self.recent_task_counts.iter().sum::<usize>() as f64
            / self.recent_task_counts.len() as f64;

        if avg_tasks > 0.8 * self.config.batch_size as f64 {
            // High load - poll frequently
            self.config.min_interval
        } else if avg_tasks < 0.2 * self.config.batch_size as f64 {
            // Low load - poll less frequently
            self.config.max_interval.min(self.config.min_interval * 4)
        } else {
            // Medium load - adaptive interval
            let ratio = 1.0 - (avg_tasks / self.config.batch_size as f64);
            let range =
                self.config.max_interval.as_secs_f64() - self.config.min_interval.as_secs_f64();
            Duration::from_secs_f64(self.config.min_interval.as_secs_f64() + ratio * range * 0.5)
        }
    }

    /// Get recent average tasks per poll
    pub fn average_tasks_per_poll(&self) -> f64 {
        if self.recent_task_counts.is_empty() {
            0.0
        } else {
            self.recent_task_counts.iter().sum::<usize>() as f64
                / self.recent_task_counts.len() as f64
        }
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
        let config = PollerConfig::default();
        assert_eq!(config.min_interval, Duration::from_millis(100));
        assert_eq!(config.max_interval, Duration::from_secs(5));
        assert_eq!(config.backoff_multiplier, 1.5);
        assert_eq!(config.batch_size, 10);
        assert_eq!(config.jitter_factor, 0.2);
    }

    #[test]
    fn test_config_builder() {
        let config = PollerConfig::new()
            .with_min_interval(Duration::from_millis(50))
            .with_max_interval(Duration::from_secs(10))
            .with_backoff_multiplier(2.0)
            .with_batch_size(20)
            .with_jitter_factor(0.5);

        assert_eq!(config.min_interval, Duration::from_millis(50));
        assert_eq!(config.max_interval, Duration::from_secs(10));
        assert_eq!(config.backoff_multiplier, 2.0);
        assert_eq!(config.batch_size, 20);
        assert_eq!(config.jitter_factor, 0.5);
    }

    #[test]
    fn test_adaptive_poller_high_load() {
        let config = PollerConfig::default();
        let mut poller = AdaptivePoller::new(config.clone());

        // Record high task counts
        for _ in 0..5 {
            poller.record_poll(9); // 90% of batch size
        }

        assert_eq!(poller.optimal_interval(), config.min_interval);
    }

    #[test]
    fn test_adaptive_poller_low_load() {
        let config = PollerConfig::default();
        let mut poller = AdaptivePoller::new(config.clone());

        // Record low task counts
        for _ in 0..5 {
            poller.record_poll(0);
        }

        // Should be slower than min but capped
        let interval = poller.optimal_interval();
        assert!(interval > config.min_interval);
        assert!(interval <= config.max_interval);
    }

    #[test]
    fn test_adaptive_poller_rolling_window() {
        let config = PollerConfig::default();
        let mut poller = AdaptivePoller::new(config);

        // Fill window
        for i in 0..15 {
            poller.record_poll(i % 10);
        }

        // Should only keep last 10
        assert_eq!(poller.recent_task_counts.len(), 10);
    }

    #[test]
    fn test_average_tasks_per_poll() {
        let config = PollerConfig::default();
        let mut poller = AdaptivePoller::new(config);

        poller.record_poll(5);
        poller.record_poll(10);
        poller.record_poll(15);

        assert_eq!(poller.average_tasks_per_poll(), 10.0);
    }

    #[test]
    fn test_jittered_interval_zero_factor() {
        let base = Duration::from_millis(500);
        let result = jittered_interval(base, 0.0);
        assert_eq!(result, base);
    }

    #[test]
    fn test_jittered_interval_bounded() {
        let base = Duration::from_millis(1000);
        let factor = 0.2;
        // Run multiple times to test randomness stays in bounds
        for _ in 0..100 {
            let result = jittered_interval(base, factor);
            assert!(result >= base, "jitter should not decrease interval");
            assert!(
                result <= base + Duration::from_millis(200),
                "jitter should not exceed 20% of base: got {:?}",
                result
            );
        }
    }

    #[test]
    fn test_jittered_interval_negative_factor() {
        let base = Duration::from_millis(500);
        let result = jittered_interval(base, -0.5);
        assert_eq!(result, base, "negative factor should be treated as zero");
    }

    #[test]
    fn test_jitter_factor_clamped() {
        let config = PollerConfig::new().with_jitter_factor(2.0);
        assert_eq!(config.jitter_factor, 1.0, "factor clamped to 1.0");

        let config = PollerConfig::new().with_jitter_factor(-0.5);
        assert_eq!(config.jitter_factor, 0.0, "factor clamped to 0.0");
    }
}
