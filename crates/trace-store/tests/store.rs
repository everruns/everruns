#![cfg(feature = "postgres-tests")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::{TimeZone, Utc};
use everruns_trace_store::{
    CATALOG_MIGRATION, EventQuery, EventRecord, Limits, PARALLEL_MIGRATION, Scope, TraceStore,
};
use futures::TryStreamExt;
use object_store::{ObjectStore, ObjectStoreExt, memory::InMemory, path::Path};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

// These fixtures model the canonical columns used by the exporter. The actual
// new platform migration is executed unmodified against a real PostgreSQL DB.
async fn setup(pool: &PgPool) -> (Arc<InMemory>, Uuid) {
    sqlx::raw_sql("CREATE TABLE sessions(id UUID PRIMARY KEY, org_id BIGINT NOT NULL); CREATE TABLE event_sequences(session_id UUID PRIMARY KEY, next_sequence INTEGER NOT NULL); CREATE TABLE events(id UUID PRIMARY KEY,session_id UUID NOT NULL,sequence INTEGER NOT NULL,event_type TEXT NOT NULL,ts TIMESTAMPTZ NOT NULL,created_at TIMESTAMPTZ NOT NULL,context JSONB NOT NULL,data JSONB NOT NULL,metadata JSONB,tags TEXT[],UNIQUE(session_id,sequence));")
        .execute(pool).await.unwrap();
    sqlx::raw_sql("CREATE TABLE archived_events (LIKE events INCLUDING ALL)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::raw_sql(CATALOG_MIGRATION)
        .execute(pool)
        .await
        .unwrap();
    sqlx::raw_sql(PARALLEL_MIGRATION)
        .execute(pool)
        .await
        .unwrap();
    let session = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions VALUES($1,1)")
        .bind(session)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO event_sequences VALUES($1,1)")
        .bind(session)
        .execute(pool)
        .await
        .unwrap();
    (Arc::new(InMemory::new()), session)
}

fn store(pool: &PgPool, objects: Arc<InMemory>, limits: Limits) -> TraceStore {
    TraceStore::new(pool.clone(), objects, "tests", Path::from("traces"), limits).unwrap()
}

async fn seed(pool: &PgPool, session: Uuid, sequences: &[i32]) -> Vec<EventRecord> {
    let mut result = Vec::new();
    for &seq in sequences {
        let event = EventRecord {
            id: Uuid::now_v7(),
            session_id: session,
            sequence: seq,
            event_type: if seq % 2 == 0 {
                "tool.completed"
            } else {
                "future.unknown"
            }
            .into(),
            ts: Utc
                .timestamp_opt(1_700_000_000 + i64::from(seq), 0)
                .unwrap(),
            created_at: Utc
                .timestamp_opt(1_700_000_000 + i64::from(seq), 0)
                .unwrap(),
            context: json!({"turn_id":"turn-1","exec_id":"exec-1","parent_exec_id":"root", "future_field":true}),
            data: json!({"text":"sensitive prompt\nwith unicode: Δ", "large": "x".repeat(16_000), "number":42}),
            metadata: Some(json!({"future_metadata":[1,2]})),
            tags: Some(vec!["tag".into()]),
        };
        sqlx::query("INSERT INTO events VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(event.id)
            .bind(session)
            .bind(seq)
            .bind(&event.event_type)
            .bind(event.ts)
            .bind(event.created_at)
            .bind(&event.context)
            .bind(&event.data)
            .bind(&event.metadata)
            .bind(&event.tags)
            .execute(pool)
            .await
            .unwrap();
        result.push(event);
    }
    if let Some(seq) = sequences.iter().max() {
        sqlx::query("UPDATE event_sequences SET next_sequence=GREATEST(next_sequence,$2) WHERE session_id=$1")
            .bind(session).bind(seq + 1).execute(pool).await.unwrap();
    }
    result
}

fn query(session: Uuid) -> EventQuery {
    EventQuery {
        session_id: Some(session),
        limit: 100,
        ..Default::default()
    }
}

async fn object_count(objects: &InMemory) -> usize {
    objects
        .list(None)
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .len()
}

#[sqlx::test(migrations = false)]
async fn export_resume_filters_roundtrip_and_compaction(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    let records = seed(&pool, session, &[1, 3, 4, 9, 10]).await;
    let small = store(
        &pool,
        objects.clone(),
        Limits {
            max_batch_rows: 2,
            ..Default::default()
        },
    );
    let scope = Scope::new(1).unwrap();
    assert_eq!(small.export_session(scope, session).await.unwrap(), 2);
    assert_eq!(small.export_session(scope, session).await.unwrap(), 2);
    assert_eq!(small.export_session(scope, session).await.unwrap(), 1);
    assert_eq!(small.export_session(scope, session).await.unwrap(), 0);
    assert_eq!(
        small
            .query(scope, &query(session))
            .await
            .unwrap()
            .events
            .iter()
            .map(|e| e.sequence)
            .collect::<Vec<_>>(),
        vec![1, 3, 4, 9, 10]
    );
    let q = EventQuery {
        limit: 2,
        ..query(session)
    };
    let page = small.query(scope, &q).await.unwrap();
    assert!(page.has_more);
    assert_eq!(page.events.len(), 2);
    let next = small
        .query(
            scope,
            &EventQuery {
                after_sequence: Some(3),
                ..query(session)
            },
        )
        .await
        .unwrap();
    assert_eq!(
        next.events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        vec![4, 9, 10]
    );
    let filtered = small
        .query(
            scope,
            &EventQuery {
                event_types: vec!["tool.completed".into()],
                from: Some(records[2].ts),
                until: Some(records[4].ts),
                turn_id: Some("turn-1".into()),
                ..query(session)
            },
        )
        .await
        .unwrap();
    assert_eq!(
        filtered
            .events
            .iter()
            .map(|e| e.sequence)
            .collect::<Vec<_>>(),
        vec![4]
    );
    // Strings remain literal expressions, never interpolated SQL.
    assert!(
        small
            .query(
                scope,
                &EventQuery {
                    event_types: vec!["' OR TRUE --".into()],
                    ..query(session)
                }
            )
            .await
            .unwrap()
            .events
            .is_empty()
    );
    for record in &records {
        assert_eq!(
            small
                .read_event(scope, session, record.id)
                .await
                .unwrap()
                .as_ref(),
            Some(record)
        );
    }
    let reopened = store(&pool, objects.clone(), Limits::default());
    assert_eq!(reopened.compact_session(scope, session).await.unwrap(), 3);
    assert_eq!(
        reopened
            .query(scope, &query(session))
            .await
            .unwrap()
            .segments_scanned,
        1
    );
    assert_eq!(object_count(&objects).await, 8);
    assert_eq!(
        reopened.collect_garbage(None, 100).await.unwrap(),
        (6, None)
    );
    for record in &records {
        assert_eq!(
            reopened
                .read_event(scope, session, record.id)
                .await
                .unwrap()
                .as_ref(),
            Some(record)
        );
    }
    let canonical: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(canonical, 5);
}

#[sqlx::test(migrations = false)]
async fn tenant_isolation_and_deletion_prevent_reexport(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    let records = seed(&pool, session, &[1, 2]).await;
    let store = store(&pool, objects.clone(), Limits::default());
    let scope = Scope::new(1).unwrap();
    let other = Scope::new(2).unwrap();
    assert!(store.export_session(other, session).await.is_err());
    store.export_session(scope, session).await.unwrap();
    assert!(
        store
            .query(other, &query(session))
            .await
            .unwrap()
            .events
            .is_empty()
    );
    assert!(
        store
            .read_event(other, session, records[0].id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(store.delete_session(other, session).await.is_err());
    store.delete_session(scope, session).await.unwrap();
    store.delete_session(scope, session).await.unwrap();
    assert!(
        store
            .query(scope, &query(session))
            .await
            .unwrap()
            .events
            .is_empty()
    );
    assert!(store.export_session(scope, session).await.is_err());
    assert_eq!(store.collect_garbage(None, 100).await.unwrap().0, 2);
    let canonical: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(canonical, 2);
}

#[sqlx::test(migrations = false)]
async fn source_deletion_hides_objects_and_gc_reclaims_them(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    seed(&pool, session, &[1]).await;
    let store = store(&pool, objects.clone(), Limits::default());
    let scope = Scope::new(1).unwrap();
    store.export_session(scope, session).await.unwrap();
    sqlx::query("DELETE FROM sessions WHERE id=$1")
        .bind(session)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        store
            .query(scope, &query(session))
            .await
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(store.collect_garbage(None, 100).await.unwrap().0, 2);
}

#[sqlx::test(migrations = false)]
async fn failed_publication_rolls_back_checkpoint_and_orphans_are_collected(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    seed(&pool, session, &[1, 2]).await;
    let store = store(&pool, objects.clone(), Limits::default());
    let scope = Scope::new(1).unwrap();
    sqlx::raw_sql("CREATE FUNCTION reject_trace() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected publish failure'; END $$; CREATE TRIGGER reject_trace BEFORE INSERT ON trace_store_segments FOR EACH ROW EXECUTE FUNCTION reject_trace();")
        .execute(&pool).await.unwrap();
    assert!(store.export_session(scope, session).await.is_err());
    assert_eq!(object_count(&objects).await, 2);
    assert!(
        store
            .query(scope, &query(session))
            .await
            .unwrap()
            .events
            .is_empty()
    );
    sqlx::raw_sql("DROP TRIGGER reject_trace ON trace_store_segments")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(store.export_session(scope, session).await.unwrap(), 2);
    assert_eq!(
        store
            .query(scope, &query(session))
            .await
            .unwrap()
            .events
            .len(),
        2
    );
    assert_eq!(store.collect_garbage(None, 100).await.unwrap().0, 2);
    assert_eq!(object_count(&objects).await, 2);
}

#[sqlx::test(migrations = false)]
async fn concurrent_exporters_do_not_duplicate_events(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    seed(&pool, session, &[1, 2, 3, 4]).await;
    let store = store(
        &pool,
        objects,
        Limits {
            max_batch_rows: 2,
            ..Default::default()
        },
    );
    let scope = Scope::new(1).unwrap();
    let (a, b) = tokio::join!(
        store.export_session(scope, session),
        store.export_session(scope, session)
    );
    assert_eq!(a.unwrap() + b.unwrap(), 4);
    assert_eq!(
        store
            .query(scope, &query(session))
            .await
            .unwrap()
            .events
            .len(),
        4
    );
}

#[sqlx::test(migrations = false)]
async fn list_queries_do_not_fetch_payload_and_detail_checks_integrity(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    let records = seed(&pool, session, &[1]).await;
    let store = store(&pool, objects.clone(), Limits::default());
    let scope = Scope::new(1).unwrap();
    store.export_session(scope, session).await.unwrap();
    let key: String = sqlx::query_scalar("SELECT payload_key FROM trace_store_segments")
        .fetch_one(&pool)
        .await
        .unwrap();
    let path = Path::from(key);
    let raw = objects.get(&path).await.unwrap().bytes().await.unwrap();
    let mut corrupt = raw.to_vec();
    corrupt[0] = b'!';
    objects.put(&path, corrupt.into()).await.unwrap();
    assert!(
        store
            .read_event(scope, session, records[0].id)
            .await
            .unwrap_err()
            .to_string()
            .contains("checksum")
    );
    objects.delete(&path).await.unwrap();
    assert_eq!(
        store
            .query(scope, &query(session))
            .await
            .unwrap()
            .events
            .len(),
        1
    );
    assert!(
        store
            .read_event(scope, session, records[0].id)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = false)]
async fn budgets_fail_explicitly_and_gc_cursor_makes_progress(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    seed(&pool, session, &[1, 2, 3]).await;
    let one = store(
        &pool,
        objects.clone(),
        Limits {
            max_batch_rows: 1,
            ..Default::default()
        },
    );
    let scope = Scope::new(1).unwrap();
    for _ in 0..3 {
        one.export_session(scope, session).await.unwrap();
    }
    let bounded = store(
        &pool,
        objects.clone(),
        Limits {
            max_query_segments: 1,
            ..Default::default()
        },
    );
    assert!(
        bounded
            .query(scope, &query(session))
            .await
            .unwrap_err()
            .to_string()
            .contains("segment budget")
    );
    let bounded = store(
        &pool,
        objects.clone(),
        Limits {
            max_query_index_bytes: 1,
            ..Default::default()
        },
    );
    assert!(
        bounded
            .query(scope, &query(session))
            .await
            .unwrap_err()
            .to_string()
            .contains("byte budget")
    );
    assert!(
        one.query(
            scope,
            &EventQuery {
                limit: 0,
                ..query(session)
            }
        )
        .await
        .is_err()
    );
    assert!(
        one.query(
            scope,
            &EventQuery {
                session_id: None,
                after_sequence: Some(1),
                limit: 10,
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    one.delete_session(scope, session).await.unwrap();
    let mut cursor = None;
    let mut deleted = 0;
    loop {
        let (count, next) = one.collect_garbage(cursor.as_deref(), 1).await.unwrap();
        deleted += count;
        if next.is_none() {
            break;
        }
        cursor = next;
    }
    assert_eq!(deleted, 6);
    assert_eq!(object_count(&objects).await, 0);
    assert!(
        one.collect_garbage(Some("another-prefix/file"), 1)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = false)]
async fn committed_head_prevents_export_of_unaccepted_events(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    seed(&pool, session, &[1, 2]).await;
    sqlx::query("UPDATE event_sequences SET next_sequence=2 WHERE session_id=$1")
        .bind(session)
        .execute(&pool)
        .await
        .unwrap();
    let store = store(&pool, objects, Limits::default());
    let scope = Scope::new(1).unwrap();
    assert_eq!(store.export_session(scope, session).await.unwrap(), 1);
    assert_eq!(store.export_session(scope, session).await.unwrap(), 0);
    sqlx::query("UPDATE event_sequences SET next_sequence=3 WHERE session_id=$1")
        .bind(session)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(store.export_session(scope, session).await.unwrap(), 1);
}

#[sqlx::test(migrations = false)]
async fn sync_discovers_sessions_and_respects_tenant_and_tombstones(pool: PgPool) {
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
    assert_eq!(
        store.sync_once(Scope::new(2).unwrap(), 10).await.unwrap(),
        0
    );
    assert_eq!(store.sync_once(scope, 10).await.unwrap(), 2);
    assert_eq!(store.sync_once(scope, 10).await.unwrap(), 1);
    assert_eq!(store.sync_once(scope, 10).await.unwrap(), 0);
    store.delete_session(scope, session).await.unwrap();
    seed(&pool, session, &[4]).await;
    assert_eq!(store.sync_once(scope, 10).await.unwrap(), 0);
}

#[sqlx::test(migrations = false)]
async fn oversized_event_fails_before_publication_and_never_skips_forward(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    seed(&pool, session, &[1, 2]).await;
    let bounded = store(
        &pool,
        objects.clone(),
        Limits {
            max_event_bytes: 1024,
            ..Default::default()
        },
    );
    assert!(
        bounded
            .export_session(Scope::new(1).unwrap(), session)
            .await
            .unwrap_err()
            .to_string()
            .contains("payload budget")
    );
    assert_eq!(object_count(&objects).await, 0);
    let store = store(&pool, objects, Limits::default());
    assert_eq!(
        store
            .export_session(Scope::new(1).unwrap(), session)
            .await
            .unwrap(),
        2
    );
}

#[sqlx::test(migrations = false)]
async fn store_prefix_registration_prevents_cross_store_collection(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    seed(&pool, session, &[1]).await;
    let first = store(&pool, objects.clone(), Limits::default());
    first
        .export_session(Scope::new(1).unwrap(), session)
        .await
        .unwrap();
    let child = TraceStore::new(
        pool.clone(),
        objects.clone(),
        "child",
        Path::from("traces/child"),
        Limits::default(),
    )
    .unwrap();
    assert!(
        child
            .collect_garbage(None, 100)
            .await
            .unwrap_err()
            .to_string()
            .contains("overlaps")
    );
    let parent = TraceStore::new(
        pool.clone(),
        objects.clone(),
        "parent",
        Path::from("trace-root"),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(parent.collect_garbage(None, 100).await.unwrap().0, 0);
    let mismatch = TraceStore::new(
        pool.clone(),
        objects,
        "tests",
        Path::from("different-root"),
        Limits::default(),
    )
    .unwrap();
    assert!(
        mismatch
            .query(Scope::new(1).unwrap(), &query(session))
            .await
            .is_err()
    );
    assert_eq!(
        first
            .query(Scope::new(1).unwrap(), &query(session))
            .await
            .unwrap()
            .events
            .len(),
        1
    );
}

#[sqlx::test(migrations = false)]
async fn compaction_merges_full_batches_and_skips_an_already_full_segment(pool: PgPool) {
    let (objects, session) = setup(&pool).await;
    seed(&pool, session, &[1, 2, 3, 4, 5, 6, 7, 8]).await;
    let store = store(
        &pool,
        objects,
        Limits {
            max_batch_rows: 2,
            max_compaction_rows: 4,
            ..Default::default()
        },
    );
    let scope = Scope::new(1).unwrap();
    for _ in 0..4 {
        assert_eq!(store.export_session(scope, session).await.unwrap(), 2);
    }
    assert_eq!(store.compact_session(scope, session).await.unwrap(), 2);
    assert_eq!(store.compact_session(scope, session).await.unwrap(), 2);
    assert_eq!(store.compact_session(scope, session).await.unwrap(), 0);
    let page = store.query(scope, &query(session)).await.unwrap();
    assert_eq!(page.segments_scanned, 2);
    assert_eq!(
        page.events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        (1..=8).collect::<Vec<_>>()
    );
}

#[path = "support/parallel.rs"]
mod parallel;
