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

use everruns_provider::OpenResponsesProtocolChatDriver;
use everruns_provider::driver_registry::{
    LlmCallConfig, LlmCompletionMetadata, LlmMessage, LlmMessageRole, LlmResponseStream,
    LlmStreamEvent, ProviderOpaqueContext,
};
use everruns_provider::{BearerAuth, CompactContent, CompactOutputItem, Provider};
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
    Reasoning {
        delta: String,
        summary: bool,
    },
    ReasoningItem {
        item_id: Option<String>,
        encrypted: Option<String>,
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
        LlmStreamEvent::ReasoningDelta { delta, summary } => Golden::Reasoning { delta, summary },
        LlmStreamEvent::ReasoningItem(item) => Golden::ReasoningItem {
            item_id: item.item_id.clone(),
            encrypted: item.encrypted.clone(),
        },
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

/// Frame a minimal completed-response SSE body.
fn sse_event(data: &str) -> String {
    format!("data: {data}\n\ndata: [DONE]\n\n")
}

async fn mount_sse(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(server)
        .await;
}

fn driver(server: &MockServer) -> Provider {
    Provider::new("openresponses-test", OpenResponsesProtocolChatDriver::new())
        .base_url(format!("{}/v1/responses", server.uri()))
        .auth(BearerAuth::new("test-key"))
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

#[tokio::test]
async fn native_compact_context_is_the_exact_ordered_responses_input() {
    let server = MockServer::start().await;
    let body = [
        r#"data: {"type":"response.completed","response":{"id":"resp_compact_retry","status":"completed","output":[],"usage":{"input_tokens":4,"output_tokens":1}}}"#,
        "",
        "data: [DONE]",
        "",
        "",
    ]
    .join("\n");
    mount_sse(&server, body).await;

    let output = vec![
        CompactOutputItem::Message {
            role: "user".to_string(),
            content: CompactContent::Text("first".to_string()),
        },
        CompactOutputItem::Compaction {
            encrypted_content: "opaque-native-context".to_string(),
        },
        CompactOutputItem::Message {
            role: "user".to_string(),
            content: CompactContent::Text("last".to_string()),
        },
    ];
    let context = ProviderOpaqueContext::OpenResponsesCompact {
        output: output.clone(),
    };
    let debug = format!("{context:?}");
    assert!(debug.contains("item_count"));
    assert!(!debug.contains("opaque-native-context"));
    let encoded = serde_json::to_value(&context).expect("context should serialize");
    assert_eq!(encoded["type"], "open_responses_compact");
    assert_eq!(
        serde_json::from_value::<ProviderOpaqueContext>(encoded).unwrap(),
        context
    );

    let mut call_config = config("gpt-5-mini");
    call_config.previous_response_id = Some("resp_must_not_be_mixed".to_string());
    call_config.provider_opaque_context = Some(context);
    let stream = driver(&server)
        .chat_completion_stream(
            vec![
                LlmMessage::text(LlmMessageRole::System, "instructions"),
                LlmMessage::text(LlmMessageRole::User, "reconstructed transcript"),
            ],
            &call_config,
        )
        .await
        .expect("stream should start");
    let _ = drain_golden(stream).await;

    let requests = server.received_requests().await.unwrap();
    let request: serde_json::Value = requests[0].body_json().unwrap();
    assert!(request.get("previous_response_id").is_none());
    assert_eq!(request["instructions"], "instructions");
    assert_eq!(
        request["input"],
        serde_json::json!([
            { "type": "message", "role": "user", "content": "first" },
            { "type": "compaction", "encrypted_content": "opaque-native-context" },
            { "type": "message", "role": "user", "content": "last" },
            { "type": "message", "role": "user", "content": "reconstructed transcript" }
        ])
    );
}

/// A reasoning request must opt into `reasoning.encrypted_content`.
///
/// The provider returns reasoning items with no payload unless asked, so
/// without this the replay path exists but never has anything to replay: a
/// stateless follow-up (after compaction, a model switch, or router failover)
/// silently drops the reasoning chain with no error anywhere.
#[tokio::test]
async fn reasoning_request_opts_into_encrypted_content() {
    let server = MockServer::start().await;
    mount_sse(
        &server,
        sse_event(
            r#"{"type":"response.completed","sequence_number":1,"response":{"id":"resp_1","usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ),
    )
    .await;

    let mut call_config = config("gpt-5.2");
    call_config.reasoning_effort = Some(everruns_provider::model::ReasoningEffort::High);
    let stream = driver(&server)
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &call_config,
        )
        .await
        .expect("stream should start");
    let _ = drain_golden(stream).await;

    let requests = server.received_requests().await.unwrap();
    let request: serde_json::Value = requests[0].body_json().unwrap();
    assert_eq!(
        request["include"],
        serde_json::json!(["reasoning.encrypted_content"]),
        "got: {request}"
    );
}

/// The opt-in is scoped to reasoning requests: a model that was not asked to
/// reason has no reasoning payload to include.
#[tokio::test]
async fn non_reasoning_request_omits_include() {
    let server = MockServer::start().await;
    mount_sse(
        &server,
        sse_event(
            r#"{"type":"response.completed","sequence_number":1,"response":{"id":"resp_1","usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ),
    )
    .await;

    let stream = driver(&server)
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &config("gpt-4o"),
        )
        .await
        .expect("stream should start");
    let _ = drain_golden(stream).await;

    let requests = server.received_requests().await.unwrap();
    let request: serde_json::Value = requests[0].body_json().unwrap();
    assert!(request.get("include").is_none(), "got: {request}");
}

/// Every reasoning item replays under the id the provider issued, and all of
/// them replay — a turn with parallel tool calls emits several. Ids were
/// previously synthesized from a counter, which the API cannot resolve.
#[tokio::test]
async fn reasoning_items_replay_under_their_provider_ids() {
    let server = MockServer::start().await;
    mount_sse(
        &server,
        sse_event(
            r#"{"type":"response.completed","sequence_number":1,"response":{"id":"resp_1","usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ),
    )
    .await;

    let mut assistant = LlmMessage::text(LlmMessageRole::Assistant, "working on it");
    assistant.reasoning = vec![
        everruns_provider::reasoning::ReasoningContentPart::opaque("openai")
            .with_item_id("rs_first")
            .with_encrypted("blob-one"),
        everruns_provider::reasoning::ReasoningContentPart::opaque("openai")
            .with_item_id("rs_second")
            .with_encrypted("blob-two"),
        // No id and no payload: not replayable, and must be dropped rather
        // than reconstructed under a made-up id.
        everruns_provider::reasoning::ReasoningContentPart::opaque("openai"),
    ];

    let stream = driver(&server)
        .chat_completion_stream(
            vec![
                LlmMessage::text(LlmMessageRole::User, "go"),
                assistant,
                LlmMessage::text(LlmMessageRole::User, "continue"),
            ],
            &config("gpt-5.2"),
        )
        .await
        .expect("stream should start");
    let _ = drain_golden(stream).await;

    let requests = server.received_requests().await.unwrap();
    let request: serde_json::Value = requests[0].body_json().unwrap();
    let reasoning_items: Vec<&serde_json::Value> = request["input"]
        .as_array()
        .expect("input array")
        .iter()
        .filter(|item| item["type"] == "reasoning")
        .collect();

    assert_eq!(
        reasoning_items,
        vec![
            // `summary` is required on a replayed reasoning item: omitting it
            // is a 400 from the live API, which is what the multi-turn live
            // test caught. An item that carried no summary replays as an empty
            // array, not as an absent field.
            &serde_json::json!({
                "type": "reasoning",
                "id": "rs_first",
                "encrypted_content": "blob-one",
                "summary": [],
            }),
            &serde_json::json!({
                "type": "reasoning",
                "id": "rs_second",
                "encrypted_content": "blob-two",
                "summary": [],
            }),
        ],
        "got: {request}"
    );
}
