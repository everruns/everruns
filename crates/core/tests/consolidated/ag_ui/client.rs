#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The HTTP client against a local mock AG-UI agent: the request it sends,
//! SSE framing, and the consumer pipeline over a real connection.

use std::fs;
use std::path::Path;

use everruns_core::ag_ui::client::{AgUiClient, ClientError};
use everruns_core::ag_ui::consumer::RunOutcome;
use everruns_core::ag_ui::{Event, Message, RunAgentInput};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// What the mock agent received.
struct Received {
    head: String,
    body: Value,
}

/// Serves one request: answers with `status`, `content_type` and `body`
/// verbatim, then closes.
async fn mock_agent(
    status: u16,
    content_type: &str,
    body: String,
) -> (String, oneshot::Receiver<Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/agent", listener.local_addr().unwrap());
    let content_type = content_type.to_owned();
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut raw = Vec::new();
        let mut buf = [0u8; 4096];
        let (head, body_start) = loop {
            let n = socket.read(&mut buf).await.unwrap();
            raw.extend_from_slice(&buf[..n]);
            if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                break (String::from_utf8_lossy(&raw[..i]).into_owned(), i + 4);
            }
        };
        let length: usize = head
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse().unwrap())
            })
            .unwrap_or(0);
        while raw.len() < body_start + length {
            let n = socket.read(&mut buf).await.unwrap();
            raw.extend_from_slice(&buf[..n]);
        }
        let request_body = serde_json::from_slice(&raw[body_start..body_start + length]).unwrap();
        let response = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\nconnection: close\r\n\r\n{body}"
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.ok();
        let _ = tx.send(Received {
            head,
            body: request_body,
        });
    });
    (url, rx)
}

fn sse(events: &[Value]) -> String {
    events
        .iter()
        .map(|e| format!("data: {e}\r\n\r\n"))
        .collect()
}

fn input() -> RunAgentInput {
    RunAgentInput {
        thread_id: "t1".into(),
        run_id: "r1".into(),
        messages: vec![Message::user("u1", "hello")],
        ..RunAgentInput::default()
    }
    .with_protocol_version()
}

#[tokio::test]
async fn run_posts_input_with_auth_and_assembles_result() {
    let body = sse(&[
        json!({ "type": "RUN_STARTED", "threadId": "t1", "runId": "r1", "protocolVersion": "1.0" }),
        json!({ "type": "TEXT_MESSAGE_CHUNK", "messageId": "m1", "delta": "Hel" }),
        json!({ "type": "TEXT_MESSAGE_CHUNK", "delta": "lo" }),
        json!({ "type": "FUTURE_EVENT" }),
        json!({ "type": "RUN_FINISHED", "threadId": "t1", "runId": "r1",
                "usage": [{ "provider": "p", "model": "m", "inputTokens": 3, "outputTokens": 1 }] }),
    ]);
    let (url, received) = mock_agent(200, "text/event-stream; charset=utf-8", body).await;
    let client = AgUiClient::new(url)
        .with_bearer_token("tok-123")
        .with_header("x-tenant", "acme")
        .unwrap();

    let mut stream = client.run(&input()).await.unwrap();
    let mut types = Vec::new();
    while let Some(event) = stream.next().await {
        types.push(event.unwrap().event_type());
    }
    assert_eq!(
        types,
        [
            "RUN_STARTED",
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
            "RUN_FINISHED"
        ]
    );
    let result = stream.result().unwrap().clone();
    assert_eq!(result.text(), "Hello");
    assert!(matches!(result.outcome, RunOutcome::Success { .. }));
    assert_eq!(result.usage[0].input_tokens, Some(3));
    assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);

    let received = received.await.unwrap();
    let head = received.head.to_ascii_lowercase();
    assert!(head.starts_with("post /agent "));
    assert!(head.contains("authorization: bearer tok-123"));
    assert!(head.contains("x-tenant: acme"));
    assert!(head.contains("accept: text/event-stream"));
    assert_eq!(received.body["threadId"], "t1");
    assert_eq!(received.body["protocolVersion"], "1.0");
    assert_eq!(received.body["messages"][0]["content"], "hello");
}

#[tokio::test]
async fn interrupt_outcome_is_reported() {
    let body = sse(&[
        json!({ "type": "RUN_STARTED", "threadId": "t1", "runId": "r1" }),
        json!({ "type": "RUN_FINISHED", "threadId": "t1", "runId": "r1",
                "outcome": { "type": "interrupt", "interrupts": [
                    { "id": "i1", "reason": "input_required", "message": "Which colour?" }
                ] } }),
    ]);
    let (url, _) = mock_agent(200, "text/event-stream", body).await;
    let result = AgUiClient::new(url)
        .run(&input())
        .await
        .unwrap()
        .into_result()
        .await
        .unwrap();
    assert_eq!(result.outcome, RunOutcome::Interrupted);
    assert_eq!(
        result.interrupts[0].message.as_deref(),
        Some("Which colour?")
    );
}

#[tokio::test]
async fn protocol_violation_ends_the_stream_with_an_error() {
    let body = sse(&[
        json!({ "type": "TEXT_MESSAGE_START", "messageId": "m1" }),
        json!({ "type": "RUN_STARTED", "threadId": "t1", "runId": "r1" }),
    ]);
    let (url, _) = mock_agent(200, "text/event-stream", body).await;
    let mut stream = AgUiClient::new(url).run(&input()).await.unwrap();
    let first = stream.next().await.unwrap();
    assert!(matches!(first, Err(ClientError::Protocol(_))));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn stream_ending_inside_a_run_is_an_error() {
    let body = sse(&[json!({ "type": "RUN_STARTED", "threadId": "t1", "runId": "r1" })]);
    let (url, _) = mock_agent(200, "text/event-stream", body).await;
    let err = AgUiClient::new(url)
        .run(&input())
        .await
        .unwrap()
        .into_result()
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("ended before the run finished"),
        "{err}"
    );
}

#[tokio::test]
async fn http_status_content_type_and_size_are_checked() {
    let (url, _) = mock_agent(401, "application/json", r#"{"error":"nope"}"#.into()).await;
    let err = AgUiClient::new(url).run(&input()).await.unwrap_err();
    assert!(
        matches!(err, ClientError::Status { status: 401, .. }),
        "{err}"
    );

    let (url, _) = mock_agent(200, "application/json", "{}".into()).await;
    let err = AgUiClient::new(url).run(&input()).await.unwrap_err();
    assert!(matches!(err, ClientError::ContentType(_)), "{err}");

    let body = format!("data: {}\n\n", "x".repeat(200));
    let (url, _) = mock_agent(200, "text/event-stream", body).await;
    let err = AgUiClient::new(url)
        .with_max_event_bytes(64)
        .run(&input())
        .await
        .unwrap()
        .into_result()
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::EventTooLarge { limit: 64 }),
        "{err}"
    );
}

/// A sample of the conformance corpus over real HTTP, so the SSE path and
/// the in-memory replay in `tests/conformance.rs` agree.
#[tokio::test]
async fn conformance_streams_over_http() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ag_ui/spec/1.0/conformance/streams");
    for name in [
        "conformant-run-is-quiet",
        "chunk-expansion-assembles",
        "two-sequential-runs-accumulate",
        "unknown-event-dropped",
        "open-message-at-run-finished-fatal",
        "malformed-known-field-fatal",
    ] {
        let fixture: Value =
            serde_json::from_str(&fs::read_to_string(dir.join(format!("{name}.json"))).unwrap())
                .unwrap();
        let (url, _) = mock_agent(
            200,
            "text/event-stream",
            sse(fixture["stream"].as_array().unwrap()),
        )
        .await;
        let result = AgUiClient::new(url)
            .run(&input())
            .await
            .unwrap()
            .into_result()
            .await;
        match fixture["expect"]["outcome"].as_str().unwrap() {
            "completed" => assert!(result.is_ok(), "{name}: {:?}", result.err()),
            _ => assert!(result.is_err(), "{name} must be rejected"),
        }
    }
}

#[test]
fn events_are_send() {
    fn assert_send<T: Send>() {}
    assert_send::<everruns_core::ag_ui::client::EventStream>();
    assert_send::<Event>();
}
