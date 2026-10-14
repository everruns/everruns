use super::*;
use everruns_contracts::driver_registry::ChatDriver;
use everruns_contracts::tool_types::ToolCall;

#[tokio::test]
async fn long_decode_resumes_only_at_a_safe_checkpoint_and_bills_committed_usage() {
    use wiremock::matchers::{body_partial_json, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    for (later_output, transport_error) in
        [(false, false), (false, true), (true, false), (true, true)]
    {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"continuationToken":"checkpoint"})))
            .respond_with(ResponseTemplate::new(200).insert_header("content-type","text/event-stream").insert_header("Connection","close")
                .set_body_string("data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"last\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":13,\"candidatesTokenCount\":1,\"thoughtsTokenCount\":3}}\n\n"))
            .expect(if later_output {0} else {1}).mount(&server).await;
        let checkpoint = json!({"candidates":[{"content":{"parts":[{"text":"first"}]},"continuationToken":"checkpoint"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":1,"thoughtsTokenCount":2}});
        let tail = if later_output {
            json!({"candidates":[{"content":{"parts":[{"text":"extra"}]}}]})
        } else {
            // Usage after the checkpoint describes work that is not committed
            // on a dropped stream and must not be charged again on resume.
            json!({"usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":1000,"thoughtsTokenCount":500}})
        };
        let mut chunks = vec![Ok(format!("data: {checkpoint}\n\ndata: {tail}\n\n")
            .into_bytes()
            .into())];
        if transport_error {
            chunks.push(Err(driver_helpers::shared_streaming_http_client()
                .get("http://[invalid")
                .build()
                .unwrap_err()));
        }
        let bytes: ByteStream = Box::pin(futures::stream::iter(chunks));
        let context = ContinuationRequest {
            driver: GeminiChatDriver::new(),
            endpoint: everruns_contracts::ProviderEndpoint::from_parts(
                server.uri(),
                everruns_contracts::StaticHeaderAuth::new("x-goog-api-key", "synthetic"),
            ),
            request: GeminiRequest {
                contents: vec![GeminiContent {
                    role: Some("user".into()),
                    parts: vec![GeminiPart::text("question")],
                }],
                system_instruction: None,
                tools: None,
                cached_content: None,
                continuation_token: None,
                generation_config: Some(GeminiGenerationConfig {
                    temperature: None,
                    max_output_tokens: Some(100_000),
                    thinking_config: None,
                }),
            },
            url: format!(
                "{}/models/gemini-4-argon:streamGenerateContent?alt=sse",
                server.uri()
            ),
            model: "gemini-4-argon".into(),
            extra_headers: vec![],
            hops: 0,
        };
        let stream = convert_gemini_stream(
            bytes,
            GeminiStreamState {
                model: "gemini-4-argon".into(),
                ..Default::default()
            },
            Some(context),
        );
        let events: Vec<_> = stream.map(|event| event.unwrap()).collect().await;
        assert!(matches!(&events[0],LlmStreamEvent::TextDelta(text) if text == "first"));
        if later_output {
            assert!(matches!(events.last(), Some(LlmStreamEvent::Error(_))));
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, LlmStreamEvent::Done(_)))
            );
        } else {
            assert!(matches!(&events[1],LlmStreamEvent::TextDelta(text) if text == "last"));
            let Some(LlmStreamEvent::Done(meta)) = events.last() else {
                panic!("missing completion")
            };
            assert_eq!(meta.prompt_tokens, Some(23));
            assert_eq!(meta.completion_tokens, Some(7));
            assert_eq!(meta.reasoning_tokens, Some(5));
        }
        server.verify().await;
    }
}

// ========================================================================
// Model-not-found detection tests
// ========================================================================

fn call_message(id: &str, name: &str, arguments: Value) -> Message {
    let mut message = Message::text(MessageRole::Assistant, "");
    message.tool_calls = Some(vec![ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }]);
    message
}

fn result_message(id: Option<&str>, text: &str) -> Message {
    let mut message = Message::text(MessageRole::Tool, text);
    message.tool_call_id = id.map(str::to_string);
    message
}

#[test]
fn transcript_preserves_system_order_and_resolves_tool_names_per_turn() {
    let messages = vec![
        Message::text(MessageRole::System, "A"),
        result_message(Some("call_0"), "future orphan"),
        Message::text(MessageRole::User, "hi"),
        call_message("call_0", "get_weather", json!({"city":"Paris"})),
        result_message(Some("call_0"), r#"{"temp":20}"#),
        Message::text(MessageRole::System, "B"),
        call_message("call_0", "get_time", json!({"zone":"UTC"})),
        result_message(Some("call_0"), "12:00"),
        result_message(Some("missing"), "orphan"),
        result_message(None, "missing id"),
    ];
    let (system, contents) = GeminiChatDriver::convert_messages(&messages);
    assert_eq!(
        serde_json::to_value(system).unwrap(),
        json!({"parts":[{"text":"A\n\nB"}]})
    );
    assert_eq!(
        serde_json::to_value(contents).unwrap(),
        json!([
            {"role":"user","parts":[{"text":"hi"}]},
            {"role":"model","parts":[{"functionCall":{"name":"get_weather","args":{"city":"Paris"}}}]},
            {"role":"user","parts":[{"functionResponse":{"name":"get_weather","response":{"temp":20}}}]},
            {"role":"model","parts":[{"functionCall":{"name":"get_time","args":{"zone":"UTC"}}}]},
            {"role":"user","parts":[{"functionResponse":{"name":"get_time","response":{"result":"12:00"}}}]}
        ])
    );
    let (system, contents) = GeminiChatDriver::convert_messages(&[
        Message::text(MessageRole::User, ""),
        result_message(Some("missing"), "orphan"),
    ]);
    assert!(system.is_none());
    assert!(contents.is_empty());
}

#[test]
fn function_response_payloads_are_objects_without_losing_scalar_values() {
    for (text, expected) in [
        (r#"{"value":3}"#, json!({"value":3})),
        ("plain result", json!({"result":"plain result"})),
        ("[1,2]", json!({"result":[1,2]})),
        ("null", json!({"result":null})),
        ("true", json!({"result":true})),
        ("42", json!({"result":42})),
        (r#""quoted""#, json!({"result":"quoted"})),
    ] {
        let (_, contents) = GeminiChatDriver::convert_messages(&[
            call_message("call_7", "lookup", json!({})),
            result_message(Some("call_7"), text),
        ]);
        assert_eq!(
            serde_json::to_value(&contents[1]).unwrap(),
            json!({"role":"user","parts":[{"functionResponse":{"name":"lookup","response":expected}}]}),
            "{text}"
        );
    }
}

#[test]
fn content_conversion_preserves_text_and_media_order_while_omitting_empty_parts() {
    let parts = MessageContent::Parts(vec![
        LlmContentPart::text(""),
        LlmContentPart::text("before"),
        LlmContentPart::image("data:image/png;base64,aGVsbG8="),
        LlmContentPart::image("data:malformed"),
        LlmContentPart::image("https://images.example/photo.jpg"),
        LlmContentPart::Audio {
            url: "data:audio/wav;base64,YQ==".into(),
        },
        LlmContentPart::text("after"),
    ]);
    assert_eq!(
        serde_json::to_value(GeminiChatDriver::convert_content(&parts)).unwrap(),
        json!([
            {"text":"before"}, {"inlineData":{"mimeType":"image/png","data":"aGVsbG8="}},
            {"fileData":{"mimeType":"image/jpeg","fileUri":"https://images.example/photo.jpg"}},
            {"text":"[Audio content not supported]"}, {"text":"after"}
        ])
    );
    assert_eq!(
        serde_json::to_value(GeminiChatDriver::convert_content(&MessageContent::Text(
            "hello".into()
        )))
        .unwrap(),
        json!([{"text":"hello"}])
    );
    assert_eq!(
        serde_json::to_value(GeminiChatDriver::convert_content(&MessageContent::Text(
            String::new()
        )))
        .unwrap(),
        json!([])
    );
}
async fn collect_wire_chunks(chunks: Vec<Vec<u8>>) -> Vec<Value> {
    let bytes: ByteStream = Box::pin(futures::stream::iter(
        chunks.into_iter().map(|chunk| Ok(chunk.into())),
    ));
    let mut stream = convert_gemini_stream(
        bytes,
        GeminiStreamState {
            model: "model".into(),
            ..Default::default()
        },
        None,
    );
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(match event.unwrap() {
            LlmStreamEvent::TextDelta(text) => json!({"text":text}),
            LlmStreamEvent::ToolCalls(calls) => json!({"calls":calls}),
            LlmStreamEvent::Done(metadata) => json!({"done":{"model":metadata.model,"reason":metadata.finish_reason,"input":metadata.prompt_tokens,"output":metadata.completion_tokens,"total":metadata.total_tokens}}),
            other => panic!("unexpected event: {other:?}"),
        });
    }
    events
}

fn expected_unicode_events() -> Vec<Value> {
    vec![
        json!({"text":"hé🙂"}),
        json!({"calls":[{"id":"call_0","name":"lookup","arguments":{"path":"café/🙂"}}]}),
        json!({"done":{"model":"model","reason":"stop","input":3,"output":1,"total":4}}),
    ]
}

#[tokio::test]
async fn stream_preserves_unicode_at_every_transport_boundary() {
    let wire = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hé🙂\"},{\"functionCall\":{\"name\":\"lookup\",\"args\":{\"path\":\"café/🙂\"}}}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":1}}\n\ndata: [DONE]\n\n".as_bytes();
    for split in 0..=wire.len() {
        assert_eq!(
            collect_wire_chunks(vec![wire[..split].to_vec(), wire[split..].to_vec()]).await,
            expected_unicode_events(),
            "byte split {split}"
        );
    }
    assert_eq!(
        collect_wire_chunks(wire.iter().map(|byte| vec![*byte]).collect()).await,
        expected_unicode_events()
    );
}

#[tokio::test]
async fn stream_accepts_multiline_sse_data_and_optional_space() {
    let wire = ": heartbeat\r\nevent: message\r\nid: frame-one\r\ndata:{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hé🙂\"},{\"functionCall\":{\"name\":\"lookup\",\"args\":{\"path\":\"café/🙂\"}}}]},\"finishReason\":\"STOP\"}],\r\ndata: \"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":1}}\r\n\r\ndata:[DONE]\r\n\r\n".as_bytes();
    assert_eq!(
        collect_wire_chunks(wire.iter().map(|byte| vec![*byte]).collect()).await,
        expected_unicode_events()
    );
}

#[test]
fn tool_schema_cleanup_preserves_complete_contract_and_literal_payloads() {
    use everruns_contracts::tool_types::{BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy};
    let parameters = json!({
        "type":"object","additionalProperties":false,"required":["items"],
        "properties":{
            "additionalProperties":{"type":"boolean","description":"user property"},
            "items":{"type":"array","items":{"type":"object","additionalProperties":false,"properties":{"path":{"type":"string"},"offset":{"type":"integer","minimum":0,"default":0}},"required":["path"]}},
            "variant":{"anyOf":[{"type":"string"},{"type":"object","additionalProperties":true}]}
        },
        "default":{"additionalProperties":"default data"},
        "examples":[{"additionalProperties":false}],
        "enum":[{"additionalProperties":"enum data"}],
        "const":{"additionalProperties":"constant data"},
        "x-extension":{"additionalProperties":"extension data"}
    });
    let tools = vec![ToolDefinition::Builtin(BuiltinTool {
        name: "inspect".into(),
        display_name: None,
        description: "Inspect paths".into(),
        parameters,
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints: ToolHints::default(),
        full_parameters: None,
    })];
    assert_eq!(
        serde_json::to_value(GeminiChatDriver::convert_tools(&tools).unwrap()).unwrap(),
        json!([{"functionDeclarations":[{"name":"inspect","description":"Inspect paths","parameters":{
            "type":"object","required":["items"],
            "properties":{
                "additionalProperties":{"type":"boolean","description":"user property"},
                "items":{"type":"array","items":{"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer","minimum":0,"default":0}},"required":["path"]}},
                "variant":{"anyOf":[{"type":"string"},{"type":"object"}]}
            },
            "default":{"additionalProperties":"default data"},"examples":[{"additionalProperties":false}],"enum":[{"additionalProperties":"enum data"}],"const":{"additionalProperties":"constant data"},"x-extension":{"additionalProperties":"extension data"}
        }}]}])
    );
    assert!(GeminiChatDriver::convert_tools(&[]).is_none());
}
#[test]
fn size_classification_requires_status_and_provider_or_context_evidence() {
    for (status, message, expected) in [
        (413, "", true),
        (400, "Request payload size exceeds the limit", true),
        (400, "content too large", true),
        (400, "TOKEN LIMIT EXCEEDED", true),
        (400, "input is too long", true),
        (400, "request exceeds the maximum context", true),
        (400, "rate limit exceeded", false),
        (401, "Invalid API key", false),
        (500, "token limit exceeded", false),
        (200, "content too large", false),
    ] {
        assert_eq!(
            is_gemini_request_too_large(reqwest::StatusCode::from_u16(status).unwrap(), message),
            expected,
            "{status}: {message}"
        );
    }
}

#[test]
fn missing_model_classification_requires_404_and_gemini_evidence() {
    for (status, message, expected) in [
        (404, r#"{"error":{"status":"NOT_FOUND"}}"#, true),
        (404, "Model does not exist", true),
        (404, "Endpoint not found", false),
        (400, r#"{"error":{"status":"NOT_FOUND"}}"#, false),
        (401, "model not found", false),
        (500, "Model does not exist", false),
    ] {
        assert_eq!(
            is_gemini_model_not_found(reqwest::StatusCode::from_u16(status).unwrap(), message),
            expected,
            "{status}: {message}"
        );
    }
}

#[tokio::test]
async fn request_limits_cache_gate_and_parallel_preference_reach_wire_contract() {
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    for (model, limit, cache, parallel, expected_limit, expected_cache) in [
        (
            "gemini-3.1-pro-preview",
            None,
            Some((false, Some("cachedContents/disabled"))),
            Some(false),
            65536,
            None,
        ),
        (
            "unknown-model",
            None,
            Some((true, None)),
            Some(true),
            8192,
            None,
        ),
        (
            "gemini-3.1-pro-preview",
            Some(7),
            Some((true, Some("cachedContents/active"))),
            None,
            7,
            Some("cachedContents/active"),
        ),
        ("unknown-model", Some(9), None, Some(false), 9, None),
    ] {
        let server = MockServer::builder().start().await;
        Mock::given(method("POST")).and(path(format!("/v1beta/models/{model}:streamGenerateContent"))).and(query_param("alt", "sse")).and(header("x-goog-api-key", "synthetic-key")).respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string("data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ok\"}]},\"finishReason\":\"STOP\"}]}\n\n")).expect(1).mount(&server).await;
        let service =
            provider("test", "synthetic-key").base_url(format!("{}/v1beta", server.uri()));
        let mut config = LlmCallConfig::new(model);
        config.temperature = Some(0.25);
        config.max_tokens = limit;
        config.prompt_cache =
            cache.map(
                |(enabled, handle)| everruns_contracts::driver_registry::PromptCacheConfig {
                    enabled,
                    gemini_cached_content: handle.map(str::to_string),
                    ..Default::default()
                },
            );
        config.parallel_tool_calls = parallel;
        let response = service
            .chat_completion(vec![Message::text(MessageRole::User, "hi")], &config)
            .await
            .unwrap();
        assert_eq!(response.text, "ok");
        assert_eq!(response.metadata.finish_reason.as_deref(), Some("stop"));
        assert!(!GeminiChatDriver::new().supports_parallel_tool_calls(model));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let mut expected = json!({"contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"temperature":0.25,"maxOutputTokens":expected_limit}});
        if let Some(handle) = expected_cache {
            expected["cachedContent"] = json!(handle);
        }
        assert_eq!(requests[0].body_json::<Value>().unwrap(), expected);
    }
}

#[test]
fn file_pdf_serializes_to_inline_data() {
    let content = MessageContent::Parts(vec![LlmContentPart::File {
        url: "data:application/pdf;base64,JVBERi0=".into(),
        filename: Some("report.pdf".into()),
    }]);
    let v = serde_json::to_value(GeminiChatDriver::convert_content(&content)).unwrap();
    assert_eq!(
        v,
        serde_json::json!([{
            "inlineData": {"mimeType": "application/pdf", "data": "JVBERi0="}
        }])
    );
}

fn web_search_config(model: &str, tools: &[&str], native: NativeWebSearchOption) -> LlmCallConfig {
    let mut config = LlmCallConfig::new(model);
    config.tools = tools
        .iter()
        .map(|name| ToolDefinition::function(*name, "a tool", json!({ "type": "object" })))
        .collect();
    if let Some(native) = native {
        let (key, value) = native.to_driver_option();
        config.driver_options.insert(key, value);
    }
    config
}

type NativeWebSearchOption = Option<everruns_contracts::native_web_search::NativeWebSearch>;

#[test]
fn google_search_replaces_a_lone_web_search_tool() {
    use everruns_contracts::native_web_search::NativeWebSearch;
    let lone = web_search_config(
        "gemini-3.8-flash",
        &["web_search"],
        Some(NativeWebSearch::default()),
    );
    let tool = GeminiChatDriver::native_web_search_tool(&lone).unwrap();
    assert_eq!(
        serde_json::to_value(vec![tool]).unwrap(),
        json!([{ "google_search": {} }])
    );
    // Gemini 2.5 grounds too.
    let older = web_search_config(
        "gemini-2.5-flash",
        &["web_search"],
        Some(NativeWebSearch::default()),
    );
    assert!(GeminiChatDriver::native_web_search_tool(&older).is_some());
}

#[test]
fn google_search_declines_what_grounding_cannot_do() {
    use everruns_contracts::native_web_search::NativeWebSearch;
    let declined = [
        // Not requested.
        web_search_config("gemini-3.8-flash", &["web_search"], None),
        // Other function tools on the call (see the decision in driver.rs).
        web_search_config(
            "gemini-3.8-flash",
            &["web_search", "web_fetch"],
            Some(NativeWebSearch::default()),
        ),
        // Grounding takes no domain filters.
        web_search_config(
            "gemini-3.8-flash",
            &["web_search"],
            Some(NativeWebSearch {
                blocked_domains: vec!["reddit.com".into()],
                ..Default::default()
            }),
        ),
        web_search_config(
            "gemini-3.8-flash",
            &["web_search"],
            Some(NativeWebSearch {
                allowed_domains: vec!["docs.rs".into()],
                ..Default::default()
            }),
        ),
        // A model without Google Search grounding.
        web_search_config(
            "gemini-1.5-pro",
            &["web_search"],
            Some(NativeWebSearch::default()),
        ),
    ];
    for config in declined {
        assert!(
            GeminiChatDriver::native_web_search_tool(&config).is_none(),
            "{:?} {:?}",
            config.model,
            config.tools.iter().map(|t| t.name()).collect::<Vec<_>>()
        );
    }
}

#[test]
fn grounding_queries_become_hosted_calls_and_billed_searches() {
    let chunk = |queries: Value, finish: Option<&str>| GeminiStreamResponse {
        model_version: None,
        candidates: Some(vec![
            serde_json::from_value(json!({
                "content": { "parts": [{ "text": "Rust 1.95." }] },
                "finishReason": finish,
                "groundingMetadata": { "webSearchQueries": queries,
                    "groundingChunks": [{ "web": { "uri": "https://example.com", "title": "x" } }] }
            }))
            .unwrap(),
        ]),
        usage_metadata: None,
    };
    for (model, billed) in [("gemini-3.8-flash", 2), ("gemini-2.5-flash", 1)] {
        let mut state = GeminiStreamState {
            model: model.into(),
            grounded: true,
            ..Default::default()
        };
        state.response(chunk(json!(["rust release", "rust 1.95"]), None));
        // A later chunk repeats the queries: counted once.
        state.response(chunk(json!(["rust release", "rust 1.95"]), Some("STOP")));
        state.finish();
        let events: Vec<_> = state.pending.drain(..).collect();
        let calls: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                LlmStreamEvent::HostedToolCall(call) => Some(call.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(calls.len(), 2, "{model}");
        assert_eq!(calls[0].tool, "web_search");
        assert_eq!(calls[0].summary.as_deref(), Some("rust release"));
        assert_eq!(
            calls[1].status,
            everruns_contracts::driver_registry::HostedToolCallStatus::Completed
        );
        let Some(LlmStreamEvent::Done(metadata)) = events.last() else {
            panic!("expected done");
        };
        assert_eq!(
            metadata.hosted_tool_calls.get("web_search_call"),
            Some(&billed),
            "{model}"
        );
    }
    // Without the search tool on the request nothing is counted.
    let mut plain = GeminiStreamState {
        model: "gemini-3.8-flash".into(),
        ..Default::default()
    };
    plain.response(chunk(json!(["q"]), Some("STOP")));
    plain.finish();
    let Some(LlmStreamEvent::Done(metadata)) = plain.pending.back() else {
        panic!("expected done");
    };
    assert!(metadata.hosted_tool_calls.is_empty());
}

#[tokio::test]
async fn native_search_request_sends_google_search_instead_of_the_function() {
    use everruns_contracts::native_web_search::NativeWebSearch;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path(
            "/v1beta/models/gemini-3.8-flash:streamGenerateContent",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(format!(
                    "data: {}\n\n",
                    json!({ "candidates": [{ "content": { "parts": [{ "text": "ok" }] },
                        "finishReason": "STOP",
                        "groundingMetadata": { "webSearchQueries": ["q"] } }] })
                )),
        )
        .mount(&server)
        .await;
    let service = provider("test", "synthetic-key").base_url(format!("{}/v1beta", server.uri()));
    for native in [Some(NativeWebSearch::default()), None] {
        let config = web_search_config("gemini-3.8-flash", &["web_search"], native.clone());
        let response = service
            .chat_completion(vec![Message::text(MessageRole::User, "news?")], &config)
            .await
            .unwrap();
        assert_eq!(response.text, "ok");
        let body: Value = server
            .received_requests()
            .await
            .unwrap()
            .last()
            .unwrap()
            .body_json()
            .unwrap();
        if native.is_some() {
            assert_eq!(body["tools"], json!([{ "google_search": {} }]));
            assert_eq!(
                response.metadata.hosted_tool_calls.get("web_search_call"),
                Some(&1)
            );
        } else {
            // Option absent: the function tool goes out unchanged.
            assert_eq!(
                body["tools"][0]["functionDeclarations"][0]["name"],
                "web_search"
            );
            assert!(body["tools"][0].get("google_search").is_none());
            assert!(response.metadata.hosted_tool_calls.is_empty());
        }
    }
}

#[test]
fn grounding_supports_become_web_citations_over_the_streamed_answer() {
    let chunk = |text: &str, grounding: Option<Value>| GeminiStreamResponse {
        model_version: None,
        candidates: Some(vec![
            serde_json::from_value(json!({
                "content": { "parts": [{ "text": text }] },
                "finishReason": grounding.as_ref().map(|_| "STOP"),
                "groundingMetadata": grounding,
            }))
            .unwrap(),
        ]),
        usage_metadata: None,
    };
    // Byte offsets into the whole answer: "Grüße 😀! " is 14 bytes, 9 chars.
    let grounding = json!({
        "webSearchQueries": ["rust 1.95"],
        "groundingChunks": [{ "web": { "uri": "https://vertexaisearch.example/r", "title": "rust-lang.org" } }],
        "groundingSupports": [{ "segment": { "startIndex": 14, "endIndex": 31, "text": "Rust 1.95 ist da." },
                                "groundingChunkIndices": [0] }]
    });
    for grounded in [true, false] {
        let mut state = GeminiStreamState {
            model: "gemini-3.8-flash".into(),
            grounded,
            ..Default::default()
        };
        state.response(chunk("Grüße 😀! ", None));
        state.response(chunk("Rust 1.95 ist da.", Some(grounding.clone())));
        state.finish();
        let Some(LlmStreamEvent::Done(metadata)) = state.pending.back() else {
            panic!("expected done");
        };
        if !grounded {
            // Not a grounded request: nothing to cite.
            assert!(metadata.web_citations.is_empty());
            continue;
        }
        let [citation] = metadata.web_citations.as_slice() else {
            panic!("expected one citation: {:?}", metadata.web_citations);
        };
        assert_eq!(citation.url, "https://vertexaisearch.example/r");
        assert_eq!(citation.title.as_deref(), Some("rust-lang.org"));
        let span = citation.span.as_ref().unwrap();
        assert_eq!((span.start, span.end), (9, 26));
        assert_eq!(span.text, "Rust 1.95 ist da.");
    }
}

#[test]
fn function_response_is_followed_by_returned_pdf() {
    let mut result = result_message(Some("call_8"), "");
    result.content = MessageContent::Parts(vec![
        LlmContentPart::text(r#"{"path":"/attachments/a1/report.pdf"}"#),
        LlmContentPart::file("data:application/pdf;base64,JVBERi0=", None),
    ]);

    let (_, contents) = GeminiChatDriver::convert_messages(&[
        call_message("call_8", "read_file", json!({})),
        result,
    ]);

    assert_eq!(
        serde_json::to_value(&contents[1]).unwrap(),
        json!({"role":"user","parts":[
            {"functionResponse":{"name":"read_file","response":{"path":"/attachments/a1/report.pdf"}}},
            {"inlineData":{"mimeType":"application/pdf","data":"JVBERi0="}}
        ]})
    );
}
