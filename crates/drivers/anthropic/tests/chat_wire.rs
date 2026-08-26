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
    CacheDiagnosticsConfig, LlmCallConfig, LlmCompletionMetadata, LlmMessage, LlmMessageRole,
    LlmResponseStream, LlmStreamEvent,
};
use everruns_provider::model::ReasoningEffort;
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
        extra_headers: Vec::new(),
        cache_diagnostics: None,
    }
}

#[derive(Debug, PartialEq)]
enum Golden {
    Text(String),
    Thinking(String),
    /// One completed reasoning block: its text paired with the signature that
    /// signs *that* block.
    ReasoningItem {
        text: Option<String>,
        signature: Option<String>,
        redacted: bool,
    },
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
        LlmStreamEvent::ReasoningDelta { delta, .. } => Golden::Thinking(delta),
        LlmStreamEvent::ReasoningItem(item) => Golden::ReasoningItem {
            text: match &item.text {
                Some(everruns_provider::reasoning::ReasoningText::Plain { text }) => {
                    Some(text.clone())
                }
                _ => None,
            },
            signature: item.signature.clone(),
            redacted: matches!(
                item.text,
                Some(everruns_provider::reasoning::ReasoningText::Redacted)
            ),
        },
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
            Golden::ReasoningItem {
                text: Some("Let me think".into()),
                signature: Some("sig-abc".into()),
                redacted: false,
            },
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

/// Cache diagnostics (`cache-diagnosis` beta) and caller-supplied headers.
///
/// The request must carry the beta flag, an explicit `diagnostics` object with
/// the previous message id, and the caller's extra headers — with an extra
/// header replacing (not duplicating) a header the driver already set. The
/// response's `diagnostics` payload and message id must land on the completion
/// metadata so the next turn can chain `previous_message_id`.
#[tokio::test]
async fn cache_diagnostics_and_extra_headers_round_trip() {
    let server = MockServer::start().await;
    let body = [
        sse_event(
            "message_start",
            r#"{"type":"message_start","message":{"id":"msg_2","model":"claude","usage":{"input_tokens":10},"diagnostics":{"divergence":{"location":"tools"}}}}"#,
        ),
        sse_event(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#,
        ),
        sse_event("message_stop", r#"{"type":"message_stop"}"#),
    ]
    .concat();
    mount_sse(&server, body).await;

    let mut call_config = config("claude-sonnet-4-5");
    call_config.cache_diagnostics = Some(CacheDiagnosticsConfig {
        enabled: true,
        previous_message_id: Some("msg_1".to_string()),
    });
    call_config.extra_headers = vec![
        ("x-trace-id".to_string(), "trace-42".to_string()),
        ("anthropic-version".to_string(), "2099-01-01".to_string()),
        ("Host".to_string(), "elsewhere.example".to_string()),
    ];

    let mut stream = driver(&server)
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &call_config,
        )
        .await
        .expect("stream should start");

    let mut metadata = None;
    while let Some(item) = stream.next().await {
        if let LlmStreamEvent::Done(meta) = item.expect("no transport error") {
            metadata = Some(*meta);
        }
    }
    let metadata = metadata.expect("stream should finish with Done");
    assert_eq!(metadata.response_id.as_deref(), Some("msg_2"));
    assert_eq!(
        metadata.cache_diagnostics,
        Some(serde_json::json!({"divergence": {"location": "tools"}}))
    );

    let requests = server.received_requests().await.expect("recorded requests");
    let request = requests.first().expect("one request");

    let beta = request
        .headers
        .get("anthropic-beta")
        .expect("beta header")
        .to_str()
        .unwrap();
    assert!(
        beta.contains("cache-diagnosis-2026-04-07"),
        "beta header should opt into diagnostics, got {beta}"
    );
    assert_eq!(
        request.headers.get("x-trace-id").unwrap().to_str().unwrap(),
        "trace-42"
    );
    // An extra header overrides the driver's own value exactly once.
    let versions: Vec<_> = request
        .headers
        .get_all("anthropic-version")
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect();
    assert_eq!(versions, vec!["2099-01-01"]);
    // Connection-level headers are never taken from the caller.
    assert_ne!(
        request.headers.get("host").unwrap().to_str().unwrap(),
        "elsewhere.example"
    );

    let sent: serde_json::Value = serde_json::from_slice(&request.body).expect("json body");
    assert_eq!(sent["diagnostics"]["previous_message_id"], "msg_1");
}

/// Opting in without a prior message must send `previous_message_id: null`
/// rather than omitting the field — that is how a first turn opts in.
#[tokio::test]
async fn cache_diagnostics_first_turn_sends_explicit_null() {
    let server = MockServer::start().await;
    mount_sse(
        &server,
        sse_event("message_stop", r#"{"type":"message_stop"}"#),
    )
    .await;

    let mut call_config = config("claude-sonnet-4-5");
    call_config.cache_diagnostics = Some(CacheDiagnosticsConfig {
        enabled: true,
        previous_message_id: None,
    });

    let stream = driver(&server)
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &call_config,
        )
        .await
        .expect("stream should start");
    let _ = drain_golden(stream).await;

    let requests = server.received_requests().await.expect("recorded requests");
    let sent: serde_json::Value =
        serde_json::from_slice(&requests.first().expect("one request").body).expect("json body");
    assert!(sent["diagnostics"].is_object());
    assert!(sent["diagnostics"]["previous_message_id"].is_null());
}

/// A request that does not opt in sends no `diagnostics` object and no
/// diagnostics beta flag.
#[tokio::test]
async fn cache_diagnostics_absent_when_not_requested() {
    let server = MockServer::start().await;
    mount_sse(
        &server,
        sse_event("message_stop", r#"{"type":"message_stop"}"#),
    )
    .await;

    let stream = driver(&server)
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &config("claude-sonnet-4-5"),
        )
        .await
        .expect("stream should start");
    let _ = drain_golden(stream).await;

    let requests = server.received_requests().await.expect("recorded requests");
    let request = requests.first().expect("one request");
    let sent: serde_json::Value = serde_json::from_slice(&request.body).expect("json body");
    assert!(sent.get("diagnostics").is_none());
    assert!(
        request
            .headers
            .get("anthropic-beta")
            .is_none_or(|value| !value.to_str().unwrap().contains("cache-diagnosis"))
    );
}

/// Interleaved thinking: two thinking blocks in one response, each signed
/// separately, with a tool call between them.
///
/// This is the shape the interleaved-thinking beta produces, and the beta is
/// enabled whenever budget thinking meets tools — the normal agent
/// configuration. The previous model concatenated both blocks' text into one
/// string and kept only the last signature, producing a block whose signature
/// does not sign its content, which Anthropic rejects. Each block must survive
/// as its own artifact with its own signature.
#[tokio::test]
async fn interleaved_thinking_keeps_each_signature_with_its_own_block() {
    let server = MockServer::start().await;
    let body = [
        sse_event(
            "message_start",
            r#"{"type":"message_start","message":{"id":"msg_i","usage":{"input_tokens":8}}}"#,
        ),
        // First thinking block.
        sse_event(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
        ),
        sse_event(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"First I check the logs"}}"#,
        ),
        sse_event(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-first"}}"#,
        ),
        sse_event(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
        ),
        // Second thinking block, separately signed.
        sse_event(
            "content_block_start",
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"thinking","thinking":""}}"#,
        ),
        sse_event(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"thinking_delta","thinking":"Now I read the diff"}}"#,
        ),
        sse_event(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"signature_delta","signature":"sig-second"}}"#,
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
            vec![LlmMessage::text(LlmMessageRole::User, "think twice")],
            &config("claude-sonnet-4-5"),
        )
        .await
        .expect("stream should start");

    let events = drain_golden(stream).await;
    let items: Vec<&Golden> = events
        .iter()
        .filter(|g| matches!(g, Golden::ReasoningItem { .. }))
        .collect();

    assert_eq!(
        items,
        vec![
            &Golden::ReasoningItem {
                text: Some("First I check the logs".into()),
                signature: Some("sig-first".into()),
                redacted: false,
            },
            &Golden::ReasoningItem {
                text: Some("Now I read the diff".into()),
                signature: Some("sig-second".into()),
                redacted: false,
            },
        ],
        "each thinking block must keep the signature that signs it; \
         concatenating them and keeping the last signature is what the \
         provider rejects"
    );
}

/// Withheld reasoning (`redacted_thinking`) arrives whole and carries no
/// readable text, but is still a replayable artifact. It was previously
/// unrepresented anywhere in the workspace and silently dropped.
#[tokio::test]
async fn redacted_thinking_survives_as_an_opaque_artifact() {
    let server = MockServer::start().await;
    let body = [
        sse_event(
            "message_start",
            r#"{"type":"message_start","message":{"id":"msg_r","usage":{"input_tokens":4}}}"#,
        ),
        sse_event(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"ENCRYPTED_BLOB"}}"#,
        ),
        sse_event(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
        ),
        sse_event(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}"#,
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

    let events = drain_golden(stream).await;
    assert!(
        events.iter().any(|g| matches!(
            g,
            Golden::ReasoningItem {
                redacted: true,
                text: None,
                ..
            }
        )),
        "redacted_thinking must survive as an opaque artifact, got: {events:?}"
    );
}

/// Every effort that asks for reasoning must put a thinking budget on the wire.
///
/// `minimal` is the case this guards. It had no arm in the effort-to-budget
/// mapping and fell through to "no budget", so the request went out with no
/// thinking block at all: selecting the lowest non-zero effort was silently
/// identical to selecting none. A user asking for a little reasoning got none,
/// with nothing logged and nothing failing.
///
/// `none` is the deliberate opposite and must stay absent from the wire.
#[tokio::test]
async fn every_reasoning_effort_sends_a_thinking_budget() {
    for (effort, expected) in [
        (ReasoningEffort::Minimal, Some(1024)),
        (ReasoningEffort::Low, Some(1024)),
        (ReasoningEffort::Medium, Some(4096)),
        (ReasoningEffort::High, Some(16384)),
        (ReasoningEffort::Xhigh, Some(32768)),
        (ReasoningEffort::None, None),
    ] {
        let server = MockServer::start().await;
        mount_sse(
            &server,
            sse_event("message_stop", r#"{"type":"message_stop"}"#),
        )
        .await;

        let mut call_config = config("claude-sonnet-4-5");
        call_config.reasoning_effort = Some(effort);

        let stream = driver(&server)
            .chat_completion_stream(
                vec![LlmMessage::text(LlmMessageRole::User, "hi")],
                &call_config,
            )
            .await
            .expect("stream should start");
        let _ = drain_golden(stream).await;

        let requests = server.received_requests().await.expect("recorded requests");
        let sent: serde_json::Value =
            serde_json::from_slice(&requests.first().expect("one request").body)
                .expect("json body");

        match expected {
            Some(budget) => {
                assert_eq!(
                    sent["thinking"]["budget_tokens"].as_u64(),
                    Some(budget),
                    "effort {effort} must request a {budget}-token thinking budget, \
                     sent: {}",
                    sent["thinking"]
                );
                assert_eq!(
                    sent["thinking"]["type"].as_str(),
                    Some("enabled"),
                    "effort {effort} must enable thinking"
                );
            }
            None => assert!(
                sent.get("thinking").is_none_or(serde_json::Value::is_null),
                "effort {effort} means no reasoning, so no thinking block belongs \
                 on the wire, sent: {}",
                sent["thinking"]
            ),
        }
    }
}
