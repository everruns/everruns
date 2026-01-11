//! Benchmark support utilities
//!
//! Provides metrics collection and HTML report generation for load tests.
//! Inspired by Gatling's reporting style.

mod metrics;
mod report;
mod runner;

pub use metrics::{BenchmarkMetrics, LatencyHistogram, ThroughputCounter};
pub use report::{BenchmarkReport, ReportConfig};
pub use runner::{ActivityDuration, BenchmarkRunner, BenchmarkScenario, ScenarioConfig};
