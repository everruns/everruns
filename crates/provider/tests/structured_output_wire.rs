// Structured output on the wire (EVE-1116).
//
// A `ResponseFormat` on the call config must reach the provider in its native
// shape (Responses `text.format`, Chat Completions `response_format`), must be
// absent when not asked for, and must fail before HTTP on a driver that cannot
// enforce it: silently dropping the schema would hand the caller free text it
// believes is validated JSON.

use async_trait::async_trait;
use everruns_provider::driver_registry::{
    ChatDriver, LlmCallConfig, LlmCallConfigBuilder, LlmResponseStream, Message, MessageRole,
};
use everruns_provider::runtime_provider::ProviderEndpoint;
use everruns_provider::structured_output::ResponseFormat;
use everruns_provider::{
    BearerAuth, OpenAIProtocolChatDriver, OpenResponsesProtocolChatDriver, Provider,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": { "answer": { "type": "integer" } },
        "required": ["answer"],
        "additionalProperties": false
    })
}

fn structured(model: &str) -> LlmCallConfig {
    LlmCallConfigBuilder::from_config(LlmCallConfig::new(model))
        .response_format(ResponseFormat::json_schema("answer", schema()))
        .build()
}

async fn last_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    serde_json::from_slice(&requests.last().expect("a request").body).unwrap()
}

async fn responses_body(config: &LlmCallConfig) -> Value {
    let server = MockServer::start().await;
    let event = json!({"type":"response.completed","sequence_number":1,"response":{
        "id":"resp_1","object":"response","created_at":1780000000,"status":"completed",
        "model":"gpt-6-luna","output":[],"usage":{"input_tokens":1,"output_tokens":1}
    }});
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!("data: {event}\n\ndata: [DONE]\n\n"),
            "text/event-stream",
        ))
        .mount(&server)
        .await;
    Provider::new("openai", OpenResponsesProtocolChatDriver::new())
        .base_url(format!("{}/v1/responses", server.uri()))
        .auth(BearerAuth::new("test-key"))
        .chat_completion(vec![Message::text(MessageRole::User, "6*7?")], config)
        .await
        .expect("completion should succeed");
    last_body(&server).await
}

#[tokio::test]
async fn responses_sends_text_format_only_when_asked() {
    let body = responses_body(&structured("gpt-6-luna")).await;
    assert_eq!(
        body["text"],
        json!({"format": {"type": "json_schema", "name": "answer", "schema": schema(), "strict": true}})
    );

    // Verbosity and format share the `text` object.
    let mut config = structured("gpt-6-luna");
    config.verbosity = Some("low".into());
    config.response_format = config.response_format.map(ResponseFormat::non_strict);
    let body = responses_body(&config).await;
    assert_eq!(body["text"]["verbosity"], "low");
    assert_eq!(body["text"]["format"]["strict"], false);

    let body = responses_body(&LlmCallConfig::new("gpt-6-luna")).await;
    assert!(body.get("text").is_none(), "{body}");
}

#[tokio::test]
async fn chat_completions_sends_response_format() {
    let server = MockServer::start().await;
    let chunk = r#"{"id":"c1","model":"gpt-5.2","choices":[{"index":0,"delta":{"content":"{\"answer\":42}"},"finish_reason":"stop"}]}"#;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!("data: {chunk}\n\ndata: [DONE]\n\n"),
            "text/event-stream",
        ))
        .mount(&server)
        .await;
    let provider = Provider::new("openai-compat", OpenAIProtocolChatDriver::new())
        .base_url(format!("{}/v1", server.uri()))
        .auth(BearerAuth::new("test-key"));

    let response = provider
        .chat_completion(
            vec![Message::text(MessageRole::User, "6*7?")],
            &structured("gpt-5.2"),
        )
        .await
        .expect("completion should succeed");
    assert_eq!(response.text, r#"{"answer":42}"#);
    assert_eq!(
        last_body(&server).await["response_format"],
        json!({"type": "json_schema", "json_schema": {"name": "answer", "schema": schema(), "strict": true}})
    );

    provider
        .chat_completion(
            vec![Message::text(MessageRole::User, "hi")],
            &LlmCallConfig::new("gpt-5.2"),
        )
        .await
        .unwrap();
    assert!(last_body(&server).await.get("response_format").is_none());
}

/// A driver with no structured-output support. It panics if called: the
/// provider must refuse the request first.
struct PlainDriver;

#[async_trait]
impl ChatDriver for PlainDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<Message>,
        _config: &LlmCallConfig,
    ) -> everruns_provider::error::Result<LlmResponseStream> {
        panic!("the request must be refused before reaching the driver");
    }
}

#[tokio::test]
async fn unsupported_driver_refuses_instead_of_dropping_the_schema() {
    let provider = Provider::new("plain", PlainDriver);
    let messages = || vec![Message::text(MessageRole::User, "hi")];
    let config = structured("plain-model");
    let errors = [
        provider.chat_completion(messages(), &config).await.err(),
        provider
            .chat_completion_stream(messages(), &config)
            .await
            .err(),
        provider
            .chat_completion_non_streaming(messages(), &config)
            .await
            .err(),
    ];
    for err in errors {
        let err = err.expect("the call must fail").to_string();
        assert!(
            err.contains(
                "Structured output (response_format) is not supported by provider 'plain'"
            ),
            "{err}"
        );
    }
}
