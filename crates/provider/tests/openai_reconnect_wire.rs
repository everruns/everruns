// Driver-level reconnect test for the OpenAI Chat Completions protocol driver.
//
// The golden wire tests (`openai_protocol_wire.rs`) use wiremock, which always
// serves a complete body and so cannot exercise the mid-stream transport flake
// (`error decoding response body`) that `stream_reconnect` recovers from. This
// test drives the real `OpenAIProtocolChatDriver` against a raw TCP server that
// *truncates* the first response body before any SSE event, then serves a full
// stream on the reconnect — proving the driver transparently reconnects and
// still yields the expected events.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use everruns_provider::OpenAIProtocolChatDriver;
use everruns_provider::driver_registry::{
    LlmCallConfig, LlmMessage, LlmMessageRole, LlmResponseStream, LlmStreamEvent,
};
use everruns_provider::llm_retry::LlmRetryConfig;
use everruns_provider::{BearerAuth, Provider};
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn config(model: &str) -> LlmCallConfig {
    LlmCallConfig {
        speed: None,
        verbosity: None,
        model: model.to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![],
        reasoning_effort: None,
        metadata: std::collections::HashMap::new(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: None,
        openrouter_routing: None,
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
        reasoning_state: None,
    }
}

fn chunk(body: &str) -> String {
    format!("{:x}\r\n{}\r\n", body.len(), body)
}

/// Full, valid OpenAI SSE stream: one content delta, a stop finish chunk, a
/// usage chunk, then `[DONE]`, framed as HTTP/1.1 chunked transfer encoding.
fn full_sse_response() -> String {
    let events = [
        r#"data: {"id":"c1","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
        "",
        r#"data: {"id":"c1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        "",
        r#"data: {"id":"c1","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#,
        "",
        "data: [DONE]",
        "",
        "",
    ]
    .join("\n");

    let mut resp = String::from(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
    );
    resp.push_str(&chunk(&events));
    resp.push_str("0\r\n\r\n");
    resp
}

/// Spawn a server that truncates the body of the first `truncate_count`
/// connections (headers + an incomplete chunk, then close) and serves a full
/// SSE stream on every connection after that. Returns its base URL and the
/// accepted-connection counter.
async fn spawn_flaky_openai_server(truncate_count: u32) -> (String, Arc<AtomicU32>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let counter = Arc::new(AtomicU32::new(0));
    let counter_task = Arc::clone(&counter);

    tokio::spawn(async move {
        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => break,
            };
            let n = counter_task.fetch_add(1, Ordering::SeqCst);

            // Drain the request head so the client's write completes.
            let mut buf = [0u8; 2048];
            let _ = socket.read(&mut buf).await;

            if n < truncate_count {
                // 200 headers, then a chunk that declares 5 bytes but sends 2,
                // then close: an incomplete body with no decodable SSE event.
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nda",
                    )
                    .await;
            } else {
                let _ = socket.write_all(full_sse_response().as_bytes()).await;
            }
            let _ = socket.flush().await;
            // Drop `socket` to close the connection.
        }
    });

    (format!("http://{addr}/v1/chat/completions"), counter)
}

/// Zero-backoff retry config so the reconnect does not actually sleep.
fn fast_retry() -> LlmRetryConfig {
    LlmRetryConfig {
        max_retries: 2,
        initial_backoff: Duration::from_millis(0),
        max_backoff: Duration::from_millis(0),
        backoff_multiplier: 1.0,
        jitter_factor: 0.0,
        ..Default::default()
    }
}

async fn drain(mut stream: LlmResponseStream) -> (Vec<String>, bool) {
    let mut texts = Vec::new();
    let mut saw_done = false;
    while let Some(item) = stream.next().await {
        match item.expect("driver stream item is Result-ok") {
            LlmStreamEvent::TextDelta(t) if !t.is_empty() => texts.push(t),
            LlmStreamEvent::Done(_) => saw_done = true,
            LlmStreamEvent::Error(e) => panic!("unexpected stream error: {e}"),
            _ => {}
        }
    }
    (texts, saw_done)
}

/// The first connection truncates before any event; the driver must reconnect
/// and stream the full response from the second connection.
#[tokio::test]
async fn driver_reconnects_on_truncated_first_response() {
    let (url, count) = spawn_flaky_openai_server(1).await;
    let driver = Provider::new(
        "openai-reconnect-test",
        OpenAIProtocolChatDriver::new().with_retry_config(fast_retry()),
    )
    .base_url(url)
    .auth(BearerAuth::new("test-key"));

    let stream = driver
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &config("gpt-5.2"),
        )
        .await
        .expect("stream should start (reconnect happens before first event)");

    let (texts, saw_done) = drain(stream).await;

    assert_eq!(
        count.load(Ordering::SeqCst),
        2,
        "driver should reconnect once"
    );
    assert_eq!(texts, vec!["Hi".to_string()], "content from the reconnect");
    assert!(saw_done, "stream should complete with a Done event");
}

/// When every attempt truncates, the driver exhausts its reconnect budget and
/// surfaces the transport error instead of hanging or panicking.
#[tokio::test]
async fn driver_surfaces_error_after_exhausting_reconnects() {
    // Always truncate. With max_retries = 2 that is 1 + 2 = 3 attempts.
    let (url, count) = spawn_flaky_openai_server(u32::MAX).await;
    let driver = Provider::new(
        "openai-reconnect-test",
        OpenAIProtocolChatDriver::new().with_retry_config(fast_retry()),
    )
    .base_url(url)
    .auth(BearerAuth::new("test-key"));

    let result = driver
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &config("gpt-5.2"),
        )
        .await;
    let error = match result {
        Ok(_) => panic!("pre-first-event reconnect exhaustion must fail stream setup"),
        Err(error) => error,
    };

    assert_eq!(count.load(Ordering::SeqCst), 3, "1 initial + 2 reconnects");
    assert!(error.is_transient_llm_error());
    assert_eq!(error.llm_retry_attempts(), 2);
    assert!(error.to_string().contains("safe to resume"));
}
