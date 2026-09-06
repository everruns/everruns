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
use std::time::Duration;

use bytes::Bytes;
use eventsource_stream::{Event, EventStreamError, Eventsource};
use futures::{Stream, StreamExt, stream};

use crate::error::AgentLoopError;
use crate::llm_retry::{LlmRetryConfig, RetryMetadata, remaining_retry_time, reserve_retry_wait};

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

/// Upper bound for the setup-time peek that enables safe pre-first-item
/// reconnects. The Reason atom starts its provider stall timer only after
/// `chat_completion_stream()` returns, so this setup wait must not inherit the
/// longer transport read timeout.
const FIRST_STREAM_ITEM_TIMEOUT: Duration = Duration::from_secs(120);

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

fn reconnect_budget_error(
    config: &LlmRetryConfig,
    driver: &str,
    metadata: &RetryMetadata,
) -> AgentLoopError {
    AgentLoopError::llm_kind(
        crate::error::LlmErrorKind::Unavailable,
        format!(
            "{driver} stream reconnect budget exhausted after {} retries over {:.1}s",
            metadata.attempts,
            config.max_retry_elapsed.as_secs_f64()
        ),
    )
    .with_retry_metadata(metadata)
}

fn reconnect_remaining(
    config: &LlmRetryConfig,
    driver: &str,
    metadata: &RetryMetadata,
    started: Option<tokio::time::Instant>,
) -> Result<Option<Duration>, AgentLoopError> {
    match started {
        None => Ok(None),
        Some(_) => match remaining_retry_time(config, started) {
            Some(remaining) if !remaining.is_zero() => Ok(Some(remaining)),
            _ => Err(reconnect_budget_error(config, driver, metadata)),
        },
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
/// provider that returns 200 but then sends no first frame is bounded by
/// `FIRST_STREAM_ITEM_TIMEOUT`, matching the Reason atom's default provider
/// stall timeout instead of the longer transport read timeout.
pub async fn connect_sse_with_reconnect<C, Fut>(
    retry_config: &LlmRetryConfig,
    driver_name: &str,
    mut connect: C,
) -> Result<(SseStream, RetryMetadata), AgentLoopError>
where
    C: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<(reqwest::Response, RetryMetadata), AgentLoopError>>,
{
    connect_sse_with_reconnect_timeout(
        retry_config,
        driver_name,
        &mut connect,
        FIRST_STREAM_ITEM_TIMEOUT,
    )
    .await
}

async fn connect_sse_with_reconnect_timeout<C, Fut>(
    retry_config: &LlmRetryConfig,
    driver_name: &str,
    connect: &mut C,
    first_item_timeout: Duration,
) -> Result<(SseStream, RetryMetadata), AgentLoopError>
where
    C: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<(reqwest::Response, RetryMetadata), AgentLoopError>>,
{
    let mut retry_metadata = RetryMetadata::default();
    let mut retry_started_at = None;
    loop {
        let connected = if let Some(remaining) =
            reconnect_remaining(retry_config, driver_name, &retry_metadata, retry_started_at)?
        {
            tokio::time::timeout(remaining, connect(retry_metadata.attempts))
                .await
                .map_err(|_| reconnect_budget_error(retry_config, driver_name, &retry_metadata))?
        } else {
            connect(retry_metadata.attempts).await
        };
        let (response, metadata) = connected?;
        if retry_started_at.is_none() && metadata.had_retries() {
            retry_started_at = tokio::time::Instant::now()
                .checked_sub(metadata.total_retry_elapsed)
                .or_else(|| Some(tokio::time::Instant::now()));
        }
        retry_metadata.absorb(metadata);
        let events = Box::pin(response.bytes_stream().eventsource());

        // Peek the first item: an immediate transport failure at stream open
        // (the observed "error decoding response body") lands here. Bound this
        // setup wait so a silent 200 response cannot sit outside the Reason
        // atom's provider stall timeout for the longer HTTP read timeout.
        let item_timeout =
            reconnect_remaining(retry_config, driver_name, &retry_metadata, retry_started_at)?
                .map_or(first_item_timeout, |remaining| {
                    remaining.min(first_item_timeout)
                });
        let (first, rest) = tokio::time::timeout(item_timeout, events.into_future())
            .await
            .map_err(|_| {
                let error = AgentLoopError::llm(format!(
                    "provider stream stall: no first event for {}s",
                    first_item_timeout.as_secs()
                ));
                if retry_metadata.had_retries() {
                    error.with_retry_metadata(&retry_metadata)
                } else {
                    error
                }
            })?;

        let reconnectable = matches!(&first, Some(Err(e)) if is_reconnectable_stream_error(e));
        if reconnectable && retry_metadata.attempts < retry_config.max_retries {
            let proposed_wait = retry_config.calculate_backoff(retry_metadata.attempts);
            let Some(wait) = reserve_retry_wait(retry_config, &mut retry_started_at, proposed_wait)
            else {
                return Err(reconnect_budget_error(
                    retry_config,
                    driver_name,
                    &retry_metadata,
                ));
            };
            retry_metadata.record_retry(wait, None);
            if let Some(Err(e)) = &first {
                tracing::warn!(
                    driver = driver_name,
                    attempt = retry_metadata.attempts,
                    max_retries = retry_config.max_retries,
                    wait_secs = wait.as_secs_f64(),
                    error = %e,
                    "streaming transport failed before first event; reconnecting"
                );
            }
            tokio::time::sleep(wait).await;
            continue;
        }

        if reconnectable {
            return Err(AgentLoopError::llm_kind(
                crate::error::LlmErrorKind::Unavailable,
                format!(
                    "{driver_name} stream transport failed after {} retries; the turn is safe to resume",
                    retry_metadata.attempts
                ),
            )
            .with_retry_metadata(&retry_metadata));
        }

        if retry_metadata.had_retries() {
            retry_metadata.total_retry_elapsed = retry_started_at
                .map(|started| started.elapsed())
                .unwrap_or_default();
            tracing::info!(
                driver = driver_name,
                retries = retry_metadata.attempts,
                "streaming reconnect succeeded"
            );
        }

        // Replay the peeked first item (0 or 1) ahead of the remaining stream.
        return Ok((Box::pin(stream::iter(first).chain(rest)), retry_metadata));
    }
}

/// Byte-stream analogue of [`connect_sse_with_reconnect`] for drivers that parse
/// SSE by hand over `reqwest::Response::bytes_stream()` (e.g. Gemini).
///
/// Reconnects when the *first* byte chunk is a reconnectable transport error and
/// attempts remain. Once any chunk has been forwarded the stream is committed
/// and errors pass through. Semantics, safety, and the first-item timeout note
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
    connect_bytes_with_reconnect_timeout(
        retry_config,
        driver_name,
        &mut connect,
        FIRST_STREAM_ITEM_TIMEOUT,
    )
    .await
}

async fn connect_bytes_with_reconnect_timeout<C, Fut>(
    retry_config: &LlmRetryConfig,
    driver_name: &str,
    connect: &mut C,
    first_item_timeout: Duration,
) -> Result<(ByteStream, RetryMetadata), AgentLoopError>
where
    C: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<(reqwest::Response, RetryMetadata), AgentLoopError>>,
{
    let mut retry_metadata = RetryMetadata::default();
    let mut retry_started_at = None;
    loop {
        let connected = if let Some(remaining) =
            reconnect_remaining(retry_config, driver_name, &retry_metadata, retry_started_at)?
        {
            tokio::time::timeout(remaining, connect(retry_metadata.attempts))
                .await
                .map_err(|_| reconnect_budget_error(retry_config, driver_name, &retry_metadata))?
        } else {
            connect(retry_metadata.attempts).await
        };
        let (response, metadata) = connected?;
        if retry_started_at.is_none() && metadata.had_retries() {
            retry_started_at = tokio::time::Instant::now()
                .checked_sub(metadata.total_retry_elapsed)
                .or_else(|| Some(tokio::time::Instant::now()));
        }
        retry_metadata.absorb(metadata);
        let bytes = Box::pin(response.bytes_stream());
        let item_timeout =
            reconnect_remaining(retry_config, driver_name, &retry_metadata, retry_started_at)?
                .map_or(first_item_timeout, |remaining| {
                    remaining.min(first_item_timeout)
                });
        let (first, rest) = tokio::time::timeout(item_timeout, bytes.into_future())
            .await
            .map_err(|_| {
                let error = AgentLoopError::llm(format!(
                    "provider stream stall: no first chunk for {}s",
                    first_item_timeout.as_secs()
                ));
                if retry_metadata.had_retries() {
                    error.with_retry_metadata(&retry_metadata)
                } else {
                    error
                }
            })?;

        let reconnectable = matches!(&first, Some(Err(e)) if is_reconnectable_reqwest_error(e));
        if reconnectable && retry_metadata.attempts < retry_config.max_retries {
            let proposed_wait = retry_config.calculate_backoff(retry_metadata.attempts);
            let Some(wait) = reserve_retry_wait(retry_config, &mut retry_started_at, proposed_wait)
            else {
                return Err(reconnect_budget_error(
                    retry_config,
                    driver_name,
                    &retry_metadata,
                ));
            };
            retry_metadata.record_retry(wait, None);
            if let Some(Err(e)) = &first {
                tracing::warn!(
                    driver = driver_name,
                    attempt = retry_metadata.attempts,
                    max_retries = retry_config.max_retries,
                    wait_secs = wait.as_secs_f64(),
                    error = %e,
                    "streaming transport failed before first chunk; reconnecting"
                );
            }
            tokio::time::sleep(wait).await;
            continue;
        }

        if reconnectable {
            return Err(AgentLoopError::llm_kind(
                crate::error::LlmErrorKind::Unavailable,
                format!(
                    "{driver_name} stream transport failed after {} retries; the turn is safe to resume",
                    retry_metadata.attempts
                ),
            )
            .with_retry_metadata(&retry_metadata));
        }

        if retry_metadata.had_retries() {
            retry_metadata.total_retry_elapsed = retry_started_at
                .map(|started| started.elapsed())
                .unwrap_or_default();
            tracing::info!(
                driver = driver_name,
                retries = retry_metadata.attempts,
                "streaming reconnect succeeded"
            );
        }

        return Ok((Box::pin(stream::iter(first).chain(rest)), retry_metadata));
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
        /// 200 + chunked headers, then no body bytes and no close.
        Silent,
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
                    Behavior::Silent => {
                        let _ = socket.flush().await;
                        std::future::pending::<()>().await;
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
            ..Default::default()
        }
    }

    async fn collect_via_reconnect(
        base: &str,
        config: &LlmRetryConfig,
    ) -> Result<Vec<SseItem>, AgentLoopError> {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
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
        .await?;
        Ok(stream.collect().await)
    }

    async fn connect_via_reconnect_with_timeout(
        base: &str,
        config: &LlmRetryConfig,
        first_item_timeout: Duration,
    ) -> Result<(SseStream, RetryMetadata), AgentLoopError> {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        connect_sse_with_reconnect_timeout(
            config,
            "test",
            &mut |_attempt| {
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
            },
            first_item_timeout,
        )
        .await
    }

    #[tokio::test]
    async fn reconnects_on_truncated_first_then_succeeds() {
        let (base, count) =
            spawn_scripted_sse_server(vec![Behavior::TruncateBeforeEvent, Behavior::FullSse]).await;
        let items = collect_via_reconnect(&base, &fast_config(2))
            .await
            .expect("reconnect succeeds");

        // Two connections: the truncated one and the successful reconnect.
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "should reconnect exactly once"
        );
        let texts: Vec<_> = items.into_iter().map(|item| item.unwrap().data).collect();
        assert_eq!(
            texts,
            [r#"{"choices":[{"delta":{"content":"hi"}}]}"#, "[DONE]"]
        );
    }

    #[tokio::test]
    async fn header_and_body_retries_share_one_attempt_budget() {
        let (base, count) =
            spawn_scripted_sse_server(vec![Behavior::TruncateBeforeEvent, Behavior::FullSse]).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let consumed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = consumed.clone();
        let (stream, metadata) = connect_sse_with_reconnect(&fast_config(2), "test", move |used| {
            observed.lock().unwrap().push(used);
            let client = client.clone();
            let base = base.clone();
            async move {
                let response = client
                    .get(base)
                    .send()
                    .await
                    .map_err(|error| AgentLoopError::llm(error.to_string()))?;
                let mut metadata = RetryMetadata::default();
                if used == 0 {
                    metadata.record_retry(Duration::ZERO, None);
                }
                Ok((response, metadata))
            }
        })
        .await
        .expect("shared budget should recover");
        let items: Vec<_> = stream.collect().await;

        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(*consumed.lock().unwrap(), vec![0, 2]);
        assert_eq!(metadata.attempts, 2);
        assert_eq!(
            items
                .into_iter()
                .map(|item| item.unwrap().data)
                .collect::<Vec<_>>(),
            [r#"{"choices":[{"delta":{"content":"hi"}}]}"#, "[DONE]"]
        );
    }

    #[tokio::test]
    async fn exhausts_reconnects_and_surfaces_error() {
        let (base, count) = spawn_scripted_sse_server(vec![Behavior::TruncateBeforeEvent]).await;
        let error = collect_via_reconnect(&base, &fast_config(2))
            .await
            .expect_err("reconnect budget should terminate");

        // 1 initial + 2 reconnects = 3 attempts, all truncated.
        assert_eq!(
            count.load(Ordering::SeqCst),
            3,
            "should exhaust max_retries"
        );
        assert!(error.llm_retry_handled());
        assert_eq!(error.llm_retry_attempts(), 2);
        assert_eq!(
            error.llm_error_kind(),
            Some(crate::error::LlmErrorKind::Unavailable)
        );
        assert_eq!(
            error.to_string(),
            "LLM error: test stream transport failed after 2 retries; the turn is safe to resume"
        );
    }

    #[tokio::test]
    async fn clean_stream_makes_single_connection() {
        let (base, count) = spawn_scripted_sse_server(vec![Behavior::FullSse]).await;
        let items = collect_via_reconnect(&base, &fast_config(2))
            .await
            .expect("clean stream succeeds");

        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "healthy stream must not reconnect"
        );
        assert_eq!(
            items
                .into_iter()
                .map(|item| item.unwrap().data)
                .collect::<Vec<_>>(),
            [r#"{"choices":[{"delta":{"content":"hi"}}]}"#, "[DONE]"]
        );
    }

    #[tokio::test]
    async fn silent_first_event_is_bounded_without_reconnect() {
        let (base, count) = spawn_scripted_sse_server(vec![Behavior::Silent]).await;

        let err = match connect_via_reconnect_with_timeout(
            &base,
            &fast_config(2),
            Duration::from_millis(20),
        )
        .await
        {
            Ok(_) => panic!("silent first event should fail at the setup-time bound"),
            Err(err) => err,
        };

        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "setup-time stall must not retry up to the transport read timeout"
        );
        assert_eq!(
            err.to_string(),
            "LLM error: provider stream stall: no first event for 0s"
        );
        assert_eq!(err.llm_retry_attempts(), 0);
        assert!(!err.llm_retry_handled());
    }

    #[tokio::test]
    async fn silent_first_chunk_is_bounded_without_reconnect() {
        let (base, count) = spawn_scripted_sse_server(vec![Behavior::Silent]).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let err = match connect_bytes_with_reconnect_timeout(
            &fast_config(2),
            "test",
            &mut |_attempt| {
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
            },
            Duration::from_millis(20),
        )
        .await
        {
            Ok(_) => panic!("silent first chunk should fail at the setup-time bound"),
            Err(err) => err,
        };

        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "setup-time stall must not retry up to the transport read timeout"
        );
        assert_eq!(
            err.to_string(),
            "LLM error: provider stream stall: no first chunk for 0s"
        );
        assert_eq!(err.llm_retry_attempts(), 0);
        assert!(!err.llm_retry_handled());
    }

    #[tokio::test]
    async fn error_after_first_event_passes_through_without_reconnect() {
        // The first item is a good event (committed); the following truncation
        // must NOT trigger a reconnect (that would duplicate emitted output).
        let (base, count) = spawn_scripted_sse_server(vec![Behavior::EventThenTruncate]).await;
        let items = collect_via_reconnect(&base, &fast_config(2))
            .await
            .expect("first event commits stream");

        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "committed stream must not reconnect after emitting an event"
        );
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].as_ref().unwrap().data,
            r#"{"choices":[{"delta":{"content":"hi"}}]}"#
        );
        assert!(matches!(&items[1], Err(EventStreamError::Transport(_))));
    }

    #[tokio::test]
    async fn malformed_sse_is_returned_without_reconnection() {
        let mut calls = 0;
        let (mut events, metadata) = connect_sse_with_reconnect(&fast_config(2), "test", |_| {
            calls += 1;
            async {
                Ok((
                    reqwest::Response::from(http::Response::new(reqwest::Body::from(vec![
                        0xff, 0xfe,
                    ]))),
                    RetryMetadata::default(),
                ))
            }
        })
        .await
        .unwrap();
        let error = events.next().await.unwrap().unwrap_err();
        assert!(matches!(error, EventStreamError::Utf8(_)));
        assert!(!is_reconnectable_stream_error(&error));
        assert_eq!(calls, 1);
        assert_eq!(metadata.attempts, 0);
    }

    #[tokio::test]
    async fn committed_byte_stream_preserves_first_chunk_and_transport_failure() {
        let mut calls = 0;
        let (mut bytes, metadata) = connect_bytes_with_reconnect(&fast_config(2), "test", |_| {
            calls += 1;
            async {
                let chunks = stream::iter([
                    Ok(Bytes::from_static(b"committed")),
                    Err(std::io::Error::other("truncated fixture")),
                ]);
                Ok((
                    reqwest::Response::from(http::Response::new(reqwest::Body::wrap_stream(
                        chunks,
                    ))),
                    RetryMetadata::default(),
                ))
            }
        })
        .await
        .unwrap();
        assert_eq!(
            bytes.next().await.unwrap().unwrap(),
            Bytes::from_static(b"committed")
        );
        let error = bytes.next().await.unwrap().unwrap_err();
        assert!(is_reconnectable_reqwest_error(&error));
        assert!(bytes.next().await.is_none());
        assert_eq!(calls, 1);
        assert_eq!(metadata.attempts, 0);
    }

    fn synthetic_response(fail: bool) -> reqwest::Response {
        let body = if fail {
            reqwest::Body::wrap_stream(stream::iter([Err::<Bytes, _>(std::io::Error::other(
                "truncated fixture",
            ))]))
        } else {
            reqwest::Body::from("data: first\n\ndata: second\n\n")
        };
        reqwest::Response::from(http::Response::builder().status(200).body(body).unwrap())
    }

    #[tokio::test(start_paused = true)]
    async fn exhausted_header_budget_cannot_restart_the_first_item_timeout() {
        for sse in [true, false] {
            let config = LlmRetryConfig {
                max_retry_elapsed: Duration::from_secs(2),
                ..fast_config(3)
            };
            let mut connect = |_| async {
                Ok((
                    synthetic_response(false),
                    RetryMetadata {
                        attempts: 1,
                        total_retry_elapsed: Duration::from_secs(3),
                        ..Default::default()
                    },
                ))
            };
            let result = if sse {
                connect_sse_with_reconnect_timeout(
                    &config,
                    "test",
                    &mut connect,
                    Duration::from_secs(120),
                )
                .await
                .map(|_| ())
            } else {
                connect_bytes_with_reconnect_timeout(
                    &config,
                    "test",
                    &mut connect,
                    Duration::from_secs(120),
                )
                .await
                .map(|_| ())
            };
            let error =
                result.expect_err("header retry elapsed time must not become a fresh timeout");
            assert_eq!(
                error.llm_error_kind(),
                Some(crate::error::LlmErrorKind::Unavailable)
            );
            assert_eq!(error.llm_retry_attempts(), 1);
            assert!(error.llm_retry_handled());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn late_reconnect_wakeup_does_not_start_another_connection() {
        for sse in [true, false] {
            let calls = Arc::new(AtomicU32::new(0));
            let observed = calls.clone();
            let task = tokio::spawn(async move {
                let config = LlmRetryConfig {
                    initial_backoff: Duration::from_secs(1),
                    max_backoff: Duration::from_secs(1),
                    max_retry_elapsed: Duration::from_secs(2),
                    ..fast_config(3)
                };
                let mut connect = move |_| {
                    let n = observed.fetch_add(1, Ordering::SeqCst);
                    async move {
                        if n == 0 {
                            Ok((synthetic_response(true), RetryMetadata::default()))
                        } else {
                            std::future::pending().await
                        }
                    }
                };
                if sse {
                    connect_sse_with_reconnect_timeout(
                        &config,
                        "test",
                        &mut connect,
                        Duration::from_secs(120),
                    )
                    .await
                    .map(|_| ())
                } else {
                    connect_bytes_with_reconnect_timeout(
                        &config,
                        "test",
                        &mut connect,
                        Duration::from_secs(120),
                    )
                    .await
                    .map(|_| ())
                }
            });
            tokio::task::yield_now().await;
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            tokio::time::advance(Duration::from_secs(3)).await;
            tokio::task::yield_now().await;
            assert!(
                task.is_finished(),
                "expired reconnect must terminate, SSE={sse}"
            );
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            let error = task.await.unwrap().unwrap_err();
            assert_eq!(error.llm_retry_attempts(), 1);
            assert!(error.llm_retry_handled());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn successful_reconnect_reports_elapsed_recovery_and_replays_all_items() {
        for sse in [true, false] {
            let config = LlmRetryConfig {
                initial_backoff: Duration::from_secs(1),
                max_backoff: Duration::from_secs(1),
                ..fast_config(2)
            };
            let mut calls = 0;
            let mut connect = |_| {
                calls += 1;
                let failed = calls == 1;
                async move { Ok((synthetic_response(failed), RetryMetadata::default())) }
            };
            let metadata = if sse {
                let (stream, metadata) = connect_sse_with_reconnect(&config, "test", &mut connect)
                    .await
                    .unwrap();
                let items: Vec<_> = stream.map(|item| item.unwrap().data).collect().await;
                assert_eq!(items, ["first", "second"]);
                metadata
            } else {
                let (stream, metadata) =
                    connect_bytes_with_reconnect(&config, "test", &mut connect)
                        .await
                        .unwrap();
                let chunks: Vec<_> = stream.map(|item| item.unwrap()).collect().await;
                assert_eq!(chunks.concat(), b"data: first\n\ndata: second\n\n");
                metadata
            };
            assert_eq!(calls, 2);
            assert_eq!(metadata.attempts, 1);
            assert_eq!(metadata.total_retry_wait, Duration::from_secs(1));
            assert_eq!(metadata.total_retry_elapsed, Duration::from_secs(1));
        }
    }
}
