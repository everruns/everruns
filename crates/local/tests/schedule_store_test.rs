// Schedule store: metadata round-trip, list, cancel, count_active.

use everruns_core::session_services::SessionScheduleStore;
use everruns_local::{LocalScheduleStore, SqliteDb};
use everruns_provider::typed_id::{PrincipalId, SessionId};

fn store() -> LocalScheduleStore {
    LocalScheduleStore::new(
        SqliteDb::open_in_memory().unwrap(),
        1,
        PrincipalId::from_seed(1),
    )
    .unwrap()
}

#[tokio::test]
async fn metadata_round_trips_through_local_column() {
    let store = store();
    let session_id = SessionId::new();
    let metadata = serde_json::json!({
        "name": "Nightly digest",
        "color": "#3366ff",
        "kind": "monitor",
        "command": "/digest",
        "model": "llmsim-model",
        "isolated": true
    });

    let schedule = store
        .create_schedule_with_metadata(
            session_id,
            "summarize the day".to_string(),
            Some("0 9 * * *".to_string()),
            None,
            "UTC".to_string(),
            metadata.clone(),
        )
        .await
        .unwrap();

    let read_back = store.get_metadata(schedule.id).await.unwrap().unwrap();
    assert_eq!(read_back, metadata, "extra fields round-trip verbatim");

    // The core schedule primitive is unchanged (no metadata field on it).
    assert_eq!(schedule.description, "summarize the day");
    assert!(schedule.enabled);
}

#[tokio::test]
async fn list_cancel_and_count_active() {
    let store = store();
    let session_id = SessionId::new();
    let a = store
        .create_schedule(
            session_id,
            "task a".to_string(),
            None,
            Some(chrono::Utc::now()),
            "UTC".to_string(),
        )
        .await
        .unwrap();
    let _b = store
        .create_schedule(
            session_id,
            "task b".to_string(),
            Some("* * * * *".to_string()),
            None,
            "UTC".to_string(),
        )
        .await
        .unwrap();

    assert_eq!(store.list_schedules(session_id).await.unwrap().len(), 2);
    assert_eq!(store.count_active_schedules(session_id).await.unwrap(), 2);

    let canceled = store.cancel_schedule(session_id, a.id).await.unwrap();
    assert!(!canceled.enabled);
    assert_eq!(store.count_active_schedules(session_id).await.unwrap(), 1);
    // Both still listed; one disabled.
    assert_eq!(store.list_schedules(session_id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn cancel_preserves_metadata() {
    let store = store();
    let session_id = SessionId::new();
    let metadata = serde_json::json!({"name": "keep me"});
    let schedule = store
        .create_schedule_with_metadata(
            session_id,
            "x".to_string(),
            None,
            Some(chrono::Utc::now()),
            "UTC".to_string(),
            metadata.clone(),
        )
        .await
        .unwrap();
    store
        .cancel_schedule(session_id, schedule.id)
        .await
        .unwrap();
    assert_eq!(
        store.get_metadata(schedule.id).await.unwrap().unwrap(),
        metadata,
        "metadata survives cancel"
    );
}

#[tokio::test]
async fn schedules_are_org_scoped() {
    let db = SqliteDb::open_in_memory().unwrap();
    let org1 = LocalScheduleStore::new(db.clone(), 1, PrincipalId::from_seed(1)).unwrap();
    let org2 = LocalScheduleStore::new(db, 2, PrincipalId::from_seed(1)).unwrap();
    let session_id = SessionId::new();

    org1.create_schedule(
        session_id,
        "org1 task".to_string(),
        None,
        Some(chrono::Utc::now()),
        "UTC".to_string(),
    )
    .await
    .unwrap();

    assert_eq!(org1.list_schedules(session_id).await.unwrap().len(), 1);
    assert_eq!(
        org2.list_schedules(session_id).await.unwrap().len(),
        0,
        "org2 must not see org1 schedules"
    );
}
