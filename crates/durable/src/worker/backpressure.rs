//! Backpressure management for worker pools
//!
//! Provides load-aware task acceptance to prevent worker overload.
//! Supports task-count watermarks and optional system resource thresholds
//! (CPU/memory) via sysinfo. Resource checks use a 20% resume margin
//! to prevent oscillation.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

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

/// Snapshot of system resource usage
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ResourceMetrics {
    /// System memory used in bytes
    pub memory_used_bytes: u64,
    /// System total memory in bytes
    pub memory_total_bytes: u64,
    /// Global CPU usage as 0.0–1.0 ratio
    pub cpu_usage: f64,
}

/// Polls system CPU and memory via sysinfo.
///
/// Call `sample()` periodically (e.g. every 2s) and feed results into
/// `BackpressureState::update_resources()`.
pub struct ResourceMonitor {
    system: System,
}

impl Default for ResourceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceMonitor {
    pub fn new() -> Self {
        let system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        Self { system }
    }

    /// Sample current system resources. Must be called at >=200ms intervals
    /// for CPU readings to be meaningful (sysinfo requirement).
    pub fn sample(&mut self) -> ResourceMetrics {
        self.system.refresh_cpu_all();
        self.system.refresh_memory();
        ResourceMetrics {
            memory_used_bytes: self.system.used_memory(),
            memory_total_bytes: self.system.total_memory(),
            cpu_usage: (self.system.global_cpu_usage() / 100.0) as f64,
        }
    }
}

/// Backpressure state for a worker
///
/// Tracks current load and determines when to accept or reject new tasks.
/// Uses atomic operations for thread-safe access without locks.
///
/// Resource thresholds (CPU/memory) use a 20% resume margin: if the threshold
/// is 80% CPU, the worker stops accepting at 80% and resumes at 64% (80% × 0.8).
pub struct BackpressureState {
    config: BackpressureConfig,
    current_load: AtomicUsize,
    max_concurrency: usize,
    accepting_tasks: AtomicBool,
    backpressure_reason: std::sync::RwLock<Option<String>>,
    /// Current memory usage in bytes (updated by resource monitor)
    memory_used_bytes: AtomicU64,
    /// Current CPU usage stored as f64 bits (updated by resource monitor)
    cpu_usage_bits: AtomicU64,
    /// Whether resource pressure is currently active (separate from load-based)
    resource_pressure: AtomicBool,
}

/// Resume margin for resource thresholds: resume when metric drops below
/// threshold × this factor. Prevents oscillation.
const RESOURCE_RESUME_FACTOR: f64 = 0.8;

impl BackpressureState {
    /// Create a new backpressure state
    pub fn new(config: BackpressureConfig, max_concurrency: usize) -> Self {
        Self {
            config,
            current_load: AtomicUsize::new(0),
            max_concurrency,
            accepting_tasks: AtomicBool::new(true),
            backpressure_reason: std::sync::RwLock::new(None),
            memory_used_bytes: AtomicU64::new(0),
            cpu_usage_bits: AtomicU64::new(0),
            resource_pressure: AtomicBool::new(false),
        }
    }

    /// Check if the worker should accept new tasks
    ///
    /// Checks both task-count watermarks and system resource thresholds.
    /// Task-count uses hysteresis (high/low watermarks). Resource thresholds
    /// use a 20% resume margin.
    pub fn should_accept(&self) -> bool {
        // Check resource pressure first (independent of task load)
        if self.check_resource_pressure() {
            return false;
        }

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

    /// Update resource metrics from a ResourceMonitor sample.
    pub fn update_resources(&self, metrics: ResourceMetrics) {
        self.memory_used_bytes
            .store(metrics.memory_used_bytes, Ordering::Relaxed);
        self.cpu_usage_bits
            .store(metrics.cpu_usage.to_bits(), Ordering::Relaxed);
    }

    /// Get current resource metrics.
    pub fn resource_metrics(&self) -> ResourceMetrics {
        ResourceMetrics {
            memory_used_bytes: self.memory_used_bytes.load(Ordering::Relaxed),
            memory_total_bytes: 0, // not tracked in state; available from monitor
            cpu_usage: f64::from_bits(self.cpu_usage_bits.load(Ordering::Relaxed)),
        }
    }

    /// Check resource thresholds. Returns true if resources are over-pressure
    /// and tasks should be rejected.
    fn check_resource_pressure(&self) -> bool {
        let has_thresholds =
            self.config.memory_threshold.is_some() || self.config.cpu_threshold.is_some();
        if !has_thresholds {
            return false;
        }

        let mem = self.memory_used_bytes.load(Ordering::Relaxed);
        let cpu = f64::from_bits(self.cpu_usage_bits.load(Ordering::Relaxed));
        let was_pressured = self.resource_pressure.load(Ordering::Relaxed);

        // Check memory threshold
        if let Some(threshold) = self.config.memory_threshold {
            let resume_threshold = (threshold as f64 * RESOURCE_RESUME_FACTOR) as u64;
            if !was_pressured && mem >= threshold as u64 {
                self.resource_pressure.store(true, Ordering::Relaxed);
                *self.backpressure_reason.write().unwrap() = Some(format!(
                    "memory {:.0} MB exceeds threshold {:.0} MB",
                    mem as f64 / 1_048_576.0,
                    threshold as f64 / 1_048_576.0,
                ));
                return true;
            }
            if was_pressured && mem >= resume_threshold {
                return true; // still under pressure
            }
        }

        // Check CPU threshold
        if let Some(threshold) = self.config.cpu_threshold {
            let resume_threshold = threshold * RESOURCE_RESUME_FACTOR;
            if !was_pressured && cpu >= threshold {
                self.resource_pressure.store(true, Ordering::Relaxed);
                *self.backpressure_reason.write().unwrap() = Some(format!(
                    "CPU {:.1}% exceeds threshold {:.1}%",
                    cpu * 100.0,
                    threshold * 100.0,
                ));
                return true;
            }
            if was_pressured && cpu >= resume_threshold {
                return true; // still under pressure
            }
        }

        // Both metrics below resume thresholds — release pressure
        if was_pressured {
            self.resource_pressure.store(false, Ordering::Relaxed);
            // Only clear reason if task-load backpressure isn't also active
            if self.accepting_tasks.load(Ordering::Relaxed) {
                *self.backpressure_reason.write().unwrap() = None;
            }
        }
        false
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

    /// Resume accepting tasks (if below low watermark and no resource pressure)
    pub fn resume(&self) {
        if self.load_ratio() <= self.config.low_watermark
            && !self.resource_pressure.load(Ordering::Relaxed)
        {
            self.accepting_tasks.store(true, Ordering::Relaxed);
            *self.backpressure_reason.write().unwrap() = None;
        }
    }

    /// Whether resource-based backpressure is currently active
    pub fn is_resource_pressured(&self) -> bool {
        self.resource_pressure.load(Ordering::Relaxed)
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

    #[test]
    fn test_memory_threshold_rejects_when_exceeded() {
        let config = BackpressureConfig::new().with_memory_threshold(1_000_000); // 1 MB
        let state = BackpressureState::new(config, 10);

        // Below threshold — accept
        state.update_resources(ResourceMetrics {
            memory_used_bytes: 500_000,
            memory_total_bytes: 2_000_000,
            cpu_usage: 0.0,
        });
        assert!(state.should_accept());

        // At threshold — reject
        state.update_resources(ResourceMetrics {
            memory_used_bytes: 1_000_000,
            memory_total_bytes: 2_000_000,
            cpu_usage: 0.0,
        });
        assert!(!state.should_accept());
        assert!(state.is_resource_pressured());
        assert!(state.backpressure_reason().unwrap().contains("memory"));
    }

    #[test]
    fn test_cpu_threshold_rejects_when_exceeded() {
        let config = BackpressureConfig::new().with_cpu_threshold(0.8); // 80%
        let state = BackpressureState::new(config, 10);

        // Below threshold — accept
        state.update_resources(ResourceMetrics {
            memory_used_bytes: 0,
            memory_total_bytes: 0,
            cpu_usage: 0.5,
        });
        assert!(state.should_accept());

        // Above threshold — reject
        state.update_resources(ResourceMetrics {
            memory_used_bytes: 0,
            memory_total_bytes: 0,
            cpu_usage: 0.85,
        });
        assert!(!state.should_accept());
        assert!(state.is_resource_pressured());
        assert!(state.backpressure_reason().unwrap().contains("CPU"));
    }

    #[test]
    fn test_resource_pressure_hysteresis() {
        let config = BackpressureConfig::new().with_cpu_threshold(0.8);
        let state = BackpressureState::new(config, 10);

        // Exceed threshold
        state.update_resources(ResourceMetrics {
            memory_used_bytes: 0,
            memory_total_bytes: 0,
            cpu_usage: 0.9,
        });
        assert!(!state.should_accept());

        // Drop below threshold but above resume point (0.8 * 0.8 = 0.64)
        state.update_resources(ResourceMetrics {
            memory_used_bytes: 0,
            memory_total_bytes: 0,
            cpu_usage: 0.7, // between 0.64 and 0.8
        });
        // Still rejected due to hysteresis
        assert!(!state.should_accept());

        // Drop below resume point
        state.update_resources(ResourceMetrics {
            memory_used_bytes: 0,
            memory_total_bytes: 0,
            cpu_usage: 0.5,
        });
        assert!(state.should_accept());
        assert!(!state.is_resource_pressured());
    }

    #[test]
    fn test_no_thresholds_skips_resource_check() {
        let config = BackpressureConfig::default(); // No memory/cpu thresholds
        let state = BackpressureState::new(config, 10);

        // Even with high resource usage, no thresholds means no rejection
        state.update_resources(ResourceMetrics {
            memory_used_bytes: u64::MAX,
            memory_total_bytes: u64::MAX,
            cpu_usage: 1.0,
        });
        assert!(state.should_accept());
    }

    #[test]
    fn test_combined_load_and_resource_pressure() {
        let config = BackpressureConfig::new()
            .with_high_watermark(0.8)
            .with_low_watermark(0.5)
            .with_cpu_threshold(0.9);
        let state = BackpressureState::new(config, 10);

        // Task load is fine but CPU is over threshold
        state.update_resources(ResourceMetrics {
            memory_used_bytes: 0,
            memory_total_bytes: 0,
            cpu_usage: 0.95,
        });
        assert!(!state.should_accept());

        // CPU recovers but task load is over high watermark
        state.update_resources(ResourceMetrics {
            memory_used_bytes: 0,
            memory_total_bytes: 0,
            cpu_usage: 0.3,
        });
        for _ in 0..9 {
            state.task_started();
        }
        assert!(!state.should_accept());
    }

    #[test]
    fn test_resource_monitor_creates() {
        let mut monitor = ResourceMonitor::new();
        let metrics = monitor.sample();
        // Should return valid metrics on any system
        assert!(metrics.cpu_usage >= 0.0);
        // memory_used_bytes should be > 0 on any running system
        assert!(metrics.memory_used_bytes > 0);
    }

    #[test]
    fn test_resume_blocked_by_resource_pressure() {
        let config = BackpressureConfig::new()
            .with_high_watermark(0.8)
            .with_low_watermark(0.5)
            .with_cpu_threshold(0.9);
        let state = BackpressureState::new(config, 10);

        // Trigger resource pressure
        state.update_resources(ResourceMetrics {
            memory_used_bytes: 0,
            memory_total_bytes: 0,
            cpu_usage: 0.95,
        });
        state.should_accept(); // trigger the check

        // Manually pause and try to resume — should be blocked by resource pressure
        state.pause("test");
        state.resume();
        assert!(!state.is_accepting());
    }
}
