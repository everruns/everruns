//! Command-line options shared by the benchmark binaries.
//!
//! Every bench accepts the same flags:
//!
//! - `--save` writes a checkpoint per scenario to `benches/checkpoints/`.
//! - `--moniker <name>` labels the environment in checkpoints and summaries.
//! - `--smoke` runs each scenario at a tiny scale. CI runs this on every
//!   durable change so a broken bench fails a pull request instead of being
//!   found months later.
//! - `--summary <file>` appends one JSON line per scenario (see
//!   [`ScenarioSummary`]). The weekly bench workflow compares these lines with
//!   the committed baseline.
//!
//! Unknown arguments are ignored, because `cargo bench` passes its own
//! (`--bench`) to every binary.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use super::checkpoint::EnvironmentInfo;
use super::metrics::BenchmarkMetrics;

/// Parsed benchmark flags.
#[derive(Debug, Clone, Default)]
pub struct BenchOptions {
    pub save_checkpoint: bool,
    pub moniker: Option<String>,
    pub smoke: bool,
    pub summary: Option<PathBuf>,
}

impl BenchOptions {
    /// Parse the process arguments.
    pub fn from_args() -> Self {
        Self::parse(std::env::args().skip(1))
    }

    /// Parse an argument list (without the program name).
    ///
    /// ```
    /// use everruns_durable::bench::BenchOptions;
    ///
    /// let opts = BenchOptions::parse(["--bench", "--smoke", "--summary", "out.jsonl"]);
    /// assert!(opts.smoke);
    /// assert_eq!(opts.pick(10_000, 50), 50);
    /// assert_eq!(opts.summary.unwrap().to_str(), Some("out.jsonl"));
    /// ```
    pub fn parse<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut opts = Self::default();
        let mut args = args.into_iter().map(Into::into);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--save" => opts.save_checkpoint = true,
                "--smoke" => opts.smoke = true,
                "--moniker" => opts.moniker = args.next(),
                "--summary" => opts.summary = args.next().map(PathBuf::from),
                _ => {}
            }
        }
        opts
    }

    /// `full` normally, `smoke` under `--smoke`.
    pub fn pick<T>(&self, full: T, smoke: T) -> T {
        if self.smoke { smoke } else { full }
    }

    /// The environment to label checkpoints and summaries with.
    pub fn environment(&self) -> EnvironmentInfo {
        match &self.moniker {
            Some(moniker) => EnvironmentInfo::detect_with_moniker(moniker),
            None => EnvironmentInfo::detect(),
        }
    }

    /// Append one scenario's result to the `--summary` file, if one was given.
    pub fn record(&self, bench: &str, scenario: &str, metrics: &BenchmarkMetrics) {
        let Some(path) = &self.summary else {
            return;
        };
        let line = ScenarioSummary::from_metrics(bench, scenario, self, metrics);
        let written = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| {
                let json = serde_json::to_string(&line).map_err(std::io::Error::other)?;
                writeln!(file, "{json}")
            });
        if let Err(e) = written {
            eprintln!("failed to write summary to {}: {e}", path.display());
        }
    }
}

/// One scenario's headline numbers: the unit of the committed baseline.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScenarioSummary {
    pub bench: String,
    pub scenario: String,
    pub moniker: String,
    pub smoke: bool,
    pub tasks: u64,
    pub tasks_per_sec: f64,
    pub s2s_p50_ms: f64,
    pub s2s_p99_ms: f64,
    pub e2e_p50_ms: f64,
    pub e2e_p99_ms: f64,
}

impl ScenarioSummary {
    fn from_metrics(
        bench: &str,
        scenario: &str,
        opts: &BenchOptions,
        metrics: &BenchmarkMetrics,
    ) -> Self {
        let ms = |d: std::time::Duration| (d.as_secs_f64() * 1000.0 * 100.0).round() / 100.0;
        let s2s = metrics.schedule_to_start.summary();
        let e2e = metrics.end_to_end.summary();
        Self {
            bench: bench.to_string(),
            scenario: scenario.to_string(),
            moniker: opts.environment().moniker,
            smoke: opts.smoke,
            tasks: metrics.tasks_completed.total(),
            tasks_per_sec: metrics.tasks_completed.throughput().round(),
            s2s_p50_ms: ms(s2s.p50),
            s2s_p99_ms: ms(s2s.p99),
            e2e_p50_ms: ms(e2e.p50),
            e2e_p99_ms: ms(e2e.p99),
        }
    }
}
