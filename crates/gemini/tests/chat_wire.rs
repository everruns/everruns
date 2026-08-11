// Golden-event wire tests for the Gemini streaming driver (EVE-672).
//
// These pin the exact `LlmStreamEvent` sequence the Gemini driver emits for a
// captured `streamGenerateContent` SSE response. They exist so the shared
// streaming refactor (EVE-672) can prove the driver's output is unchanged:
// change the conversion internals and these fixtures must still produce the
// same golden events.
//
// The driver talks to a wiremock server over its `base_url`, so no live Gemini
// credentials are needed. Gemini streams `data: {json}` SSE frames and ends the
// stream by closing the connection (no `[DONE]` marker), which the driver's
// finish handling collapses into a single terminal `Done` event.

use everruns_gemini::GeminiChatDriver;
use everruns_provider::driver_registry::{
    LlmCallConfig, LlmCompletionMetadata, LlmMessage, LlmMessageRole, LlmResponseStream,
    LlmStreamEvent,
};
use everruns_provider::{Provider, StaticHeaderAuth};
use futures::StreamExt;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
    }
}

fn provider(server: &MockServer) -> Provider {
    Provider::new("gemini-test", GeminiChatDriver::new())
        .base_url(server.uri())
        .auth(StaticHeaderAuth::new("x-goog-api-key", "test-key"))
}

/// Normalized, comparable form of an `LlmStreamEvent` for golden assertions.
/// Only the fields the driver populates deterministically are captured (usage
/// counts, finish reason, tool-call name/args), so the golden sequence is a
/// stable contract independent of `retry_metadata`/timing.
#[derive(Debug, PartialEq)]
enum Golden {
    Text(String),
    ToolCall {
        name: String,
        args: String,
    },
    Done {
        total: Option<u32>,
        prompt: Option<u32>,
        completion: Option<u32>,
        cache_read: Option<u32>,
        finish: Option<String>,
    },
    Error(String),
}

fn golden(event: LlmStreamEvent) -> Golden {
    match event {
        LlmStreamEvent::TextDelta(t) => Golden::Text(t),
        LlmStreamEvent::ToolCalls(calls) => {
            let tc = &calls[0];
            Golden::ToolCall {
                name: tc.name.clone(),
                args: tc.arguments.to_string(),
            }
        }
        LlmStreamEvent::Done(meta) => {
            let LlmCompletionMetadata {
                total_tokens,
                prompt_tokens,
                completion_tokens,
                cache_read_tokens,
                finish_reason,
                ..
            } = *meta;
            Golden::Done {
                total: total_tokens,
                prompt: prompt_tokens,
                completion: completion_tokens,
                cache_read: cache_read_tokens,
                finish: finish_reason,
            }
        }
        LlmStreamEvent::Error(e) => Golden::Error(e.to_string()),
        other => panic!("unexpected event variant in golden capture: {other:?}"),
    }
}

/// Drain a stream into its golden-event sequence, dropping the empty text
/// deltas the driver emits as no-op filler between meaningful events (these are
/// an internal artifact, not part of the observable contract).
async fn drain_golden(mut stream: LlmResponseStream) -> Vec<Golden> {
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        let event = item.expect("stream item should not be a transport error");
        let g = golden(event);
        if matches!(&g, Golden::Text(t) if t.is_empty()) {
            continue;
        }
        out.push(g);
    }
    out
}

async fn mount_sse(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path_regex(r"^/models/.+:streamGenerateContent$"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(server)
        .await;
}

/// A text-only completion: two content deltas, a STOP finish, and usage. The
/// golden sequence is the two text deltas followed by a single `Done` carrying
/// the disjoint token buckets (Gemini's promptTokenCount is cache-inclusive, so
/// the driver subtracts the cached count).
#[tokio::test]
async fn text_stream_golden_events() {
    let server = MockServer::start().await;
    let body = [
        r#"data: {"candidates":[{"content":{"parts":[{"text":"Hello"}]}}]}"#,
        "",
        r#"data: {"candidates":[{"content":{"parts":[{"text":", world"}]}}]}"#,
        "",
        r#"data: {"candidates":[{"content":{"parts":[]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":4,"cachedContentTokenCount":3}}"#,
        "",
        "",
    ]
    .join("\n");
    mount_sse(&server, body).await;

    let driver = provider(&server);
    let stream = driver
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &config("gemini-2.5-flash"),
        )
        .await
        .expect("gemini stream should start");

    let events = drain_golden(stream).await;
    assert_eq!(
        events,
        vec![
            Golden::Text("Hello".into()),
            Golden::Text(", world".into()),
            Golden::Done {
                total: Some(16), // prompt(12) + completion(4)
                prompt: Some(9), // 12 - 3 cached (disjoint convention)
                completion: Some(4),
                cache_read: Some(3),
                finish: Some("stop".into()),
            },
        ]
    );
}

/// A function-call completion: the driver accumulates the `functionCall` part
/// and, at the STOP finish, emits a single `ToolCalls` event (with a synthetic
/// `call_0` id). It drains the accumulator there but does not mark the stream
/// done, so the subsequent end-of-stream emits a terminal `Done` carrying the
/// usage. This `[ToolCalls, Done]` shape is the driver's current contract and
/// what the shared refactor must preserve.
#[tokio::test]
async fn function_call_stream_golden_events() {
    let server = MockServer::start().await;
    let body = [
        r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"get_weather","args":{"city":"Paris"}}}]}}]}"#,
        "",
        r#"data: {"candidates":[{"content":{"parts":[]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":20,"candidatesTokenCount":6}}"#,
        "",
        "",
    ]
    .join("\n");
    mount_sse(&server, body).await;

    let driver = provider(&server);
    let stream = driver
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "weather?")],
            &config("gemini-2.5-flash"),
        )
        .await
        .expect("gemini stream should start");

    let events = drain_golden(stream).await;
    assert_eq!(
        events,
        vec![
            Golden::ToolCall {
                name: "get_weather".into(),
                args: r#"{"city":"Paris"}"#.into(),
            },
            Golden::Done {
                total: Some(26), // prompt(20) + completion(6)
                prompt: Some(20),
                completion: Some(6),
                cache_read: None,
                finish: Some("stop".into()),
            },
        ]
    );
}

/// When the provider closes the stream without any finish chunk, the driver
/// still emits a terminal `Done` (end-of-stream fallback), defaulting the
/// finish reason to "stop".
#[tokio::test]
async fn eos_without_finish_reason_emits_done() {
    let server = MockServer::start().await;
    let body = [
        r#"data: {"candidates":[{"content":{"parts":[{"text":"partial"}]}}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":1}}"#,
        "",
        "",
    ]
    .join("\n");
    mount_sse(&server, body).await;

    let driver = provider(&server);
    let stream = driver
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &config("gemini-2.5-flash"),
        )
        .await
        .expect("gemini stream should start");

    let events = drain_golden(stream).await;
    assert_eq!(
        events,
        vec![
            Golden::Text("partial".into()),
            Golden::Done {
                total: Some(6),
                prompt: Some(5),
                completion: Some(1),
                cache_read: None,
                finish: Some("stop".into()),
            },
        ]
    );
}
