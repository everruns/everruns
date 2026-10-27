//! System doctor: one plain-language report on whether this install is set
//! up right and working. See `knowledge/operations/system-doctor.md`.
//!
//! Decisions:
//! - The doctor discovers the install from its configuration and live state.
//!   There is no "hosting mode" setting: a check that does not apply reports
//!   `info` with what that means, never a failure.
//! - Strictness follows the deployment grade. A missing worker token is a
//!   problem in `prod` and a warning on a laptop.
//! - A report is cached for [`REPORT_TTL`] and a refresh is honored at most
//!   every [`MIN_REFRESH`], because some checks touch S3, NATS and Valkey and
//!   a dashboard must not hammer them.
//! - Secrets never enter a report: checks read lengths and presence, and
//!   connection strings are never echoed.

mod checks;
mod env;
pub mod instances;
pub mod preflight;
pub mod types;

#[cfg(test)]
mod tests;

pub(crate) use checks::readiness;
pub use env::DoctorEnv;
pub use types::{CheckArea, CheckStatus, DoctorCheck, DoctorReport};

use crate::storage::StorageBackend;
use crate::storage::encryption::EncryptionService;
use chrono::{DateTime, Utc};
use everruns_durable::WorkflowEventStore;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// How long a report is served before the next request runs the checks again.
const REPORT_TTL: Duration = Duration::from_secs(60);

/// The shortest gap between two refreshes, whatever callers ask for.
const MIN_REFRESH: Duration = Duration::from_secs(5);

/// Upper bound for one check that leaves the process (a network probe).
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Unix seconds of the last `/metrics` read on this process; 0 = never.
static LAST_METRICS_SCRAPE: AtomicI64 = AtomicI64::new(0);

/// Record that something read `/metrics`, which is how the doctor tells
/// "metrics are exposed" from "something is actually watching".
pub fn note_metrics_scrape() {
    LAST_METRICS_SCRAPE.store(Utc::now().timestamp(), Ordering::Relaxed);
}

pub(crate) fn last_metrics_scrape() -> Option<DateTime<Utc>> {
    match LAST_METRICS_SCRAPE.load(Ordering::Relaxed) {
        0 => None,
        at => DateTime::from_timestamp(at, 0),
    }
}

/// How often each server refreshes its doctor gauges.
const METRICS_SWEEP: Duration = Duration::from_secs(60);

/// A check's status as a gauge value: what an alert compares against.
pub(crate) fn status_gauge(status: CheckStatus) -> f64 {
    match status {
        CheckStatus::Ok | CheckStatus::Info => 0.0,
        CheckStatus::Warning => 1.0,
        CheckStatus::Problem => 2.0,
    }
}

/// Export the report as Prometheus gauges every [`METRICS_SWEEP`], so
/// existing alerting covers the doctor with no new alert channel.
///
/// Decision: every replica sweeps, like the other DB-derived gauges, rather
/// than one replica under a cluster lock. A gauge set only by the current
/// lock holder would keep its last value on the replicas that lost the lock.
/// The report cache keeps it to one run per replica per minute.
pub fn spawn_metrics_sweep(doctor: Arc<SystemDoctor>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(METRICS_SWEEP);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let report = doctor.report(false).await;
            for check in &report.checks {
                metrics::gauge!(
                    crate::metrics_names::DOCTOR_CHECK_STATUS,
                    "area" => area_label(check.area),
                    "check" => check.id.clone(),
                )
                .set(status_gauge(check.status));
            }
            metrics::gauge!(crate::metrics_names::DOCTOR_STATUS).set(status_gauge(report.status));
        }
    })
}

fn area_label(area: CheckArea) -> &'static str {
    match area {
        CheckArea::ControlPlane => "control_plane",
        CheckArea::Database => "database",
        CheckArea::Workers => "workers",
        CheckArea::Security => "security",
        CheckArea::Messaging => "messaging",
        CheckArea::Storage => "storage",
        CheckArea::Observability => "observability",
        CheckArea::Models => "models",
    }
}

/// Runs the checks against the live services of this server.
pub struct SystemDoctor {
    pub(crate) db: Arc<StorageBackend>,
    pub(crate) workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    pub(crate) encryption: Option<Arc<EncryptionService>>,
    pub(crate) env: DoctorEnv,
    /// Run from the command line rather than inside a server: the replica
    /// checks must not count this process as a server.
    pub(crate) preflight: bool,
    started: Instant,
    cache: Mutex<Option<(Instant, DoctorReport)>>,
}

impl SystemDoctor {
    pub fn new(
        db: Arc<StorageBackend>,
        workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
        encryption: Option<Arc<EncryptionService>>,
        env: DoctorEnv,
    ) -> Self {
        Self {
            db,
            workflow_store,
            encryption,
            env,
            preflight: false,
            started: Instant::now(),
            cache: Mutex::new(None),
        }
    }

    /// Mark this doctor as a command-line run (see `preflight`).
    pub fn preflight(mut self) -> Self {
        self.preflight = true;
        self
    }

    /// How long this server has been running.
    pub(crate) fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    /// The current report: cached unless `refresh` asks for a fresh run.
    pub async fn report(&self, refresh: bool) -> DoctorReport {
        let mut cache = self.cache.lock().await;
        if let Some((at, report)) = cache.as_ref() {
            let age = at.elapsed();
            if age < MIN_REFRESH || (!refresh && age < REPORT_TTL) {
                return report.clone();
            }
        }
        let report = self.run().await;
        *cache = Some((Instant::now(), report.clone()));
        report
    }

    /// Run every check once.
    pub async fn run(&self) -> DoctorReport {
        let findings = checks::run_all(self).await;
        let mut checks = findings.checks;
        // Problems, then warnings, then healthy checks, then "not used" notes;
        // by area within each, keeping each area's own order.
        checks.sort_by_key(|check| (check.status.display_rank(), check.area));
        // `info` describes the install; it never makes the verdict worse.
        let status = checks
            .iter()
            .map(|check| check.status)
            .filter(|status| *status > CheckStatus::Info)
            .max()
            .unwrap_or(CheckStatus::Ok);
        DoctorReport {
            status,
            checked_at: Utc::now(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            grade: self.env.grade_name(),
            auth_mode: self.env.auth_mode_name(),
            setup: findings.setup,
            checks,
        }
    }
}
