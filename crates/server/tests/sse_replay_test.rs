//! SSE cursor coverage: a client that holds no events must still receive
//! everything written before it subscribed.
//!
//! Regression: the chat UI opened the stream without a cursor when its initial
//! REST snapshot was empty (a brand-new thread). The server then started the
//! stream live, so the first user message — written between the snapshot and the
//! subscription — never reached the transcript, and the optimistic bubble for it
//! stayed pinned below every later message.
//!
//! Run with: cargo test -p everruns-server --test sse_replay_test

mod test_harness;

use std::time::Duration;

use axum::http::StatusCode;
use everruns_platform::Session;
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
