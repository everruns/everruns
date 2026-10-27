//! SSE cursor coverage: a client that holds no events must still receive
//! everything written before it subscribed.
//!
//! Regression: the chat UI opened the stream without a cursor when its initial
//! REST snapshot was empty (a brand-new thread). The server then started the
//! stream live, so the first user message — written between the snapshot and the
//! subscription — never reached the transcript, and the optimistic bubble for it
//! stayed pinned below every later message.
//!
//! Run with: cargo test -p everruns-server --lib --test contracts sse_replay_test::

use crate::test_harness;

use std::time::Duration;

use everruns_contracts::typed_id::TurnId;

use axum::http::StatusCode;
use everruns_server::domains::sessions::record::Session;
use serde_json::{Value, json};
use test_harness::TestServer;

const IDLE: Duration = Duration::from_secs(2);
const MAX_BYTES: usize = 64 * 1024;

async fn session_with_message(server: &TestServer, text: &str) -> Session {
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({ "harness_id": server.seed_base_harness_id }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let _: Value = server
        .post(
            &format!("/v1/sessions/{}/messages", session.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": text}]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    session
}

#[tokio::test]
async fn sse_after_sequence_zero_replays_events_written_before_subscribing() {
    let server = TestServer::in_memory().await;
    let session = session_with_message(&server, "replay me").await;

    let stream = server
        .get_stream_prefix(
            &format!("/v1/sessions/{}/sse?after_sequence=0", session.id),
            MAX_BYTES,
            IDLE,
        )
        .await;

    assert!(
        stream.contains("event: input.message"),
        "expected the earlier input.message to be replayed, got: {stream}"
    );
    assert!(
        stream.contains("replay me"),
        "expected the replayed message text, got: {stream}"
    );
}

#[tokio::test]
async fn sse_without_a_cursor_still_starts_live() {
    let server = TestServer::in_memory().await;
    let session = session_with_message(&server, "not replayed").await;

    let stream = server
        .get_stream_prefix(&format!("/v1/sessions/{}/sse", session.id), MAX_BYTES, IDLE)
        .await;

    assert!(
        stream.contains("event: connected"),
        "expected the connected frame, got: {stream}"
    );
    assert!(
        !stream.contains("not replayed"),
        "no cursor means no replay, got: {stream}"
    );
}

#[tokio::test]
async fn sse_rejects_since_id_combined_with_after_sequence() {
    let server = TestServer::in_memory().await;
    let session = session_with_message(&server, "hello").await;

    let events: Value = server
        .get(&format!("/v1/sessions/{}/events", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let first_id = events["data"][0]["id"].as_str().expect("event id");

    server
        .get(&format!(
            "/v1/sessions/{}/sse?since_id={first_id}&after_sequence=0",
            session.id
        ))
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    server
        .get(&format!(
            "/v1/sessions/{}/sse?after_sequence=-1",
            session.id
        ))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

/// Push can drop an event (EKS load test, 2026-10-26: a turn ended in the log
/// but its open stream never got `session.idled`). A stream inside an open turn
/// reads PG after a quiet spell and sends what push missed.
#[tokio::test]
async fn sse_catches_up_an_event_push_never_delivered() {
    let server = TestServer::in_memory().await;
    let session = session_with_message(&server, "open turn").await;

    // Written straight to the log, never published: the dropped push.
    let pool = server.pool.clone();
    let session_id = session.id.uuid();
    let writer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        sqlx::query(
            r#"
            INSERT INTO events (session_id, sequence, event_type, ts, context, data)
            SELECT $1, COALESCE(MAX(sequence), 0) + 1, 'session.idled', NOW(), '{}',
                   jsonb_build_object('turn_id', $2::text)
            FROM events WHERE session_id = $1
            "#,
        )
        .bind(session_id)
        .bind(TurnId::new().to_string())
        .execute(&pool)
        .await
        .expect("insert session.idled");
    });

    // Longer than the 5 s catch-up wait, so the stream is still open for it.
    let stream = server
        .get_stream_prefix(
            &format!("/v1/sessions/{}/sse?after_sequence=0", session.id),
            MAX_BYTES,
            Duration::from_secs(8),
        )
        .await;
    writer.await.unwrap();

    assert!(
        stream.contains("event: session.idled"),
        "expected the missed session.idled to be caught up, got: {stream}"
    );
}
