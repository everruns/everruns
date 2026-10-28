//! Event writes on a real database.

use crate::storage::test_database::test_session_row;
use crate::storage::{CreateEventRow, StorageBackend};
use everruns_contracts::typed_id::SessionId;
use sqlx::postgres::PgListener;
use std::time::Duration;

async fn session(db: &StorageBackend) -> SessionId {
    db.create_session(test_session_row(everruns_core::DEFAULT_ORG_ID))
        .await
        .expect("create session")
        .id
}

fn row(session_id: SessionId, event_type: &str) -> CreateEventRow {
    CreateEventRow {
        session_id,
        event_type: event_type.to_string(),
        ts: chrono::Utc::now(),
        context: serde_json::json!({}),
        data: serde_json::json!({}),
        metadata: None,
        tags: None,
    }
}

async fn listener(db: &StorageBackend) -> PgListener {
    let url = db.test_database_handle().expect("test database").url();
    let mut listener = PgListener::connect(url).await.expect("connect listener");
    listener.listen("event_available").await.expect("listen");
    listener
}

async fn next_payload(listener: &mut PgListener) -> String {
    tokio::time::timeout(Duration::from_secs(5), listener.recv())
        .await
        .expect("a notification within 5s")
        .expect("receive")
        .payload()
        .to_string()
}

#[tokio::test]
async fn event_writes_send_no_notify() {
    let db = StorageBackend::test_database();
    let id = session(&db).await;
    let mut listener = listener(&db).await;

    db.create_events(vec![row(id, "turn.started"), row(id, "reason.started")])
        .await
        .expect("batch");
    db.create_event(row(id, "turn.completed"))
        .await
        .expect("single");
    sqlx::query("SELECT pg_notify('event_available', 'marker')")
        .execute(db.pool())
        .await
        .expect("marker");

    assert_eq!(
        next_payload(&mut listener).await,
        "marker",
        "no event write may NOTIFY: each one would take the database-wide \
         notify lock at commit, and nothing listens"
    );
}
