//! The checks, one module per area. Each area returns its checks and the
//! facts it learned about the install's shape.

pub(super) mod control_plane;
pub(super) mod database;
pub(super) mod messaging;
pub(super) mod models;
pub(super) mod observability;
pub(crate) mod replicas;
pub(super) mod security;
pub(super) mod storage;
pub(super) mod worker_reports;
pub(super) mod workers;

pub(crate) use database::readiness;

use super::{SystemDoctor, extension};

pub(super) type Findings = super::DoctorFindings;

/// Run every area concurrently; the slowest network probe bounds the run.
pub(super) async fn run_all(doctor: &SystemDoctor) -> Findings {
    // One quick read shared by the checks that compare server replicas.
    let replicas = replicas::load(doctor).await;
    let (control_plane, database, workers, security, messaging, storage, observability, models) = tokio::join!(
        control_plane::run(doctor),
        database::run(doctor),
        workers::run(doctor),
        security::run(doctor),
        messaging::run(doctor, &replicas),
        storage::run(doctor),
        observability::run(doctor),
        models::run(doctor),
    );
    let extensions = futures::future::join_all(
        doctor
            .extensions
            .iter()
            .map(|extension| extension::run_bounded(extension.as_ref())),
    );
    let extensions = extensions.await;
    let mut all = Findings::default();
    all.extend(replicas::run(doctor, &replicas));
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
    for found in extensions {
        all.extend(found);
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
