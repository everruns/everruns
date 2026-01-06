//! Backpressure management for worker pools
//!
//! Provides load-aware task acceptance to prevent worker overload.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

/// Backpressure configuration
///
/// Controls when workers start rejecting new tasks based on load.
///
/// # Example
///
/// ```
/// use everruns_durable::worker::BackpressureConfig;
///
/// let config = BackpressureConfig::default()
///     .with_high_watermark(0.85)
///     .with_low_watermark(0.65);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackpressureConfig {
    /// High watermark - stop accepting tasks when load exceeds this ratio
    /// (e.g., 0.9 = 90% of max_concurrency)
    pub high_watermark: f64,

    /// Low watermark - resume accepting tasks when load drops below this ratio
    /// (e.g., 0.7 = 70% of max_concurrency)
    pub low_watermark: f64,

    /// Memory pressure threshold in bytes (optional)
    pub memory_threshold: Option<usize>,

    /// CPU pressure threshold as percentage (optional)
    pub cpu_threshold: Option<f64>,
}

impl Default for BackpressureConfig {
    fn default() -> Self {
        Self {
            high_watermark: 0.9,
            low_watermark: 0.7,
            memory_threshold: None,
            cpu_threshold: None,
        }
    }
}

impl BackpressureConfig {
    /// Create a new backpressure configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the high watermark (when to stop accepting tasks)
    pub fn with_high_watermark(mut self, watermark: f64) -> Self {
        self.high_watermark = watermark.clamp(0.0, 1.0);
        self
    }

    /// Set the low watermark (when to resume accepting tasks)
    pub fn with_low_watermark(mut self, watermark: f64) -> Self {
        self.low_watermark = watermark.clamp(0.0, 1.0);
        self
    }

    /// Set memory pressure threshold
    pub fn with_memory_threshold(mut self, bytes: usize) -> Self {
        self.memory_threshold = Some(bytes);
        self
    }

    /// Set CPU pressure threshold (0.0 - 1.0)
    pub fn with_cpu_threshold(mut self, threshold: f64) -> Self {
        self.cpu_threshold = Some(threshold.clamp(0.0, 1.0));
        self
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<(), BackpressureError> {
        if self.low_watermark >= self.high_watermark {
            return Err(BackpressureError::InvalidConfig(
                "low_watermark must be less than high_watermark".into(),
            ));
        }
        Ok(())
    }
}

/// Backpressure-related errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum BackpressureError {
    /// Invalid configuration
    #[error("invalid backpressure configuration: {0}")]
    InvalidConfig(String),
}

/// Backpressure state for a worker
///
/// Tracks current load and determines when to accept or reject new tasks.
/// Uses atomic operations for thread-safe access without locks.
pub struct BackpressureState {
    config: BackpressureConfig,
    current_load: AtomicUsize,
    max_concurrency: usize,
    accepting_tasks: AtomicBool,
    backpressure_reason: std::sync::RwLock<Option<String>>,
}

impl BackpressureState {
    /// Create a new backpressure state
    pub fn new(config: BackpressureConfig, max_concurrency: usize) -> Self {
        Self {
            config,
            current_load: AtomicUsize::new(0),
            max_concurrency,
            accepting_tasks: AtomicBool::new(true),
            backpressure_reason: std::sync::RwLock::new(None),
        }
    }

    /// Check if the worker should accept new tasks
    ///
    /// Implements hysteresis using high/low watermarks to prevent oscillation.
    pub fn should_accept(&self) -> bool {
        let currently_accepting = self.accepting_tasks.load(Ordering::Relaxed);
        let load = self.current_load.load(Ordering::Relaxed);
        let load_ratio = load as f64 / self.max_concurrency.max(1) as f64;

        if currently_accepting {
            // If accepting, check if we should stop (high watermark)
            if load_ratio >= self.config.high_watermark {
                self.accepting_tasks.store(false, Ordering::Relaxed);
                *self.backpressure_reason.write().unwrap() = Some(format!(
                    "load ratio {:.1}% exceeds high watermark",
                    load_ratio * 100.0
                ));
                return false;
            }
            true
        } else {
            // If not accepting, check if we should resume (low watermark)
            if load_ratio <= self.config.low_watermark {
                self.accepting_tasks.store(true, Ordering::Relaxed);
                *self.backpressure_reason.write().unwrap() = None;
                return true;
            }
            false
        }
    }

    /// Get the current load
    pub fn current_load(&self) -> usize {
        self.current_load.load(Ordering::Relaxed)
    }

    /// Get the maximum concurrency
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// Get the load ratio (current_load / max_concurrency)
    pub fn load_ratio(&self) -> f64 {
        self.current_load.load(Ordering::Relaxed) as f64 / self.max_concurrency.max(1) as f64
    }

    /// Check if currently accepting tasks
    pub fn is_accepting(&self) -> bool {
        self.accepting_tasks.load(Ordering::Relaxed)
    }

    /// Get the backpressure reason (if any)
    pub fn backpressure_reason(&self) -> Option<String> {
        self.backpressure_reason.read().unwrap().clone()
    }

    /// Record that a task has been started
    pub fn task_started(&self) {
        self.current_load.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that a task has completed
    pub fn task_completed(&self) {
        self.current_load.fetch_sub(1, Ordering::Relaxed);
    }

    /// Get the number of available slots
    pub fn available_slots(&self) -> usize {
        let load = self.current_load.load(Ordering::Relaxed);
        self.max_concurrency.saturating_sub(load)
    }

    /// Force the worker to stop accepting tasks
    pub fn pause(&self, reason: &str) {
        self.accepting_tasks.store(false, Ordering::Relaxed);
        *self.backpressure_reason.write().unwrap() = Some(reason.to_string());
    }

    /// Resume accepting tasks (if below low watermark)
    pub fn resume(&self) {
        if self.load_ratio() <= self.config.low_watermark {
            self.accepting_tasks.store(true, Ordering::Relaxed);
            *self.backpressure_reason.write().unwrap() = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = BackpressureConfig::default();
        assert_eq!(config.high_watermark, 0.9);
        assert_eq!(config.low_watermark, 0.7);
        assert!(config.memory_threshold.is_none());
        assert!(config.cpu_threshold.is_none());
    }

    #[test]
    fn test_config_builder() {
        let config = BackpressureConfig::new()
            .with_high_watermark(0.85)
            .with_low_watermark(0.65)
            .with_memory_threshold(1024 * 1024 * 1024);

        assert_eq!(config.high_watermark, 0.85);
        assert_eq!(config.low_watermark, 0.65);
        assert_eq!(config.memory_threshold, Some(1024 * 1024 * 1024));
    }

    #[test]
    fn test_config_validation() {
        let invalid = BackpressureConfig::new()
            .with_high_watermark(0.5)
            .with_low_watermark(0.8); // Invalid: low > high

        assert!(invalid.validate().is_err());
    }

    #[test]
    fn test_backpressure_state_accepts_initially() {
        let state = BackpressureState::new(BackpressureConfig::default(), 10);
        assert!(state.should_accept());
        assert!(state.is_accepting());
    }

    #[test]
    fn test_backpressure_stops_at_high_watermark() {
        let config = BackpressureConfig::new()
            .with_high_watermark(0.8)
            .with_low_watermark(0.5);
        let state = BackpressureState::new(config, 10);

        // Add 8 tasks (80% = high watermark)
        for _ in 0..8 {
            state.task_started();
        }

        // Should stop accepting at high watermark
        assert!(!state.should_accept());
        assert!(!state.is_accepting());
        assert!(state.backpressure_reason().is_some());
    }

    #[test]
    fn test_backpressure_resumes_at_low_watermark() {
        let config = BackpressureConfig::new()
            .with_high_watermark(0.8)
            .with_low_watermark(0.5);
        let state = BackpressureState::new(config, 10);

        // Hit high watermark
        for _ in 0..9 {
            state.task_started();
        }
        assert!(!state.should_accept());

        // Complete tasks to get to low watermark
        for _ in 0..5 {
            state.task_completed();
        }

        // Should resume at low watermark
        assert!(state.should_accept());
        assert!(state.is_accepting());
        assert!(state.backpressure_reason().is_none());
    }

    #[test]
    fn test_hysteresis_prevents_oscillation() {
        let config = BackpressureConfig::new()
            .with_high_watermark(0.8)
            .with_low_watermark(0.5);
        let state = BackpressureState::new(config, 10);

        // Hit high watermark
        for _ in 0..8 {
            state.task_started();
        }
        assert!(!state.should_accept());

        // Complete 1 task (70% load - between watermarks)
        state.task_completed();
        // Should still not accept (hysteresis)
        assert!(!state.should_accept());

        // Complete more to reach low watermark
        for _ in 0..2 {
            state.task_completed();
        }
        // Now should accept (at 50%)
        assert!(state.should_accept());
    }

    #[test]
    fn test_pause_and_resume() {
        let state = BackpressureState::new(BackpressureConfig::default(), 10);

        // Force pause
        state.pause("manual pause");
        assert!(!state.is_accepting());
        assert_eq!(
            state.backpressure_reason(),
            Some("manual pause".to_string())
        );

        // Resume
        state.resume();
        assert!(state.is_accepting());
        assert!(state.backpressure_reason().is_none());
    }

    #[test]
    fn test_available_slots() {
        let state = BackpressureState::new(BackpressureConfig::default(), 10);

        assert_eq!(state.available_slots(), 10);

        state.task_started();
        state.task_started();
        state.task_started();

        assert_eq!(state.available_slots(), 7);
        assert_eq!(state.current_load(), 3);
    }
}
