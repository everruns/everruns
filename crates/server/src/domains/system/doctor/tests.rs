use super::checks::database::{MigrationState, judge_migrations};
use super::checks::security::stored_secrets_check;
use super::checks::workers::capacity_check;
use super::*;
use crate::auth::config::AuthMode;
use crate::storage::encryption::EncryptionService;
use everruns_core::DeploymentGrade;
use everruns_durable::WorkerInfo;
use std::collections::BTreeMap;

fn env(grade: DeploymentGrade) -> DoctorEnv {
    DoctorEnv {
        grade,
        auth_mode: AuthMode::Full,
        base_url: "https://app.example.com/api".to_string(),
        jwt_secret_len: 64,
        worker_token_len: Some(64),
        dev_mode: false,
        utility_llm_configured: true,
        nats_url: None,
        valkey_url: None,
        blob_backend: "db".to_string(),
        s3: None,
        metrics_enabled: false,
        metrics_exposed: false,
        otlp_endpoint: false,
        otel_disabled: false,
        rust_log: None,
        provider_keys: vec!["OpenAI"],
    }
}

fn doctor(env: DoctorEnv) -> SystemDoctor {
    SystemDoctor::new(Arc::new(StorageBackend::test_database()), None, None, env)
}

fn check<'a>(report: &'a DoctorReport, id: &str) -> &'a DoctorCheck {
    report
        .checks
        .iter()
        .find(|check| check.id == id)
        .unwrap_or_else(|| panic!("no check {id} in {:#?}", report.checks))
}

#[test]
fn migrations_current_when_every_expected_version_applied() {
    let applied = [(20261011000000, true), (20261020000000, true)];
    assert_eq!(
        judge_migrations(&applied, &[20261011000000, 20261020000000]),
        MigrationState::Current(2)
    );
}

#[test]
fn migrations_report_missing_failed_newer_and_legacy() {
    let expected = [20261011000000, 20261020000000];
    assert_eq!(
        judge_migrations(&[(20261011000000, true)], &expected),
        MigrationState::Missing(vec![20261020000000])
    );
    assert_eq!(
        judge_migrations(
            &[(20261011000000, true), (20261020000000, false)],
            &expected
        ),
        MigrationState::Failed(vec![20261020000000])
    );
    assert_eq!(
        judge_migrations(
            &[
                (20261011000000, true),
                (20261020000000, true),
                (20261101000000, true)
            ],
            &expected
        ),
        MigrationState::Newer(vec![20261101000000])
    );
    assert_eq!(
        judge_migrations(&[(1, true), (222, true)], &expected),
        MigrationState::Legacy
    );
}

fn worker(id: &str, max: u32, load: u32, accepting: bool) -> WorkerInfo {
    let mut worker = WorkerInfo::new(id, ["actor"]);
    worker.max_concurrency = max;
    worker.current_load = load;
    worker.accepting_tasks = accepting;
    if !accepting {
        worker.backpressure_reason = Some("memory".to_string());
    }
    worker
}

#[test]
fn capacity_is_a_problem_only_when_full_with_work_waiting() {
    let full = [worker("w1", 10, 10, true)];
    assert_eq!(capacity_check(&full, Some(3)).status, CheckStatus::Problem);
    assert_eq!(capacity_check(&full, Some(0)).status, CheckStatus::Warning);
    let roomy = [worker("w1", 10, 2, true)];
    assert_eq!(capacity_check(&roomy, Some(0)).status, CheckStatus::Ok);
}

#[test]
fn capacity_names_workers_that_refuse_work() {
    let one_refusing = [worker("w1", 10, 0, true), worker("w2", 10, 0, false)];
    let check = capacity_check(&one_refusing, Some(0));
    assert_eq!(check.status, CheckStatus::Warning);
    assert!(check.summary.contains("w2 (memory)"), "{}", check.summary);
    let all_refusing = [worker("w1", 10, 0, false)];
    assert_eq!(
        capacity_check(&all_refusing, Some(0)).status,
        CheckStatus::Problem
    );
}

fn encryption() -> EncryptionService {
    // Current key: 32 bytes of 0x01; previous key: 32 zero bytes.
    EncryptionService::new(
        "kek-v2:AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",
        &["kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="],
    )
    .expect("valid keys")
}

#[test]
fn stored_secrets_flag_lost_keys_before_unfinished_rotation() {
    let encryption = encryption();
    let census = |pairs: &[(&str, i64)]| -> BTreeMap<String, i64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    };

    let ok = stored_secrets_check(&encryption, &census(&[("kek-v2", 4)]), Some(true));
    assert_eq!(ok.status, CheckStatus::Ok);

    let rotating = stored_secrets_check(
        &encryption,
        &census(&[("kek-v2", 4), ("kek-v1", 1)]),
        Some(true),
    );
    assert_eq!(rotating.status, CheckStatus::Warning);

    let lost = stored_secrets_check(
        &encryption,
        &census(&[("kek-v1", 1), ("kek-v0", 2)]),
        Some(true),
    );
    assert_eq!(lost.status, CheckStatus::Problem);
    assert!(lost.summary.contains("kek-v0"), "{}", lost.summary);

    let changed = stored_secrets_check(&encryption, &census(&[("kek-v2", 1)]), Some(false));
    assert_eq!(changed.status, CheckStatus::Problem);
}

#[tokio::test]
async fn healthy_dev_install_has_no_problems_and_reports_its_shape() {
    let report = doctor(env(DeploymentGrade::Dev)).run().await;
    assert_eq!(check(&report, "database.reachable").status, CheckStatus::Ok);
    assert_eq!(
        check(&report, "database.migrations").status,
        CheckStatus::Ok
    );
    assert!(
        report
            .setup
            .iter()
            .any(|fact| fact.starts_with("PostgreSQL ")),
        "{:?}",
        report.setup
    );
    assert!(
        report
            .checks
            .iter()
            .all(|c| c.status != CheckStatus::Problem),
        "{:#?}",
        report.checks
    );
    // Not-used parts are notes, and notes never make the verdict worse.
    assert_eq!(check(&report, "messaging.nats").status, CheckStatus::Info);
    assert_ne!(report.status, CheckStatus::Info);
}

#[tokio::test]
async fn production_without_safety_settings_is_a_problem() {
    let mut env = env(DeploymentGrade::Prod);
    env.auth_mode = AuthMode::None;
    env.worker_token_len = None;
    let report = doctor(env).run().await;
    assert_eq!(report.status, CheckStatus::Problem);
    for id in [
        "security.sign_in",
        "workers.authentication",
        "security.encryption_key",
    ] {
        assert_eq!(check(&report, id).status, CheckStatus::Problem, "{id}");
    }
    // Problems come first.
    assert_eq!(report.checks[0].status, CheckStatus::Problem);
}

#[tokio::test]
async fn unreachable_nats_is_a_problem_and_never_echoes_its_credentials() {
    let mut env = env(DeploymentGrade::Dev);
    // Port 9 (discard) refuses connections on a test machine.
    env.nats_url = Some("nats://agent:s3cret@127.0.0.1:9".to_string());
    let report = doctor(env).run().await;
    let nats = check(&report, "messaging.nats");
    assert_eq!(nats.status, CheckStatus::Problem);
    let text = serde_json::to_string(&report).unwrap();
    assert!(!text.contains("s3cret"), "{text}");
}

#[tokio::test]
async fn reports_are_cached_until_refreshed() {
    let doctor = doctor(env(DeploymentGrade::Dev));
    let first = doctor.report(false).await;
    let cached = doctor.report(false).await;
    assert_eq!(first.checked_at, cached.checked_at);
    // A refresh inside the minimum gap still shares the last run.
    let refreshed = doctor.report(true).await;
    assert_eq!(first.checked_at, refreshed.checked_at);
}

#[tokio::test]
async fn readiness_requires_an_applied_schema() {
    let db = StorageBackend::test_database();
    assert_eq!(readiness(db.pool()).await, Ok(()));
    sqlx::query(
        "DELETE FROM _sqlx_migrations WHERE version = (SELECT max(version) FROM _sqlx_migrations)",
    )
    .execute(db.pool())
    .await
    .unwrap();
    assert_eq!(readiness(db.pool()).await, Err("migrations not applied"));
}
