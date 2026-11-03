#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::{EventQuery, EventRecord, Limits, Scope, query, segment};
use chrono::Utc;
use object_store::{ObjectStore, ObjectStoreExt, memory::InMemory, path::Path};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

fn event(session: Uuid, seq: i32) -> EventRecord {
    EventRecord {
        id: Uuid::now_v7(),
        session_id: session,
        sequence: seq,
        event_type: "tool.completed".into(),
        ts: Utc::now(),
        created_at: Utc::now(),
        context: json!({"turn_id":"t1","exec_id":"e1"}),
        data: json!({"result":"a large private payload"}),
        metadata: None,
        tags: None,
    }
}

#[tokio::test]
async fn parquet_predicates_isolate_tenants_without_reading_payloads() {
    let objects: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let prefix = Path::from("traces");
    let session = Uuid::now_v7();
    let events = vec![event(session, 1), event(session, 3)];
    let limits = Limits::default();
    let scope = Scope::new(1).unwrap();
    let manifest = segment::write(&objects, &prefix, scope, &events, &limits)
        .await
        .unwrap();
    objects
        .delete(&Path::from(manifest.payload_key.as_str()))
        .await
        .unwrap();
    let q = EventQuery {
        session_id: Some(session),
        after_sequence: Some(1),
        turn_id: Some("t1".into()),
        limit: 10,
        ..Default::default()
    };
    let hits = query::execute(
        objects.clone(),
        &prefix,
        scope,
        std::slice::from_ref(&manifest),
        &q,
        &limits,
        None,
    )
    .await
    .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].summary.sequence, 3);
    let mut altered = manifest;
    // Even if the metastore points an org at an index containing a different
    // tenant, row-level predicates cannot expose the other tenant's events.
    let root = prefix
        .clone()
        .join("org-2")
        .join(session.to_string())
        .join(altered.id.to_string());
    let new_key = root.clone().join("index.parquet").to_string();
    let bytes = objects
        .get(&Path::from(altered.index_key.as_str()))
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    objects
        .put(&Path::from(new_key.as_str()), bytes.into())
        .await
        .unwrap();
    altered.index_key = new_key;
    altered.payload_key = root.join("payloads.jsonl").to_string();
    assert!(
        query::execute(
            objects,
            &prefix,
            Scope::new(2).unwrap(),
            &[altered],
            &q,
            &limits,
            None
        )
        .await
        .unwrap()
        .is_empty()
    );
}

#[tokio::test]
async fn segment_rejects_duplicate_sequences_and_oversized_payloads() {
    let objects: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let prefix = Path::from("traces");
    let session = Uuid::now_v7();
    let scope = Scope::new(1).unwrap();
    assert!(
        segment::write(
            &objects,
            &prefix,
            scope,
            &[event(session, 1), event(session, 1)],
            &Limits::default()
        )
        .await
        .is_err()
    );
    let mut large = event(session, 1);
    large.data = json!({"body":"x".repeat(3000)});
    assert!(
        segment::write(
            &objects,
            &prefix,
            scope,
            &[large],
            &Limits {
                max_event_bytes: 1024,
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    assert!(objects.list(None).try_next().await.unwrap().is_none());
}

#[test]
fn caller_cannot_disable_budgets_or_use_a_cross_session_sequence_cursor() {
    assert!(Scope::new(0).is_err());
    assert!(
        Limits {
            max_query_segments: 0,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        EventQuery {
            limit: 1001,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        EventQuery {
            after_sequence: Some(10),
            limit: 10,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}

use futures::TryStreamExt;
