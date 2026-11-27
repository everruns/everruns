//! Request layout must follow the actual protocol, endpoint and features, even
//! under an arbitrary Framework provider key. HTTP is intercepted, never paid.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(any(feature = "openai", feature = "openrouter", feature = "gemini"))]

use everruns_contracts::driver_registry::{ChatDriver, LlmCallConfig, Message, MessageRole};
#[cfg(any(feature = "openai", feature = "openrouter"))]
use everruns_contracts::http_transport::{
    HeaderMap, HttpError, HttpRequest, HttpResponse, HttpTransport, StatusCode,
};
use everruns_contracts::tool_changes::ToolChanges;
use everruns_contracts::tool_types::ToolDefinition;
use everruns_contracts::{Provider, ProviderEndpoint, ToolRequestProjection};
use serde_json::{Value, json};
#[cfg(any(feature = "openai", feature = "openrouter"))]
use std::sync::{Arc, Mutex};

#[cfg(any(feature = "openai", feature = "openrouter"))]
#[derive(Default)]
struct Capture(Mutex<Option<Value>>);
#[cfg(any(feature = "openai", feature = "openrouter"))]
#[async_trait::async_trait]
impl HttpTransport for Capture {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        *self.0.lock().unwrap() = Some(serde_json::from_slice(request.body()).unwrap());
        Ok(HttpResponse::new(
            StatusCode::BAD_REQUEST,
            HeaderMap::new(),
            Box::pin(futures::stream::once(async {
                Ok(
                    br#"{"error":{"message":"captured","type":"invalid_request_error"}}"#
                        .to_vec()
                        .into(),
                )
            })),
        ))
    }
}

fn input(model: &str) -> (Vec<Message>, LlmCallConfig) {
    let tool =
        |name| ToolDefinition::function(name, name, json!({"type":"object","properties":{}}));
    let mut config = LlmCallConfig::new(model);
    config.tools = vec![tool("current")];
    config.tool_baseline = Some(vec![tool("old")]);
    config.system_prompt_baseline = Some("Original instructions".into());
    let mut last = Message::text(MessageRole::User, "Use the current tool");
    last.tool_changes = Some(ToolChanges {
        added: config.tools.clone(),
        removed: vec!["old".into()],
        ..Default::default()
    });
    (
        vec![
            Message::text(MessageRole::System, "Original instructions"),
            Message::text(MessageRole::User, "Earlier"),
            Message::text(MessageRole::Assistant, "Done"),
            last,
        ],
        config,
    )
}

#[cfg(any(feature = "openai", feature = "openrouter"))]
async fn check(
    provider: Provider,
    capture: &Capture,
    messages: Vec<Message>,
    config: LlmCallConfig,
    expected: ToolRequestProjection,
    expected_name: &str,
) {
    let bound = provider
        .auth(everruns_contracts::BearerAuth::new("test-key"))
        .into_boxed_driver();
    // The caller deliberately supplies no endpoint: the wrapper must use its bound one.
    assert_eq!(
        bound.tool_request_projection(&ProviderEndpoint::default(), &messages, &config),
        expected
    );
    let _ = bound
        .chat_completion_stream(&ProviderEndpoint::default(), messages, &config)
        .await;
    let body = capture.0.lock().unwrap().clone().expect("request captured");
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools[0]["name"], expected_name, "{body}");
    assert_eq!(tools.len(), 1, "no hidden baseline declarations: {body}");
}

#[cfg(feature = "openai")]
#[tokio::test]
async fn openai_projection_matches_wire_on_native_and_custom_endpoints() {
    use everruns_drivers::openai::OpenAIChatDriver;
    for (url, expected, name) in [
        (
            "https://api.openai.com/v1",
            ToolRequestProjection::BaselineWithChanges,
            "old",
        ),
        (
            "https://gateway.example/v1",
            ToolRequestProjection::Current,
            "current",
        ),
    ] {
        let capture = Arc::new(Capture::default());
        let provider = Provider::new(
            "application-owned-key",
            OpenAIChatDriver::new().with_transport(capture.clone()),
        )
        .base_url(url);
        let (messages, config) = input("gpt-6.1-sol");
        check(provider, &capture, messages, config, expected, name).await;
    }
}

#[cfg(feature = "openrouter")]
#[tokio::test]
async fn gateway_with_native_model_profile_and_arbitrary_key_sends_only_current_tools() {
    use everruns_drivers::openrouter::OpenRouterChatDriver;
    let capture = Arc::new(Capture::default());
    let provider = Provider::new(
        "my-company-proxy",
        OpenRouterChatDriver::new().with_transport(capture.clone()),
    )
    .base_url("https://openrouter.ai/api/v1");
    let (messages, config) = input("openai/gpt-6.1-sol");
    check(
        provider,
        &capture,
        messages,
        config,
        ToolRequestProjection::Current,
        "current",
    )
    .await;
}

#[cfg(feature = "openai")]
#[tokio::test]
async fn add_only_model_redefinition_falls_back_in_both_plan_and_wire() {
    use everruns_drivers::openai::OpenAIChatDriver;
    let capture = Arc::new(Capture::default());
    let provider = Provider::new(
        "custom-key",
        OpenAIChatDriver::new().with_transport(capture.clone()),
    )
    .base_url("https://api.openai.com/v1");
    let (mut messages, config) = input("gpt-5.5");
    messages
        .last_mut()
        .unwrap()
        .tool_changes
        .as_mut()
        .unwrap()
        .redefined = config.tools.clone();
    check(
        provider,
        &capture,
        messages,
        config,
        ToolRequestProjection::Current,
        "current",
    )
    .await;
}

#[cfg(feature = "openai")]
#[tokio::test]
async fn chat_completions_ignores_the_inline_baseline_in_both_plan_and_wire() {
    use everruns_drivers::openai::OpenAICompletionsChatDriver;
    let capture = Arc::new(Capture::default());
    let bound = Provider::new(
        "arbitrary-compatible-provider",
        OpenAICompletionsChatDriver::new().with_transport(capture.clone()),
    )
    .base_url("https://compatible.example/v1")
    .auth(everruns_contracts::BearerAuth::new("test-key"))
    .into_boxed_driver();
    let (messages, config) = input("gpt-6.1-sol");
    assert_eq!(
        bound.tool_request_projection(&ProviderEndpoint::default(), &messages, &config),
        ToolRequestProjection::Current
    );
    let _ = bound
        .chat_completion_stream(&ProviderEndpoint::default(), messages, &config)
        .await;
    let body = capture.0.lock().unwrap().clone().unwrap();
    assert_eq!(body["tools"][0]["function"]["name"], "current");
    assert_eq!(body["tools"].as_array().unwrap().len(), 1);
}

#[cfg(feature = "gemini")]
#[tokio::test]
async fn gemini_budgets_only_the_declarations_it_serializes() {
    use everruns_drivers::gemini::GeminiChatDriver;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error":{"message":"captured","status":"INVALID_ARGUMENT"}})),
        )
        .mount(&server)
        .await;
    let bound = Provider::new("application-gemini-route", GeminiChatDriver::new())
        .base_url(format!("{}/v1beta", server.uri()))
        .auth(everruns_contracts::StaticHeaderAuth::new(
            "x-goog-api-key",
            "test-key",
        ))
        .into_boxed_driver();
    let (messages, config) = input("gemini-2.5-flash");
    assert_eq!(
        bound.tool_request_projection(&ProviderEndpoint::default(), &messages, &config),
        ToolRequestProjection::Current
    );
    let _ = bound
        .chat_completion_stream(&ProviderEndpoint::default(), messages, &config)
        .await;
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(body["tools"].to_string().contains("current"), "{body}");
    assert!(!body["tools"].to_string().contains("old"), "{body}");
}
