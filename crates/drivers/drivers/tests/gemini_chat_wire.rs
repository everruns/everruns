#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
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

// Every mock answers with `Connection: close`. The drivers share one process-wide
// HTTP client, and wiremock recycles a dropped server's port to the next test, so a
// pooled keep-alive connection driven by another test's runtime could otherwise
// serve this test and fail mid-stream (see anthropic_computer_toolset_wire.rs).

use everruns_contracts::driver_registry::{
    LlmCallConfig, LlmCompletionMetadata, LlmResponseStream, LlmStreamEvent, Message, MessageRole,
};
use everruns_contracts::{Provider, StaticHeaderAuth};
use everruns_drivers::gemini::GeminiChatDriver;
use futures::StreamExt;
use wiremock::matchers::{method, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse(frames: &[serde_json::Value]) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .insert_header("Connection", "close")
        .set_body_string(
            frames
                .iter()
                .map(|f| format!("data: {f}\n\n"))
                .collect::<String>(),
        )
}

#[tokio::test]
async fn long_decode_preserves_each_hops_grounding_and_searches() {
    use everruns_contracts::{ToolDefinition, native_web_search::NativeWebSearch};
    use serde_json::json;
    use wiremock::matchers::body_partial_json;

    // Exercise both public entry points, including a final hop without grounding.
    for streaming in [false, true] {
        for grounded_second_hop in [false, true] {
            let server = MockServer::start().await;
            let grounding = |start, url| {
                json!({
                    "webSearchQueries": ["same query", "", "same query"],
                    "groundingChunks": [{"web": {"uri": url}}],
                    "groundingSupports": [{
                        "segment": {"startIndex": start, "endIndex": start + 5, "text": "Same."},
                        "groundingChunkIndices": [0, 9]
                    }]
                })
            };
            Mock::given(method("POST"))
                .and(body_partial_json(json!({"continuationToken":"second"})))
                .respond_with(sse(&[
                    json!({"candidates":[{"content":{"parts":[{"text":" Done."}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":30,"candidatesTokenCount":3,"thoughtsTokenCount":4}}),
                ]))
                .with_priority(1).expect(1).mount(&server).await;
            let second_grounding =
                grounded_second_hop.then(|| grounding(1, "https://example.com/second"));
            Mock::given(method("POST"))
                .and(body_partial_json(json!({"continuationToken":"first"})))
                .respond_with(sse(&[
                    json!({"candidates":[{"content":{"parts":[{"text":" Same."}]},"groundingMetadata":second_grounding}]}),
                    json!({"candidates":[{"finishReason":"CONTINUATION","continuationToken":"second","groundingMetadata":second_grounding}],"usageMetadata":{"promptTokenCount":20,"candidatesTokenCount":2,"thoughtsTokenCount":3}}),
                ]))
                .with_priority(2).expect(1).mount(&server).await;
            Mock::given(method("POST"))
                .respond_with(sse(&[
                    json!({"candidates":[{"content":{"parts":[{"text":"Grüße 😀! Same."}]},"groundingMetadata":grounding(14, "https://example.com/first")}]}),
                    json!({"candidates":[{"finishReason":"CONTINUATION","continuationToken":"first"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":1,"thoughtsTokenCount":2}}),
                    // Repeated metadata can trail the terminal candidate.
                    json!({"candidates":[{"groundingMetadata":grounding(14, "https://example.com/first") }]}),
                ]))
                .with_priority(3).expect(1).mount(&server).await;

            let mut cfg = config("gemini-4-argon");
            cfg.max_tokens = Some(100_000);
            cfg.tools = vec![ToolDefinition::function(
                "web_search",
                "search",
                json!({"type":"object"}),
            )];
            let (key, value) = NativeWebSearch::default().to_driver_option();
            cfg.driver_options.insert(key, value);
            let messages = vec![Message::text(MessageRole::User, "search")];
            let (text, metadata) = if streaming {
                let mut stream = provider(&server)
                    .chat_completion_stream(messages, &cfg)
                    .await
                    .unwrap();
                let mut text = String::new();
                let mut metadata = None;
                let mut searches = Vec::new();
                while let Some(event) = stream.next().await {
                    match event.unwrap() {
                        LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
                        LlmStreamEvent::HostedToolCall(call) => searches.push(call),
                        LlmStreamEvent::Done(done) => {
                            assert!(metadata.replace(*done).is_none());
                        }
                        LlmStreamEvent::Error(error) => panic!("{error}"),
                        _ => {}
                    }
                }
                assert_eq!(searches.len(), if grounded_second_hop { 2 } else { 1 });
                for (index, call) in searches.iter().enumerate() {
                    assert_eq!(call.id, format!("google_search_{}", index + 1));
                    assert_eq!(call.summary.as_deref(), Some("same query"));
                }
                (text, metadata.unwrap())
            } else {
                let response = provider(&server)
                    .chat_completion(messages, &cfg)
                    .await
                    .unwrap();
                (response.text, response.metadata)
            };
            assert_eq!(text, "Grüße 😀! Same. Same. Done.");
            let citations = &metadata.web_citations;
            assert_eq!(citations.len(), if grounded_second_hop { 2 } else { 1 });
            for (citation, (url, start, end)) in citations.iter().zip([
                ("https://example.com/first", 9, 14),
                ("https://example.com/second", 15, 20),
            ]) {
                assert_eq!(citation.url, url);
                let span = citation.span.as_ref().unwrap();
                assert_eq!(
                    (span.start, span.end, span.text.as_str()),
                    (start, end, "Same.")
                );
            }
            assert_eq!(
                metadata.hosted_tool_calls.get("web_search_call"),
                Some(&if grounded_second_hop { 2 } else { 1 })
            );
            assert_eq!(metadata.total_tokens, Some(75));
            assert_eq!(metadata.prompt_tokens, Some(60));
            assert_eq!(metadata.completion_tokens, Some(15));
            assert_eq!(metadata.reasoning_tokens, Some(9));
            assert_eq!(metadata.provider_finish_reason.as_deref(), Some("STOP"));
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 3);
            for request in requests {
                assert_eq!(
                    request.body_json::<serde_json::Value>().unwrap()["tools"],
                    json!([{"google_search":{}}])
                );
            }
        }
    }
}

#[tokio::test]
async fn long_decode_stops_at_the_requested_budget_and_keeps_the_default_cap() {
    use serde_json::json;
    for (budget, tokens, expected) in [
        (None, 7, 65_536),
        (Some(65_536), 7, 65_536),
        (Some(100_000), 100_000, 100_000),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(sse(&[json!({
            "candidates":[{"content":{"parts":[{"text":"partial"},{"functionCall":{"name":"cut_off","args":{"x":1}}}]},"finishReason":"CONTINUATION","continuationToken":"unused"}],
            "usageMetadata":{"promptTokenCount":10,"thoughtsTokenCount":tokens}
        })])).expect(1).mount(&server).await;
        let mut cfg = config("gemini-4-argon");
        cfg.max_tokens = budget;
        cfg.temperature = Some(0.7);
        assert_eq!(
            provider(&server)
                .into_boxed_driver()
                .stream_stall_timeout(&cfg),
            budget
                .filter(|&tokens| tokens > 65_536)
                .map(|_| std::time::Duration::from_secs(1_200)),
        );
        let response = provider(&server)
            .chat_completion(vec![Message::text(MessageRole::User, "hi")], &cfg)
            .await
            .unwrap();
        assert_eq!(response.text, "partial");
        assert!(response.tool_calls.is_none());
        assert_eq!(response.metadata.tool_calls_dropped, 1);
        assert_eq!(response.metadata.finish_reason.as_deref(), Some("length"));
        let request = replayed_request(&server)
            .await
            .body_json::<serde_json::Value>()
            .unwrap();
        assert_eq!(
            request["generationConfig"],
            json!({"maxOutputTokens":expected})
        );
    }
}

#[tokio::test]
async fn long_decode_releases_all_hops_tools_only_after_accepted_completion() {
    use serde_json::json;
    use wiremock::matchers::body_partial_json;
    for reason in ["STOP", "SAFETY", "MAX_TOKENS"] {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(body_partial_json(json!({"continuationToken":"next"})))
            .respond_with(sse(&[json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"accepted","args":{"x":1}}}]},"finishReason":reason}]})]))
            .with_priority(1).expect(1).mount(&server).await;
        Mock::given(method("POST"))
            .respond_with(sse(&[json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"first_hop","args":{"first":true}},"thoughtSignature":"signed-first-call"}]},"finishReason":"CONTINUATION","continuationToken":"next"}]})]))
            .with_priority(2).expect(1).mount(&server).await;
        let mut cfg = config("gemini-4-argon");
        cfg.max_tokens = Some(100_000);
        let response = provider(&server)
            .chat_completion(vec![Message::text(MessageRole::User, "hi")], &cfg)
            .await
            .unwrap();
        if reason == "STOP" {
            let calls = response.tool_calls.unwrap();
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[0].name, "first_hop");
            assert_eq!(calls[0].arguments, json!({"first":true}));
            assert_eq!(calls[1].name, "accepted");
            assert_eq!(
                response.reasoning[0].bound_tool_call_id.as_deref(),
                Some(calls[0].id.as_str())
            );
            assert_eq!(response.metadata.tool_calls_dropped, 0);
        } else {
            assert!(response.tool_calls.is_none());
            assert_eq!(response.metadata.tool_calls_dropped, 2);
        }
    }
}

#[tokio::test]
async fn dropping_long_decode_stream_prevents_the_next_hop() {
    use serde_json::json;
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(sse(&[json!({"candidates":[{"content":{"parts":[{"text":"first"}]},"finishReason":"CONTINUATION","continuationToken":"next"}]})]))
        .expect(1).mount(&server).await;
    let mut cfg = config("gemini-4-argon");
    cfg.max_tokens = Some(100_000);
    let mut stream = provider(&server)
        .chat_completion_stream(vec![Message::text(MessageRole::User, "hi")], &cfg)
        .await
        .unwrap();
    assert!(
        matches!(stream.next().await.unwrap().unwrap(), LlmStreamEvent::TextDelta(t) if t == "first")
    );
    drop(stream);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn argon_thinking_levels_replace_budgets_without_affecting_older_models() {
    use everruns_contracts::ReasoningEffort;
    use serde_json::json;
    for (effort, expected) in [
        (None, None),
        (Some(ReasoningEffort::None), None),
        (Some(ReasoningEffort::Minimal), Some("low")),
        (Some(ReasoningEffort::Low), Some("low")),
        (Some(ReasoningEffort::Medium), Some("medium")),
        (Some(ReasoningEffort::High), Some("high")),
        (Some(ReasoningEffort::Xhigh), Some("high")),
        (Some(ReasoningEffort::Max), Some("high")),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(sse(&[json!({"candidates":[{"finishReason":"STOP"}]})]))
            .expect(1)
            .mount(&server)
            .await;
        let mut cfg = config("gemini-4-argon");
        cfg.reasoning_effort = effort;
        provider(&server)
            .chat_completion(vec![Message::text(MessageRole::User, "hi")], &cfg)
            .await
            .unwrap();
        let request = replayed_request(&server)
            .await
            .body_json::<serde_json::Value>()
            .unwrap();
        assert_eq!(
            request["generationConfig"]["thinkingConfig"],
            expected.map_or(
                json!(null),
                |level| json!({"thinkingLevel":level,"includeThoughts":true})
            )
        );
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&[json!({"candidates":[{"finishReason":"STOP"}]})]))
        .expect(1)
        .mount(&server)
        .await;
    let mut cfg = config("gemini-2.5-pro");
    cfg.reasoning_effort = Some(ReasoningEffort::Low);
    provider(&server)
        .chat_completion(vec![Message::text(MessageRole::User, "hi")], &cfg)
        .await
        .unwrap();
    assert_eq!(
        replayed_request(&server)
            .await
            .body_json::<serde_json::Value>()
            .unwrap()["generationConfig"]["thinkingConfig"],
        json!({"thinkingBudget":1024,"includeThoughts":true})
    );
}

#[tokio::test]
async fn long_decode_merges_hops_and_bills_each_hops_final_usage() {
    use serde_json::json;
    use wiremock::matchers::body_partial_json;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"continuationToken":"opaque-one"})))
        .respond_with(sse(&[
            json!({"candidates":[{"content":{"parts":[{"text":"answer"}]}}],"usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":1,"thoughtsTokenCount":2,"cachedContentTokenCount":80}}),
            json!({"candidates":[{"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":3,"thoughtsTokenCount":5,"cachedContentTokenCount":90}}),
        ]))
        .with_priority(1).expect(1).mount(&server).await;
    Mock::given(method("POST"))
        .respond_with(sse(&[
            json!({"candidates":[{"content":{"parts":[{"text":"thinking","thought":true}]}}],"usageMetadata":{"promptTokenCount":10,"thoughtsTokenCount":2}}),
            json!({"candidates":[{"finishReason":"CONTINUATION","continuationToken":"opaque-one"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":0,"thoughtsTokenCount":7}}),
        ]))
        .with_priority(2).expect(1).mount(&server).await;
    let mut cfg = config("gemini-4-argon");
    cfg.max_tokens = Some(100_000);
    cfg.reasoning_effort = Some(everruns_contracts::ReasoningEffort::High);
    cfg.extra_headers = vec![("x-test".into(), "preserved".into())];
    let response = provider(&server)
        .chat_completion(
            vec![
                Message::text(MessageRole::System, "system"),
                Message::text(MessageRole::User, "question"),
            ],
            &cfg,
        )
        .await
        .unwrap();
    assert_eq!(response.text, "answer");
    assert_eq!(response.metadata.prompt_tokens, Some(20));
    assert_eq!(response.metadata.cache_read_tokens, Some(90));
    assert_eq!(response.metadata.completion_tokens, Some(15));
    assert_eq!(response.metadata.reasoning_tokens, Some(12));
    assert_eq!(response.metadata.total_tokens, Some(125));
    assert_eq!(response.metadata.finish_reason.as_deref(), Some("stop"));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let original = requests[0].body_json::<serde_json::Value>().unwrap();
    let mut continued = requests[1].body_json::<serde_json::Value>().unwrap();
    assert_eq!(
        continued
            .as_object_mut()
            .unwrap()
            .remove("continuationToken"),
        Some(json!("opaque-one"))
    );
    assert_eq!(continued, original);
    assert_eq!(
        original["generationConfig"],
        json!({"maxOutputTokens":100_000,"thinkingConfig":{"thinkingLevel":"high","includeThoughts":true}})
    );
    for request in requests {
        assert_eq!(request.headers["x-test"], "preserved");
        assert_eq!(request.headers["x-goog-api-key"], "test-key");
    }
}

#[tokio::test]
async fn long_decode_rejects_missing_or_repeated_tokens_and_later_http_failure() {
    use serde_json::json;
    use wiremock::matchers::body_partial_json;
    for (next, error, requests) in [
        (None, "continuation token", 1),
        (
            Some(sse(&[
                json!({"candidates":[{"finishReason":"CONTINUATION","continuationToken":"same"}]} ),
            ])),
            "progress",
            2,
        ),
        (
            Some(ResponseTemplate::new(400).set_body_string("invalid continuation")),
            "400",
            2,
        ),
    ] {
        let server = MockServer::start().await;
        if let Some(next) = next {
            Mock::given(method("POST"))
                .and(body_partial_json(json!({"continuationToken":"same"})))
                .respond_with(next)
                .with_priority(1)
                .mount(&server)
                .await;
        }
        let token = (requests > 1).then_some("same");
        Mock::given(method("POST")).respond_with(sse(&[json!({"candidates":[{"content":{"parts":[{"text":"partial"}]},"finishReason":"CONTINUATION","continuationToken":token}]})]))
            .with_priority(2).mount(&server).await;
        let mut cfg = config("gemini-4-argon");
        cfg.max_tokens = Some(100_000);
        let result = provider(&server)
            .chat_completion(vec![Message::text(MessageRole::User, "question")], &cfg)
            .await;
        assert!(result.unwrap_err().to_string().contains(error));
        assert_eq!(server.received_requests().await.unwrap().len(), requests);
    }
}

fn config(model: &str) -> LlmCallConfig {
    LlmCallConfig::new(model)
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

/// The one request the driver made, tolerating a reconnect that replayed it.
///
/// `connect_sse_with_reconnect` peeks the first SSE item and, when opening the
/// body fails transiently, reconnects (see `stream_reconnect.rs`). That is
/// correct driver behaviour and nothing the test controls: a loaded CI runner
/// produces it, and asserting `requests.len() == 1` turned it into a failure
/// of whichever test happened to hit it.
///
/// So assert what actually matters instead, and rather more than the count
/// did: every attempt must be a byte-identical replay, since a reconnect that
/// re-sent a *different* request would be a real bug.
async fn replayed_request(server: &MockServer) -> wiremock::Request {
    let requests = server
        .received_requests()
        .await
        .expect("wiremock records requests");
    let (first, retries) = requests.split_first().expect("driver sent a request");
    for retry in retries {
        assert_eq!(
            (&retry.method, &retry.url, &retry.body),
            (&first.method, &first.url, &first.body),
            "a reconnect must replay the same request, not a different one"
        );
    }
    requests.into_iter().next_back().expect("checked non-empty")
}

async fn mount_sse(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path_regex(r"^/models/.+:streamGenerateContent$"))
        .and(query_param("alt", "sse"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("connection", "close")
                .set_body_raw(body, "text/event-stream"),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn provider_reported_model_is_distinct_and_optional() {
    for reported in [Some("gemini-2.5-flash-2026-09-01"), None] {
        let server = MockServer::start().await;
        let mut chunk = serde_json::json!({
            "candidates": [{
                "content": {"parts": [{"text": "Hello"}]},
                "finishReason": "STOP"
            }]
        });
        if let Some(model) = reported {
            chunk["modelVersion"] = serde_json::json!(model);
        }
        mount_sse(&server, format!("data: {chunk}\n\n")).await;

        let response = provider(&server)
            .chat_completion(
                vec![Message::text(MessageRole::User, "hi")],
                &config("gemini-2.5-flash-latest"),
            )
            .await
            .expect("completion should succeed");

        assert_eq!(
            response.metadata.model.as_deref(),
            Some("gemini-2.5-flash-latest")
        );
        assert_eq!(response.metadata.response_model.as_deref(), reported);
    }
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
            vec![Message::text(MessageRole::User, "hi")],
            &config("gemini-2.5-flash"),
        )
        .await
        .expect("gemini stream should start");

    let request = replayed_request(&server).await;
    assert_eq!(
        request.url.path(),
        "/models/gemini-2.5-flash:streamGenerateContent"
    );
    assert_eq!(request.url.query(), Some("alt=sse"));
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
            vec![Message::text(MessageRole::User, "weather?")],
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
/// still emits a terminal `Done` (end-of-stream fallback). The finish reason
/// stays `None`: the provider never gave one, and a host must be able to tell
/// that apart from an explicit stop.
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
            vec![Message::text(MessageRole::User, "hi")],
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
                finish: None,
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
            vec![Message::text(MessageRole::User, "think")],
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
            vec![Message::text(MessageRole::User, "weather?")],
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
    let mut call = Message::text(MessageRole::Assistant, "");
    call.tool_calls = Some(vec![everruns_contracts::tool_types::ToolCall {
        id: "call_17".into(),
        name: "get_weather".into(),
        arguments: serde_json::json!({"city":"Paris"}),
    }]);
    let mut result = Message::text(MessageRole::Tool, "[18,20]");
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
    let request = replayed_request(&server).await;
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
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
            vec![Message::text(MessageRole::User, "hi")],
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
                vec![Message::text(MessageRole::User, "hi")],
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

/// The discard above is reported, not silent: the dropped count and Gemini's
/// raw finish reason ride on the completion metadata. A `STOP` frame drops
/// nothing and its calls run.
#[tokio::test]
async fn terminal_frame_reports_dropped_tool_calls_with_raw_reason() {
    for (reason, dropped) in [("MAX_TOKENS", 2), ("SAFETY", 2), ("STOP", 0)] {
        let server = MockServer::start().await;
        let frame = serde_json::json!({"candidates":[{"content":{"parts":[
            {"functionCall":{"name":"a","args":{}}},
            {"functionCall":{"name":"b","args":{"x":1}}}
        ]},"finishReason":reason}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":2}});
        mount_sse(&server, format!("data: {frame}\n\n")).await;
        let response = provider(&server)
            .chat_completion(
                vec![Message::text(MessageRole::User, "hi")],
                &config("gemini-2.5-flash"),
            )
            .await
            .unwrap();
        assert_eq!(response.metadata.tool_calls_dropped, dropped, "{reason}");
        assert_eq!(
            response.metadata.provider_finish_reason.as_deref(),
            Some(reason)
        );
        assert_eq!(response.metadata.completion_tokens, Some(2), "{reason}");
        assert_eq!(
            response.tool_calls.map_or(0, |calls| calls.len()),
            if dropped == 0 { 2 } else { 0 },
            "{reason}"
        );
    }
}

/// A reconnect leaves the mock server holding two identical records, which is
/// exactly what two identical calls produce. This does not reproduce a live
/// transport failure (wiremock cannot sever a body mid-read, and only
/// `EventStreamError::Transport` reconnects), but it is the state
/// `replayed_request` has to tolerate, and the state that used to fail CI.
#[tokio::test]
async fn replayed_request_tolerates_an_identical_replay() {
    let server = MockServer::start().await;
    mount_sse(
        &server,
        "data: {\"candidates\":[{\"content\":{\"parts\":[]},\"finishReason\":\"STOP\"}]}\n\n"
            .into(),
    )
    .await;
    for _ in 0..2 {
        let stream = provider(&server)
            .chat_completion_stream(
                vec![Message::text(MessageRole::User, "hi")],
                &config("gemini-2.5-flash"),
            )
            .await
            .expect("stream should start");
        drain_golden(stream).await;
    }

    // Two deliberate calls, and a reconnect may add more:
    // `connect_bytes_with_reconnect` reissues the request on a transport
    // error, so under CI load this legitimately sees three. Asserting
    // equality made the very condition the test exists to tolerate fail it.
    assert!(server.received_requests().await.unwrap().len() >= 2);
    let request = replayed_request(&server).await;
    assert_eq!(
        request.url.path(),
        "/models/gemini-2.5-flash:streamGenerateContent"
    );
}

/// The count assertion was not worthless, only aimed wrong: a second attempt
/// that sent *different* bytes would be a real bug, and that is what
/// `replayed_request` still catches.
#[tokio::test]
#[should_panic(expected = "a reconnect must replay the same request")]
async fn replayed_request_rejects_a_replay_that_changed_the_request() {
    let server = MockServer::start().await;
    mount_sse(
        &server,
        "data: {\"candidates\":[{\"content\":{\"parts\":[]},\"finishReason\":\"STOP\"}]}\n\n"
            .into(),
    )
    .await;
    for prompt in ["hi", "something else entirely"] {
        let stream = provider(&server)
            .chat_completion_stream(
                vec![Message::text(MessageRole::User, prompt)],
                &config("gemini-2.5-flash"),
            )
            .await
            .expect("stream should start");
        drain_golden(stream).await;
    }

    replayed_request(&server).await;
}

/// Asked to, the driver hands back the body it sent, byte for byte the JSON
/// the provider received; not asked, it records nothing.
#[tokio::test]
async fn captured_request_body_is_the_body_on_the_wire() {
    for capture in [true, false] {
        let server = MockServer::start().await;
        let chunk = serde_json::json!({
            "candidates": [{"content": {"parts": [{"text": "Hello"}]}, "finishReason": "STOP"}]
        });
        mount_sse(&server, format!("data: {chunk}\n\n")).await;
        let mut config = config("gemini-2.5-flash");
        config.capture_request = capture;

        let response = provider(&server)
            .chat_completion(vec![Message::text(MessageRole::User, "hi")], &config)
            .await
            .expect("completion should succeed");

        let sent: serde_json::Value = replayed_request(&server).await.body_json().unwrap();
        assert_eq!(response.metadata.request_body, capture.then_some(sent));
    }
}
