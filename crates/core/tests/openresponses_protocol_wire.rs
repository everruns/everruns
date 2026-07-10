// Golden-event wire tests for the Open Responses protocol driver
// (`OpenResponsesProtocolChatDriver`) — EVE-672.
//
// This driver backs the OpenAI (Responses API) and OpenRouter providers. The
// fixtures below pin the exact `LlmStreamEvent` sequence for text streaming and
// for a fragmented function call terminated by `response.output_item.done` and
// `response.completed`. They give the shared streaming refactor a golden
// contract to preserve.
//
// The SSE frames use the provider's `type`-tagged JSON events (the same shape
// exercised by the OpenRouter wire tests), so the fixtures stay readable while
// still driving the driver's real stream-conversion path end to end.

use everruns_core::OpenResponsesProtocolChatDriver;
use everruns_core::driver_registry::{
    ChatDriver, LlmCallConfig, LlmCompletionMetadata, LlmMessage, LlmMessageRole,
    LlmResponseStream, LlmStreamEvent,
};
use futures::StreamExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config(model: &str) -> LlmCallConfig {
    LlmCallConfig {
        speed: None,
        model: model.to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![],
        reasoning_effort: None,
        metadata: std::collections::HashMap::new(),
        previous_response_id: None,
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
        LlmStreamEvent::Error(e) => Golden::Error(e),
        other => panic!("unexpected event variant in golden capture: {other:?}"),
    }
}

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

async fn mount_sse(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(server)
        .await;
}

fn driver(server: &MockServer) -> OpenResponsesProtocolChatDriver {
    OpenResponsesProtocolChatDriver::with_base_url(
        "test-key",
        format!("{}/v1/responses", server.uri()),
    )
}

/// Text streaming: two output-text deltas then a `response.completed` carrying
/// usage. The golden output is the two text deltas plus a single `Done` with the
/// disjoint token buckets (the driver subtracts the cached-read subset from the
/// cache-inclusive input count).
#[tokio::test]
async fn text_stream_golden_events() {
    let server = MockServer::start().await;
    let body = [
        r#"data: {"type":"response.output_text.delta","delta":"Hello"}"#,
        "",
        r#"data: {"type":"response.output_text.delta","delta":", world"}"#,
        "",
        r#"data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","output":[],"usage":{"input_tokens":10,"output_tokens":2,"input_tokens_details":{"cached_tokens":4}}}}"#,
        "",
        "data: [DONE]",
        "",
        "",
    ]
    .join("\n");
    mount_sse(&server, body).await;

    let stream = driver(&server)
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &config("gpt-5-mini"),
        )
        .await
        .expect("stream should start");

    assert_eq!(
        drain_golden(stream).await,
        vec![
            Golden::Text("Hello".into()),
            Golden::Text(", world".into()),
            Golden::Done {
                total: Some(12), // input(10) + output(2)
                prompt: Some(6), // 10 - 4 cached
                completion: Some(2),
                cache_read: Some(4),
                finish: Some("stop".into()),
            },
        ]
    );
}

/// Function call: an `output_item.added` announces the call, arguments arrive
/// fragmented via `function_call_arguments.delta`, `output_item.done` flushes
/// the assembled `ToolCalls` event, and `response.completed` closes with a
/// `tool_calls` finish reason.
#[tokio::test]
async fn fragmented_function_call_golden_events() {
    let server = MockServer::start().await;
    let body = [
        r#"data: {"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"get_weather"}}"#,
        "",
        r#"data: {"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"city\":"}"#,
        "",
        r#"data: {"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"\"Paris\"}"}"#,
        "",
        r#"data: {"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"get_weather"}}"#,
        "",
        r#"data: {"type":"response.completed","response":{"id":"resp_2","status":"completed","output":[],"usage":{"input_tokens":15,"output_tokens":8}}}"#,
        "",
        "data: [DONE]",
        "",
        "",
    ]
    .join("\n");
    mount_sse(&server, body).await;

    let stream = driver(&server)
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "weather?")],
            &config("gpt-5-mini"),
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
                total: Some(23),
                prompt: Some(15),
                completion: Some(8),
                cache_read: None,
                finish: Some("tool_calls".into()),
            },
        ]
    );
}
