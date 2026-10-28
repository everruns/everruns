#![cfg(feature = "mistral")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use everruns_contracts::driver_registry::{
    ChatDriver, DriverId, DriverRegistry, LlmCallConfig, LlmContentPart, LlmStreamEvent, Message,
    MessageContent, MessageRole, ProviderConfig,
};
use everruns_contracts::model::ReasoningEffort;
use everruns_contracts::reasoning::ReasoningContentPart;
use everruns_contracts::{ProviderEndpoint, ToolDefinition};
use everruns_drivers::mistral::{MISTRAL_RESPONSES_OPTION, provider, register_driver};
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config() -> LlmCallConfig {
    let mut config = LlmCallConfig::new("mistral-large-4");
    config
        .driver_options
        .insert(MISTRAL_RESPONSES_OPTION.into(), json!(true));
    config
}

fn completed(output: Value) -> Value {
    json!({"type": "response.completed", "response": {
        "id": "resp_mistral", "status": "completed", "model": "mistral-large-4",
        "output": output, "usage": {"input_tokens": 12, "output_tokens": 7,
            "input_tokens_details": {"cached_tokens": 4}}
    }})
}

fn sse(frames: Vec<Value>) -> String {
    frames
        .into_iter()
        .map(|frame| format!("data: {frame}\n\n"))
        .collect()
}

async fn mount(server: &MockServer, frames: Vec<Value>) {
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer synthetic-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("connection", "close")
                .set_body_raw(sse(frames), "text/event-stream"),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn registered_and_direct_calls_preserve_stateless_history_and_controls() {
    for registered in [false, true] {
        let server = MockServer::start().await;
        mount(
            &server,
            vec![
                json!({"type": "response.output_text.delta", "delta": "Paris"}),
                completed(json!([])),
            ],
        )
        .await;
        let mut cfg = config();
        cfg.previous_response_id = Some("must-not-be-sent".into());
        cfg.reasoning_effort = Some(ReasoningEffort::Medium);
        cfg.max_tokens = Some(4096);
        cfg.parallel_tool_calls = Some(false);
        cfg.metadata.insert("session_id".into(), "session-1".into());
        let messages = vec![
            Message::text(MessageRole::System, "Be concise"),
            Message::text(MessageRole::User, "France's capital?"),
            Message::text(MessageRole::Assistant, "Paris"),
            Message::text(MessageRole::User, "Repeat it"),
        ];
        let response = if registered {
            let mut registry = DriverRegistry::new();
            register_driver(&mut registry);
            let driver = registry
                .create_chat_driver(
                    &ProviderConfig::new(DriverId::Mistral)
                        .with_api_key("synthetic-key")
                        .with_base_url(format!("{}/v1", server.uri())),
                )
                .unwrap();
            assert!(!driver.supports_stateful_responses());
            assert!(!driver.supports_compact());
            driver
                .chat_completion_non_streaming(&ProviderEndpoint::default(), messages, &cfg)
                .await
        } else {
            provider("mistral", "synthetic-key")
                .base_url(format!("{}/v1", server.uri()))
                .chat_completion_non_streaming(messages, &cfg)
                .await
        }
        .unwrap();
        assert_eq!(response.text, "Paris");
        assert_eq!(response.metadata.prompt_tokens, Some(8));
        assert_eq!(response.metadata.cache_read_tokens, Some(4));
        assert_eq!(response.metadata.completion_tokens, Some(7));
        let requests = server.received_requests().await.unwrap();
        let body = requests[0].body_json::<Value>().unwrap();
        assert_eq!(body["input"].as_array().unwrap().len(), 3);
        assert_eq!(body["instructions"], "Be concise");
        assert_eq!(body["reasoning"], json!({"effort":"high"}));
        assert_eq!(body["max_output_tokens"], 4096);
        assert_eq!(body["parallel_tool_calls"], false);
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert!(body.get("include").is_none());
        assert!(body.get("previous_response_id").is_none());
    }
}

#[tokio::test]
async fn reasoning_and_fragmented_tools_round_trip_with_multimodal_results() {
    let server = MockServer::start().await;
    let reasoning = json!({"type":"reasoning","id":"r1","summary":[],
        "content":[{"type":"reasoning_text","text":"I should inspect the document."}]});
    let call = json!({"type":"function_call","id":"fc1","call_id":"call1", "name":"inspect", "arguments":"{\"page\":1}", "status":"completed"});
    mount(&server, vec![
        json!({"type":"response.reasoning_text.delta","sequence_number":1,"item_id":"r1","output_index":0,"content_index":0,"delta":"I should inspect the document."}),
        json!({"type":"response.output_item.done","sequence_number":2,"output_index":0,"item":reasoning}),
        json!({"type":"response.output_item.added","sequence_number":3,"output_index":1,"item":{"type":"function_call","id":"fc1","call_id":"call1","name":"inspect","arguments":"","status":"in_progress"}}),
        json!({"type":"response.function_call_arguments.delta","sequence_number":4,"item_id":"fc1","output_index":1,"delta":"{\"page\":"}),
        json!({"type":"response.function_call_arguments.delta","sequence_number":5,"item_id":"fc1","output_index":1,"delta":"1}"}),
        json!({"type":"response.output_item.done","sequence_number":6,"output_index":1,"item":call}),
        completed(json!([reasoning,call])),
    ]).await;
    let service = provider("mistral", "synthetic-key").base_url(format!("{}/v1", server.uri()));
    let mut cfg = config();
    cfg.tools = vec![ToolDefinition::function(
        "inspect",
        "Inspect a document",
        json!({"type":"object","properties":{"page":{"type":"integer"}},"required":["page"]}),
    )];
    let user = Message::text(MessageRole::User, "Inspect page one");
    let response = service
        .chat_completion(vec![user.clone()], &cfg)
        .await
        .unwrap();
    assert_eq!(response.reasoning.len(), 1);
    assert_eq!(response.reasoning[0].provider, "mistral");
    assert_eq!(
        response.reasoning[0].display_text().as_deref(),
        Some("I should inspect the document.")
    );
    let calls = response.tool_calls.unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].arguments, json!({"page":1}));
    let mut assistant = Message::text(MessageRole::Assistant, "");
    assistant.reasoning = response.reasoning;
    // Artifacts from another provider must never cross this boundary.
    assistant.reasoning.push(
        ReasoningContentPart::opaque("openai")
            .with_item_id("rs_foreign")
            .with_encrypted("private"),
    );
    assistant.tool_calls = Some(calls);
    let mut result = Message::text(MessageRole::Tool, "");
    result.tool_call_id = Some("call1".into());
    result.content = MessageContent::Parts(vec![
        LlmContentPart::Text {
            text: "See attached".into(),
        },
        LlmContentPart::Image {
            url: "data:image/png;base64,aGVsbG8=".into(),
        },
        LlmContentPart::File {
            url: "https://example.com/report.pdf".into(),
            filename: None,
        },
        LlmContentPart::File {
            url: "data:application/pdf;base64,aGVsbG8=".into(),
            filename: Some("report.pdf".into()),
        },
    ]);
    service
        .chat_completion(vec![user, assistant, result], &cfg)
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let body = requests[1].body_json::<Value>().unwrap();
    assert_eq!(body["input"][1], reasoning);
    assert_eq!(body["input"][2]["call_id"], "call1");
    assert_eq!(body["input"][3]["call_id"], "call1");
    assert_eq!(body["input"][3]["output"][1]["type"], "input_image");
    assert_eq!(
        body["input"][3]["output"][2]["file_url"],
        "https://example.com/report.pdf"
    );
    assert!(body["input"][3]["output"][2].get("file_data").is_none());
    assert_eq!(
        body["input"][3]["output"][3]["file_data"],
        "data:application/pdf;base64,aGVsbG8="
    );
    assert!(!body.to_string().contains("private"));
    assert_eq!(body["tools"][0]["type"], "function");
}

#[tokio::test]
async fn terminal_failures_and_truncated_streams_never_succeed() {
    for terminal in [
        Some(
            json!({"type":"response.failed","response":{"error":{"code":"server_error","message":"Failed generation"}}}),
        ),
        Some(
            json!({"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}),
        ),
        Some(json!({"type":"error","code":"invalid_request","message":"Rejected"})),
        None,
    ] {
        let server = MockServer::start().await;
        let mut frames = vec![json!({"type":"response.output_text.delta","delta":"Partial"})];
        frames.extend(terminal);
        mount(&server, frames).await;
        let service = provider("mistral", "synthetic-key").base_url(format!("{}/v1", server.uri()));
        let result = service
            .chat_completion(vec![Message::text(MessageRole::User, "hi")], &config())
            .await;
        assert!(result.is_err(), "{result:?}");
    }
}

#[tokio::test]
async fn http_errors_surface_and_invalid_options_fail_before_network() {
    for status in [400, 401, 403, 422] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(json!({"message":"Request rejected"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let result = provider("mistral", "synthetic-key")
            .base_url(format!("{}/v1", server.uri()))
            .chat_completion(vec![Message::text(MessageRole::User, "hi")], &config())
            .await;
        assert!(result.is_err());
    }
    let server = MockServer::start().await;
    let service = provider("mistral", "synthetic-key").base_url(format!("{}/v1", server.uri()));
    let mut cfg = config();
    cfg.driver_options
        .insert(MISTRAL_RESPONSES_OPTION.into(), json!("true"));
    assert!(
        service
            .chat_completion(vec![Message::text(MessageRole::User, "hi")], &cfg)
            .await
            .is_err()
    );
    let mut message = Message::text(MessageRole::User, "");
    message.content = MessageContent::Parts(vec![LlmContentPart::Audio {
        url: "data:audio/wav;base64,AA==".into(),
    }]);
    assert!(
        service
            .chat_completion(vec![message], &config())
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn stream_exposes_text_before_completion() {
    let server = MockServer::start().await;
    mount(
        &server,
        vec![
            json!({"type":"response.output_text.delta","delta":"hello"}),
            completed(json!([])),
        ],
    )
    .await;
    let service = provider("mistral", "synthetic-key").base_url(format!("{}/v1", server.uri()));
    let mut stream = service
        .chat_completion_stream(vec![Message::text(MessageRole::User, "hi")], &config())
        .await
        .unwrap();
    assert!(
        matches!(stream.next().await.unwrap().unwrap(),LlmStreamEvent::TextDelta(text) if text == "hello")
    );
    assert!(matches!(
        stream.next().await.unwrap().unwrap(),
        LlmStreamEvent::Done(_)
    ));
}

#[tokio::test]
async fn user_media_and_structured_output_use_mistrals_request_shape() {
    let server = MockServer::start().await;
    mount(&server, vec![completed(json!([]))]).await;
    let mut cfg = config();
    cfg.response_format = Some(
        everruns_contracts::structured_output::ResponseFormat::json_schema(
            "answer",
            json!({"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"],"additionalProperties":false}),
        ),
    );
    cfg.verbosity = Some("low".into());
    cfg.speed = Some("priority".into());
    cfg.prompt_cache = Some(everruns_contracts::driver_registry::PromptCacheConfig {
        enabled: true,
        ..Default::default()
    });
    cfg.metadata
        .insert("session_id".into(), "mistral-media-test".into());
    let mut message = Message::text(MessageRole::User, "");
    message.content = MessageContent::Parts(vec![
        LlmContentPart::Text {
            text: "Summarize".into(),
        },
        LlmContentPart::Image {
            url: "https://example.com/photo.png".into(),
        },
        LlmContentPart::File {
            url: "https://example.com/report.pdf".into(),
            filename: None,
        },
    ]);
    provider("mistral", "synthetic-key")
        .base_url(format!("{}/v1", server.uri()))
        .chat_completion(vec![message], &cfg)
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let body = requests[0].body_json::<Value>().unwrap();
    assert_eq!(
        body["input"][0]["content"][1]["image_url"],
        "https://example.com/photo.png"
    );
    assert_eq!(
        body["input"][0]["content"][2]["file_url"],
        "https://example.com/report.pdf"
    );
    assert_eq!(body["text"]["format"]["type"], "json_schema");
    assert!(body["text"].get("verbosity").is_none());
    assert!(body.get("service_tier").is_none());
    assert!(
        body["prompt_cache_key"]
            .as_str()
            .unwrap()
            .starts_with("everruns:")
    );
}

#[tokio::test]
async fn rate_limit_retries_preserve_the_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "0")
                .set_body_json(json!({"message":"Rate limit exceeded"})),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    mount(&server, vec![completed(json!([]))]).await;
    provider("mistral", "synthetic-key")
        .base_url(format!("{}/v1", server.uri()))
        .chat_completion(vec![Message::text(MessageRole::User, "hi")], &config())
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body, requests[1].body);
}
