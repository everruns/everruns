//! Private, low-cardinality measurement surface. Never serialize driver errors,
//! tenant/session identifiers, object paths, payloads or credentials into metrics.
use crate::{CacheStats, ReadProfile, ShadowOutcome, ShadowReport};
use axum::{
    Router,
    extract::State,
    http::{StatusCode, header},
    routing::get,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const BOUNDS: [f64; 20] = [
    0.00025, 0.0005, 0.001, 0.0025, 0.005, 0.0075, 0.01, 0.015, 0.02, 0.025, 0.05, 0.075, 0.1,
    0.15, 0.2, 0.5, 1.0, 5.0, 15.0, 60.0,
];
const KINDS: [&str; 4] = [
    "postgres_summary",
    "trace_summary",
    "postgres_payload",
    "object_payload",
];
const OUTCOMES: [ShadowOutcome; 6] = [
    ShadowOutcome::Match,
    ShadowOutcome::Mismatch,
    ShadowOutcome::Lagging,
    ShadowOutcome::Skipped,
    ShadowOutcome::Error,
    ShadowOutcome::MissingSource,
];
#[derive(Default)]
struct Histogram {
    buckets: [u64; 20],
    count: u64,
    sum: f64,
}
impl Histogram {
    fn observe(&mut self, value: f64) {
        if value <= 0.0 || !value.is_finite() {
            return;
        }
        self.count += 1;
        self.sum += value;
        for (i, bound) in BOUNDS.iter().enumerate() {
            if value <= *bound {
                self.buckets[i] += 1;
            }
        }
    }
}
#[derive(Default)]
struct Measurements {
    cycles: u64,
    errors: u64,
    copied: u64,
    outcomes: [u64; 6],
    latencies: [Histogram; 4],
    last_cycle: Option<Instant>,
    healthy: bool,
    pending: i64,
    lag: i64,
    index_bytes: i64,
    payload_bytes: i64,
    backfill_pending: i64,
    backfill_failed: i64,
    unhealthy_checks: i64,
    reads: BTreeMap<(&'static str, &'static str, &'static str), Histogram>,
    stages: BTreeMap<(&'static str, &'static str, &'static str), Histogram>,
    cache: CacheStats,
    cache_hits: u64,
    cache_misses: u64,
    index_bytes_read: u64,
}
#[derive(Clone, Default)]
pub struct Monitor {
    inner: Arc<Mutex<Measurements>>,
}
impl Monitor {
    pub fn record(&self, cycle: &crate::companion::CycleReport) {
        let Ok(mut m) = self.inner.lock() else {
            return;
        };
        m.cycles += 1;
        m.errors += cycle.errors as u64;
        m.copied += cycle.exported as u64;
        m.last_cycle = Some(Instant::now());
        m.healthy = cycle.errors == 0 && cycle.backfill_failed == 0 && cycle.unhealthy_checks == 0;
        m.pending = cycle.pending_sessions;
        m.lag = cycle.lag_sequences;
        m.index_bytes = cycle.index_bytes;
        m.payload_bytes = cycle.payload_bytes;
        m.backfill_pending = cycle.backfill_pending;
        m.backfill_failed = cycle.backfill_failed;
        m.unhealthy_checks = cycle.unhealthy_checks;
        m.cache = cycle.cache;
        for report in &cycle.comparisons {
            Self::observe(&mut m, report);
        }
    }
    fn observe(m: &mut Measurements, report: &ShadowReport) {
        if let Some(i) = OUTCOMES.iter().position(|s| *s == report.outcome) {
            m.outcomes[i] += 1;
        }
        if matches!(
            report.outcome,
            ShadowOutcome::Mismatch | ShadowOutcome::Error | ShadowOutcome::MissingSource
        ) {
            m.healthy = false;
        }
        for (h, t) in m.latencies.iter_mut().zip([
            report.source_seconds,
            report.trace_seconds,
            report.source_payload_seconds,
            report.trace_payload_seconds,
        ]) {
            h.observe(t);
        }
        for (kind, profile) in [
            ("summary", &report.trace_profile),
            ("summary", &report.warm_trace_profile),
            ("summary", &report.datafusion_profile),
            ("payload", &report.payload_profile),
        ] {
            if let Some(profile) = profile {
                Self::profile(m, kind, profile);
            }
        }
    }
    fn profile(m: &mut Measurements, kind: &'static str, profile: &ReadProfile) {
        m.reads
            .entry((kind, profile.engine.label(), profile.cache_state()))
            .or_default()
            .observe(profile.total_seconds);
        m.cache_hits += profile.cache_hits as u64;
        m.cache_misses += profile.cache_misses as u64;
        m.index_bytes_read += profile.index_bytes_read;
        for (stage, seconds) in [
            ("catalog", profile.catalog_seconds),
            ("index_io", profile.index_io_seconds),
            ("index_decode", profile.index_decode_seconds),
            ("planning", profile.planning_seconds),
            ("execution", profile.execution_seconds),
            ("payload_io", profile.payload_io_seconds),
            ("payload_decode", profile.payload_decode_seconds),
        ] {
            // Unexecuted stages never become artificial zero-latency samples.
            if seconds > 0.0 {
                m.stages
                    .entry((kind, profile.engine.label(), stage))
                    .or_default()
                    .observe(seconds);
            }
        }
    }
    pub fn ready(&self, max_age: Duration) -> bool {
        self.inner
            .lock()
            .is_ok_and(|m| m.healthy && m.last_cycle.is_some_and(|t| t.elapsed() <= max_age))
    }
    pub fn render(&self) -> String {
        let Ok(m) = self.inner.lock() else {
            return String::new();
        };
        let mut out = String::new();
        let mut scalar = |name: &str, kind: &str, value: String| {
            out.push_str(&format!(
                "# TYPE everruns_trace_store_{name} {kind}\neverruns_trace_store_{name} {value}\n"
            ));
        };
        scalar("cycles_total", "counter", m.cycles.to_string());
        scalar("cycle_errors_total", "counter", m.errors.to_string());
        scalar("events_exported_total", "counter", m.copied.to_string());
        scalar("pending_sessions", "gauge", m.pending.to_string());
        scalar("lag_sequences", "gauge", m.lag.to_string());
        scalar("unhealthy_checks", "gauge", m.unhealthy_checks.to_string());
        scalar("index_bytes", "gauge", m.index_bytes.to_string());
        scalar("payload_bytes", "gauge", m.payload_bytes.to_string());
        scalar("cache_bytes", "gauge", m.cache.bytes.to_string());
        scalar("cache_entries", "gauge", m.cache.entries.to_string());
        scalar(
            "cache_evictions_total",
            "counter",
            m.cache.evictions.to_string(),
        );
        scalar("cache_hits_total", "counter", m.cache_hits.to_string());
        scalar("cache_misses_total", "counter", m.cache_misses.to_string());
        scalar(
            "index_bytes_read_total",
            "counter",
            m.index_bytes_read.to_string(),
        );
        // The optional companion runs on Linux; report observed RSS rather
        // than treating a configured memory limit as a measurement.
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for (field, metric) in [
                ("VmRSS:", "process_resident_memory_bytes"),
                ("VmHWM:", "process_peak_resident_memory_bytes"),
            ] {
                if let Some(kib) = status.lines().find_map(|line| {
                    line.strip_prefix(field)
                        .and_then(|value| value.split_whitespace().next())
                        .and_then(|value| value.parse::<u64>().ok())
                }) {
                    scalar(metric, "gauge", kib.saturating_mul(1024).to_string());
                }
            }
        }
        #[cfg(target_os = "linux")]
        if let Ok(stat) = std::fs::read_to_string("/proc/self/stat") {
            // /proc/PID/stat aggregates the process's threads. Field 2 may
            // contain spaces/parentheses; fields after its final ')' start at 3.
            if let Some((_, fields)) = stat.rsplit_once(')') {
                let mut fields = fields.split_whitespace().skip(11);
                let user = fields.next().and_then(|v| v.parse::<u64>().ok());
                let system = fields.next().and_then(|v| v.parse::<u64>().ok());
                let ticks = rustix::param::clock_ticks_per_second();
                if let Some(total) = user.zip(system).and_then(|(u, s)| u.checked_add(s))
                    && ticks > 0
                {
                    scalar(
                        "process_cpu_seconds_total",
                        "counter",
                        (total as f64 / ticks as f64).to_string(),
                    );
                }
            }
        }
        scalar(
            "backfill_pending_sessions",
            "gauge",
            m.backfill_pending.to_string(),
        );
        scalar(
            "backfill_failed_sessions",
            "gauge",
            m.backfill_failed.to_string(),
        );
        scalar(
            "last_cycle_age_seconds",
            "gauge",
            m.last_cycle
                .map_or(-1.0, |t| t.elapsed().as_secs_f64())
                .to_string(),
        );
        out.push_str("# TYPE everruns_trace_store_comparisons_total counter\n");
        for (outcome, count) in OUTCOMES.iter().zip(m.outcomes) {
            out.push_str(&format!(
                "everruns_trace_store_comparisons_total{{outcome=\"{}\"}} {count}\n",
                outcome.label()
            ));
        }
        for (kind, h) in KINDS.iter().zip(&m.latencies) {
            histogram(&mut out, "query_seconds", &format!("kind=\"{kind}\""), h);
        }
        for ((kind, engine, cache), h) in &m.reads {
            histogram(
                &mut out,
                "read_seconds",
                &format!("kind=\"{kind}\",engine=\"{engine}\",cache=\"{cache}\""),
                h,
            );
        }
        for ((kind, engine, stage), h) in &m.stages {
            histogram(
                &mut out,
                "stage_seconds",
                &format!("kind=\"{kind}\",engine=\"{engine}\",stage=\"{stage}\""),
                h,
            );
        }
        out
    }
    pub fn router(&self, max_age: Duration) -> Router {
        Router::new()
            .route(
                "/metrics",
                get(
                    |State((monitor, _)): State<(Monitor, Duration)>| async move {
                        (
                            [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
                            monitor.render(),
                        )
                    },
                ),
            )
            .route("/healthz", get(|| async { "alive" }))
            .route(
                "/readyz",
                get(
                    |State((monitor, age)): State<(Monitor, Duration)>| async move {
                        if monitor.ready(age) {
                            (StatusCode::OK, "ready")
                        } else {
                            (StatusCode::SERVICE_UNAVAILABLE, "not ready")
                        }
                    },
                ),
            )
            .with_state((self.clone(), max_age))
    }
}

fn histogram(out: &mut String, name: &str, labels: &str, h: &Histogram) {
    // One TYPE declaration per metric family, even when it has many series.
    let declaration = format!("# TYPE everruns_trace_store_{name} histogram\n");
    if !out.contains(&declaration) {
        out.push_str(&declaration);
    }
    for (bound, count) in BOUNDS.iter().zip(h.buckets) {
        out.push_str(&format!(
            "everruns_trace_store_{name}_bucket{{{labels},le=\"{bound}\"}} {count}\n"
        ));
    }
    out.push_str(&format!("everruns_trace_store_{name}_bucket{{{labels},le=\"+Inf\"}} {}\neverruns_trace_store_{name}_count{{{labels}}} {}\neverruns_trace_store_{name}_sum{{{labels}}} {}\n", h.count, h.count, h.sum));
}
