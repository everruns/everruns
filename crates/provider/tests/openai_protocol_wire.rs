// Golden-event wire tests for the OpenAI Chat Completions protocol driver
// (`OpenAIProtocolChatDriver`) — EVE-672.
//
// This driver backs Fireworks and Microsoft MAI, and its streamed tool-call
// accumulation is the target of the shared `StreamAccumulator` unification. The
// fixtures below pin the exact `LlmStreamEvent` sequence for text streaming,
// for tool calls whose arguments arrive fragmented across chunks, and for the
// EVE-522 edge case where an empty `content: ""` delta rides along with a
// `finish_reason: "tool_calls"` chunk. Refactors under these tests must keep the
// golden output byte-identical.

use everruns_provider::OpenAIProtocolChatDriver;
use everruns_provider::driver_registry::{
    LlmCallConfig, LlmCompletionMetadata, LlmMessage, LlmMessageRole, LlmResponseStream,
    LlmStreamEvent,
};
use everruns_provider::{BearerAuth, Provider};
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
        extra_headers: Vec::new(),
        cache_diagnostics: None,
        reasoning_state: None,
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
    Reasoning(String),
    ReasoningItem {
        text: Option<String>,
    },
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
        LlmStreamEvent::ReasoningDelta { delta, .. } => Golden::Reasoning(delta),
        LlmStreamEvent::ReasoningItem(part) => Golden::ReasoningItem {
            text: part.display_text(),
        },
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

async fn mount_sse(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(server)
        .await;
}

fn driver(server: &MockServer) -> Provider {
    Provider::new("openai-protocol-test", OpenAIProtocolChatDriver::new())
        .base_url(format!("{}/v1/chat/completions", server.uri()))
        .auth(BearerAuth::new("test-key"))
}

/// Text streaming: two content deltas, a `stop` finish chunk, then the usage
/// chunk and `[DONE]`. The golden sequence is the two text deltas plus a single
/// `Done` with the disjoint token buckets (OpenAI's prompt count is
/// cache-inclusive; the driver subtracts cached reads).
#[tokio::test]
async fn text_stream_golden_events() {
    let server = MockServer::start().await;
    let body = [
        r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#,
        "",
        r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":", world"},"finish_reason":null}]}"#,
        "",
        r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        "",
        r#"data: {"id":"chatcmpl-1","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12,"prompt_tokens_details":{"cached_tokens":4}}}"#,
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
            &config("gpt-5.2"),
        )
        .await
        .expect("stream should start");

    assert_eq!(
        drain_golden(stream).await,
        vec![
            Golden::Text("Hello".into()),
            Golden::Text(", world".into()),
            Golden::Done {
                total: Some(12), // prompt(10) + completion(2)
                prompt: Some(6), // 10 - 4 cached
                completion: Some(2),
                cache_read: Some(4),
                finish: Some("stop".into()),
            },
        ]
    );
}

/// Tool-call streaming where the function name and JSON arguments arrive
/// fragmented across three chunks, terminated by a `finish_reason: "tool_calls"`
/// chunk. The accumulator must reassemble the fragments into one parsed call and
/// emit a single `ToolCalls` event at the finish, followed by `Done`.
#[tokio::test]
async fn fragmented_tool_call_golden_events() {
    let server = MockServer::start().await;
    let body = [
        r#"data: {"id":"chatcmpl-2","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_abc","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}"#,
        "",
        r#"data: {"id":"chatcmpl-2","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":"}}]},"finish_reason":null}]}"#,
        "",
        r#"data: {"id":"chatcmpl-2","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"Paris\"}"}}]},"finish_reason":null}]}"#,
        "",
        r#"data: {"id":"chatcmpl-2","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        "",
        r#"data: {"id":"chatcmpl-2","choices":[],"usage":{"prompt_tokens":15,"completion_tokens":8,"total_tokens":23}}"#,
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
            &config("gpt-5.2"),
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

/// EVE-522 edge case: some OpenAI-compatible gateways send an empty
/// `content: ""` delta in the *same* chunk that carries
/// `finish_reason: "tool_calls"`. The empty content must not short-circuit the
/// finish handler, so the accumulated call is still flushed exactly once.
#[tokio::test]
async fn empty_content_with_tool_calls_finish_golden_events() {
    let server = MockServer::start().await;
    let body = [
        r#"data: {"id":"chatcmpl-3","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_x","function":{"name":"ping","arguments":"{}"}}]},"finish_reason":null}]}"#,
        "",
        r#"data: {"id":"chatcmpl-3","choices":[{"index":0,"delta":{"content":""},"finish_reason":"tool_calls"}]}"#,
        "",
        r#"data: {"id":"chatcmpl-3","choices":[],"usage":{"prompt_tokens":9,"completion_tokens":1,"total_tokens":10}}"#,
        "",
        "data: [DONE]",
        "",
        "",
    ]
    .join("\n");
    mount_sse(&server, body).await;

    let stream = driver(&server)
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "ping")],
            &config("gpt-5.2"),
        )
        .await
        .expect("stream should start");

    assert_eq!(
        drain_golden(stream).await,
        vec![
            Golden::ToolCall {
                name: "ping".into(),
                args: "{}".into(),
            },
            Golden::Done {
                total: Some(10),
                prompt: Some(9),
                completion: Some(1),
                cache_read: None,
                finish: Some("tool_calls".into()),
            },
        ]
    );
}

/// Caller-supplied headers reach the wire on the shared Chat Completions
/// protocol, override the driver's own value in place, and never carry a
/// connection-level header from config.
#[tokio::test]
async fn extra_headers_reach_the_wire() {
    let server = MockServer::start().await;
    mount_sse(&server, "data: [DONE]\n\n".to_string()).await;

    let mut call_config = config("gpt-5.2");
    call_config.extra_headers = vec![
        ("x-trace-id".to_string(), "trace-42".to_string()),
        (
            "Content-Type".to_string(),
            "application/vnd+json".to_string(),
        ),
        ("Host".to_string(), "elsewhere.example".to_string()),
    ];

    let stream = driver(&server)
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &call_config,
        )
        .await
        .expect("stream should start");
    let _ = drain_golden(stream).await;

    let requests = server.received_requests().await.expect("recorded requests");
    let request = requests.first().expect("one request");
    assert_eq!(
        request.headers.get("x-trace-id").unwrap().to_str().unwrap(),
        "trace-42"
    );
    let content_types: Vec<_> = request
        .headers
        .get_all("content-type")
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect();
    assert_eq!(content_types, vec!["application/vnd+json"]);
    assert_ne!(
        request.headers.get("host").unwrap().to_str().unwrap(),
        "elsewhere.example"
    );
}

/// Reasoning models reached over Chat Completions stream their reasoning on the
/// delta, not as content. Before this was parsed, DeepSeek-R1, Qwen, Groq and
/// Fireworks reasoning was received and discarded: the field was never read, so
/// the reasoning channel stayed empty and nothing was persisted to replay.
///
/// Vendors split between two names for the same field, so both must map.
#[tokio::test]
async fn chat_completions_reasoning_content_reaches_the_reasoning_channel() {
    for field in ["reasoning_content", "reasoning"] {
        let server = MockServer::start().await;
        let body = [
            format!(
                r#"data: {{"id":"chatcmpl-r","choices":[{{"index":0,"delta":{{"{field}":"weighing the options"}},"finish_reason":null}}]}}"#
            ),
            String::new(),
            r#"data: {"id":"chatcmpl-r","choices":[{"index":0,"delta":{"content":"Answer"},"finish_reason":null}]}"#.to_string(),
            String::new(),
            r#"data: {"id":"chatcmpl-r","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#.to_string(),
            String::new(),
            "data: [DONE]".to_string(),
            String::new(),
            String::new(),
        ]
        .join("\n");
        mount_sse(&server, body).await;

        let stream = driver(&server)
            .chat_completion_stream(
                vec![LlmMessage::text(LlmMessageRole::User, "think")],
                &config("deepseek-r1"),
            )
            .await
            .expect("stream should start");

        let events = drain_golden(stream).await;

        assert!(
            events
                .iter()
                .any(|g| matches!(g, Golden::Reasoning(t) if t == "weighing the options")),
            "`{field}` should reach the reasoning channel, got: {events:?}"
        );

        // The durable artifact is what survives the turn; a delta alone leaves
        // nothing persisted.
        assert!(
            events.iter().any(|g| matches!(
                g,
                Golden::ReasoningItem { text: Some(t) } if t == "weighing the options"
            )),
            "`{field}` should produce a durable reasoning artifact, got: {events:?}"
        );

        // Reasoning must not double as the answer: routed to the text channel
        // it would persist as the model's reply and replay as its own output.
        assert!(
            !events
                .iter()
                .any(|g| matches!(g, Golden::Text(t) if t.contains("weighing"))),
            "reasoning from `{field}` leaked into the answer text: {events:?}"
        );
    }
}
