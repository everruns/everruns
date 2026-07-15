// Transparent reconnect for streaming (SSE) LLM responses.
//
// The per-request retry in [`crate::llm_retry::retry_request`] only covers the
// phase up to receiving response *headers*: it retries connect/TLS/send failures
// and retryable HTTP statuses (429/5xx). Once the body starts streaming, a
// transport failure — `error decoding response body`, a truncated/incomplete
// body, a connection reset mid-stream, a stale pooled keep-alive connection the
// peer closed, or a read-timeout before the first token — is surfaced by reqwest
// *after* a 200 and was previously fatal to the turn with no retry.
//
// The official OpenAI/Anthropic SDKs retry this connection-error class
// (`APIConnectionError`) with bounded exponential backoff. This module brings
// the native Rust drivers to parity: it reconnects when the SSE stream fails
// *before the first event is decoded*, which is exactly the window where these
// transport failures land (an immediate body-decode error). Once any event has
// been decoded and forwarded, the stream is "committed" and errors pass through
// unchanged — the consumer may already have acted on emitted deltas/tool-calls,
// so re-sending would duplicate output. Non-transport SSE errors (UTF-8/parser)
// are never reconnected, so a genuine protocol/driver bug is not masked (this is
// the failure mode a blanket "skip on transport-ish error" would hide).

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use eventsource_stream::{Event, EventStreamError, Eventsource};
use futures::{Stream, StreamExt, stream};

use crate::error::AgentLoopError;
use crate::llm_retry::{LlmRetryConfig, RetryMetadata};

/// SSE item type produced by `reqwest::Response::bytes_stream().eventsource()`.
pub type SseItem = Result<Event, EventStreamError<reqwest::Error>>;

/// A `'static` SSE event stream. Boxed so the returned stream is independent of
/// the `connect` closure's borrows (the concrete stream owns its
/// `reqwest::Response` and borrows nothing), letting callers build the
/// `'static` `LlmResponseStream` they need.
pub type SseStream = Pin<Box<dyn Stream<Item = SseItem> + Send>>;

/// A `'static` raw byte stream, for drivers that parse SSE by hand over
/// `reqwest::Response::bytes_stream()` (e.g. Gemini).
pub type ByteStream = Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>;

/// Classify a raw `reqwest` stream error as a transient transport failure that
/// is safe to reconnect: a body/decode error ("error decoding response body",
/// truncated/incomplete body, mid-body reset), a connect/request failure (a
/// stale pooled keep-alive connection discovered dead), or a read timeout before
/// the first byte. A status/redirect/builder error is not a mid-stream transport
/// flake and is not reconnected.
pub fn is_reconnectable_reqwest_error(err: &reqwest::Error) -> bool {
    err.is_body() || err.is_decode() || err.is_connect() || err.is_request() || err.is_timeout()
}

/// Classify an SSE stream error as a transient transport failure that is safe to
/// reconnect.
///
/// Only `Transport` (reqwest) errors qualify. A UTF-8 or parser error means the
/// bytes that *did* arrive were a malformed SSE payload — a real protocol error
/// we must surface, not paper over with a reconnect.
pub fn is_reconnectable_stream_error(err: &EventStreamError<reqwest::Error>) -> bool {
    match err {
        EventStreamError::Transport(e) => is_reconnectable_reqwest_error(e),
        EventStreamError::Utf8(_) | EventStreamError::Parser(_) => false,
    }
}

/// Establish an SSE event stream with transparent reconnect on a
/// pre-first-event transport failure.
///
/// `connect` performs one full send attempt — including
/// [`retry_request`](crate::llm_retry::retry_request)'s header-phase retries —
/// and returns the raw `reqwest::Response` plus its [`RetryMetadata`]. It is
/// invoked once per reconnect attempt; because it re-sends the identical request
/// and no body bytes have been consumed yet, retrying is safe/idempotent.
///
/// Reconnect fires only when the *first* SSE item is a reconnectable transport
/// error and attempts remain (bounded by `retry_config.max_retries`, with the
/// shared exponential backoff + jitter). The returned stream replays the peeked
/// first item at its head, so no events are lost.
///
/// Note: this awaits the first SSE frame before returning, so a caller's
/// `chat_completion_stream()` now resolves at first-token time rather than
/// lazily. End-to-end latency is unchanged (the caller immediately polls the
/// stream) and connection failures surface at a single, well-defined point. A
/// provider that returns 200 but then sends no first frame is bounded by the
/// streaming client's read timeout (`HTTP_STREAM_READ_TIMEOUT`), which then
/// classifies as a reconnectable timeout.
pub async fn connect_sse_with_reconnect<C, Fut>(
    retry_config: &LlmRetryConfig,
    driver_name: &str,
    mut connect: C,
) -> Result<(SseStream, RetryMetadata), AgentLoopError>
where
    C: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<(reqwest::Response, RetryMetadata), AgentLoopError>>,
{
    let mut attempt: u32 = 0;
    loop {
        let (response, metadata) = connect(attempt).await?;
        let events = Box::pin(response.bytes_stream().eventsource());

        // Peek the first item: an immediate transport failure at stream open
        // (the observed "error decoding response body") lands here.
        let (first, rest) = events.into_future().await;

        let reconnectable = matches!(&first, Some(Err(e)) if is_reconnectable_stream_error(e));
        if reconnectable && attempt < retry_config.max_retries {
            attempt += 1;
            let wait = retry_config.calculate_backoff(attempt - 1);
            if let Some(Err(e)) = &first {
                tracing::warn!(
                    driver = driver_name,
                    attempt,
                    max_retries = retry_config.max_retries,
                    wait_secs = wait.as_secs_f64(),
                    error = %e,
                    "streaming transport failed before first event; reconnecting"
                );
            }
            tokio::time::sleep(wait).await;
            continue;
        }

        if attempt > 0 && !reconnectable {
            tracing::info!(
                driver = driver_name,
                reconnects = attempt,
                "streaming reconnect succeeded"
            );
        }

        // Replay the peeked first item (0 or 1) ahead of the remaining stream.
        return Ok((Box::pin(stream::iter(first).chain(rest)), metadata));
    }
}

/// Byte-stream analogue of [`connect_sse_with_reconnect`] for drivers that parse
/// SSE by hand over `reqwest::Response::bytes_stream()` (e.g. Gemini).
///
/// Reconnects when the *first* byte chunk is a reconnectable transport error and
/// attempts remain. Once any chunk has been forwarded the stream is committed
/// and errors pass through. Semantics, safety, and the first-token latency note
/// match [`connect_sse_with_reconnect`].
pub async fn connect_bytes_with_reconnect<C, Fut>(
    retry_config: &LlmRetryConfig,
    driver_name: &str,
    mut connect: C,
) -> Result<(ByteStream, RetryMetadata), AgentLoopError>
where
    C: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<(reqwest::Response, RetryMetadata), AgentLoopError>>,
{
    let mut attempt: u32 = 0;
    loop {
        let (response, metadata) = connect(attempt).await?;
        let bytes = Box::pin(response.bytes_stream());
        let (first, rest) = bytes.into_future().await;

        let reconnectable = matches!(&first, Some(Err(e)) if is_reconnectable_reqwest_error(e));
        if reconnectable && attempt < retry_config.max_retries {
            attempt += 1;
            let wait = retry_config.calculate_backoff(attempt - 1);
            if let Some(Err(e)) = &first {
                tracing::warn!(
                    driver = driver_name,
                    attempt,
                    max_retries = retry_config.max_retries,
                    wait_secs = wait.as_secs_f64(),
                    error = %e,
                    "streaming transport failed before first chunk; reconnecting"
                );
            }
            tokio::time::sleep(wait).await;
            continue;
        }

        if attempt > 0 && !reconnectable {
            tracing::info!(
                driver = driver_name,
                reconnects = attempt,
                "streaming reconnect succeeded"
            );
        }

        return Ok((Box::pin(stream::iter(first).chain(rest)), metadata));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Per-connection server behavior for the scripted SSE test server.
    #[derive(Clone, Copy)]
    enum Behavior {
        /// 200 + chunked headers, then an incomplete chunk and abrupt close.
        /// reqwest yields an incomplete-body transport error as the first item.
        TruncateBeforeEvent,
        /// 200 + one complete SSE data event, then an incomplete chunk + close.
        /// First item is a good event (committed); the truncation follows.
        EventThenTruncate,
        /// 200 + a complete SSE stream (one content delta + `[DONE]`).
        FullSse,
    }

    const CONTENT_EVENT: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n";
    const DONE_EVENT: &str = "data: [DONE]\n\n";

    fn chunk(body: &str) -> String {
        format!("{:x}\r\n{}\r\n", body.len(), body)
    }

    /// Spawn a TCP server that serves `behaviors[n]` to the n-th connection
    /// (clamping to the last entry). Returns its base URL and a counter of
    /// accepted connections.
    async fn spawn_scripted_sse_server(behaviors: Vec<Behavior>) -> (String, Arc<AtomicU32>) {
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
                let idx = counter_task.fetch_add(1, Ordering::SeqCst) as usize;
                let behavior = behaviors[idx.min(behaviors.len() - 1)];

                // Drain the request head so the client's write side completes.
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;

                let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
                let _ = socket.write_all(headers.as_bytes()).await;

                match behavior {
                    Behavior::TruncateBeforeEvent => {
                        // Declare a 5-byte chunk but send 2 bytes, then close:
                        // an incomplete message with no decodable event.
                        let _ = socket.write_all(b"5\r\nda").await;
                    }
                    Behavior::EventThenTruncate => {
                        let _ = socket.write_all(chunk(CONTENT_EVENT).as_bytes()).await;
                        let _ = socket.write_all(b"5\r\nda").await;
                    }
                    Behavior::FullSse => {
                        let _ = socket.write_all(chunk(CONTENT_EVENT).as_bytes()).await;
                        let _ = socket.write_all(chunk(DONE_EVENT).as_bytes()).await;
                        let _ = socket.write_all(b"0\r\n\r\n").await;
                    }
                }
                let _ = socket.flush().await;
                // Drop `socket` to close the connection.
            }
        });

        (format!("http://{addr}/"), counter)
    }

    /// Zero-backoff config so reconnect tests don't actually sleep.
    fn fast_config(max_retries: u32) -> LlmRetryConfig {
        LlmRetryConfig {
            max_retries,
            initial_backoff: Duration::from_millis(0),
            max_backoff: Duration::from_millis(0),
            backoff_multiplier: 1.0,
            jitter_factor: 0.0,
        }
    }

    async fn collect_via_reconnect(base: &str, config: &LlmRetryConfig) -> Vec<SseItem> {
        let client = reqwest::Client::new();
        let (stream, _meta) = connect_sse_with_reconnect(config, "test", |_attempt| {
            let client = client.clone();
            let base = base.to_string();
            async move {
                let resp = client
                    .get(&base)
                    .send()
                    .await
                    .map_err(|e| AgentLoopError::llm(e.to_string()))?;
                Ok((resp, RetryMetadata::default()))
            }
        })
        .await
        .expect("connect should not terminally fail");
        stream.collect().await
    }

    #[tokio::test]
    async fn reconnects_on_truncated_first_then_succeeds() {
        let (base, count) =
            spawn_scripted_sse_server(vec![Behavior::TruncateBeforeEvent, Behavior::FullSse]).await;
        let items = collect_via_reconnect(&base, &fast_config(2)).await;

        // Two connections: the truncated one and the successful reconnect.
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "should reconnect exactly once"
        );
        // The reconnected stream yields the content event and [DONE], no error.
        let texts: Vec<String> = items
            .iter()
            .filter_map(|i| i.as_ref().ok())
            .map(|ev| ev.data.clone())
            .collect();
        assert!(
            texts.iter().any(|d| d.contains("hi")),
            "expected content event after reconnect, got {texts:?}"
        );
        assert!(
            items.iter().all(|i| i.is_ok()),
            "reconnected stream should carry no transport error"
        );
    }

    #[tokio::test]
    async fn exhausts_reconnects_and_surfaces_error() {
        let (base, count) = spawn_scripted_sse_server(vec![Behavior::TruncateBeforeEvent]).await;
        let items = collect_via_reconnect(&base, &fast_config(2)).await;

        // 1 initial + 2 reconnects = 3 attempts, all truncated.
        assert_eq!(
            count.load(Ordering::SeqCst),
            3,
            "should exhaust max_retries"
        );
        assert!(
            items.last().is_some_and(|i| i.is_err()),
            "exhausted stream should surface the transport error, got {items:?}"
        );
    }

    #[tokio::test]
    async fn clean_stream_makes_single_connection() {
        let (base, count) = spawn_scripted_sse_server(vec![Behavior::FullSse]).await;
        let items = collect_via_reconnect(&base, &fast_config(2)).await;

        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "healthy stream must not reconnect"
        );
        assert!(items.iter().all(|i| i.is_ok()));
        assert!(items.iter().filter_map(|i| i.as_ref().ok()).count() >= 1);
    }

    #[tokio::test]
    async fn error_after_first_event_passes_through_without_reconnect() {
        // The first item is a good event (committed); the following truncation
        // must NOT trigger a reconnect (that would duplicate emitted output).
        let (base, count) = spawn_scripted_sse_server(vec![Behavior::EventThenTruncate]).await;
        let items = collect_via_reconnect(&base, &fast_config(2)).await;

        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "committed stream must not reconnect after emitting an event"
        );
        assert!(
            items.first().is_some_and(|i| i.is_ok()),
            "first item should be the committed event"
        );
        assert!(
            items.last().is_some_and(|i| i.is_err()),
            "trailing transport error should pass through"
        );
    }

    #[test]
    fn classifier_rejects_non_transport_errors() {
        // A UTF-8 decode failure is a malformed payload, not a transport flake.
        let utf8_err = String::from_utf8(vec![0xff, 0xfe]).unwrap_err();
        let err: EventStreamError<reqwest::Error> = EventStreamError::Utf8(utf8_err);
        assert!(!is_reconnectable_stream_error(&err));
    }
}
