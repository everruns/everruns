//! The checks, one module per area. Each area returns its checks and the
//! facts it learned about the install's shape.

pub(super) mod control_plane;
pub(super) mod database;
pub(super) mod messaging;
pub(super) mod models;
pub(super) mod observability;
pub(super) mod security;
pub(super) mod storage;
pub(super) mod workers;

pub(crate) use database::readiness;

use super::{DoctorCheck, SystemDoctor};

#[derive(Default)]
pub(super) struct Findings {
    pub checks: Vec<DoctorCheck>,
    pub setup: Vec<String>,
}

impl Findings {
    fn check(&mut self, check: DoctorCheck) {
        self.checks.push(check);
    }

    fn fact(&mut self, fact: impl Into<String>) {
        self.setup.push(fact.into());
    }

    fn extend(&mut self, other: Findings) {
        self.checks.extend(other.checks);
        self.setup.extend(other.setup);
    }
}

/// Run every area concurrently; the slowest network probe bounds the run.
pub(super) async fn run_all(doctor: &SystemDoctor) -> Findings {
    let (control_plane, database, workers, security, messaging, storage, observability, models) = tokio::join!(
        control_plane::run(doctor),
        database::run(doctor),
        workers::run(doctor),
        security::run(doctor),
        messaging::run(doctor),
        storage::run(doctor),
        observability::run(doctor),
        models::run(doctor),
    );
    let mut all = Findings::default();
    for area in [
        database,
        workers,
        messaging,
        storage,
        control_plane,
        security,
        observability,
        models,
    ] {
        all.extend(area);
    }
    all
}

/// "1 worker", "3 workers".
fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}
