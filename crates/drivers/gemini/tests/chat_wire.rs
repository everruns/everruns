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
use wiremock::matchers::{method, path_regex, query_param};
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
    ToolCalls(serde_json::Value),
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
        signature: Option<String>,
        bound_tool_call_id: Option<String>,
    },
}

fn golden(event: LlmStreamEvent) -> Golden {
    match event {
        LlmStreamEvent::TextDelta(t) => Golden::Text(t),
        LlmStreamEvent::ToolCalls(calls) => Golden::ToolCalls(serde_json::to_value(calls).unwrap()),
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
            signature: part.signature.clone(),
            bound_tool_call_id: part.bound_tool_call_id.clone(),
        },
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
        .and(query_param("alt", "sse"))
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
        r#"data: {"candidates":[{"content":{"parts":[{"text":"Hello"}]},"finishReason":"FINISH_REASON_UNSPECIFIED"}]}"#,
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

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url.path(),
        "/models/gemini-2.5-flash:streamGenerateContent"
    );
    assert_eq!(requests[0].url.query(), Some("alt=sse"));
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
            Golden::ToolCalls(
                serde_json::json!([{"id":"call_0","name":"get_weather","arguments":{"city":"Paris"}}])
            ),
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

/// Thought parts reach the reasoning channel and keep their signature.
///
/// Gemini marks reasoning with `thought: true` on an otherwise ordinary text
/// part, so a driver that ignores the flag serves the model's reasoning to the
/// user as its answer. `thoughtSignature` is the replay handle: without it a
/// multi-turn conversation loses thought continuity.
#[tokio::test]
async fn thought_parts_become_reasoning_with_signature() {
    let server = MockServer::start().await;
    let body = [
        r#"data: {"candidates":[{"content":{"parts":[{"text":"weighing the options","thought":true,"thoughtSignature":"sig-thought-1"}]}}]}"#,
        "",
        r#"data: {"candidates":[{"content":{"parts":[{"text":"The answer."}]}}]}"#,
        "",
        r#"data: {"candidates":[{"content":{"parts":[]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":4}}"#,
        "",
        "",
    ]
    .join("\n");
    mount_sse(&server, body).await;

    let stream = provider(&server)
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "think")],
            &config("gemini-2.5-flash"),
        )
        .await
        .expect("gemini stream should start");

    let events = drain_golden(stream).await;

    assert_eq!(
        events,
        vec![
            Golden::ReasoningItem {
                text: Some("weighing the options".into()),
                signature: Some("sig-thought-1".into()),
                bound_tool_call_id: None
            },
            Golden::Text("The answer.".into()),
            Golden::Done {
                total: Some(14),
                prompt: Some(10),
                completion: Some(4),
                cache_read: None,
                finish: Some("stop".into())
            }
        ]
    );
}

/// A signature that arrives on a `functionCall` part binds to that call.
///
/// Gemini attaches `thoughtSignature` to the function-call part when reasoning
/// led to a tool call, and replaying it requires knowing which call it belongs
/// to. An unbound signature cannot be put back in the right place on the next
/// turn.
#[tokio::test]
async fn function_call_thought_signature_binds_to_its_call() {
    let server = MockServer::start().await;
    let body = [
        r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"get_weather","args":{"city":"Paris"}},"thoughtSignature":"sig-call-1"}]}}]}"#,
        "",
        r#"data: {"candidates":[{"content":{"parts":[]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":3}}"#,
        "",
        "",
    ]
    .join("\n");
    mount_sse(&server, body).await;

    let stream = provider(&server)
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
            Golden::ReasoningItem {
                text: None,
                signature: Some("sig-call-1".into()),
                bound_tool_call_id: Some("call_0".into())
            },
            Golden::ToolCalls(
                serde_json::json!([{"id":"call_0","name":"get_weather","arguments":{"city":"Paris"}}])
            ),
            Golden::Done {
                total: Some(15),
                prompt: Some(12),
                completion: Some(3),
                cache_read: None,
                finish: Some("stop".into())
            }
        ]
    );
}

#[tokio::test]
async fn tool_results_replay_function_names_and_object_payloads_on_wire() {
    let server = MockServer::start().await;
    mount_sse(&server, "data: {\"candidates\":[{\"content\":{\"parts\":[]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":2,\"candidatesTokenCount\":0}}\n\n".into()).await;
    let mut call = LlmMessage::text(LlmMessageRole::Assistant, "");
    call.tool_calls = Some(vec![everruns_provider::tool_types::ToolCall {
        id: "call_17".into(),
        name: "get_weather".into(),
        arguments: serde_json::json!({"city":"Paris"}),
    }]);
    let mut result = LlmMessage::text(LlmMessageRole::Tool, "[18,20]");
    result.tool_call_id = Some("call_17".into());
    let stream = provider(&server)
        .chat_completion_stream(vec![call, result], &config("gemini-2.5-flash"))
        .await
        .unwrap();
    assert_eq!(
        drain_golden(stream).await,
        vec![Golden::Done {
            total: Some(2),
            prompt: Some(2),
            completion: Some(0),
            cache_read: None,
            finish: Some("stop".into())
        }]
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["contents"],
        serde_json::json!([
            {"role":"model","parts":[{"functionCall":{"name":"get_weather","args":{"city":"Paris"}}}]},
            {"role":"user","parts":[{"functionResponse":{"name":"get_weather","response":{"result":[18,20]}}}]}
        ])
    );
}

#[tokio::test]
async fn one_frame_preserves_all_parts_calls_signatures_and_terminal_usage() {
    let server = MockServer::start().await;
    let frame = serde_json::json!({"candidates":[{"content":{"parts":[
        {"text":"hello"},{"text":" world"},
        {"text":"checked","thought":true,"thoughtSignature":"sig-thought"},
        {"functionCall":{"name":"weather","args":{"city":"Paris"}},"thoughtSignature":"sig-call"},
        {"functionCall":{"name":"clock","args":{"zone":"UTC"}}}
    ]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":4,"cachedContentTokenCount":3}});
    let late = serde_json::json!({"candidates":[{"content":{"parts":[{"text":"must not reopen the answer"}]}}],"usageMetadata":{"promptTokenCount":20,"candidatesTokenCount":5,"cachedContentTokenCount":4}});
    mount_sse(&server, format!("data: {frame}\n\ndata: {late}\n\n")).await;
    let stream = provider(&server)
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &config("gemini-2.5-flash"),
        )
        .await
        .unwrap();
    assert_eq!(
        drain_golden(stream).await,
        vec![
            Golden::Text("hello".into()),
            Golden::Text(" world".into()),
            Golden::ReasoningItem {
                text: Some("checked".into()),
                signature: Some("sig-thought".into()),
                bound_tool_call_id: None
            },
            Golden::ReasoningItem {
                text: None,
                signature: Some("sig-call".into()),
                bound_tool_call_id: Some("call_0".into())
            },
            Golden::ToolCalls(serde_json::json!([
                {"id":"call_0","name":"weather","arguments":{"city":"Paris"}},
                {"id":"call_1","name":"clock","arguments":{"zone":"UTC"}}
            ])),
            Golden::Done {
                total: Some(25),
                prompt: Some(16),
                completion: Some(5),
                cache_read: Some(4),
                finish: Some("stop".into())
            }
        ]
    );
}

#[tokio::test]
async fn rejected_terminal_frame_never_releases_pending_tool_calls() {
    for (reason, expected) in [("MAX_TOKENS", "length"), ("SAFETY", "content_filter")] {
        let server = MockServer::start().await;
        let frame = serde_json::json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"delete_file","args":{"path":"/important"}}}]},"finishReason":reason}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":2}});
        mount_sse(&server, format!("data: {frame}\n\n")).await;
        let stream = provider(&server)
            .chat_completion_stream(
                vec![LlmMessage::text(LlmMessageRole::User, "hi")],
                &config("gemini-2.5-flash"),
            )
            .await
            .unwrap();
        assert_eq!(
            drain_golden(stream).await,
            vec![Golden::Done {
                total: Some(7),
                prompt: Some(5),
                completion: Some(2),
                cache_read: None,
                finish: Some(expected.into())
            }],
            "{reason}"
        );
    }
}
