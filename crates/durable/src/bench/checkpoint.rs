//! Benchmark checkpointing for historical comparison
//!
//! Save benchmark results with environment info (moniker) for tracking
//! performance over time across different machines/configurations.

use std::fs;
use std::io::{self, BufReader, BufWriter};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

use super::metrics::MetricsSnapshot;

/// Environment information for a benchmark run
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EnvironmentInfo {
    /// Human-readable moniker (e.g., "local-M4-46GB", "ci-4cpu-8gb")
    pub moniker: String,
    /// Operating system name and version
    pub os: String,
    /// CPU model/name
    pub cpu_name: String,
    /// Number of CPU cores
    pub cpu_cores: usize,
    /// Total system memory in GB
    pub memory_gb: f64,
    /// Hostname (optional)
    pub hostname: Option<String>,
}

impl EnvironmentInfo {
    /// Detect environment information automatically
    pub fn detect() -> Self {
        let system = System::new_with_specifics(
            RefreshKind::new()
                .with_cpu(CpuRefreshKind::new())
                .with_memory(MemoryRefreshKind::everything()),
        );

        let cpu_name = system
            .cpus()
            .first()
            .map(|cpu| cpu.brand().to_string())
            .unwrap_or_else(|| "Unknown CPU".to_string());

        let cpu_cores = system.cpus().len();
        let memory_gb = system.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0);

        let os = format!(
            "{} {}",
            System::name().unwrap_or_default(),
            System::os_version().unwrap_or_default()
        );
        let hostname = System::host_name();

        // Generate default moniker from environment
        let moniker = Self::generate_moniker(&cpu_name, cpu_cores, memory_gb);

        Self {
            moniker,
            os,
            cpu_name,
            cpu_cores,
            memory_gb,
            hostname,
        }
    }

    /// Detect with custom moniker
    pub fn detect_with_moniker(moniker: impl Into<String>) -> Self {
        let mut info = Self::detect();
        info.moniker = moniker.into();
        info
    }

    /// Generate a default moniker from CPU/memory info
    fn generate_moniker(cpu_name: &str, cores: usize, memory_gb: f64) -> String {
        // Extract short CPU identifier
        let cpu_short = if cpu_name.contains("Apple") {
            // Apple Silicon: "Apple M4 Pro" -> "M4-Pro"
            cpu_name
                .replace("Apple ", "")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("-")
        } else if cpu_name.contains("Intel") {
            // Intel: "Intel(R) Core(TM) i9-12900K" -> "i9-12900K"
            cpu_name
                .split_whitespace()
                .find(|s| s.starts_with('i') && s.contains('-'))
                .unwrap_or("Intel")
                .to_string()
        } else if cpu_name.contains("AMD") {
            // AMD: "AMD Ryzen 9 5950X" -> "R9-5950X"
            let parts: Vec<&str> = cpu_name.split_whitespace().collect();
            if parts.len() >= 4 {
                format!(
                    "R{}-{}",
                    parts.get(2).unwrap_or(&""),
                    parts.get(3).unwrap_or(&"")
                )
            } else {
                "AMD".to_string()
            }
        } else {
            format!("{}c", cores)
        };

        format!("local-{}-{}GB", cpu_short, memory_gb.round() as u64)
    }
}

/// A single benchmark result checkpoint
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkCheckpoint {
    /// Unique ID for this checkpoint
    pub id: String,
    /// Benchmark name/scenario
    pub benchmark_name: String,
    /// Timestamp when benchmark was run
    pub timestamp: DateTime<Utc>,
    /// Environment information
    pub environment: EnvironmentInfo,
    /// Scenario configuration (serialized for flexibility)
    pub config: serde_json::Value,
    /// Metrics snapshot
    pub metrics: MetricsSnapshot,
    /// Optional tags for filtering
    pub tags: Vec<String>,
    /// Optional notes
    pub notes: Option<String>,
}

impl BenchmarkCheckpoint {
    /// Create a new checkpoint
    pub fn new(
        benchmark_name: impl Into<String>,
        environment: EnvironmentInfo,
        metrics: MetricsSnapshot,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string(),
            benchmark_name: benchmark_name.into(),
            timestamp: Utc::now(),
            environment,
            config: serde_json::Value::Null,
            metrics,
            tags: Vec::new(),
            notes: None,
        }
    }

    /// Set scenario configuration
    pub fn with_config<T: serde::Serialize>(mut self, config: &T) -> Self {
        self.config = serde_json::to_value(config).unwrap_or(serde_json::Value::Null);
        self
    }

    /// Add tags for filtering
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Add a note
    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }

    /// Generate a filename for this checkpoint
    pub fn filename(&self) -> String {
        format!(
            "{}_{}_{}_{}.json",
            self.benchmark_name.replace(' ', "_").to_lowercase(),
            self.environment.moniker.replace(' ', "_").to_lowercase(),
            self.timestamp.format("%Y%m%d_%H%M%S"),
            &self.id[..8]
        )
    }
}

/// Manages benchmark checkpoints storage and retrieval
pub struct CheckpointStore {
    /// Directory where checkpoints are stored
    directory: PathBuf,
}

impl CheckpointStore {
    /// Create a new checkpoint store
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    /// Default store location (crates/durable/benches/checkpoints - under source control)
    pub fn default_location() -> Self {
        Self::new("crates/durable/benches/checkpoints")
    }

    /// Ensure the directory exists
    fn ensure_dir(&self) -> io::Result<()> {
        fs::create_dir_all(&self.directory)
    }

    /// Save a checkpoint
    pub fn save(&self, checkpoint: &BenchmarkCheckpoint) -> io::Result<PathBuf> {
        self.ensure_dir()?;

        let path = self.directory.join(checkpoint.filename());
        let file = fs::File::create(&path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, checkpoint)?;

        // Return absolute path for display
        let absolute = path.canonicalize().unwrap_or(path);
        Ok(absolute)
    }

    /// Load a checkpoint by ID or filename
    pub fn load(&self, id_or_filename: &str) -> io::Result<BenchmarkCheckpoint> {
        let path = if id_or_filename.ends_with(".json") {
            self.directory.join(id_or_filename)
        } else {
            // Search by ID prefix
            self.find_by_id(id_or_filename)?
        };

        let file = fs::File::open(&path)?;
        let reader = BufReader::new(file);
        let checkpoint: BenchmarkCheckpoint = serde_json::from_reader(reader)?;
        Ok(checkpoint)
    }

    /// Find checkpoint file by ID prefix
    fn find_by_id(&self, id_prefix: &str) -> io::Result<PathBuf> {
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(checkpoint) = self.load(path.file_name().unwrap().to_str().unwrap()) {
                    if checkpoint.id.starts_with(id_prefix) {
                        return Ok(path);
                    }
                }
            }
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Checkpoint not found: {}", id_prefix),
        ))
    }

    /// List all checkpoints, optionally filtered
    pub fn list(&self, filter: Option<&CheckpointFilter>) -> io::Result<Vec<BenchmarkCheckpoint>> {
        if !self.directory.exists() {
            return Ok(Vec::new());
        }

        let mut checkpoints = Vec::new();

        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(checkpoint) = self.load(path.file_name().unwrap().to_str().unwrap()) {
                    if filter.map(|f| f.matches(&checkpoint)).unwrap_or(true) {
                        checkpoints.push(checkpoint);
                    }
                }
            }
        }

        // Sort by timestamp (newest first)
        checkpoints.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(checkpoints)
    }

    /// Get checkpoints for comparison (same benchmark, same or different env)
    pub fn get_comparison(
        &self,
        benchmark_name: &str,
        moniker: Option<&str>,
        limit: usize,
    ) -> io::Result<Vec<BenchmarkCheckpoint>> {
        let filter = CheckpointFilter {
            benchmark_name: Some(benchmark_name.to_string()),
            moniker: moniker.map(|s| s.to_string()),
            ..Default::default()
        };

        let mut checkpoints = self.list(Some(&filter))?;
        checkpoints.truncate(limit);
        Ok(checkpoints)
    }

    /// Delete old checkpoints, keeping the most recent N per benchmark+moniker
    pub fn cleanup(&self, keep_per_combo: usize) -> io::Result<usize> {
        let all = self.list(None)?;

        // Group by benchmark+moniker
        let mut groups: std::collections::HashMap<String, Vec<BenchmarkCheckpoint>> =
            std::collections::HashMap::new();

        for checkpoint in all {
            let key = format!(
                "{}:{}",
                checkpoint.benchmark_name, checkpoint.environment.moniker
            );
            groups.entry(key).or_default().push(checkpoint);
        }

        let mut deleted = 0;
        for (_, mut checkpoints) in groups {
            // Sort by timestamp (newest first)
            checkpoints.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

            // Delete all but the most recent N
            for checkpoint in checkpoints.into_iter().skip(keep_per_combo) {
                let path = self.directory.join(checkpoint.filename());
                if fs::remove_file(&path).is_ok() {
                    deleted += 1;
                }
            }
        }

        Ok(deleted)
    }
}

/// Filter for listing checkpoints
#[derive(Debug, Clone, Default)]
pub struct CheckpointFilter {
    /// Filter by benchmark name
    pub benchmark_name: Option<String>,
    /// Filter by environment moniker
    pub moniker: Option<String>,
    /// Filter by tag
    pub tag: Option<String>,
    /// Filter by date range (after)
    pub after: Option<DateTime<Utc>>,
    /// Filter by date range (before)
    pub before: Option<DateTime<Utc>>,
}

impl CheckpointFilter {
    /// Check if a checkpoint matches this filter
    pub fn matches(&self, checkpoint: &BenchmarkCheckpoint) -> bool {
        if let Some(ref name) = self.benchmark_name {
            if !checkpoint.benchmark_name.contains(name) {
                return false;
            }
        }

        if let Some(ref moniker) = self.moniker {
            if !checkpoint.environment.moniker.contains(moniker) {
                return false;
            }
        }

        if let Some(ref tag) = self.tag {
            if !checkpoint.tags.contains(tag) {
                return false;
            }
        }

        if let Some(after) = self.after {
            if checkpoint.timestamp < after {
                return false;
            }
        }

        if let Some(before) = self.before {
            if checkpoint.timestamp > before {
                return false;
            }
        }

        true
    }
}

/// Comparison result between two checkpoints
#[derive(Debug, Clone, serde::Serialize)]
pub struct CheckpointComparison {
    /// Current checkpoint
    pub current: CheckpointSummary,
    /// Baseline checkpoint
    pub baseline: CheckpointSummary,
    /// Percentage changes (positive = regression, negative = improvement)
    pub changes: PerformanceChanges,
}

/// Summary of a checkpoint for comparison
#[derive(Debug, Clone, serde::Serialize)]
pub struct CheckpointSummary {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub moniker: String,
    pub throughput: f64,
    pub e2e_p50_ms: f64,
    pub e2e_p99_ms: f64,
    pub s2s_p99_ms: f64,
}

/// Performance changes between checkpoints
#[derive(Debug, Clone, serde::Serialize)]
pub struct PerformanceChanges {
    /// Throughput change (positive = faster, which is good)
    pub throughput_pct: f64,
    /// P50 change (positive = slower, which is bad)
    pub e2e_p50_pct: f64,
    /// P99 change (positive = slower, which is bad)
    pub e2e_p99_pct: f64,
    /// S2S P99 change (positive = slower, which is bad)
    pub s2s_p99_pct: f64,
}

impl CheckpointComparison {
    /// Compare two checkpoints
    pub fn compare(current: &BenchmarkCheckpoint, baseline: &BenchmarkCheckpoint) -> Self {
        let current_throughput = current.metrics.tasks_completed.throughput();
        let baseline_throughput = baseline.metrics.tasks_completed.throughput();

        let current_e2e = current.metrics.end_to_end.summary();
        let baseline_e2e = baseline.metrics.end_to_end.summary();

        let current_s2s = current.metrics.schedule_to_start.summary();
        let baseline_s2s = baseline.metrics.schedule_to_start.summary();

        let pct_change = |current: f64, baseline: f64| -> f64 {
            if baseline == 0.0 {
                0.0
            } else {
                ((current - baseline) / baseline) * 100.0
            }
        };

        Self {
            current: CheckpointSummary {
                id: current.id.clone(),
                timestamp: current.timestamp,
                moniker: current.environment.moniker.clone(),
                throughput: current_throughput,
                e2e_p50_ms: current_e2e.p50.as_secs_f64() * 1000.0,
                e2e_p99_ms: current_e2e.p99.as_secs_f64() * 1000.0,
                s2s_p99_ms: current_s2s.p99.as_secs_f64() * 1000.0,
            },
            baseline: CheckpointSummary {
                id: baseline.id.clone(),
                timestamp: baseline.timestamp,
                moniker: baseline.environment.moniker.clone(),
                throughput: baseline_throughput,
                e2e_p50_ms: baseline_e2e.p50.as_secs_f64() * 1000.0,
                e2e_p99_ms: baseline_e2e.p99.as_secs_f64() * 1000.0,
                s2s_p99_ms: baseline_s2s.p99.as_secs_f64() * 1000.0,
            },
            changes: PerformanceChanges {
                // For throughput, higher is better, so we invert the sign
                throughput_pct: pct_change(current_throughput, baseline_throughput),
                e2e_p50_pct: pct_change(
                    current_e2e.p50.as_secs_f64(),
                    baseline_e2e.p50.as_secs_f64(),
                ),
                e2e_p99_pct: pct_change(
                    current_e2e.p99.as_secs_f64(),
                    baseline_e2e.p99.as_secs_f64(),
                ),
                s2s_p99_pct: pct_change(
                    current_s2s.p99.as_secs_f64(),
                    baseline_s2s.p99.as_secs_f64(),
                ),
            },
        }
    }

    /// Check if there are significant regressions
    pub fn has_regression(&self, threshold_pct: f64) -> bool {
        // Throughput decrease is bad
        self.changes.throughput_pct < -threshold_pct
            // Latency increase is bad
            || self.changes.e2e_p50_pct > threshold_pct
            || self.changes.e2e_p99_pct > threshold_pct
            || self.changes.s2s_p99_pct > threshold_pct
    }

    /// Print comparison summary to console
    pub fn print_summary(&self) {
        println!("\n📊 Performance Comparison");
        println!(
            "   Current:  {} @ {}",
            self.current.moniker,
            self.current.timestamp.format("%Y-%m-%d %H:%M")
        );
        println!(
            "   Baseline: {} @ {}",
            self.baseline.moniker,
            self.baseline.timestamp.format("%Y-%m-%d %H:%M")
        );
        println!();

        let arrow = |pct: f64, higher_is_better: bool| -> &'static str {
            let is_better = if higher_is_better {
                pct > 0.0
            } else {
                pct < 0.0
            };
            if pct.abs() < 1.0 {
                "≈"
            } else if is_better {
                "✅"
            } else {
                "❌"
            }
        };

        println!(
            "   Throughput:    {:>8.1} → {:>8.1} tasks/sec  ({:+.1}%) {}",
            self.baseline.throughput,
            self.current.throughput,
            self.changes.throughput_pct,
            arrow(self.changes.throughput_pct, true)
        );
        println!(
            "   E2E P50:       {:>8.2} → {:>8.2} ms          ({:+.1}%) {}",
            self.baseline.e2e_p50_ms,
            self.current.e2e_p50_ms,
            self.changes.e2e_p50_pct,
            arrow(self.changes.e2e_p50_pct, false)
        );
        println!(
            "   E2E P99:       {:>8.2} → {:>8.2} ms          ({:+.1}%) {}",
            self.baseline.e2e_p99_ms,
            self.current.e2e_p99_ms,
            self.changes.e2e_p99_pct,
            arrow(self.changes.e2e_p99_pct, false)
        );
        println!(
            "   S2S P99:       {:>8.2} → {:>8.2} ms          ({:+.1}%) {}",
            self.baseline.s2s_p99_ms,
            self.current.s2s_p99_ms,
            self.changes.s2s_p99_pct,
            arrow(self.changes.s2s_p99_pct, false)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_environment_detection() {
        let env = EnvironmentInfo::detect();
        assert!(!env.moniker.is_empty());
        assert!(env.cpu_cores > 0);
        assert!(env.memory_gb > 0.0);
    }

    #[test]
    fn test_moniker_generation() {
        // Apple Silicon
        assert!(EnvironmentInfo::generate_moniker("Apple M4 Pro", 12, 48.0).contains("M4-Pro"));

        // Intel
        assert!(EnvironmentInfo::generate_moniker(
            "Intel(R) Core(TM) i9-12900K @ 3.20GHz",
            24,
            64.0
        )
        .contains("i9-12900K"));

        // AMD
        assert!(
            EnvironmentInfo::generate_moniker("AMD Ryzen 9 5950X", 32, 128.0).contains("R9-5950X")
        );
    }

    #[test]
    fn test_checkpoint_filter() {
        let filter = CheckpointFilter {
            benchmark_name: Some("throughput".to_string()),
            moniker: Some("local".to_string()),
            ..Default::default()
        };

        let env = EnvironmentInfo {
            moniker: "local-M4-48GB".to_string(),
            os: "macOS".to_string(),
            cpu_name: "Apple M4".to_string(),
            cpu_cores: 12,
            memory_gb: 48.0,
            hostname: None,
        };

        let metrics = MetricsSnapshot {
            name: "throughput_test".to_string(),
            elapsed_ms: 1000,
            schedule_to_start: super::super::metrics::LatencyHistogramSnapshot {
                samples_micros: vec![],
                sum_micros: 0,
                count: 0,
                min_micros: u64::MAX,
                max_micros: 0,
            },
            execution: super::super::metrics::LatencyHistogramSnapshot {
                samples_micros: vec![],
                sum_micros: 0,
                count: 0,
                min_micros: u64::MAX,
                max_micros: 0,
            },
            end_to_end: super::super::metrics::LatencyHistogramSnapshot {
                samples_micros: vec![],
                sum_micros: 0,
                count: 0,
                min_micros: u64::MAX,
                max_micros: 0,
            },
            tasks_completed: super::super::metrics::ThroughputSnapshot {
                total: 1000,
                elapsed_ms: 1000,
                timeseries: vec![],
            },
            tasks_enqueued: super::super::metrics::ThroughputSnapshot {
                total: 1000,
                elapsed_ms: 1000,
                timeseries: vec![],
            },
            resources: super::super::metrics::ResourceMonitorSnapshot {
                snapshots: vec![],
                elapsed_ms: 1000,
            },
            custom: std::collections::HashMap::new(),
        };

        let checkpoint = BenchmarkCheckpoint::new("throughput_test", env, metrics);
        assert!(filter.matches(&checkpoint));
    }
}
