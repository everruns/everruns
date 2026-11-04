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
        email: Ok(Some("Resend")),
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

// --- what workers report about themselves ---------------------------------

use super::checks::worker_reports::{
    drain_check, rejected_logins_check, self_report_checks, versions_check,
};

fn reporting(id: &str, version: Option<&str>, protocol: Option<u32>) -> WorkerInfo {
    let mut worker = WorkerInfo::new(id, ["actor"]);
    worker.version = version.map(str::to_string);
    worker.metadata = protocol.map(|protocol| serde_json::json!({ "protocol_version": protocol }));
    worker
}

#[test]
fn versions_flag_skew_and_old_protocols() {
    let same = [reporting("w1", Some("1.0.0"), Some(1))];
    assert_eq!(versions_check(&same, "1.0.0").status, CheckStatus::Ok);

    let skew = [
        reporting("w1", Some("1.0.0"), Some(1)),
        reporting("w2", Some("0.9.0"), Some(1)),
        reporting("w3", None, None),
    ];
    let check = versions_check(&skew, "1.0.0");
    assert_eq!(check.status, CheckStatus::Warning);
    assert!(
        check.summary.contains("1 worker on 0.9.0"),
        "{}",
        check.summary
    );
    assert!(
        check.summary.contains("too old to say"),
        "{}",
        check.summary
    );

    let old = [reporting("w1", Some("1.0.0"), Some(0))];
    let check = versions_check(&old, "1.0.0");
    assert_eq!(check.status, CheckStatus::Problem);
    assert!(check.summary.contains("w1"));
}

#[test]
fn only_long_drains_are_flagged() {
    let now = chrono::Utc::now();
    let draining = |minutes: i64| {
        let mut worker = WorkerInfo::new("w1", ["actor"]);
        worker.status = "draining".to_string();
        let since = now - chrono::Duration::minutes(minutes);
        worker.metadata = Some(serde_json::json!({ "draining_since": since.to_rfc3339() }));
        worker
    };
    assert_eq!(drain_check(&[], now).status, CheckStatus::Ok);
    assert_eq!(drain_check(&[draining(2)], now).status, CheckStatus::Info);
    let stuck = drain_check(&[draining(40)], now);
    assert_eq!(stuck.status, CheckStatus::Warning);
    assert!(stuck.summary.contains("w1 (40 min)"), "{}", stuck.summary);
}

#[test]
fn recent_rejected_worker_logins_lock_workers_out() {
    let now = chrono::Utc::now();
    assert_eq!(
        rejected_logins_check(0, None, true, now).status,
        CheckStatus::Ok
    );
    let recent = rejected_logins_check(3, Some(now - chrono::Duration::minutes(1)), true, now);
    assert_eq!(recent.status, CheckStatus::Problem);
    assert!(recent.summary.contains("3 worker calls"));
    let dev = rejected_logins_check(3, Some(now), false, now);
    assert_eq!(dev.status, CheckStatus::Warning);
    let old = rejected_logins_check(3, Some(now - chrono::Duration::hours(2)), true, now);
    assert_eq!(old.status, CheckStatus::Info);
}

fn with_report(id: &str, checks: serde_json::Value, age_minutes: i64) -> WorkerInfo {
    let mut worker = WorkerInfo::new(id, ["actor"]);
    let at = chrono::Utc::now() - chrono::Duration::minutes(age_minutes);
    worker.metadata = Some(serde_json::json!({
        "self_checks": checks,
        "self_checked_at": at.to_rfc3339(),
    }));
    worker
}

#[test]
fn self_reports_merge_across_workers_and_name_the_broken_ones() {
    let now = chrono::Utc::now();
    let proxy = |status: &str| {
        serde_json::json!([{
            "id": "egress_proxy", "title": "Egress proxy from this worker",
            "status": status, "summary": format!("proxy {status}."),
            "fix": "Start the proxy.",
        }])
    };
    let workers = [
        with_report("w1", proxy("ok"), 1),
        with_report("w2", proxy("problem"), 1),
        // A stale report is ignored, whatever it says.
        with_report("w3", proxy("problem"), 60),
    ];
    let checks = self_report_checks(&workers, false, now);
    assert_eq!(checks.len(), 1);
    let check = &checks[0];
    assert_eq!(check.id, "workers.self.egress_proxy");
    assert_eq!(check.status, CheckStatus::Problem);
    assert!(
        check.summary.starts_with("1 of 2 workers"),
        "{}",
        check.summary
    );
    assert!(check.summary.contains("(w2)"), "{}", check.summary);
    assert_eq!(check.fix.as_deref(), Some("Start the proxy."));

    let healthy = self_report_checks(&[with_report("w1", proxy("ok"), 1)], false, now);
    assert_eq!(healthy[0].status, CheckStatus::Ok);
}

#[test]
fn nats_must_match_between_workers_and_server() {
    let now = chrono::Utc::now();
    let nats = |status: &str| serde_json::json!([{ "id": "nats", "status": status }]);
    let status =
        |workers: &[WorkerInfo], server: bool| self_report_checks(workers, server, now)[0].status;
    let with = [with_report("w1", nats("ok"), 1)];
    let without = [with_report("w1", nats("info"), 1)];
    assert_eq!(status(&with, true), CheckStatus::Ok);
    assert_eq!(status(&without, false), CheckStatus::Info);
    assert_eq!(status(&without, true), CheckStatus::Warning);
    assert_eq!(status(&with, false), CheckStatus::Problem);
}

#[tokio::test]
async fn the_report_reads_versions_and_drains_from_the_registry() {
    use everruns_durable::{InMemoryWorkflowEventStore, WorkerRegistry};
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    store
        .register_worker(reporting("new", Some(env!("CARGO_PKG_VERSION")), Some(1)))
        .await
        .unwrap();
    store
        .register_worker(reporting("old", Some("0.0.1"), Some(1)))
        .await
        .unwrap();
    store
        .register_worker(reporting(
            "leaving",
            Some(env!("CARGO_PKG_VERSION")),
            Some(1),
        ))
        .await
        .unwrap();
    store.drain_worker("leaving").await.unwrap();

    let doctor = SystemDoctor::new(
        Arc::new(StorageBackend::test_database()),
        Some(store),
        None,
        env(DeploymentGrade::Dev),
    );
    let report = doctor.run().await;
    let versions = check(&report, "workers.versions");
    assert_eq!(versions.status, CheckStatus::Warning);
    assert!(versions.summary.contains("0.0.1"), "{}", versions.summary);
    // The draining worker is not counted as capacity.
    assert!(
        check(&report, "workers.connected")
            .summary
            .starts_with("2 workers"),
        "{:#?}",
        report.checks
    );
    assert_eq!(check(&report, "workers.draining").status, CheckStatus::Info);
}

// --- server replicas -------------------------------------------------------

use super::checks::messaging::{disagreement, event_delivery, nats_check, valkey_check};
use super::checks::replicas::{Replicas, servers_check, versions_check as server_versions_check};
use super::instances::{Instance, InstanceDetails, InstanceRow, KIND_SERVER, recent, record};
use chrono::TimeDelta;

fn replica(id: &str, version: &str, started_ago: i64, seen_ago: i64) -> InstanceRow {
    let now = chrono::Utc::now();
    InstanceRow {
        id: id.to_string(),
        hostname: Some(format!("host-{id}")),
        version: version.to_string(),
        schema_version: Some(20261027072427),
        details: InstanceDetails::default(),
        started_at: now - TimeDelta::minutes(started_ago),
        last_seen_at: now - TimeDelta::seconds(seen_ago),
    }
}

fn replicas(rows: Vec<InstanceRow>) -> Replicas {
    Replicas::from_rows(chrono::Utc::now(), rows)
}

#[test]
fn stopped_servers_are_listed_and_a_missing_own_beat_is_flagged() {
    let seen = replicas(vec![
        replica("a", "1.0.0", 60, 5),
        replica("b", "1.0.0", 60, 600),
    ]);
    let check = servers_check(&seen, Some("a"), TimeDelta::minutes(5));
    assert_eq!(check.status, CheckStatus::Ok);
    assert!(
        check.summary.contains("One server is running."),
        "{}",
        check.summary
    );
    assert!(
        check
            .summary
            .contains("1 server stopped in the last hour: host-b"),
        "{}",
        check.summary
    );

    // This process just started: its first beat may not be in yet.
    assert_eq!(
        servers_check(&seen, Some("c"), TimeDelta::seconds(10)).status,
        CheckStatus::Ok
    );
    assert_eq!(
        servers_check(&seen, Some("c"), TimeDelta::minutes(5)).status,
        CheckStatus::Warning
    );
    assert_eq!(seen.live_count(Some("c")), 2);
    assert_eq!(seen.live_count(Some("a")), 1);
    // A command-line run never counts itself.
    assert_eq!(seen.live_count(None), 1);
    let preflight = servers_check(&seen, None, TimeDelta::zero());
    assert_eq!(preflight.summary, "One server is running: host-a (1.0.0).");
    let none = servers_check(&replicas(vec![]), None, TimeDelta::zero());
    assert_eq!(none.status, CheckStatus::Info);
}

#[test]
fn server_version_skew_is_a_warning_during_a_rollout_and_a_problem_after() {
    let same = replicas(vec![
        replica("a", "1.0.0", 60, 5),
        replica("b", "1.0.0", 30, 5),
    ]);
    assert_eq!(server_versions_check(&same).status, CheckStatus::Ok);

    let rolling = replicas(vec![
        replica("a", "1.0.0", 60, 5),
        replica("b", "1.1.0", 3, 5),
    ]);
    let check = server_versions_check(&rolling);
    assert_eq!(check.status, CheckStatus::Warning);
    assert!(
        check
            .summary
            .contains("1 server on 1.0.0, 1 server on 1.1.0"),
        "{}",
        check.summary
    );

    let stuck = replicas(vec![
        replica("a", "1.0.0", 60, 5),
        replica("b", "1.1.0", 25, 5),
    ]);
    assert_eq!(server_versions_check(&stuck).status, CheckStatus::Problem);

    // A stopped replica on another version is not skew.
    let gone = replicas(vec![
        replica("a", "1.1.0", 60, 5),
        replica("b", "1.0.0", 60, 900),
    ]);
    assert_eq!(server_versions_check(&gone).status, CheckStatus::Ok);
}

#[tokio::test]
async fn several_servers_need_nats_and_valkey_and_must_agree_on_them() {
    assert_eq!(nats_check(None, 1).await.status, CheckStatus::Info);
    assert_eq!(nats_check(None, 2).await.status, CheckStatus::Warning);
    let valkey = valkey_check(None, 3).await;
    assert_eq!(valkey.status, CheckStatus::Warning);
    assert!(
        valkey
            .why
            .as_deref()
            .unwrap_or_default()
            .contains("3 times")
    );

    let mut with = replica("a", "1.0.0", 60, 5);
    with.details.nats = true;
    let without = replica("b", "1.0.0", 60, 5);
    let live = [with, without];
    let ok = DoctorCheck::new(CheckArea::Messaging, "messaging.nats", "NATS").ok("Connected.");
    let split = disagreement(ok, &live, "NATS", |row| row.details.nats);
    assert_eq!(split.status, CheckStatus::Warning);
    assert!(
        split.summary.contains("host-a use it, host-b do not"),
        "{}",
        split.summary
    );
}

#[test]
fn servers_that_cannot_share_live_updates_are_flagged() {
    let on = |id: &str, backend: &str| {
        let mut row = replica(id, "1.0.0", 60, 5);
        row.details.event_delivery = Some(backend.to_string());
        row
    };
    let ok = || DoctorCheck::new(CheckArea::Messaging, "messaging.nats", "NATS").ok("Connected.");

    // One server on in-memory delivery is the single-server setup.
    assert_eq!(
        event_delivery(ok(), &[on("a", "in_memory")]).status,
        CheckStatus::Ok
    );

    // With two, the in-memory one streams only to its own clients.
    let split = event_delivery(ok(), &[on("a", "nats"), on("b", "in_memory")]);
    assert_eq!(split.status, CheckStatus::Problem);
    assert!(split.summary.contains("host-b"), "{}", split.summary);

    // Polling keeps clients current, but it is degraded.
    let polling = event_delivery(ok(), &[on("a", "polling"), on("b", "polling")]);
    assert_eq!(polling.status, CheckStatus::Warning);
    assert!(
        polling.summary.contains("host-a, host-b"),
        "{}",
        polling.summary
    );

    // Heartbeats from builds that do not report it change nothing.
    let old = [replica("a", "1.0.0", 60, 5), replica("b", "1.0.0", 60, 5)];
    assert_eq!(event_delivery(ok(), &old).status, CheckStatus::Ok);
}

#[tokio::test]
async fn heartbeats_round_trip_and_a_running_doctor_sees_its_own_server() {
    let db = StorageBackend::test_database();
    let this = Instance::this_server();
    record(db.pool(), this).await.unwrap();
    // A second beat updates the row in place.
    record(db.pool(), this).await.unwrap();
    let (now, rows) = recent(db.pool(), KIND_SERVER, TimeDelta::hours(1))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, this.id);
    assert_eq!(rows[0].version, env!("CARGO_PKG_VERSION"));
    assert!(rows[0].schema_version.is_some());
    assert!(now - rows[0].last_seen_at < TimeDelta::seconds(30));

    let doctor = SystemDoctor::new(Arc::new(db), None, None, env(DeploymentGrade::Dev));
    let report = doctor.run().await;
    let servers = check(&report, "control_plane.servers");
    assert_eq!(servers.status, CheckStatus::Ok);
    assert_eq!(servers.summary, "One server is running.");
    assert!(
        report
            .checks
            .iter()
            .all(|c| c.id != "control_plane.server_versions")
    );
}

#[test]
fn gauges_put_notes_with_healthy_checks_and_problems_highest() {
    use super::status_gauge;
    assert_eq!(status_gauge(CheckStatus::Ok), 0.0);
    assert_eq!(status_gauge(CheckStatus::Info), 0.0);
    assert_eq!(status_gauge(CheckStatus::Warning), 1.0);
    assert_eq!(status_gauge(CheckStatus::Problem), 2.0);
}

#[test]
fn preflight_output_shows_fixes_for_failing_checks_and_exits_non_zero() {
    use super::preflight::{exit_code, render};
    let failing = DoctorCheck::new(
        CheckArea::Database,
        "database.migrations",
        "Migrations applied",
    )
    .problem("2 migrations are missing.")
    .why("New features fail.")
    .fix("Run everruns-migrate.");
    let note = DoctorCheck::new(CheckArea::Messaging, "messaging.nats", "NATS")
        .info("Not used.")
        .why("hidden for notes");
    let report = DoctorReport {
        status: CheckStatus::Problem,
        checked_at: chrono::Utc::now(),
        version: "1.0.0".to_string(),
        grade: "prod".to_string(),
        auth_mode: "full".to_string(),
        setup: vec!["PostgreSQL 17.2".to_string()],
        checks: vec![failing, note],
    };
    let text = render(&report);
    assert!(
        text.starts_with("Everruns 1.0.0 system doctor: problems\nSetup: PostgreSQL 17.2\n"),
        "{text}"
    );
    assert!(text.contains("✗ Migrations applied: 2 migrations are missing.\n    Why: New features fail.\n    Fix: Run everruns-migrate.\n"), "{text}");
    assert!(text.contains("· NATS: Not used.\n"), "{text}");
    assert!(!text.contains("hidden for notes"), "{text}");
    assert_eq!(exit_code(&report), 1);
    let ok = DoctorReport {
        status: CheckStatus::Warning,
        ..report
    };
    assert_eq!(exit_code(&ok), 0);
}

struct Gateway;

#[async_trait::async_trait]
impl DoctorExtension for Gateway {
    fn name(&self) -> &str {
        "gateway"
    }

    async fn run(&self) -> DoctorFindings {
        let mut findings = DoctorFindings::default();
        findings.fact("Model gateway");
        findings.check(
            DoctorCheck::new(
                CheckArea::Integrations,
                "integrations.gateway",
                "Model gateway",
            )
            .problem("The model gateway does not answer.")
            .fix("Start it."),
        );
        findings
    }
}

struct Hangs;

#[async_trait::async_trait]
impl DoctorExtension for Hangs {
    fn name(&self) -> &str {
        "billing"
    }

    async fn run(&self) -> DoctorFindings {
        std::future::pending().await
    }
}

#[tokio::test]
async fn host_checks_join_the_report_and_its_verdict() {
    let report = doctor(env(DeploymentGrade::Dev))
        .with_extensions(DoctorExtensions::new(vec![Arc::new(Gateway)]))
        .run()
        .await;
    let gateway = check(&report, "integrations.gateway");
    assert_eq!(gateway.status, CheckStatus::Problem);
    assert_eq!(report.status, CheckStatus::Problem);
    assert!(report.setup.iter().any(|fact| fact == "Model gateway"));
}

#[tokio::test(start_paused = true)]
async fn a_host_check_that_hangs_is_a_warning_not_a_stuck_report() {
    let findings = super::extension::run_bounded(&Hangs).await;
    assert_eq!(findings.checks.len(), 1);
    assert_eq!(findings.checks[0].id, "integrations.billing");
    assert_eq!(findings.checks[0].status, CheckStatus::Warning);
}

#[test]
fn system_email_is_needed_only_where_people_sign_in_with_accounts() {
    use super::checks::control_plane::email_check;
    let mut env = env(DeploymentGrade::Prod);
    assert_eq!(email_check(&env).status, CheckStatus::Ok);
    env.email = Ok(None);
    assert_eq!(email_check(&env).status, CheckStatus::Warning);
    env.auth_mode = AuthMode::Admin;
    assert_eq!(email_check(&env).status, CheckStatus::Info);
    env.email = Err("RESEND_API_KEY is empty".to_string());
    let broken = email_check(&env);
    assert_eq!(broken.status, CheckStatus::Problem);
    assert!(broken.summary.contains("RESEND_API_KEY is empty"));
}
