//! Task wake-up NOTIFYs on a real database: a connection marked as sending
//! wake-ups over NATS writes runnable actors without a NOTIFY, an unmarked one
//! still notifies (migration `20261108233430_task_wakeups_over_nats`).

use crate::storage::StorageBackend;
use crate::storage::test_database::test_session_row;
use sqlx::Connection;
use sqlx::postgres::{PgConnection, PgListener};
use std::time::Duration;

const ORG: i64 = 2;
const MARK: &str = "SET everruns.task_wakeups = 'nats'";

async fn connection(db: &StorageBackend, marked: bool) -> PgConnection {
    let url = db.test_database_handle().expect("test database").url();
    let mut conn = PgConnection::connect(url).await.expect("connect");
    if marked {
        sqlx::query(MARK).execute(&mut conn).await.expect("mark");
    }
    conn
}

/// Makes a fresh session's actor runnable on `conn`, the way input does.
async fn make_runnable(db: &StorageBackend, conn: &mut PgConnection) {
    let id = db
        .create_session(test_session_row(ORG))
        .await
        .expect("create session")
        .id;
    sqlx::query(
        "INSERT INTO session_actors (session_id, org_id, inbox_seq, next_input_seq, runnable_since) \
         VALUES ($1, $2, 1, 1, NOW())",
    )
    .bind(id.uuid())
    .bind(ORG)
    .execute(&mut *conn)
    .await
    .expect("insert runnable actor");
}

async fn via_postgres(conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT public.task_wakeups_via_postgres()")
        .fetch_one(conn)
        .await
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
async fn a_connection_marked_for_nats_makes_an_actor_runnable_without_a_notify() {
    let db = StorageBackend::test_database();
    let url = db.test_database_handle().expect("test database").url();
    let mut listener = PgListener::connect(url).await.expect("listener");
    listener.listen("task_available").await.expect("listen");

    let mut marked = connection(&db, true).await;
    make_runnable(&db, &mut marked).await;
    // A marker sent after the write arrives first only if the write sent nothing.
    sqlx::query("SELECT pg_notify('task_available', 'marker')")
        .execute(&mut marked)
        .await
        .expect("marker");
    assert_eq!(next_payload(&mut listener).await, "marker");

    let mut unmarked = connection(&db, false).await;
    make_runnable(&db, &mut unmarked).await;
    assert_eq!(next_payload(&mut listener).await, "session_actor");
}

#[tokio::test]
async fn only_the_nats_mark_turns_postgres_wakeups_off() {
    let db = StorageBackend::test_database();
    let mut unmarked = connection(&db, false).await;
    assert!(via_postgres(&mut unmarked).await.expect("unmarked"));

    let mut other = connection(&db, false).await;
    sqlx::query("SET everruns.task_wakeups = 'postgres'")
        .execute(&mut other)
        .await
        .expect("set");
    assert!(via_postgres(&mut other).await.expect("other value"));

    let mut marked = connection(&db, true).await;
    assert!(!via_postgres(&mut marked).await.expect("marked"));
}
