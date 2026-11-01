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
