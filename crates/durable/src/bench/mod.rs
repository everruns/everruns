//! Benchmark support utilities
//!
//! Provides metrics collection, HTML report generation, and checkpointing
//! for load tests. Inspired by Gatling's reporting style.
//!
//! # Checkpointing
//!
//! Save benchmark results for historical comparison:
//!
//! ```ignore
//! let runner = BenchmarkRunner::new(config);
//! runner.run(scenario).await;
//!
//! // Save checkpoint with auto-detected environment
//! let result = runner.save_checkpoint(None)?;
//! result.print_summary();
//!
//! // Or with custom moniker
//! let result = runner.save_checkpoint_with_moniker("ci-4cpu-8gb")?;
//! ```

mod checkpoint;
mod metrics;
mod report;
mod runner;

pub use checkpoint::{
    BenchmarkCheckpoint, CheckpointComparison, CheckpointFilter, CheckpointStore,
    CheckpointSummary, EnvironmentInfo, PerformanceChanges,
};
pub use metrics::{
    BenchmarkMetrics, LatencyHistogram, LatencyHistogramSnapshot, LatencySummary, MetricsSnapshot,
    ResourceMonitorSnapshot, ResourceSnapshot, ThroughputCounter, ThroughputSnapshot,
};
pub use report::{BenchmarkReport, ReportConfig};
pub use runner::{
    ActivityDuration, BenchmarkRunner, BenchmarkScenario, CheckpointSaveResult, ScenarioConfig,
    clear_terminal_progress, set_terminal_progress,
};
