//! Latency samples for the benchmark.

use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;

const CAP: usize = 200_000;

/// A bounded sample of durations, in microseconds.
#[derive(Default)]
pub struct Histogram {
    samples: Mutex<Vec<u32>>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, Default)]
pub struct Summary {
    pub count: usize,
    pub p50_us: u32,
    pub p90_us: u32,
    pub p99_us: u32,
    pub max_us: u32,
}

impl Histogram {
    pub fn record(&self, elapsed: Duration) {
        let mut samples = self.samples.lock();
        if samples.len() < CAP {
            samples.push(elapsed.as_micros().min(u32::MAX as u128) as u32);
        }
    }

    pub fn summary(&self) -> Summary {
        let mut samples = self.samples.lock().clone();
        if samples.is_empty() {
            return Summary::default();
        }
        samples.sort_unstable();
        let at = |q: f64| samples[((samples.len() as f64 - 1.0) * q) as usize];
        Summary {
            count: samples.len(),
            p50_us: at(0.5),
            p90_us: at(0.9),
            p99_us: at(0.99),
            max_us: samples.last().copied().unwrap_or_default(),
        }
    }

    pub fn reset(&self) {
        self.samples.lock().clear();
    }
}

/// Per-node latency samples.
#[derive(Default)]
pub struct Metrics {
    /// Executor append: propose, quorum commit, local apply.
    pub append: Histogram,
    /// Any proposal from this node, end to end.
    pub propose: Histogram,
    /// Events dropped from memory by unloading idle sessions.
    pub unloaded_events: std::sync::atomic::AtomicU64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct MetricsReport {
    pub append: Summary,
    pub propose: Summary,
    #[serde(default)]
    pub unloaded_events: u64,
}

impl Metrics {
    pub fn report(&self) -> MetricsReport {
        MetricsReport {
            append: self.append.summary(),
            propose: self.propose.summary(),
            unloaded_events: self
                .unloaded_events
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    pub fn reset(&self) {
        self.append.reset();
        self.propose.reset();
    }
}
