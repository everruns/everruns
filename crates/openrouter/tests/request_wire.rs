// Wire-level tests for OpenRouter request decoration.
//
// These exercise the full path: `OpenRouterChatDriver` builds an Open Responses
// request through the core driver, and its attached `OpenRouterRequestExtension`
// layers OpenRouter's `models` / `route` / `provider` / `plugins` / `session_id`
// fields onto the outgoing body. A wiremock server captures the request so we can
// assert the exact JSON sent.

use everruns_core::llm_driver_registry::{
    ChatDriver, LlmCallConfig, LlmMessage, LlmMessageRole, OpenRouterDataCollection,
    OpenRouterMaxPrice, OpenRouterPluginConfig, OpenRouterProviderRouting, OpenRouterProviderSort,
    OpenRouterProviderSortBy, OpenRouterProviderSortOptions, OpenRouterRoute,
    OpenRouterRoutingConfig, OpenRouterSortPartition, OpenRouterWebSearchPlugin,
};
use everruns_openrouter::OpenRouterChatDriver;
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn base_config(model: &str) -> LlmCallConfig {
    LlmCallConfig {
        model: model.to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![],
        reasoning_effort: None,
        metadata: std::collections::HashMap::new(),
        previous_response_id: None,
        tool_search: None,
        prompt_cache: None,
        openrouter_routing: None,
    }
}

async fn capture_request_body(config: &LlmCallConfig) -> serde_json::Value {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let api_url = format!("{}/v1/responses", server.uri());
    let driver = OpenRouterChatDriver::with_base_url("test-key", api_url);

    let messages = vec![LlmMessage::text(LlmMessageRole::User, "hello")];
    let _ = driver.chat_completion_stream(messages, config).await;

    let requests = server
        .received_requests()
        .await
        .expect("mock server recorded requests");
    assert_eq!(requests.len(), 1);
    requests[0].body_json().expect("request body is JSON")
}

#[tokio::test]
async fn sends_routing_controls_and_session_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let api_url = format!("{}/v1/responses", server.uri());
    let driver = OpenRouterChatDriver::with_base_url("test-key", api_url);

    let mut config = base_config("openai/gpt-5-mini");
    config
        .metadata
        .insert("session_id".to_string(), "session_abc123".to_string());
    config.openrouter_routing = Some(OpenRouterRoutingConfig {
        models: vec![
            "openai/gpt-5-mini".to_string(),
            "anthropic/claude-sonnet-4.5".to_string(),
        ],
        route: Some(OpenRouterRoute::Fallback),
        provider: Some(OpenRouterProviderRouting {
            order: vec!["openai".to_string()],
            allow_fallbacks: Some(false),
            require_parameters: Some(true),
            data_collection: Some(OpenRouterDataCollection::Deny),
            zdr: Some(true),
            sort: Some(OpenRouterProviderSort::Advanced(
                OpenRouterProviderSortOptions {
                    by: OpenRouterProviderSortBy::Latency,
                    partition: Some(OpenRouterSortPartition::None),
                },
            )),
            max_price: Some(OpenRouterMaxPrice {
                prompt: Some(1.0),
                completion: Some(2.0),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    });

    let messages = vec![LlmMessage::text(LlmMessageRole::User, "hello")];
    let _ = driver.chat_completion_stream(messages, &config).await;

    let requests = server
        .received_requests()
        .await
        .expect("mock server recorded requests");
    assert_eq!(requests.len(), 1, "exactly one request should be sent");
    let body: serde_json::Value = requests[0].body_json().expect("request body is JSON");

    assert_eq!(
        body["models"],
        json!(["openai/gpt-5-mini", "anthropic/claude-sonnet-4.5"])
    );
    assert_eq!(body["route"], "fallback");
    assert_eq!(
        body["provider"],
        json!({
            "order": ["openai"],
            "allow_fallbacks": false,
            "require_parameters": true,
            "data_collection": "deny",
            "zdr": true,
            "sort": { "by": "latency", "partition": "none" },
            "max_price": { "prompt": 1.0, "completion": 2.0 }
        })
    );
    // Session-tracking key is forwarded at the request root for OpenRouter.
    assert_eq!(body["session_id"], "session_abc123");
}

#[tokio::test]
async fn excludes_reasoning_by_default() {
    let config = base_config("nvidia/nemotron-3-super-120b-a12b:free");

    let body = capture_request_body(&config).await;

    assert_eq!(body["reasoning"], json!({ "exclude": true }));
}

#[tokio::test]
async fn excludes_reasoning_with_explicit_effort() {
    let mut config = base_config("nvidia/nemotron-3-super-120b-a12b:free");
    config.reasoning_effort = Some("high".to_string());

    let body = capture_request_body(&config).await;

    assert_eq!(
        body["reasoning"],
        json!({
            "effort": "high",
            "exclude": true
        })
    );
    assert!(
        body["reasoning"].get("summary").is_none(),
        "OpenRouter requests must not ask for visible reasoning summaries by default"
    );
}

#[tokio::test]
async fn sends_reasoning_none_to_disable_openrouter_reasoning() {
    let mut config = base_config("nvidia/nemotron-3-super-120b-a12b:free");
    config.reasoning_effort = Some("none".to_string());

    let body = capture_request_body(&config).await;

    assert_eq!(
        body["reasoning"],
        json!({
            "effort": "none",
            "exclude": true
        })
    );
}

#[tokio::test]
async fn rejects_invalid_routing_before_dispatch() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let api_url = format!("{}/v1/responses", server.uri());
    let driver = OpenRouterChatDriver::with_base_url("test-key", api_url);

    // Primary model absent from the fallback list.
    let mut mismatch = base_config("openai/gpt-5-mini");
    mismatch.openrouter_routing = Some(OpenRouterRoutingConfig {
        models: vec!["anthropic/claude-sonnet-4.5".to_string()],
        route: Some(OpenRouterRoute::Fallback),
        ..Default::default()
    });
    let err = match driver
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &mismatch,
        )
        .await
    {
        Ok(_) => panic!("invalid OpenRouter routing should fail before dispatch"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("models[0]"));

    // Fallback route with no models.
    let mut empty_fallback = base_config("openai/gpt-5-mini");
    empty_fallback.openrouter_routing = Some(OpenRouterRoutingConfig {
        models: vec![],
        route: Some(OpenRouterRoute::Fallback),
        ..Default::default()
    });
    let err = match driver
        .chat_completion_stream(
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &empty_fallback,
        )
        .await
    {
        Ok(_) => panic!("empty OpenRouter fallback routing should fail before dispatch"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("requires at least one model"));

    let requests = server
        .received_requests()
        .await
        .expect("mock server recorded requests");
    assert!(
        requests.is_empty(),
        "invalid routing must be rejected before request dispatch"
    );
}

#[tokio::test]
async fn includes_plugins_in_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let api_url = format!("{}/v1/responses", server.uri());
    let driver = OpenRouterChatDriver::with_base_url("test-key", api_url);

    let mut config = base_config("openai/gpt-5-mini");
    config.openrouter_routing = Some(OpenRouterRoutingConfig {
        plugins: Some(OpenRouterPluginConfig {
            web: Some(OpenRouterWebSearchPlugin {
                max_results: Some(5),
                search_prompt: None,
            }),
            file: None,
        }),
        ..Default::default()
    });

    let messages = vec![LlmMessage::text(LlmMessageRole::User, "search the web")];
    let _ = driver.chat_completion_stream(messages, &config).await;

    let requests = server
        .received_requests()
        .await
        .expect("mock server recorded requests");
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value = requests[0].body_json().expect("request body is JSON");

    let plugins = body["plugins"]
        .as_array()
        .expect("plugins array should be present");
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0]["id"], "web");
    assert_eq!(plugins[0]["max_results"], 5);
}
