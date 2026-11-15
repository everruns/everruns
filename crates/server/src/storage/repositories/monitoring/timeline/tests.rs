use super::*;
use crate::storage::{CreateEventRow, StorageBackend, test_database::test_session_row};
use everruns_contracts::typed_id::SessionId;

async fn runnable(db: &StorageBackend, org: i64) -> SessionId {
    let id = db.create_session(test_session_row(org)).await.unwrap().id;
    db.mark_session_actor_runnable(id, org, 3).await.unwrap();
    id
}

async fn event(
    db: &StorageBackend,
    id: SessionId,
    name: &str,
    ts: DateTime<Utc>,
    data: serde_json::Value,
) {
    db.create_event(CreateEventRow {
        session_id: id,
        event_type: name.into(),
        ts,
        context: serde_json::json!({}),
        data,
        metadata: None,
        tags: None,
    })
    .await
    .unwrap();
}

fn minute(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

#[tokio::test]
async fn samples_fold_into_minutes_and_idle_samples_count_as_zero() {
    let db = StorageBackend::test_database();
    runnable(&db, 2).await;
    runnable(&db, 2).await;
    runnable(&db, 3).await;
    let at = minute("2026-11-01T06:00:05Z");
    let orgs = db.timeline_sample(Utc::now()).await.unwrap();
    assert_eq!(orgs.iter().map(|o| o.active).sum::<i64>(), 3);
    let workers = [TimelineWorkerSample {
        id: "worker-a".into(),
        load: 4,
        capacity: 10,
    }];
    db.timeline_record_sample(at, &orgs, Some(&workers))
        .await
        .unwrap();
    // Second sample in the same minute: org 2 went idle, org 3 still runs.
    let later: Vec<_> = orgs.iter().filter(|o| o.org_id == 3).cloned().collect();
    db.timeline_record_sample(at + Duration::seconds(10), &later, Some(&workers))
        .await
        .unwrap();

    let from = minute("2026-11-01T06:00:00Z");
    let to = from + Duration::minutes(5);
    let platform = db
        .timeline_buckets(PLATFORM_ORG, from, to, 300)
        .await
        .unwrap();
    assert_eq!(platform.len(), 1);
    let p = &platform[0];
    assert_eq!((p.samples, p.active_sum, p.active_peak), (2, 4, 3));
    assert_eq!((p.runnable_sum, p.runnable_peak), (4, 3));
    assert_eq!(
        (p.workers, p.worker_capacity, p.worker_load_sum),
        (Some(1), Some(10), Some(8))
    );
    assert_eq!(p.organizations_peak, 2);
    assert!(!p.backfilled);

    // Org 2 was active in one of two samples: its average is 2 / 2 samples.
    let org = db.timeline_buckets(2, from, to, 300).await.unwrap();
    assert_eq!(
        (org[0].samples, org[0].active_sum, org[0].active_peak),
        (2, 2, 2)
    );
    // Organization rows never carry platform worker telemetry.
    assert_eq!(org[0].workers, None);

    let strips = db.timeline_worker_buckets(from, to, 300).await.unwrap();
    assert_eq!(strips[&from]["worker-a"], (8, 2, 10));
}

#[tokio::test]
async fn closing_a_minute_counts_turns_tools_and_failures_per_org() {
    let db = StorageBackend::test_database();
    let a = db.create_session(test_session_row(2)).await.unwrap().id;
    let b = db.create_session(test_session_row(3)).await.unwrap().id;
    let t = minute("2026-11-01T06:10:20Z");
    event(&db, a, "turn.started", t, serde_json::json!({})).await;
    event(&db, a, "tool.started", t, serde_json::json!({})).await;
    event(
        &db,
        a,
        "tool.completed",
        t,
        serde_json::json!({"status": "error"}),
    )
    .await;
    event(
        &db,
        a,
        "turn.failed",
        t,
        serde_json::json!({"error_code": "provider_rate_limited"}),
    )
    .await;
    event(&db, b, "turn.started", t, serde_json::json!({})).await;
    event(&db, b, "turn.completed", t, serde_json::json!({})).await;
    // Outside the closed range: ignored.
    event(
        &db,
        b,
        "turn.started",
        t + Duration::minutes(5),
        serde_json::json!({}),
    )
    .await;

    let from = minute("2026-11-01T06:10:00Z");
    db.timeline_close_minutes(from, from + Duration::minutes(1))
        .await
        .unwrap();
    // Closing again is idempotent.
    db.timeline_close_minutes(from, from + Duration::minutes(1))
        .await
        .unwrap();

    let p = &db
        .timeline_buckets(PLATFORM_ORG, from, from + Duration::minutes(1), 60)
        .await
        .unwrap()[0];
    assert_eq!(
        (p.turns_started, p.turns_completed, p.turns_failed),
        (2, 1, 1)
    );
    assert_eq!((p.tool_calls, p.tool_failures), (1, 1));
    let o = &db
        .timeline_buckets(2, from, from + Duration::minutes(1), 60)
        .await
        .unwrap()[0];
    assert_eq!(
        (o.turns_started, o.turns_failed, o.tool_failures),
        (1, 1, 1)
    );

    let failures = db
        .timeline_failures(Some(2), from, from + Duration::minutes(1))
        .await
        .unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(
        (failures[0].code.as_str(), failures[0].turns),
        ("provider_rate_limited", 1)
    );
    assert!(
        db.timeline_failures(Some(3), from, from + Duration::minutes(1))
            .await
            .unwrap()
            .is_empty()
    );

    let busiest = db
        .timeline_busiest(Some(3), from, from + Duration::minutes(1))
        .await
        .unwrap();
    assert_eq!((busiest[0].sessions, busiest[0].turns), (1, 1));
}

#[tokio::test]
async fn backfill_rebuilds_running_turns_without_overwriting_samples() {
    let db = StorageBackend::test_database();
    let s = db.create_session(test_session_row(2)).await.unwrap().id;
    let start = minute("2026-10-30T12:00:30Z");
    // A turn running 12:00:30 – 12:02:00.
    sqlx::query(
        "INSERT INTO session_trace_turns (session_id, turn_no, turn_id, start_sequence, started_at, ended_at, status) \
         VALUES ($1, 1, $2, 1, $3, $4, 'completed')",
    )
    .bind(s.uuid())
    .bind(Uuid::now_v7())
    .bind(start)
    .bind(start + Duration::seconds(90))
    .execute(db.pool())
    .await
    .unwrap();
    event(&db, s, "turn.started", start, serde_json::json!({})).await;
    // A sampled minute inside the backfill range must survive.
    let sampled = minute("2026-10-30T12:05:00Z");
    db.timeline_record_sample(sampled, &[], None).await.unwrap();

    let from = minute("2026-10-30T00:00:00Z");
    db.timeline_backfill(from, minute("2026-10-31T00:00:00Z"))
        .await
        .unwrap();

    let rows = db
        .timeline_buckets(
            2,
            minute("2026-10-30T12:00:00Z"),
            minute("2026-10-30T12:03:00Z"),
            60,
        )
        .await
        .unwrap();
    let peaks: Vec<_> = rows.iter().map(|r| (r.active_peak, r.active_sum)).collect();
    // 12:00 half a minute, 12:01 a full minute, nothing after 12:02.
    assert_eq!(peaks, vec![(1, 1), (1, 1)]);
    assert!(rows.iter().all(|r| r.backfilled));
    assert_eq!(rows[0].turns_started, 1);

    let kept = db
        .timeline_buckets(PLATFORM_ORG, sampled, sampled + Duration::minutes(1), 60)
        .await
        .unwrap();
    assert!(!kept[0].backfilled);
    // The marker row records progress for a day without activity.
    assert_eq!(db.timeline_earliest_minute().await.unwrap(), Some(from));
}

fn report(
    cell: &str,
    at: &str,
    term: i64,
    orgs: Vec<(i64, i64, i64, i64, i64)>,
) -> CellMinuteReport {
    CellMinuteReport {
        cell: cell.into(),
        data_plane: "dp".into(),
        minute: minute(at),
        leader: "n1".into(),
        nodes: 3,
        nodes_answering: 3,
        sessions: orgs.iter().map(|o| o.1).sum(),
        running: orgs.iter().map(|o| o.2).sum(),
        queued: 0,
        resumed: 0,
        rss_peak_bytes: 512,
        memory_limit_bytes: 1024,
        cpu_peak_cores: 0.5,
        cpu_limit_cores: 2.0,
        admit_p99_ms: 4,
        ship_lag_p99_ms: 9,
        raft_term: term,
        forwarded: 0,
        orgs,
    }
}

#[tokio::test]
async fn a_cell_minute_report_counts_once_for_platform_and_tenants() {
    let db = StorageBackend::test_database();
    let first = report("cell-a", "2026-11-01T06:00:00Z", 1, vec![(2, 10, 4, 0, 3)]);
    assert!(db.monitoring_record_cell_report(&first).await.unwrap());
    // A new leader resends the minute with other numbers: ignored.
    let resent = report("cell-a", "2026-11-01T06:00:00Z", 2, vec![(2, 99, 99, 0, 9)]);
    assert!(!db.monitoring_record_cell_report(&resent).await.unwrap());
    db.monitoring_record_cell_report(&report(
        "cell-b",
        "2026-11-01T06:00:00Z",
        1,
        vec![(2, 5, 1, 0, 1), (3, 7, 2, 0, 2)],
    ))
    .await
    .unwrap();
    // cell-a's next minute: a new term is an election.
    db.monitoring_record_cell_report(&report(
        "cell-a",
        "2026-11-01T06:01:00Z",
        3,
        vec![(2, 10, 2, 0, 1)],
    ))
    .await
    .unwrap();

    let from = minute("2026-11-01T06:00:00Z");
    let to = from + Duration::minutes(5);
    let platform = db
        .timeline_fleet_buckets(PLATFORM_ORG, from, to, 60)
        .await
        .unwrap();
    let p = &platform[0];
    assert_eq!(
        (p.dp_reports, p.dp_sessions_sum, p.dp_running_sum),
        (2, 22, 7)
    );
    assert_eq!(p.dp_agents_working_sum, 6);
    let org2 = db.timeline_fleet_buckets(2, from, to, 300).await.unwrap();
    assert_eq!(
        (
            org2[0].dp_minutes,
            org2[0].dp_sessions_peak,
            org2[0].dp_running_peak
        ),
        (2, 15, 5)
    );

    let cells = db.timeline_cells_buckets(from, to, 300).await.unwrap();
    assert_eq!(
        (cells[0].cells, cells[0].nodes, cells[0].elections),
        (2, 6, 2)
    );
    assert_eq!(cells[0].memory_peak, Some(0.5));
    assert_eq!(cells[0].cpu_peak, Some(0.25));
    let markers = db.timeline_cell_markers(from, to).await.unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(
        (markers[0].kind.as_str(), markers[0].label.as_str()),
        ("leader_change", "cell-a")
    );
}

#[tokio::test]
async fn the_cells_list_shows_each_cells_newest_report() {
    let db = StorageBackend::test_database();
    let now = timeline_minute(Utc::now());
    let at = |m: i64| (now - Duration::minutes(m)).to_rfc3339();
    for (cell, ago, term) in [("cell-a", 2, 1), ("cell-a", 1, 2), ("cell-b", 1, 7)] {
        db.monitoring_record_cell_report(&report(cell, &at(ago), term, vec![]))
            .await
            .unwrap();
    }
    let latest = db.monitoring_cells_latest().await.unwrap();
    let terms: Vec<_> = latest
        .iter()
        .map(|c| (c.cell.as_str(), c.raft_term))
        .collect();
    assert_eq!(terms, vec![("cell-a", Some(2)), ("cell-b", Some(7))]);
}

#[tokio::test]
async fn cell_counters_add_to_tenants_platform_and_the_cell() {
    let db = StorageBackend::test_database();
    let at = minute("2026-11-01T06:00:30Z");
    let mut counters = CellBatchCounters::default();
    let c = counters.entry(2, at);
    c.turns_completed += 2;
    c.turns_failed += 1;
    c.model_calls += 3;
    c.cost_usd += 0.5;
    counters.entry(3, at).turns_cancelled += 1;
    db.monitoring_add_cell_counters("cell-a", "dp", &counters)
        .await
        .unwrap();
    db.monitoring_add_cell_counters("cell-a", "dp", &counters)
        .await
        .unwrap();

    let from = minute("2026-11-01T06:00:00Z");
    let to = from + Duration::minutes(5);
    let p = &db
        .timeline_fleet_buckets(PLATFORM_ORG, from, to, 300)
        .await
        .unwrap()[0];
    assert_eq!(
        (
            p.dp_turns_completed,
            p.dp_turns_failed,
            p.dp_turns_cancelled,
            p.dp_model_calls
        ),
        (4, 2, 2, 6)
    );
    assert!((p.dp_cost_usd - 1.0).abs() < 1e-9);
    // Counters alone are not a report: no cell population for the minute.
    assert_eq!(p.dp_minutes, 0);
    let org3 = &db.timeline_fleet_buckets(3, from, to, 300).await.unwrap()[0];
    assert_eq!(org3.dp_turns_cancelled, 2);
    let cell = db.timeline_cell_buckets(from, to, 300).await.unwrap();
    assert_eq!(cell.len(), 1);
    assert_eq!(
        (
            cell[0].reports,
            cell[0].turns_completed,
            cell[0].turns_failed,
            cell[0].turns_cancelled
        ),
        (0, 4, 2, 2)
    );
}

#[tokio::test]
async fn open_sessions_are_counted_into_rows_that_exist_and_carried_forward() {
    let db = StorageBackend::test_database();
    runnable(&db, 2).await;
    runnable(&db, 2).await;
    db.create_session(test_session_row(4)).await.unwrap();
    let at = minute("2026-11-01T06:00:05Z");
    let orgs = db.timeline_sample(Utc::now()).await.unwrap();
    assert!(orgs.iter().all(|o| o.agents_working <= o.active));
    db.timeline_record_sample(at, &orgs, None).await.unwrap();
    db.timeline_record_minute_counts(at).await.unwrap();

    let from = minute("2026-11-01T06:00:00Z");
    let to = from + Duration::minutes(5);
    let p = &db
        .timeline_fleet_buckets(PLATFORM_ORG, from, to, 300)
        .await
        .unwrap()[0];
    assert_eq!(p.sessions, Some(3));
    assert_eq!(p.servers, Some(0));
    let org2 = &db.timeline_fleet_buckets(2, from, to, 300).await.unwrap()[0];
    assert_eq!(org2.sessions, Some(2));
    // Org 4 had no activity, so no row; nothing is written for it.
    assert!(
        db.timeline_fleet_buckets(4, from, to, 300)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        db.timeline_counts_before(2, to).await.unwrap().0,
        Some(2),
        "a later range starts from the newest count"
    );
    let (sessions, _) = db.timeline_live_counts(Some(4)).await.unwrap();
    assert_eq!(sessions, 1);
}
