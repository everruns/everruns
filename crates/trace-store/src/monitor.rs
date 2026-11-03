//! Private, low-cardinality measurement surface. Never serialize driver errors,
//! tenant/session identifiers, object paths, payloads or credentials into metrics.
use crate::{ShadowOutcome, ShadowReport};
use axum::{
    Router,
    extract::State,
    http::{StatusCode, header},
    routing::get,
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const BOUNDS: [f64; 10] = [0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 15.0, 60.0];
const KINDS: [&str; 4] = [
    "postgres_summary",
    "datafusion_summary",
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
    buckets: [u64; 10],
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
        out.push_str("# TYPE everruns_trace_store_query_seconds histogram\n");
        for (kind, h) in KINDS.iter().zip(&m.latencies) {
            for (bound, count) in BOUNDS.iter().zip(h.buckets) {
                out.push_str(&format!("everruns_trace_store_query_seconds_bucket{{kind=\"{kind}\",le=\"{bound}\"}} {count}\n"));
            }
            out.push_str(&format!("everruns_trace_store_query_seconds_bucket{{kind=\"{kind}\",le=\"+Inf\"}} {}\neverruns_trace_store_query_seconds_count{{kind=\"{kind}\"}} {}\neverruns_trace_store_query_seconds_sum{{kind=\"{kind}\"}} {}\n",h.count,h.count,h.sum));
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
