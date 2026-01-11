//! Metrics collection for benchmarks
//!
//! Collects latency distributions, throughput, and resource usage.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};

/// Histogram for latency measurements with configurable buckets
#[derive(Debug)]
pub struct LatencyHistogram {
    /// Raw samples (for percentile calculation)
    samples: Mutex<Vec<Duration>>,
    /// Sum of all samples (for mean calculation)
    sum_micros: AtomicU64,
    /// Count of samples
    count: AtomicU64,
    /// Min latency observed
    min_micros: AtomicU64,
    /// Max latency observed
    max_micros: AtomicU64,
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl LatencyHistogram {
    pub fn new() -> Self {
        Self {
            samples: Mutex::new(Vec::with_capacity(100_000)),
            sum_micros: AtomicU64::new(0),
            count: AtomicU64::new(0),
            min_micros: AtomicU64::new(u64::MAX),
            max_micros: AtomicU64::new(0),
        }
    }

    /// Record a latency sample
    pub fn record(&self, duration: Duration) {
        let micros = duration.as_micros() as u64;

        self.samples.lock().push(duration);
        self.sum_micros.fetch_add(micros, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);

        // Update min (compare-and-swap loop)
        let mut current_min = self.min_micros.load(Ordering::Relaxed);
        while micros < current_min {
            match self.min_micros.compare_exchange_weak(
                current_min,
                micros,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(new) => current_min = new,
            }
        }

        // Update max
        let mut current_max = self.max_micros.load(Ordering::Relaxed);
        while micros > current_max {
            match self.max_micros.compare_exchange_weak(
                current_max,
                micros,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(new) => current_max = new,
            }
        }
    }

    /// Get the count of samples
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Get the mean latency
    pub fn mean(&self) -> Duration {
        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            return Duration::ZERO;
        }
        let sum = self.sum_micros.load(Ordering::Relaxed);
        Duration::from_micros(sum / count)
    }

    /// Get the minimum latency
    pub fn min(&self) -> Duration {
        let min = self.min_micros.load(Ordering::Relaxed);
        if min == u64::MAX {
            Duration::ZERO
        } else {
            Duration::from_micros(min)
        }
    }

    /// Get the maximum latency
    pub fn max(&self) -> Duration {
        Duration::from_micros(self.max_micros.load(Ordering::Relaxed))
    }

    /// Calculate percentile (0.0 to 1.0)
    pub fn percentile(&self, p: f64) -> Duration {
        let mut samples = self.samples.lock();
        if samples.is_empty() {
            return Duration::ZERO;
        }

        samples.sort();
        let idx = ((samples.len() as f64 * p) as usize).min(samples.len() - 1);
        samples[idx]
    }

    /// Get summary statistics
    pub fn summary(&self) -> LatencySummary {
        LatencySummary {
            count: self.count(),
            mean: self.mean(),
            min: self.min(),
            max: self.max(),
            p50: self.percentile(0.50),
            p95: self.percentile(0.95),
            p99: self.percentile(0.99),
        }
    }

    /// Get all samples for charting
    pub fn samples(&self) -> Vec<Duration> {
        self.samples.lock().clone()
    }
}

/// Summary statistics for latency
#[derive(Debug, Clone)]
pub struct LatencySummary {
    pub count: u64,
    pub mean: Duration,
    pub min: Duration,
    pub max: Duration,
    pub p50: Duration,
    pub p95: Duration,
    pub p99: Duration,
}

/// Counter for throughput measurement
#[derive(Debug)]
pub struct ThroughputCounter {
    /// Start time
    start: Instant,
    /// Count of operations
    count: AtomicU64,
    /// Time-series data: (timestamp_ms, cumulative_count)
    timeseries: Mutex<Vec<(u64, u64)>>,
    /// Last sample time
    last_sample: Mutex<Instant>,
}

impl ThroughputCounter {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            count: AtomicU64::new(0),
            timeseries: Mutex::new(vec![(0, 0)]),
            last_sample: Mutex::new(now),
        }
    }

    /// Increment the counter
    pub fn increment(&self) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment by N
    pub fn increment_by(&self, n: u64) {
        self.count.fetch_add(n, Ordering::Relaxed);
    }

    /// Sample current value for timeseries (call periodically)
    pub fn sample(&self) {
        let elapsed_ms = self.start.elapsed().as_millis() as u64;
        let count = self.count.load(Ordering::Relaxed);
        self.timeseries.lock().push((elapsed_ms, count));
        *self.last_sample.lock() = Instant::now();
    }

    /// Get total count
    pub fn total(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Get elapsed time
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Get throughput (ops/sec)
    pub fn throughput(&self) -> f64 {
        let elapsed = self.start.elapsed().as_secs_f64();
        if elapsed == 0.0 {
            return 0.0;
        }
        self.count.load(Ordering::Relaxed) as f64 / elapsed
    }

    /// Get timeseries data for charting
    pub fn timeseries(&self) -> Vec<(u64, u64)> {
        self.timeseries.lock().clone()
    }
}

impl Default for ThroughputCounter {
    fn default() -> Self {
        Self::new()
    }
}

/// Resource usage snapshot
#[derive(Debug, Clone, Default)]
pub struct ResourceSnapshot {
    pub timestamp_ms: u64,
    pub memory_rss_mb: f64,
    pub cpu_percent: f32,
}

/// Collects resource usage over time
pub struct ResourceMonitor {
    start: Instant,
    pid: Pid,
    system: Mutex<System>,
    snapshots: Mutex<Vec<ResourceSnapshot>>,
}

impl ResourceMonitor {
    pub fn new() -> Self {
        let pid = Pid::from_u32(std::process::id());
        let system = System::new_with_specifics(
            RefreshKind::new().with_processes(ProcessRefreshKind::everything()),
        );

        Self {
            start: Instant::now(),
            pid,
            system: Mutex::new(system),
            snapshots: Mutex::new(Vec::new()),
        }
    }

    /// Sample current resource usage
    pub fn sample(&self) {
        let mut system = self.system.lock();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[self.pid]),
            true,
            ProcessRefreshKind::everything(),
        );

        let snapshot = if let Some(process) = system.process(self.pid) {
            ResourceSnapshot {
                timestamp_ms: self.start.elapsed().as_millis() as u64,
                memory_rss_mb: process.memory() as f64 / (1024.0 * 1024.0),
                cpu_percent: process.cpu_usage(),
            }
        } else {
            ResourceSnapshot {
                timestamp_ms: self.start.elapsed().as_millis() as u64,
                ..Default::default()
            }
        };

        self.snapshots.lock().push(snapshot);
    }

    /// Get all snapshots
    pub fn snapshots(&self) -> Vec<ResourceSnapshot> {
        self.snapshots.lock().clone()
    }

    /// Get peak memory usage
    pub fn peak_memory_mb(&self) -> f64 {
        self.snapshots
            .lock()
            .iter()
            .map(|s| s.memory_rss_mb)
            .fold(0.0, f64::max)
    }

    /// Get average CPU usage
    pub fn avg_cpu_percent(&self) -> f32 {
        let snapshots = self.snapshots.lock();
        if snapshots.is_empty() {
            return 0.0;
        }
        snapshots.iter().map(|s| s.cpu_percent).sum::<f32>() / snapshots.len() as f32
    }
}

impl Default for ResourceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Aggregated benchmark metrics
pub struct BenchmarkMetrics {
    /// Name of the benchmark
    pub name: String,
    /// Schedule-to-start latency (enqueue → claim)
    pub schedule_to_start: Arc<LatencyHistogram>,
    /// Execution latency (claim → complete)
    pub execution: Arc<LatencyHistogram>,
    /// End-to-end latency (enqueue → complete)
    pub end_to_end: Arc<LatencyHistogram>,
    /// Tasks completed counter
    pub tasks_completed: Arc<ThroughputCounter>,
    /// Tasks enqueued counter
    pub tasks_enqueued: Arc<ThroughputCounter>,
    /// Resource monitor
    pub resources: Arc<ResourceMonitor>,
    /// Start time
    pub start: Instant,
    /// Custom metrics
    pub custom: Mutex<std::collections::HashMap<String, Arc<LatencyHistogram>>>,
}

impl BenchmarkMetrics {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            schedule_to_start: Arc::new(LatencyHistogram::new()),
            execution: Arc::new(LatencyHistogram::new()),
            end_to_end: Arc::new(LatencyHistogram::new()),
            tasks_completed: Arc::new(ThroughputCounter::new()),
            tasks_enqueued: Arc::new(ThroughputCounter::new()),
            resources: Arc::new(ResourceMonitor::new()),
            start: Instant::now(),
            custom: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Record a custom latency metric
    pub fn record_custom(&self, name: &str, duration: Duration) {
        let mut custom = self.custom.lock();
        let histogram = custom
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(LatencyHistogram::new()));
        histogram.record(duration);
    }

    /// Get elapsed time since start
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Sample all time-series data (call periodically)
    pub fn sample(&self) {
        self.tasks_completed.sample();
        self.tasks_enqueued.sample();
        self.resources.sample();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_latency_histogram() {
        let hist = LatencyHistogram::new();

        for i in 1..=100 {
            hist.record(Duration::from_micros(i));
        }

        assert_eq!(hist.count(), 100);
        assert_eq!(hist.min(), Duration::from_micros(1));
        assert_eq!(hist.max(), Duration::from_micros(100));

        // P50 should be around 50
        let p50 = hist.percentile(0.50);
        assert!(p50 >= Duration::from_micros(49) && p50 <= Duration::from_micros(51));
    }

    #[test]
    fn test_throughput_counter() {
        let counter = ThroughputCounter::new();

        for _ in 0..1000 {
            counter.increment();
        }

        assert_eq!(counter.total(), 1000);
        assert!(counter.throughput() > 0.0);
    }
}
