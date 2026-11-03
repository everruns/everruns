use super::*;
use everruns_trace_store::{CycleReport, Monitor, ShadowOutcome};

#[sqlx::test(migrations = false)]
async fn finite_backfill_reads_archive_and_resumes_while_new_events_arrive(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    let records = seed(&pool, session, &[1, 3, 4]).await;
    sqlx::query("INSERT INTO archived_events SELECT * FROM events WHERE sequence IN (1,3)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM events WHERE sequence=1")
        .execute(&pool)
        .await
        .unwrap();
    let store = store(
        &pool,
        objects.clone(),
        Limits {
            max_batch_rows: 2,
            ..Default::default()
        },
    );
    let scope = Scope::new(1).unwrap();
    let run = store.start_backfill(scope).await.unwrap();
    assert_eq!(run.total, 1);
    assert!(!run.complete());
    seed(&pool, session, &[9]).await;
    assert!(
        !store
            .advance_backfill(scope, run.run_id, 1)
            .await
            .unwrap()
            .complete()
    );
    let done = store.advance_backfill(scope, run.run_id, 1).await.unwrap();
    assert!(done.complete());
    assert_eq!(done.copied, 1);
    let page = store.query(scope, &query(session)).await.unwrap();
    assert_eq!(
        page.events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        vec![1, 3, 4]
    );
    for e in records {
        assert_eq!(
            store.read_event(scope, session, e.id).await.unwrap(),
            Some(e)
        );
    }
    assert!(
        store
            .backfill_status(Scope::new(2).unwrap(), run.run_id)
            .await
            .is_err()
    );
    assert_eq!(
        store.resume_backfill(scope).await.unwrap().run_id,
        run.run_id
    );
    assert_eq!(store.sync_once(scope, 1).await.unwrap(), 1);
    assert_eq!(
        store
            .shadow_session(scope, session, 100)
            .await
            .unwrap()
            .outcome,
        ShadowOutcome::Match
    );
}

#[sqlx::test(migrations = false)]
async fn lag_is_not_mismatch_and_comparison_cursor_rotates(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    seed(&pool, session, &[1, 2, 3, 4, 5, 6, 7]).await;
    let store = store(
        &pool,
        objects,
        Limits {
            max_batch_rows: 2,
            ..Default::default()
        },
    );
    let scope = Scope::new(1).unwrap();
    store.export_session(scope, session).await.unwrap();
    let lag = store.shadow_session(scope, session, 2).await.unwrap();
    assert_eq!(lag.outcome, ShadowOutcome::Lagging);
    assert_eq!(lag.summary_matches, Some(true));
    assert_eq!(lag.payload_matches, Some(true));
    assert_eq!(lag.through_sequence, 2);
    for _ in 0..3 {
        store.export_session(scope, session).await.unwrap();
    }
    let mut seen = Vec::new();
    for _ in 0..4 {
        let report = store.shadow_session(scope, session, 2).await.unwrap();
        assert_eq!(report.outcome, ShadowOutcome::Match);
        seen.push((report.after_sequence, report.next_sequence));
    }
    assert_eq!(seen, vec![(2, 4), (4, 6), (6, 7), (0, 2)]);
}

#[sqlx::test(migrations = false)]
async fn durable_mismatch_and_payload_failure_remain_visible(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    let records = seed(&pool, session, &[1]).await;
    let store = store(&pool, objects.clone(), Limits::default());
    let scope = Scope::new(1).unwrap();
    store.export_session(scope, session).await.unwrap();
    sqlx::query("UPDATE events SET data=$1")
        .bind(json!({"private":"do not log this body"}))
        .execute(&pool)
        .await
        .unwrap();
    let report = store.shadow_session(scope, session, 100).await.unwrap();
    assert_eq!(report.outcome, ShadowOutcome::Mismatch);
    assert_eq!(report.summary_matches, Some(true));
    assert_eq!(report.payload_matches, Some(false));
    assert_eq!(store.progress(scope).await.unwrap().unhealthy_checks, 1);
    let serialized = serde_json::to_string(&report).unwrap();
    assert!(!serialized.contains("private"));
    sqlx::query("UPDATE events SET data=$1")
        .bind(&records[0].data)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        store
            .shadow_session(scope, session, 100)
            .await
            .unwrap()
            .outcome,
        ShadowOutcome::Match
    );
    assert_eq!(store.progress(scope).await.unwrap().unhealthy_checks, 0);
    let key: String = sqlx::query_scalar("SELECT payload_key FROM trace_store_segments LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    objects.delete(&Path::from(key)).await.unwrap();
    assert_eq!(
        store
            .shadow_session(scope, session, 100)
            .await
            .unwrap()
            .outcome,
        ShadowOutcome::Error
    );
    assert_eq!(store.progress(scope).await.unwrap().unhealthy_checks, 1);
}

#[sqlx::test(migrations = false)]
async fn deleted_and_missing_history_are_not_reported_as_matches(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    seed(&pool, session, &[1]).await;
    let store = store(&pool, objects, Limits::default());
    let scope = Scope::new(1).unwrap();
    let run = store.start_backfill(scope).await.unwrap();
    sqlx::query("DELETE FROM events")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        store
            .advance_backfill(scope, run.run_id, 1)
            .await
            .unwrap()
            .complete()
    );
    assert_eq!(
        store
            .shadow_session(scope, session, 100)
            .await
            .unwrap()
            .outcome,
        ShadowOutcome::MissingSource
    );
    store.delete_session(scope, session).await.unwrap();
    assert_eq!(
        store
            .shadow_session(scope, session, 100)
            .await
            .unwrap()
            .outcome,
        ShadowOutcome::Skipped
    );
    assert_eq!(store.progress(scope).await.unwrap().unhealthy_checks, 0);
    assert_eq!(
        store
            .shadow_session(Scope::new(2).unwrap(), session, 100)
            .await
            .unwrap()
            .outcome,
        ShadowOutcome::Skipped
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM trace_store_shadow_checks WHERE org_id=2")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = false)]
async fn failed_backfill_target_does_not_starve_others_and_can_be_retried(pool: PgPool) {
    let (objects, bad) = setup(&pool).await;
    seed(&pool, bad, &[1]).await;
    sqlx::query("UPDATE events SET data=$1")
        .bind(json!({"oversized":"x".repeat(100_000)}))
        .execute(&pool)
        .await
        .unwrap();
    let good = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions VALUES($1,1)")
        .bind(good)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO event_sequences VALUES($1,1)")
        .bind(good)
        .execute(&pool)
        .await
        .unwrap();
    seed(&pool, good, &[1]).await;
    let small = store(
        &pool,
        objects.clone(),
        Limits {
            max_event_bytes: 20_000,
            ..Default::default()
        },
    );
    let scope = Scope::new(1).unwrap();
    let run = small.start_backfill(scope).await.unwrap();
    let status = small.advance_backfill(scope, run.run_id, 2).await.unwrap();
    assert_eq!(status.failed, 1);
    assert_eq!(status.copied, 1);
    assert!(!status.complete());
    let larger = store(&pool, objects, Limits::default());
    let status = larger.advance_backfill(scope, run.run_id, 2).await.unwrap();
    assert!(status.complete());
    assert_eq!(status.copied, 2);
}

#[sqlx::test(migrations = false)]
async fn conflicting_source_identities_are_rejected_and_sequence_gaps_progress(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    seed(&pool, session, &[1]).await;
    sqlx::query("INSERT INTO archived_events SELECT * FROM events")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE archived_events SET id=$1")
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .unwrap();
    let store = store(&pool, objects, Limits::default());
    let scope = Scope::new(1).unwrap();
    assert!(
        store
            .export_session(scope, session)
            .await
            .unwrap_err()
            .to_string()
            .contains("conflicting")
    );
    sqlx::query("DELETE FROM archived_events")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE event_sequences SET next_sequence=6")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(store.export_session(scope, session).await.unwrap(), 1);
    assert_eq!(store.export_session(scope, session).await.unwrap(), 0);
    assert_eq!(store.progress(scope).await.unwrap().lag_sequences, 0);
    assert_eq!(
        store
            .shadow_session(scope, session, 100)
            .await
            .unwrap()
            .outcome,
        ShadowOutcome::Match
    );
}

#[sqlx::test(migrations = false)]
async fn complete_parallel_cycle_backfills_then_catches_up_without_changing_source(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    seed(&pool, session, &[1, 2, 3]).await;
    let store = store(
        &pool,
        objects,
        Limits {
            max_batch_rows: 2,
            ..Default::default()
        },
    );
    let scope = Scope::new(1).unwrap();
    let first = store.parallel_cycle(&[scope], 1).await.unwrap();
    assert_eq!(first.errors, 0);
    assert_eq!(first.exported, 2);
    assert_eq!(first.backfill_pending, 1);
    let second = store.parallel_cycle(&[scope], 1).await.unwrap();
    assert_eq!(second.errors, 0);
    assert_eq!(second.exported, 1);
    assert_eq!(second.pending_sessions, 0);
    assert_eq!(second.comparisons[0].outcome, ShadowOutcome::Match);
    seed(&pool, session, &[4]).await;
    let next = store.parallel_cycle(&[scope], 1).await.unwrap();
    assert_eq!(next.exported, 1);
    assert_eq!(next.comparisons[0].outcome, ShadowOutcome::Match);
    let monitor = Monitor::default();
    assert!(!monitor.ready(std::time::Duration::from_secs(60)));
    monitor.record(&next);
    assert!(monitor.ready(std::time::Duration::from_secs(60)));
    let metrics = monitor.render();
    assert!(metrics.contains("outcome=\"match\""));
    assert!(metrics.contains("query_seconds_bucket"));
    assert!(!metrics.contains(&session.to_string()));
    monitor.record(&CycleReport {
        unhealthy_checks: 1,
        ..Default::default()
    });
    assert!(!monitor.ready(std::time::Duration::from_secs(60)));
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 4);
}

#[sqlx::test(migrations = false)]
async fn shadow_prunes_to_the_sample_window_in_a_large_catalog(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    seed(&pool, session, &(1..=50).collect::<Vec<_>>()).await;
    let store = store(
        &pool,
        objects,
        Limits {
            max_batch_rows: 10,
            max_query_segments: 2,
            ..Default::default()
        },
    );
    let scope = Scope::new(1).unwrap();
    for _ in 0..5 {
        store.export_session(scope, session).await.unwrap();
    }
    assert!(store.query(scope, &query(session)).await.is_err());
    let report = store.shadow_session(scope, session, 5).await.unwrap();
    assert_eq!(report.outcome, ShadowOutcome::Match);
    assert_eq!(report.segments_scanned, 1);
    assert_eq!(report.window_through_sequence, 6);
}

#[cfg(feature = "s3-tests")]
#[sqlx::test(migrations = false)]
async fn s3_backfill_and_parallel_reads_use_the_real_object_protocol(pool: PgPool) {
    use object_store::aws::AmazonS3Builder;
    let endpoint = std::env::var("TEST_TRACE_S3_ENDPOINT").expect("local S3 endpoint required");
    let url = url::Url::parse(&endpoint).unwrap();
    assert!(
        matches!(url.host_str(), Some("127.0.0.1" | "localhost")),
        "S3 fixtures must be local"
    );
    assert!(url.username().is_empty() && url.password().is_none());
    let (_, session) = setup(&pool).await;
    let records = seed(&pool, session, &[1, 2]).await;
    let objects = Arc::new(
        AmazonS3Builder::new()
            .with_endpoint(endpoint)
            .with_allow_http(true)
            .with_region("us-east-1")
            .with_bucket_name("traces")
            .with_access_key_id("test-only")
            .with_secret_access_key("test-only")
            .build()
            .unwrap(),
    );
    let id = Uuid::now_v7();
    let store = TraceStore::new(
        pool.clone(),
        objects,
        "s3-test",
        Path::from(format!("qualification/tests/{id}")),
        Limits::default(),
    )
    .unwrap();
    let scope = Scope::new(1).unwrap();
    let run = store.start_backfill(scope).await.unwrap();
    assert!(
        store
            .advance_backfill(scope, run.run_id, 1)
            .await
            .unwrap()
            .complete()
    );
    assert_eq!(
        store
            .read_event_at(scope, session, records[0].id, 1)
            .await
            .unwrap(),
        Some(records[0].clone())
    );
    seed(&pool, session, &[3]).await;
    let cycle = store.parallel_cycle(&[scope], 1).await.unwrap();
    assert_eq!(cycle.errors, 0);
    assert_eq!(cycle.exported, 1);
    assert_eq!(cycle.comparisons[0].outcome, ShadowOutcome::Match);
    store.delete_session(scope, session).await.unwrap();
    assert_eq!(store.collect_garbage(None, 100).await.unwrap().0, 4);
}

#[sqlx::test(migrations = false)]
async fn companion_catalog_migration_is_idempotent_and_keeps_app_history_unchanged(pool: PgPool) {
    setup(&pool).await;
    sqlx::query("CREATE TABLE _sqlx_migrations(version BIGINT PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO _sqlx_migrations VALUES(1)")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        everruns_trace_store::migrate_catalog(&pool).await.unwrap(),
        2
    );
    assert_eq!(
        everruns_trace_store::migrate_catalog(&pool).await.unwrap(),
        0
    );
    let versions: Vec<i64> = sqlx::query_scalar("SELECT version FROM _sqlx_migrations")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(versions, vec![1]);
    sqlx::query("UPDATE trace_store_schema SET checksum='corrupt'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(everruns_trace_store::migrate_catalog(&pool).await.is_err());
}

#[sqlx::test(migrations = false)]
async fn all_org_observer_rotates_durably_and_discovers_new_history(pool: PgPool) {
    let (objects, first) = setup(&pool).await;
    let store = store(&pool, objects.clone(), Limits::default());
    assert!(store.next_org_scopes(2).await.unwrap().is_empty());
    seed(&pool, first, &[1]).await;
    for org in [2_i64, 3] {
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO sessions VALUES($1,$2)")
            .bind(id)
            .bind(org)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO event_sequences VALUES($1,1)")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        seed(&pool, id, &[1]).await;
    }
    let ids = |scopes: Vec<Scope>| scopes.into_iter().map(|s| s.org_id()).collect::<Vec<_>>();
    assert_eq!(ids(store.next_org_scopes(2).await.unwrap()), vec![1, 2]);
    let restarted = super::store(&pool, objects, Limits::default());
    assert_eq!(ids(restarted.next_org_scopes(2).await.unwrap()), vec![3]);
    assert_eq!(ids(restarted.next_org_scopes(2).await.unwrap()), vec![1, 2]);
    let new = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions VALUES($1,4)")
        .bind(new)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO event_sequences VALUES($1,1)")
        .bind(new)
        .execute(&pool)
        .await
        .unwrap();
    seed(&pool, new, &[1]).await;
    assert_eq!(ids(restarted.next_org_scopes(2).await.unwrap()), vec![3, 4]);
    assert!(restarted.next_org_scopes(0).await.is_err());
    let mut report = restarted
        .parallel_cycle(&[Scope::new(1).unwrap()], 1)
        .await
        .unwrap();
    restarted
        .refresh_all_org_progress(&mut report)
        .await
        .unwrap();
    assert_eq!(report.pending_sessions, 3);
    assert!(report.payload_bytes > 0);
    sqlx::query("UPDATE trace_store_shadow_checks SET report=jsonb_set(report,'{outcome}','\"mismatch\"'::jsonb) WHERE org_id=1")
        .execute(&pool).await.unwrap();
    let mut next = restarted
        .parallel_cycle(&[Scope::new(2).unwrap()], 1)
        .await
        .unwrap();
    assert_eq!(next.unhealthy_checks, 0);
    restarted.refresh_all_org_progress(&mut next).await.unwrap();
    assert_eq!(next.unhealthy_checks, 1);
    assert_eq!(next.pending_sessions, 2);
    let monitor = Monitor::default();
    monitor.record(&next);
    assert!(!monitor.ready(std::time::Duration::from_secs(60)));
    assert!(
        monitor
            .render()
            .contains("everruns_trace_store_unhealthy_checks 1")
    );
    let run = restarted
        .start_backfill(Scope::new(3).unwrap())
        .await
        .unwrap();
    sqlx::query("DELETE FROM sessions WHERE org_id=3")
        .execute(&pool)
        .await
        .unwrap();
    assert!(ids(restarted.next_org_scopes(100).await.unwrap()).contains(&3));
    let skipped = restarted
        .advance_backfill(Scope::new(3).unwrap(), run.run_id, 1)
        .await
        .unwrap();
    assert!(skipped.complete());
    assert_eq!(skipped.skipped, 1);
}
