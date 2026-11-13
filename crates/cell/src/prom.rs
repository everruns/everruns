//! Prometheus text exposition (`GET /metrics`), scraped by VictoriaMetrics
//! or Prometheus.
//!
//! Decision: hand-rolled rather than a metrics crate: a few counter and
//! histogram families with string labels are all the runtime needs, and the
//! output stays easy to read in a terminal.
//!
//! Two kinds of series:
//! - per node (`cell`, `node` labels): what this process did, such as turns
//!   it executed and events it appended. Summing across nodes gives the
//!   fleet total.
//! - per cell (`cell` label only), emitted by the cell's leader alone: the
//!   replicated state (sessions, running turns, completed turns). Every
//!   replica holds the same numbers, so only one reports them, and a leader
//!   change continues the same series.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// Upper bounds in seconds; spans a commit (ms) to a long turn (minutes).
/// 1.5 and 2 are there for the shipping lag, whose bar is 2 s.
const BUCKETS: &[f64] = &[
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 1.5, 2.0, 2.5, 5.0, 10.0, 30.0,
    60.0, 120.0, 300.0,
];

/// A counter family keyed by rendered labels (`agent="x",type="y"`).
#[derive(Default)]
pub struct Counters(Mutex<BTreeMap<String, u64>>);

impl Counters {
    pub fn inc(&self, labels: &[(&str, &str)]) {
        self.add(labels, 1);
    }

    pub fn add(&self, labels: &[(&str, &str)], n: u64) {
        *self.0.lock().entry(render(labels)).or_insert(0) += n;
    }

    /// The count for exactly `labels`.
    pub fn get(&self, labels: &[(&str, &str)]) -> u64 {
        self.0.lock().get(&render(labels)).copied().unwrap_or(0)
    }
}

/// A gauge family that moves up and down (running turns per agent).
#[derive(Default)]
pub struct Gauges(Mutex<BTreeMap<String, i64>>);

impl Gauges {
    pub fn add(&self, labels: &[(&str, &str)], n: i64) {
        *self.0.lock().entry(render(labels)).or_insert(0) += n;
    }
}

#[derive(Clone, Default)]
struct Buckets {
    counts: Vec<u64>,
    sum: f64,
    count: u64,
}

/// A histogram family with fixed buckets.
#[derive(Default)]
pub struct Histograms(Mutex<BTreeMap<String, Buckets>>);

impl Histograms {
    pub fn observe(&self, labels: &[(&str, &str)], elapsed: Duration) {
        let secs = elapsed.as_secs_f64();
        let mut map = self.0.lock();
        let b = map.entry(render(labels)).or_insert_with(|| Buckets {
            counts: vec![0; BUCKETS.len()],
            ..Default::default()
        });
        for (i, bound) in BUCKETS.iter().enumerate() {
            if secs <= *bound {
                b.counts[i] += 1;
            }
        }
        b.sum += secs;
        b.count += 1;
    }
}

/// The last minute of one SLO's samples, for its p99 gauge (`cell_slo_*`).
///
/// Decision: each node reports its own one-minute p99 as a gauge, beside
/// the cumulative histograms. A scraper that keeps only gauges (Vector into
/// CloudWatch, step 10's dashboard) charts the budget as is, and the worst
/// minute in a period is the gauge's maximum. Capped, so a burst costs
/// bounded memory: past the cap the oldest samples go first.
#[derive(Default)]
pub struct Window(Mutex<VecDeque<(Instant, f64)>>);

const WINDOW: Duration = Duration::from_secs(60);
const WINDOW_CAP: usize = 50_000;

impl Window {
    pub fn observe(&self, elapsed: Duration) {
        self.observe_at(Instant::now(), elapsed);
    }

    fn observe_at(&self, now: Instant, elapsed: Duration) {
        let mut samples = self.0.lock();
        samples.push_back((now, elapsed.as_secs_f64()));
        while samples.len() > WINDOW_CAP {
            samples.pop_front();
        }
        trim(&mut samples, now);
    }

    /// The p99 of the last minute's samples, in seconds; 0 with none.
    pub fn p99(&self) -> f64 {
        self.p99_at(Instant::now())
    }

    fn p99_at(&self, now: Instant) -> f64 {
        let mut samples = self.0.lock();
        trim(&mut samples, now);
        let mut values: Vec<f64> = samples.iter().map(|(_, v)| *v).collect();
        drop(samples);
        if values.is_empty() {
            return 0.0;
        }
        values.sort_by(f64::total_cmp);
        let rank = ((values.len() as f64) * 0.99).ceil() as usize;
        values[rank.clamp(1, values.len()) - 1]
    }
}

fn trim(samples: &mut VecDeque<(Instant, f64)>, now: Instant) {
    while samples
        .front()
        .is_some_and(|(at, _)| now.saturating_duration_since(*at) > WINDOW)
    {
        samples.pop_front();
    }
}

/// Everything a node counts for `/metrics`.
#[derive(Default)]
pub struct Prom {
    /// Turn executions on this node by agent and outcome.
    pub turns_executed: Counters,
    /// Wall time of each turn execution on this node.
    pub turn_seconds: Histograms,
    /// Turns running on this node now, by agent.
    pub turns_running: Gauges,
    /// Durable events this node's executors appended, by event type.
    pub events_appended: Counters,
    /// Executor append: propose, quorum commit, local apply.
    pub append_seconds: Histograms,
    /// Any proposal from this node, end to end.
    pub propose_seconds: Histograms,
    /// Writes shipped before they were answered (`ship::ships_first`):
    /// the wait for the object store.
    pub ship_first_seconds: Histograms,
    /// How long the oldest entry waited to be shipped, per shipping round
    /// (an upper bound, to the tick): the most a lost cell could lose of
    /// the log.
    pub ship_lag_seconds: Histograms,
    /// The last minute of each, for the SLO gauges.
    pub ship_first_window: Window,
    pub ship_lag_window: Window,
    /// Messages admitted through this node's gateway.
    pub messages_admitted: Counters,
    /// Model calls by model and result, with duration and time to first
    /// token (from `llm.generation` events).
    pub model_calls: Counters,
    pub model_seconds: Histograms,
    pub model_ttft_seconds: Histograms,
    /// Tool calls by tool and status, with duration (`tool.completed`).
    pub tool_calls: Counters,
    pub tool_seconds: Histograms,
}

impl Prom {
    /// Count what a durable event says about model and tool calls.
    pub fn observe_event(&self, event: &everruns_core::events::Event) {
        let kind = event.event_type.as_str();
        if kind != "llm.generation" && kind != "tool.completed" {
            return;
        }
        let data = serde_json::to_value(&event.data).unwrap_or_default();
        let ms = |v: &serde_json::Value| v.as_u64().map(Duration::from_millis);
        match kind {
            "llm.generation" => {
                let m = &data["metadata"];
                let model = m["model"].as_str().unwrap_or("unknown");
                let ok = if m["success"].as_bool().unwrap_or(true) {
                    "true"
                } else {
                    "false"
                };
                self.model_calls.inc(&[("model", model), ("success", ok)]);
                if let Some(d) = ms(&m["duration_ms"]) {
                    self.model_seconds.observe(&[("model", model)], d);
                }
                if let Some(d) = ms(&m["time_to_first_token_ms"]) {
                    self.model_ttft_seconds.observe(&[("model", model)], d);
                }
            }
            "tool.completed" => {
                let tool = data["tool_name"].as_str().unwrap_or("unknown");
                let status = data["status"].as_str().unwrap_or("unknown");
                self.tool_calls.inc(&[("tool", tool), ("status", status)]);
                if let Some(d) = ms(&data["duration_ms"]) {
                    self.tool_seconds.observe(&[("tool", tool)], d);
                }
            }
            _ => {}
        }
    }
}

pub fn render(labels: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (i, (k, v)) in labels.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let v = v
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        let _ = write!(out, "{k}=\"{v}\"");
    }
    out
}

fn join(base: &str, extra: &str) -> String {
    match (base.is_empty(), extra.is_empty()) {
        (true, _) => extra.to_string(),
        (_, true) => base.to_string(),
        _ => format!("{base},{extra}"),
    }
}

/// Writes families in the text format.
pub struct Writer {
    pub out: String,
    base: String,
}

impl Writer {
    /// `base` labels go on every series (`cell`, `node`).
    pub fn new(base: &[(&str, &str)]) -> Self {
        Self {
            out: String::new(),
            base: render(base),
        }
    }

    fn head(&mut self, name: &str, kind: &str, help: &str) {
        let _ = writeln!(self.out, "# HELP {name} {help}");
        let _ = writeln!(self.out, "# TYPE {name} {kind}");
    }

    /// One sample with explicit labels (no base labels).
    pub fn sample_raw(&mut self, name: &str, kind: &str, help: &str, labels: &str, value: f64) {
        self.head(name, kind, help);
        let _ = writeln!(self.out, "{name}{{{labels}}} {value}");
    }

    pub fn gauge(&mut self, name: &str, help: &str, value: f64) {
        self.head(name, "gauge", help);
        let _ = writeln!(self.out, "{name}{{{}}} {value}", self.base);
    }

    pub fn counter(&mut self, name: &str, help: &str, value: f64) {
        self.head(name, "counter", help);
        let _ = writeln!(self.out, "{name}{{{}}} {value}", self.base);
    }

    pub fn counters(&mut self, name: &str, help: &str, family: &Counters) {
        self.head(name, "counter", help);
        for (labels, value) in family.0.lock().iter() {
            let _ = writeln!(self.out, "{name}{{{}}} {value}", join(&self.base, labels));
        }
    }

    pub fn gauges(&mut self, name: &str, help: &str, family: &Gauges) {
        self.head(name, "gauge", help);
        for (labels, value) in family.0.lock().iter() {
            let _ = writeln!(self.out, "{name}{{{}}} {value}", join(&self.base, labels));
        }
    }

    /// A labelled family computed on the spot, under explicit labels.
    pub fn family(
        &mut self,
        name: &str,
        kind: &str,
        help: &str,
        base: &str,
        rows: impl IntoIterator<Item = (String, f64)>,
    ) {
        self.head(name, kind, help);
        for (labels, value) in rows {
            let _ = writeln!(self.out, "{name}{{{}}} {value}", join(base, &labels));
        }
    }

    pub fn histograms(&mut self, name: &str, help: &str, family: &Histograms) {
        self.head(name, "histogram", help);
        for (labels, b) in family.0.lock().iter() {
            let labels = join(&self.base, labels);
            for (bound, count) in BUCKETS.iter().zip(&b.counts) {
                let _ = writeln!(
                    self.out,
                    "{name}_bucket{{{}}} {count}",
                    join(&labels, &format!("le=\"{bound}\""))
                );
            }
            let _ = writeln!(
                self.out,
                "{name}_bucket{{{}}} {}",
                join(&labels, "le=\"+Inf\""),
                b.count
            );
            let _ = writeln!(self.out, "{name}_sum{{{labels}}} {}", b.sum);
            let _ = writeln!(self.out, "{name}_count{{{labels}}} {}", b.count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_reports_the_last_minutes_p99() {
        let w = Window::default();
        let start = Instant::now();
        assert_eq!(w.p99_at(start), 0.0);
        for ms in 1..=100 {
            w.observe_at(start, Duration::from_millis(ms));
        }
        assert!(
            (w.p99_at(start) - 0.099).abs() < 1e-9,
            "{}",
            w.p99_at(start)
        );
        // One slow sample a minute later: the old ones age out.
        let later = start + Duration::from_secs(61);
        w.observe_at(later, Duration::from_secs(3));
        assert_eq!(w.p99_at(later), 3.0);
        assert_eq!(w.p99_at(later + Duration::from_secs(61)), 0.0);
    }

    #[test]
    fn histogram_buckets_are_cumulative() {
        let h = Histograms::default();
        h.observe(&[("agent", "a")], Duration::from_millis(3));
        h.observe(&[("agent", "a")], Duration::from_secs(2));
        let mut w = Writer::new(&[("cell", "0")]);
        w.histograms("t", "test", &h);
        assert!(
            w.out
                .contains("t_bucket{cell=\"0\",agent=\"a\",le=\"0.005\"} 1")
        );
        assert!(
            w.out
                .contains("t_bucket{cell=\"0\",agent=\"a\",le=\"2.5\"} 2")
        );
        assert!(w.out.contains("t_count{cell=\"0\",agent=\"a\"} 2"));
    }
}
