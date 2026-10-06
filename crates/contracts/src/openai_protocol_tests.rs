use super::*;
use serde_json::{Value, json};

// ========================================================================
// Request-too-large detection tests
// ========================================================================

// ========================================================================
// Model-not-found detection tests
// ========================================================================

// ========================================================================
// Reasoning effort guard tests
// ========================================================================

// ------------------------------------------------------------------
// EVE-522: streaming chunk handling (process_stream_choice)
// ------------------------------------------------------------------

fn choice(json_str: &str) -> OpenAiStreamChoice {
    serde_json::from_str(json_str).unwrap()
}

/// EVE-522 regression: providers such as OpenRouter/DeepInfra send an empty
/// `content: ""` in the same chunk that carries `finish_reason: "tool_calls"`.
/// The accumulated tool calls must still be emitted exactly once.
#[test]
fn test_empty_content_finish_chunk_still_emits_tool_calls() {
    let mut total_tokens = 0u32;
    let mut acc = StreamToolCallAccumulator::new();
    let mut finish_reason: Option<String> = None;

    // Chunk 2: tool_calls delta opens the call (id + name).
    let e = process_stream_choice(
        &choice(
            r#"{"delta":{"content":null,"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":""}}]},"finish_reason":null}"#,
        ),
        &mut total_tokens,
        &mut acc,
        &mut finish_reason,
    );
    assert!(e.is_none());

    // Chunk 3: tool_calls delta streams the arguments.
    let e = process_stream_choice(
        &choice(
            r#"{"delta":{"content":null,"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\"Cargo.toml\"}"}}]},"finish_reason":null}"#,
        ),
        &mut total_tokens,
        &mut acc,
        &mut finish_reason,
    );
    assert!(e.is_none());

    // Chunk 4: content:"" alongside finish_reason:"tool_calls" — must NOT
    // short-circuit; emits the accumulated call with parsed JSON arguments.
    let e = process_stream_choice(
        &choice(r#"{"delta":{"content":""},"finish_reason":"tool_calls"}"#),
        &mut total_tokens,
        &mut acc,
        &mut finish_reason,
    );
    match e {
        Some(LlmStreamEvent::ToolCalls(calls)) => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].id, "call_1");
            assert_eq!(calls[0].name, "read_file");
            assert_eq!(calls[0].arguments, json!({"path": "Cargo.toml"}));
        }
        other => panic!("expected ToolCalls, got {:?}", other),
    }
    assert_eq!(finish_reason.as_deref(), Some("tool_calls"));

    // Chunk 5: second finish chunk with content:"" — the accumulator was
    // drained, so the same call must not be emitted again.
    let e = process_stream_choice(
        &choice(r#"{"delta":{"content":""},"finish_reason":"tool_calls"}"#),
        &mut total_tokens,
        &mut acc,
        &mut finish_reason,
    );
    assert!(e.is_none(), "tool calls must only be emitted once");
}

/// Non-empty content deltas are still emitted and counted as output tokens.
#[test]
fn test_non_empty_content_is_emitted() {
    let mut total_tokens = 0u32;
    let mut acc = StreamToolCallAccumulator::new();
    let mut finish_reason: Option<String> = None;

    let e = process_stream_choice(
        &choice(r#"{"delta":{"content":"hello"},"finish_reason":null}"#),
        &mut total_tokens,
        &mut acc,
        &mut finish_reason,
    );
    assert!(matches!(e, Some(LlmStreamEvent::TextDelta(s)) if s == "hello"));
    assert_eq!(total_tokens, 1);
}

/// EVE-636: streamed tool-call arguments must concatenate exactly across
/// many small chunks (accumulated as a raw string, parsed zero times
/// mid-stream) and be parsed exactly once at the `tool_calls` finish chunk.
#[test]
fn test_tool_call_arguments_accumulate_across_many_chunks() {
    let mut total_tokens = 0u32;
    let mut acc = StreamToolCallAccumulator::new();
    let mut finish_reason: Option<String> = None;

    // Open the call (id + name, empty initial arguments).
    process_stream_choice(
        &choice(
            r#"{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"write_file","arguments":""}}]},"finish_reason":null}"#,
        ),
        &mut total_tokens,
        &mut acc,
        &mut finish_reason,
    );

    let payload = r#"{"path":"a.rs","contents":"a fairly long contents value streamed one character at a time to exceed one hundred chunks","n":987654321}"#;

    // Stream the arguments one character per chunk.
    for ch in payload.chars() {
        let frag = ch.to_string();
        let chunk = json!({
            "delta": {"tool_calls": [{"index": 0, "function": {"arguments": frag}}]},
            "finish_reason": null
        })
        .to_string();
        process_stream_choice(
            &choice(&chunk),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );
    }

    // Mid-stream the shared accumulator holds the fragments as a raw string
    // (parsed once at finalize); its own unit tests cover that internal, so
    // here we assert the observable finish-chunk result concatenates exactly.

    // Finish chunk: parsed exactly once into the structured value.
    let e = process_stream_choice(
        &choice(r#"{"delta":{},"finish_reason":"tool_calls"}"#),
        &mut total_tokens,
        &mut acc,
        &mut finish_reason,
    );
    match e {
        Some(LlmStreamEvent::ToolCalls(calls)) => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].id, "call_1");
            assert_eq!(calls[0].name, "write_file");
            assert_eq!(
                calls[0].arguments,
                serde_json::from_str::<serde_json::Value>(payload).unwrap()
            );
        }
        other => panic!("expected ToolCalls, got {:?}", other),
    }
}

/// OpenAI's native path sends `delta: {}` (no content key) in the finish
/// chunk; the existing behavior of emitting tool calls there is preserved.
#[test]
fn test_finish_chunk_without_content_emits_tool_calls() {
    let mut total_tokens = 0u32;
    let mut acc = StreamToolCallAccumulator::new();
    let mut finish_reason: Option<String> = None;

    process_stream_choice(
        &choice(
            r#"{"delta":{"tool_calls":[{"index":0,"id":"call_9","function":{"name":"list_dir","arguments":"{}"}}]},"finish_reason":null}"#,
        ),
        &mut total_tokens,
        &mut acc,
        &mut finish_reason,
    );

    let e = process_stream_choice(
        &choice(r#"{"delta":{},"finish_reason":"tool_calls"}"#),
        &mut total_tokens,
        &mut acc,
        &mut finish_reason,
    );
    match e {
        Some(LlmStreamEvent::ToolCalls(calls)) => {
            assert_eq!(calls.len(), 1);
            assert_eq!(
                serde_json::to_value(&calls).unwrap(),
                json!([{"id":"call_9","name":"list_dir","arguments":{}}])
            );
        }
        other => panic!("expected ToolCalls, got {:?}", other),
    }
}

/// Seed a single tool-call slot into an accumulator the way the streamed
/// chunks would (id + name + raw argument buffer), so the fallback-flush
/// tests exercise the real accumulation path.
fn seeded_acc(id: &str, name: &str, arguments: &str) -> StreamToolCallAccumulator {
    let mut acc = StreamToolCallAccumulator::new();
    acc.apply_indexed_delta(0, Some(id), Some(name), Some(arguments));
    acc
}

/// The [DONE] fallback flushes accumulated-but-unemitted tool calls when no
/// finish reason was reported and drains the accumulator; once drained it
/// returns None.
#[test]
fn test_take_pending_tool_calls_flushes_then_drains_without_finish_reason() {
    let mut acc = seeded_acc("call_1", "read_file", r#"{"path":"Cargo.toml"}"#);

    match take_pending_tool_calls(&mut acc, None) {
        Some(LlmStreamEvent::ToolCalls(calls)) => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].id, "call_1");
            assert_eq!(calls[0].name, "read_file");
            assert_eq!(calls[0].arguments, json!({"path": "Cargo.toml"}));
        }
        other => panic!("expected ToolCalls, got {:?}", other),
    }
    assert!(acc.is_empty(), "accumulator must be drained after flush");
    assert!(take_pending_tool_calls(&mut acc, None).is_none());
}

#[test]
fn test_take_pending_tool_calls_discards_non_tool_finish_reason() {
    let mut acc = seeded_acc("call_cut", "read_file", r#"{"path":"#);

    assert!(take_pending_tool_calls(&mut acc, Some("length")).is_none());
    assert!(
        acc.is_empty(),
        "discarded unsafe fallback calls must still drain the accumulator"
    );
}

#[test]
fn test_take_pending_tool_calls_rejects_malformed_fallback_arguments() {
    let mut acc = seeded_acc("call_cut", "read_file", r#"{"path":"#);

    assert!(take_pending_tool_calls(&mut acc, None).is_none());
    assert!(
        acc.is_empty(),
        "malformed fallback calls must be drained instead of re-emitted"
    );
}

#[test]
fn test_non_tool_finish_reason_leaves_pending_calls_for_done_discard() {
    let mut total_tokens = 0u32;
    let mut acc = StreamToolCallAccumulator::new();
    let mut finish_reason: Option<String> = None;

    process_stream_choice(
        &choice(
            r#"{"delta":{"tool_calls":[{"index":0,"id":"call_cut","function":{"name":"read_file","arguments":"{\"path\":"}}]},"finish_reason":null}"#,
        ),
        &mut total_tokens,
        &mut acc,
        &mut finish_reason,
    );

    let e = process_stream_choice(
        &choice(r#"{"delta":{},"finish_reason":"length"}"#),
        &mut total_tokens,
        &mut acc,
        &mut finish_reason,
    );

    assert!(e.is_none());
    assert_eq!(finish_reason.as_deref(), Some("length"));
    assert!(take_pending_tool_calls(&mut acc, finish_reason.as_deref()).is_none());
    assert!(acc.is_empty());
}

#[test]
fn function_tools_serialize_strict_only_for_compatible_schemas() {
    use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy};
    let make_tool = |parameters| {
        ToolDefinition::Builtin(BuiltinTool {
            name: "lookup".into(),
            display_name: None,
            description: "Lookup".into(),
            parameters,
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::Never,
            hints: ToolHints::default(),
            full_parameters: None,
        })
    };
    let compatible = OpenAIProtocolChatDriver::convert_tools(&[make_tool(json!({
        "type":"object","properties":{"query":{"type":"string"}}
    }))]);
    assert_eq!(
        serde_json::to_value(&compatible).unwrap(),
        json!([{"type":"function","function":{"name":"lookup","description":"Lookup","strict":true,"parameters":{"type":"object","properties":{"query":{"type":["string","null"]}},"required":["query"],"additionalProperties":false}}}])
    );
    let incompatible = OpenAIProtocolChatDriver::convert_tools(&[make_tool(json!({
        "type":"object","allOf":[{"type":"object"}]
    }))]);
    assert_eq!(
        serde_json::to_value(&incompatible).unwrap(),
        json!([{"type":"function","function":{"name":"lookup","description":"Lookup","parameters":{"type":"object","allOf":[{"type":"object"}]}}}])
    );
}
#[test]
fn the_request_capture_is_opt_in_and_is_what_goes_on_the_wire() {
    let request = OpenAiRequest {
        model: "gpt-5-mini".into(),
        messages: vec![],
        temperature: Some(0.5),
        max_tokens: Some(64),
        max_completion_tokens: None,
        stream: true,
        stream_options: None,
        tools: None,
        parallel_tool_calls: None,
        reasoning_effort: None,
        service_tier: None,
        verbosity: None,
        metadata: None,
        response_format: None,
    };

    let mut config = call_config();
    assert_eq!(
        capture_request_body(&config, &request),
        None,
        "the prompt is not recorded unless the caller asked"
    );

    config.capture_request = true;
    let captured = capture_request_body(&config, &request).expect("the body is captured");
    // The same value the wire gets, so the capture cannot drift from it.
    assert_eq!(captured, serde_json::to_value(&request).unwrap());
    assert_eq!(captured["model"], json!("gpt-5-mini"));
    assert_eq!(captured["temperature"], json!(0.5));
    // Authentication travels in headers; nothing credential-shaped is here.
    assert!(captured.get("api_key").is_none());
}

fn call_config() -> LlmCallConfig {
    LlmCallConfig::new("model")
}
async fn mock_provider(sse: &str) -> (wiremock::MockServer, crate::Provider) {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    // A pooled server can reuse connections owned by an earlier test runtime.
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer synthetic-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider = crate::Provider::new(
        "test",
        OpenAIProtocolChatDriver::new().with_retry_config(LlmRetryConfig {
            max_retries: 0,
            ..Default::default()
        }),
    )
    .base_url(format!("{}/v1", server.uri()))
    .auth(crate::BearerAuth::new("synthetic-key"));
    (server, provider)
}

async fn mock_json_provider(body: serde_json::Value) -> (wiremock::MockServer, crate::Provider) {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer synthetic-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(body),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider = crate::Provider::new(
        "test",
        OpenAIProtocolChatDriver::new().with_retry_config(LlmRetryConfig {
            max_retries: 0,
            ..Default::default()
        }),
    )
    .base_url(format!("{}/v1", server.uri()))
    .auth(crate::BearerAuth::new("synthetic-key"));
    (server, provider)
}

#[tokio::test]
async fn non_streaming_completion_waits_for_full_json_response() {
    let (server, provider) = mock_json_provider(json!({
        "id": "chatcmpl-123",
        "model": "gpt-5.2-2026-09-01",
        "choices": [{
            "message": {"role": "assistant", "content": "done"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 3}
    }))
    .await;
    assert!(provider.supports_native_non_streaming());
    let messages = vec![Message::text(MessageRole::User, "Hello")];
    let response = provider
        .chat_completion_non_streaming(messages, &call_config())
        .await
        .unwrap();
    assert_eq!(response.text, "done");
    assert_eq!(response.metadata.finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        response.metadata.response_id.as_deref(),
        Some("chatcmpl-123")
    );
    assert_eq!(
        response.metadata.response_model.as_deref(),
        Some("gpt-5.2-2026-09-01")
    );
    assert_eq!(response.metadata.prompt_tokens, Some(10));
    assert_eq!(response.metadata.completion_tokens, Some(3));
    assert!(response.tool_calls.is_none());
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let sent = requests[0].body_json::<Value>().unwrap();
    assert_eq!(sent["stream"], json!(false));
    assert!(sent.get("stream_options").is_none());
}

#[tokio::test]
async fn non_streaming_completion_maps_tool_calls_and_drops_malformed_arguments() {
    let (server, provider) = mock_json_provider(json!({
        "id": "chatcmpl-456",
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    {"id": "call_1", "type": "function",
                     "function": {"name": "get_weather", "arguments": "{\"city\":\"Oslo\"}"}},
                    {"id": "call_2", "type": "function",
                     "function": {"name": "broken", "arguments": "{oops"}}
                ]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {"prompt_tokens": 20, "completion_tokens": 15}
    }))
    .await;
    let messages = vec![Message::text(MessageRole::User, "Weather?")];
    let response = provider
        .chat_completion_non_streaming(messages, &call_config())
        .await
        .unwrap();
    let tool_calls = response.tool_calls.expect("tool calls survive");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].name, "get_weather");
    assert_eq!(tool_calls[0].arguments, json!({"city": "Oslo"}));
    assert_eq!(
        response.metadata.finish_reason.as_deref(),
        Some("tool_calls")
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0].body_json::<Value>().unwrap()["stream"],
        json!(false)
    );
}

#[tokio::test]
async fn request_options_reach_complete_http_payload_without_reimplementing_filters() {
    for (reasoning, parallel, tier, verbosity) in [
        (Some(crate::model::ReasoningEffort::None), None, None, None),
        (
            Some(crate::model::ReasoningEffort::High),
            Some(true),
            Some("priority"),
            Some("high"),
        ),
        (None, Some(false), Some("flex"), Some("low")),
    ] {
        let (server, provider) = mock_provider("data: [DONE]\n\n").await;
        let mut config = call_config();
        config.reasoning_effort = reasoning;
        config.parallel_tool_calls = parallel;
        config.speed = tier.map(str::to_string);
        config.verbosity = verbosity.map(str::to_string);
        config.temperature = Some(0.5);
        config.max_tokens = Some(128);
        if tier.is_some() {
            config
                .metadata
                .insert("session_id".into(), "session_abc123".into());
            config
                .metadata
                .insert("agent_id".into(), "agent_xyz789".into());
        }
        let messages = vec![
            Message::text(MessageRole::System, "A"),
            Message::text(MessageRole::User, "Hello"),
            Message::text(MessageRole::System, "B"),
        ];
        provider.chat_completion(messages, &config).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let mut expected = json!({"model":"model","messages":[{"role":"system","content":"A"},{"role":"user","content":"Hello"},{"role":"system","content":"B"}],"temperature":0.5,"max_tokens":128,"stream":true,"stream_options":{"include_usage":true}});
        if tier == Some("priority") {
            expected["reasoning_effort"] = json!("high");
        }
        if let Some(value) = parallel {
            expected["parallel_tool_calls"] = json!(value);
        }
        if let Some(value) = tier {
            expected["service_tier"] = json!(value);
            expected["metadata"] = json!({"session_id":"session_abc123","agent_id":"agent_xyz789"});
        }
        if let Some(value) = verbosity {
            expected["verbosity"] = json!(value);
        }
        assert_eq!(requests[0].body_json::<Value>().unwrap(), expected);
    }
}

#[tokio::test]
async fn streamed_usage_and_first_id_reach_completion_metadata() {
    for (reported, usage_index) in [(0, 0), (2, 1), (3, 2)] {
        let mut chunks = [
            json!({"choices":[{"delta":{"content":"Hello"}}]}),
            json!({"choices":[{"delta":{"content":"!"},"finish_reason":"stop"}]}),
            json!({"choices":[]}),
        ];
        chunks[usage_index]["usage"] = json!({"prompt_tokens":10,"completion_tokens":reported});
        let sse = chunks
            .iter()
            .map(|chunk| format!("data: {chunk}\n\n"))
            .collect::<String>()
            + "data: [DONE]\n\n";
        let (_server, provider) = mock_provider(&sse).await;
        let response = provider
            .chat_completion(vec![], &call_config())
            .await
            .unwrap();
        assert_eq!(response.text, "Hello!");
        assert_eq!(response.metadata.completion_tokens, Some(reported));
        assert_eq!(response.metadata.total_tokens, Some(10 + reported));
    }
    for (usage, expected) in [
        (
            json!({"prompt_tokens":150,"completion_tokens":42}),
            (150, 42, None, None, 192),
        ),
        (
            json!({"prompt_tokens":150,"completion_tokens":42,"prompt_tokens_details":{"cached_tokens":100}}),
            (50, 42, Some(100), None, 192),
        ),
        (
            json!({"prompt_tokens":194,"completion_tokens":2,"cost":0.00095}),
            (194, 2, None, Some(0.00095), 196),
        ),
        (
            json!({"prompt_tokens":10,"completion_tokens":5,"cost":0.0}),
            (10, 5, None, Some(0.0), 15),
        ),
    ] {
        let sse = format!(
            "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":{"content":"Hello"}}]}),
            json!({"id":"first-id","choices":[{"delta":{},"finish_reason":"stop"}]}),
            json!({"id":"later-id","choices":[],"usage":usage})
        );
        let (_server, provider) = mock_provider(&sse).await;
        let response = provider
            .chat_completion(vec![], &call_config())
            .await
            .unwrap();
        assert_eq!(response.text, "Hello");
        assert!(response.tool_calls.is_none());
        assert!(response.reasoning.is_empty());
        let meta = response.metadata;
        assert_eq!(
            (
                meta.prompt_tokens,
                meta.completion_tokens,
                meta.cache_read_tokens,
                meta.provider_cost_usd
            ),
            (Some(expected.0), Some(expected.1), expected.2, expected.3)
        );
        assert_eq!(meta.total_tokens, Some(expected.4));
        assert_eq!(meta.response_id.as_deref(), Some("first-id"));
        assert_eq!(meta.finish_reason.as_deref(), Some("stop"));
        assert_eq!(meta.model.as_deref(), Some("model"));
        assert!(meta.cache_creation_tokens.is_none());
        assert!(meta.retry_metadata.is_none());
    }
    let (_server, provider) = mock_provider(
        "data: {\"choices\":[{\"delta\":{\"content\":\"text\"}}]}\n\ndata: [DONE]\n\n",
    )
    .await;
    let response = provider
        .chat_completion(vec![], &call_config())
        .await
        .unwrap();
    assert_eq!(response.metadata.response_id, None);
    assert_eq!(response.metadata.response_model, None);
    assert_eq!(response.metadata.completion_tokens, Some(1));
    assert_eq!(response.metadata.provider_cost_usd, None);
}

#[tokio::test]
async fn terminal_reasons_survive_final_deltas_and_block_truncated_tool_execution() {
    for reason in ["length", "content_filter"] {
        for delta in [
            json!({"content":"partial"}),
            json!({"tool_calls":[{"index":0,"id":"call","function":{"name":"run","arguments":"{}"}}]}),
        ] {
            let sse = format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"delta":delta,"finish_reason":reason}]})
            );
            let (_server, provider) = mock_provider(&sse).await;
            let response = provider
                .chat_completion(vec![], &call_config())
                .await
                .unwrap();
            assert_eq!(response.metadata.finish_reason.as_deref(), Some(reason));
            assert!(
                response.tool_calls.is_none(),
                "truncated/rejected calls must not execute"
            );
            // ...but the discard is reported, not silent.
            let dropped = u32::from(delta.get("tool_calls").is_some());
            assert_eq!(response.metadata.tool_calls_dropped, dropped);
            assert_eq!(
                response.metadata.provider_finish_reason.as_deref(),
                Some(reason)
            );
            assert_eq!(
                response.text,
                if delta.get("content").is_some() {
                    "partial"
                } else {
                    ""
                }
            );
        }
    }
}

#[test]
fn azure_host_detection_rejects_lookalikes_and_ignores_url_components() {
    for (url, azure) in [
        (
            "https://example.openai.azure.com/openai/v1/chat/completions",
            true,
        ),
        (
            "https://example.services.ai.azure.com/openai/v1/responses",
            true,
        ),
        ("https://EXAMPLE.OPENAI.AZURE.COM:8443/path?x=1", true),
        ("https://api.openai.com/v1/chat/completions", false),
        ("https://example.openai.azure.com.evil.test/path", false),
        ("https://example.openai.azure.com@evil.test/", false),
        ("https://evil.test/?host=example.openai.azure.com", false),
        ("not a URL", false),
    ] {
        assert_eq!(is_azure_openai_api_url(url), azure, "{url}");
    }
}

#[test]
fn orphan_filter_preserves_complete_matched_transcript_and_rejects_missing_ids() {
    use crate::tool_types::ToolCall;
    let mut assistant = Message::text(MessageRole::Assistant, "");
    assistant.tool_calls = Some(vec![ToolCall {
        id: "call".into(),
        name: "read_file".into(),
        arguments: json!({"path":"a"}),
    }]);
    let tool = |id: Option<&str>, text: &str| {
        let mut m = Message::text(MessageRole::Tool, text);
        m.tool_call_id = id.map(str::to_string);
        m
    };
    let messages = vec![
        Message::text(MessageRole::User, "hello"),
        assistant,
        tool(Some("call"), "file content"),
        tool(Some("trimmed"), "orphan"),
        tool(None, "missing id"),
    ];
    let filtered = drop_orphaned_tool_messages(&messages);
    let wire: Vec<_> = filtered
        .iter()
        .map(crate::openai_message_convert::convert_message)
        .collect();
    assert_eq!(
        serde_json::to_value(wire).unwrap(),
        json!([
            {"role":"user","content":"hello"},
            {"role":"assistant","content":"","tool_calls":[{"id":"call","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a\"}"}}]},
            {"role":"tool","content":"file content","tool_call_id":"call"}
        ])
    );
    let only_user = drop_orphaned_tool_messages(&[
        messages[0].clone(),
        messages[3].clone(),
        messages[4].clone(),
    ]);
    assert_eq!(
        serde_json::to_value(
            only_user
                .iter()
                .map(crate::openai_message_convert::convert_message)
                .collect::<Vec<_>>()
        )
        .unwrap(),
        json!([{"role":"user","content":"hello"}])
    );
}
#[test]
fn discovery_urls_preserve_origin_queries_and_custom_paths() {
    for (input, expected) in [
        (
            "https://api.openai.com/v1/responses",
            "https://api.openai.com/v1/models",
        ),
        (
            "https://openrouter.ai/api/v1/responses?route=a%20b#section",
            "https://openrouter.ai/api/v1/models?route=a%20b#section",
        ),
        (
            "https://api.fireworks.ai/inference/v1/chat/completions/",
            "https://api.fireworks.ai/inference/v1/models",
        ),
        (
            "https://api.meta.ai/v1/models?x=1",
            "https://api.meta.ai/v1/models?x=1",
        ),
        (
            "https://resource.openai.azure.com/custom?api-version=preview",
            "https://resource.openai.azure.com/custom/models?api-version=preview",
        ),
        (
            "https://resource.services.ai.azure.com/openai/v1/?api-version=preview",
            "https://resource.services.ai.azure.com/openai/v1/models?api-version=preview",
        ),
        (
            "https://proxy.example:8443/tenant%20one",
            "https://proxy.example:8443/tenant%20one/models",
        ),
        ("https://proxy.example", "https://proxy.example/models"),
        ("not a URL", "not a URL"),
    ] {
        assert_eq!(models_url_for_api_url(input), expected, "{input}");
    }
}

#[test]
fn models_status_error_classifies_auth_separately_from_outage() {
    // Credential checks branch on the kind, so a rejected key and a dead
    // provider must not collapse into the same classification.
    assert_eq!(
        models_api_status_error(reqwest::StatusCode::UNAUTHORIZED).llm_error_kind(),
        Some(LlmErrorKind::Authentication)
    );
    assert_eq!(
        models_api_status_error(reqwest::StatusCode::FORBIDDEN).llm_error_kind(),
        Some(LlmErrorKind::Authentication)
    );
    assert_eq!(
        models_api_status_error(reqwest::StatusCode::SERVICE_UNAVAILABLE).llm_error_kind(),
        Some(LlmErrorKind::Unavailable)
    );
}

#[test]
fn file_part_serializes_to_openai_file() {
    let part = OpenAiContentPart::File {
        r#type: "file".to_string(),
        file: OpenAiFile {
            filename: Some("report.pdf".to_string()),
            file_data: "data:application/pdf;base64,JVBERi0=".to_string(),
        },
    };
    let v = serde_json::to_value(&part).unwrap();
    assert_eq!(v["type"], serde_json::json!("file"));
    assert_eq!(
        v["file"]["file_data"],
        serde_json::json!("data:application/pdf;base64,JVBERi0=")
    );
    assert_eq!(v["file"]["filename"], serde_json::json!("report.pdf"));
}
