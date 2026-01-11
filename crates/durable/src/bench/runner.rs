//! Benchmark runner framework
//!
//! Provides infrastructure for running load tests with multiple workers.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::metrics::BenchmarkMetrics;
use super::report::{BenchmarkReport, ReportConfig};

/// Configuration for a benchmark scenario
#[derive(Debug, Clone)]
pub struct ScenarioConfig {
    /// Name of the scenario
    pub name: String,
    /// Number of workers (concurrent executors)
    pub workers: usize,
    /// Total number of tasks to execute
    pub total_tasks: u64,
    /// Warmup duration (results discarded)
    pub warmup: Duration,
    /// Maximum duration for the benchmark
    pub max_duration: Duration,
    /// Sampling interval for metrics
    pub sample_interval: Duration,
    /// Target rate (tasks/sec), None for max throughput
    pub target_rate: Option<f64>,
}

impl Default for ScenarioConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            workers: 10,
            total_tasks: 10_000,
            warmup: Duration::from_secs(1),
            max_duration: Duration::from_secs(60),
            sample_interval: Duration::from_millis(100),
            target_rate: None,
        }
    }
}

/// Trait for benchmark scenarios
pub trait BenchmarkScenario: Send + Sync {
    /// Setup the scenario (create store, enqueue tasks, etc.)
    fn setup(&self) -> impl Future<Output = ()> + Send;

    /// Execute a single task, returning (schedule_to_start, execution_time)
    fn execute_task(&self, task_id: u64) -> impl Future<Output = (Duration, Duration)> + Send;

    /// Cleanup after the scenario
    fn cleanup(&self) -> impl Future<Output = ()> + Send;
}

/// Runs benchmark scenarios and collects metrics
pub struct BenchmarkRunner {
    config: ScenarioConfig,
    metrics: Arc<BenchmarkMetrics>,
    running: Arc<AtomicBool>,
    completed: Arc<AtomicU64>,
}

impl BenchmarkRunner {
    pub fn new(config: ScenarioConfig) -> Self {
        Self {
            metrics: Arc::new(BenchmarkMetrics::new(&config.name)),
            running: Arc::new(AtomicBool::new(false)),
            completed: Arc::new(AtomicU64::new(0)),
            config,
        }
    }

    /// Run the benchmark scenario
    pub async fn run<S: BenchmarkScenario + 'static>(&self, scenario: Arc<S>) {
        println!("🚀 Starting benchmark: {}", self.config.name);
        println!(
            "   Workers: {}, Tasks: {}, Max duration: {:?}",
            self.config.workers, self.config.total_tasks, self.config.max_duration
        );

        // Setup
        scenario.setup().await;
        self.running.store(true, Ordering::Release);

        // Start metrics sampler
        let metrics = self.metrics.clone();
        let sample_interval = self.config.sample_interval;
        let running = self.running.clone();
        let sampler = tokio::spawn(async move {
            while running.load(Ordering::Acquire) {
                metrics.sample();
                tokio::time::sleep(sample_interval).await;
            }
        });

        // Warmup phase
        if self.config.warmup > Duration::ZERO {
            println!("   Warmup: {:?}...", self.config.warmup);
            let warmup_end = Instant::now() + self.config.warmup;
            let warmup_tasks = (self.config.total_tasks / 10).max(100);

            self.run_tasks(scenario.clone(), warmup_tasks, Some(warmup_end))
                .await;

            // Reset metrics after warmup
            self.completed.store(0, Ordering::Release);
        }

        // Main benchmark
        println!("   Running main benchmark...");
        let start = Instant::now();
        let deadline = start + self.config.max_duration;

        self.run_tasks(scenario.clone(), self.config.total_tasks, Some(deadline))
            .await;

        self.running.store(false, Ordering::Release);
        sampler.abort();

        // Final sample
        self.metrics.sample();

        // Cleanup
        scenario.cleanup().await;

        println!("✅ Benchmark complete");
        self.print_summary();
    }

    async fn run_tasks<S: BenchmarkScenario + 'static>(
        &self,
        scenario: Arc<S>,
        total: u64,
        deadline: Option<Instant>,
    ) {
        let semaphore = Arc::new(Semaphore::new(self.config.workers));
        let mut tasks = JoinSet::new();

        // Rate limiter for target rate
        let rate_limiter = self
            .config
            .target_rate
            .map(|_rate| Arc::new(tokio::sync::Mutex::new(Instant::now())));

        for task_id in 0..total {
            // Check deadline
            if let Some(deadline) = deadline {
                if Instant::now() >= deadline {
                    break;
                }
            }

            // Rate limiting
            if let Some(ref limiter) = rate_limiter {
                let mut last = limiter.lock().await;
                let next = *last + Duration::from_secs_f64(1.0 / self.config.target_rate.unwrap());
                if next > Instant::now() {
                    tokio::time::sleep_until(tokio::time::Instant::from_std(next)).await;
                }
                *last = Instant::now();
            }

            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let scenario = scenario.clone();
            let metrics = self.metrics.clone();
            let completed = self.completed.clone();

            tasks.spawn(async move {
                let task_start = Instant::now();

                let (schedule_to_start, execution_time) = scenario.execute_task(task_id).await;

                let end_to_end = task_start.elapsed();

                metrics.schedule_to_start.record(schedule_to_start);
                metrics.execution.record(execution_time);
                metrics.end_to_end.record(end_to_end);
                metrics.tasks_completed.increment();

                completed.fetch_add(1, Ordering::Relaxed);

                drop(permit);
            });

            // Progress reporting
            if task_id > 0 && task_id % 1000 == 0 {
                let completed = self.completed.load(Ordering::Relaxed);
                let rate = completed as f64 / self.metrics.elapsed().as_secs_f64();
                println!(
                    "   Progress: {}/{} tasks ({:.1} tasks/sec)",
                    completed, total, rate
                );
            }
        }

        // Wait for all tasks to complete
        while let Some(result) = tasks.join_next().await {
            if let Err(e) = result {
                eprintln!("Task error: {:?}", e);
            }
        }
    }

    fn print_summary(&self) {
        let e2e = self.metrics.end_to_end.summary();
        let s2s = self.metrics.schedule_to_start.summary();

        println!("\n📊 Results:");
        println!(
            "   Total tasks:     {}",
            self.metrics.tasks_completed.total()
        );
        println!(
            "   Duration:        {:.2}s",
            self.metrics.elapsed().as_secs_f64()
        );
        println!(
            "   Throughput:      {:.1} tasks/sec",
            self.metrics.tasks_completed.throughput()
        );
        println!();
        println!("   End-to-End Latency:");
        println!("     P50:  {:.2}ms", e2e.p50.as_secs_f64() * 1000.0);
        println!("     P95:  {:.2}ms", e2e.p95.as_secs_f64() * 1000.0);
        println!("     P99:  {:.2}ms", e2e.p99.as_secs_f64() * 1000.0);
        println!("     Max:  {:.2}ms", e2e.max.as_secs_f64() * 1000.0);
        println!();
        println!("   Schedule-to-Start Latency:");
        println!("     P50:  {:.2}ms", s2s.p50.as_secs_f64() * 1000.0);
        println!("     P95:  {:.2}ms", s2s.p95.as_secs_f64() * 1000.0);
        println!("     P99:  {:.2}ms", s2s.p99.as_secs_f64() * 1000.0);
        println!();
        println!(
            "   Peak Memory:     {:.1} MB",
            self.metrics.resources.peak_memory_mb()
        );
        println!(
            "   Avg CPU:         {:.1}%",
            self.metrics.resources.avg_cpu_percent()
        );
    }

    /// Generate HTML report
    pub fn generate_report(&self, config: ReportConfig) -> std::io::Result<String> {
        let report = BenchmarkReport::new(config);
        report.generate(&self.metrics)
    }

    /// Get metrics for custom analysis
    pub fn metrics(&self) -> Arc<BenchmarkMetrics> {
        self.metrics.clone()
    }
}

/// Activity duration distribution based on real-world patterns
#[derive(Debug, Clone, Copy)]
pub enum ActivityDuration {
    /// Fast: 100-200ms (60% of tasks)
    Fast,
    /// Medium: 1-10s (30% of tasks)
    Medium,
    /// Slow: 10-30s (9% of tasks)
    Slow,
    /// Very long: 30s-2min (1% of tasks)
    VeryLong,
}

impl ActivityDuration {
    /// Sample a duration based on weighted distribution
    pub fn sample() -> Duration {
        let r: f64 = rand::random();
        let category = if r < 0.60 {
            Self::Fast
        } else if r < 0.90 {
            Self::Medium
        } else if r < 0.99 {
            Self::Slow
        } else {
            Self::VeryLong
        };

        category.random_duration()
    }

    /// Get a random duration within this category
    pub fn random_duration(self) -> Duration {
        let (min_ms, max_ms) = match self {
            Self::Fast => (100, 200),
            Self::Medium => (1000, 10000),
            Self::Slow => (10000, 30000),
            Self::VeryLong => (30000, 120000),
        };

        let ms = min_ms + rand::random::<u64>() % (max_ms - min_ms);
        Duration::from_millis(ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activity_duration_distribution() {
        let mut fast = 0;
        let mut medium = 0;
        let mut slow = 0;
        let mut very_long = 0;

        for _ in 0..10000 {
            let d = ActivityDuration::sample();
            if d < Duration::from_millis(500) {
                fast += 1;
            } else if d < Duration::from_secs(15) {
                medium += 1;
            } else if d < Duration::from_secs(35) {
                slow += 1;
            } else {
                very_long += 1;
            }
        }

        // Check rough distribution (with tolerance)
        assert!(fast > 5000, "Expected ~60% fast, got {}", fast);
        assert!(medium > 2000, "Expected ~30% medium, got {}", medium);
        assert!(slow > 500, "Expected ~9% slow, got {}", slow);
        assert!(very_long > 50, "Expected ~1% very_long, got {}", very_long);
    }
}
