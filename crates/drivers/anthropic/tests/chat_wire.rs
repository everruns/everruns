// Golden-event wire tests for the Anthropic streaming driver (EVE-672).
//
// These pin the exact `LlmStreamEvent` sequence the Anthropic driver emits for
// captured Messages-API SSE responses (text, extended thinking, and a
// fragmented tool_use). They give the shared streaming refactor a golden
// contract to preserve: the conversion internals may change, but these fixtures
// must still produce the same events.
//
// Anthropic frames each SSE event with an `event:` type line and a `data:` JSON
// line, and reports disjoint token buckets (input tokens are already
// cache-exclusive), so `prompt_tokens` passes through unchanged.

use everruns_anthropic::AnthropicChatDriver;
use everruns_provider::driver_registry::{
    LlmCallConfig, LlmCompletionMetadata, LlmMessage, LlmMessageRole, LlmResponseStream,
    LlmStreamEvent,
};
use everruns_provider::{Provider, StaticHeaderAuth};
use futures::StreamExt;
use wiremock::matchers::{method, path};
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

#[derive(Debug, PartialEq)]
enum Golden {
    Text(String),
    Thinking(String),
    ThinkingSignature(String),
    ToolCall {
        name: String,
        args: String,
    },
    Done {
        total: Option<u32>,
        prompt: Option<u32>,
        completion: Option<u32>,
        cache_read: Option<u32>,
        cache_creation: Option<u32>,
        finish: Option<String>,
    },
    Error(String),
}

fn golden(event: LlmStreamEvent) -> Golden {
    match event {
        LlmStreamEvent::TextDelta(t) => Golden::Text(t),
        LlmStreamEvent::ThinkingDelta(t) => Golden::Thinking(t),
        LlmStreamEvent::ThinkingSignature(s) => Golden::ThinkingSignature(s),
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
                cache_creation_tokens,
                finish_reason,
                ..
            } = *meta;
            Golden::Done {
                total: total_tokens,
                prompt: prompt_tokens,
                completion: completion_tokens,
                cache_read: cache_read_tokens,
                cache_creation: cache_creation_tokens,
                finish: finish_reason,
            }
        }
        LlmStreamEvent::Error(e) => Golden::Error(e.to_string()),
        other => panic!("unexpected event variant in golden capture: {other:?}"),
    }
}

/// Drain a stream into its golden-event sequence, dropping the empty text
/// deltas the driver emits as internal filler between meaningful events.
async fn drain_golden(mut stream: LlmResponseStream) -> Vec<Golden> {
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        let g = golden(item.expect("stream item should not be a transport error"));
        if matches!(&g, Golden::Text(t) if t.is_empty()) {
            continue;
        }
        out.push(g);
    }
    out
}

/// Frame an Anthropic SSE event: `event: <type>` + `data: <json>` + blank line.
fn sse_event(event: &str, data: &str) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

async fn mount_sse(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(server)
        .await;
}

fn driver(server: &MockServer) -> Provider {
    Provider::new("anthropic-test", AnthropicChatDriver::new())
        .base_url(format!("{}/v1/messages", server.uri()))
        .auth(StaticHeaderAuth::new("x-api-key", "test-key"))
}

/// Text streaming: `message_start` carries input/cache usage, two text deltas
/// stream, and `message_delta` + `message_stop` close with output tokens. The
/// golden output is the two text deltas plus a `Done` with disjoint buckets
/// (Anthropic reports input tokens already excluding cache reads).
#[tokio::test]
async fn text_stream_golden_events() {
    let server = MockServer::start().await;
    let body = [
        sse_event(
            "message_start",
            r#"{"type":"message_start","message":{"id":"msg_1","model":"claude","usage":{"input_tokens":10,"cache_read_input_tokens":4,"cache_creation_input_tokens":2}}}"#,
        ),
        sse_event(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        ),
        sse_event(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
        ),
        sse_event(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":", world"}}"#,
        ),
        sse_event(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
        ),
        sse_event(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#,
        ),
        sse_event("message_stop", r#"{"type":"message_stop"}"#),
    ]
    .concat();
    mount_sse(&server, body).await;

    let stream = driver(&server)
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &config("claude-sonnet-4-5"),
        )
        .await
        .expect("stream should start");

    assert_eq!(
        drain_golden(stream).await,
        vec![
            Golden::Text("Hello".into()),
            Golden::Text(", world".into()),
            Golden::Done {
                total: Some(15),  // input(10) + output(5)
                prompt: Some(10), // disjoint: input passes through
                completion: Some(5),
                cache_read: Some(4),
                cache_creation: Some(2),
                finish: Some("stop".into()),
            },
        ]
    );
}

/// Extended-thinking streaming: a thinking block emits `thinking_delta`s and a
/// terminal `signature_delta`, followed by a text block. The golden output
/// pins the thinking deltas, the signature, and the visible text.
#[tokio::test]
async fn thinking_stream_golden_events() {
    let server = MockServer::start().await;
    let body = [
        sse_event(
            "message_start",
            r#"{"type":"message_start","message":{"id":"msg_2","usage":{"input_tokens":8}}}"#,
        ),
        sse_event(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
        ),
        sse_event(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think"}}"#,
        ),
        sse_event(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-abc"}}"#,
        ),
        sse_event(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
        ),
        sse_event(
            "content_block_start",
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
        ),
        sse_event(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Answer"}}"#,
        ),
        sse_event(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":1}"#,
        ),
        sse_event(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}"#,
        ),
        sse_event("message_stop", r#"{"type":"message_stop"}"#),
    ]
    .concat();
    mount_sse(&server, body).await;

    let stream = driver(&server)
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "think")],
            &config("claude-sonnet-4-5"),
        )
        .await
        .expect("stream should start");

    assert_eq!(
        drain_golden(stream).await,
        vec![
            Golden::Thinking("Let me think".into()),
            Golden::ThinkingSignature("sig-abc".into()),
            Golden::Text("Answer".into()),
            Golden::Done {
                total: Some(15),
                prompt: Some(8),
                completion: Some(7),
                cache_read: None,
                cache_creation: None,
                finish: Some("stop".into()),
            },
        ]
    );
}

/// Tool-use streaming: a `tool_use` content block whose input JSON arrives
/// fragmented via `input_json_delta`, finalized at `content_block_stop`, then a
/// `message_delta` with `stop_reason: "tool_use"` emits the `ToolCalls` event.
#[tokio::test]
async fn fragmented_tool_use_golden_events() {
    let server = MockServer::start().await;
    let body = [
        sse_event(
            "message_start",
            r#"{"type":"message_start","message":{"id":"msg_3","usage":{"input_tokens":12}}}"#,
        ),
        sse_event(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"get_weather"}}"#,
        ),
        sse_event(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"city\":"}}"#,
        ),
        sse_event(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"Paris\"}"}}"#,
        ),
        sse_event(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
        ),
        sse_event(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":9}}"#,
        ),
        sse_event("message_stop", r#"{"type":"message_stop"}"#),
    ]
    .concat();
    mount_sse(&server, body).await;

    let stream = driver(&server)
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "weather?")],
            &config("claude-sonnet-4-5"),
        )
        .await
        .expect("stream should start");

    assert_eq!(
        drain_golden(stream).await,
        vec![
            Golden::ToolCall {
                name: "get_weather".into(),
                args: r#"{"city":"Paris"}"#.into(),
            },
            Golden::Done {
                total: Some(21),
                prompt: Some(12),
                completion: Some(9),
                cache_read: None,
                cache_creation: None,
                finish: Some("tool_calls".into()),
            },
        ]
    );
}
