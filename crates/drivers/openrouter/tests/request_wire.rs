// Wire-level tests for OpenRouter request decoration.
//
// These exercise the full path: `OpenRouterChatDriver` builds an Open Responses
// request through the core driver, and its attached `OpenRouterRequestExtension`
// layers OpenRouter's `models` / `route` / `provider` / `plugins` / `session_id`
// fields onto the outgoing body. A wiremock server captures the request so we can
// assert the exact JSON sent.

use everruns_openrouter::OpenRouterChatDriver;
use everruns_provider::driver_registry::{
    LlmCallConfig, LlmMessage, LlmMessageRole, OpenRouterDataCollection, OpenRouterMaxPrice,
    OpenRouterPluginConfig, OpenRouterProviderRouting, OpenRouterProviderSort,
    OpenRouterProviderSortBy, OpenRouterProviderSortOptions, OpenRouterRoute,
    OpenRouterRoutingConfig, OpenRouterServerTool, OpenRouterServerToolKind,
    OpenRouterSortPartition, OpenRouterWebSearchPlugin,
};
use everruns_provider::model::ReasoningEffort;
use everruns_provider::{BearerAuth, Provider};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn base_config(model: &str) -> LlmCallConfig {
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

fn provider(api_url: String) -> Provider {
    Provider::new("openrouter-test", OpenRouterChatDriver::new())
        .base_url(api_url)
        .auth(BearerAuth::new("test-key"))
}

async fn capture_request_body(config: &LlmCallConfig) -> serde_json::Value {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let api_url = format!("{}/v1/responses", server.uri());
    let driver = provider(api_url);

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
    let driver = provider(api_url);

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
async fn sends_openrouter_attribution_headers_from_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let api_url = format!("{}/v1/responses", server.uri());
    let driver = provider(api_url);

    let mut config = base_config("openai/gpt-5-mini");
    config.metadata.insert(
        "openrouter.http_referer".to_string(),
        "https://app.example".to_string(),
    );
    config
        .metadata
        .insert("openrouter.x_title".to_string(), "Example App".to_string());
    config
        .metadata
        .insert("custom_key".to_string(), "custom_value".to_string());

    let messages = vec![LlmMessage::text(LlmMessageRole::User, "hello")];
    let _ = driver.chat_completion_stream(messages, &config).await;

    let requests = server
        .received_requests()
        .await
        .expect("mock server recorded requests");
    assert_eq!(requests.len(), 1, "exactly one request should be sent");
    assert_eq!(
        requests[0]
            .headers
            .get("http-referer")
            .and_then(|v| v.to_str().ok()),
        Some("https://app.example")
    );
    assert_eq!(
        requests[0]
            .headers
            .get("x-title")
            .and_then(|v| v.to_str().ok()),
        Some("Example App")
    );
    let body: serde_json::Value = requests[0].body_json().expect("request body is JSON");
    let metadata = body["metadata"]
        .as_object()
        .expect("metadata object remains");
    assert_eq!(metadata.get("custom_key"), Some(&json!("custom_value")));
    assert!(
        !metadata.contains_key("openrouter.http_referer")
            && !metadata.contains_key("openrouter.x_title"),
        "attribution keys are header-only and should not be mirrored in body metadata: {metadata:?}"
    );
}

#[tokio::test]
async fn skips_blank_openrouter_attribution_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let api_url = format!("{}/v1/responses", server.uri());
    let driver = provider(api_url);

    let mut config = base_config("openai/gpt-5-mini");
    config
        .metadata
        .insert("openrouter.http_referer".to_string(), "  ".to_string());
    config
        .metadata
        .insert("openrouter.x_title".to_string(), "\t".to_string());

    let messages = vec![LlmMessage::text(LlmMessageRole::User, "hello")];
    let _ = driver.chat_completion_stream(messages, &config).await;

    let requests = server
        .received_requests()
        .await
        .expect("mock server recorded requests");
    assert_eq!(requests.len(), 1, "exactly one request should be sent");
    assert!(requests[0].headers.get("http-referer").is_none());
    assert!(requests[0].headers.get("x-title").is_none());
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
    config.reasoning_effort = Some(ReasoningEffort::High);

    let body = capture_request_body(&config).await;

    assert_eq!(
        body["reasoning"],
        json!({
            "effort": "high",
            "exclude": true
        })
    );
}

#[tokio::test]
async fn sends_reasoning_none_to_disable_openrouter_reasoning() {
    let mut config = base_config("nvidia/nemotron-3-super-120b-a12b:free");
    config.reasoning_effort = Some(ReasoningEffort::None);

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
async fn omits_parallel_tool_calls_when_unset() {
    // EVE-598: OpenRouter wraps the Open Responses driver, so the body carries
    // `parallel_tool_calls` from the Responses serialization. Omitted when None.
    let config = base_config("openai/gpt-5-mini");

    let body = capture_request_body(&config).await;

    assert!(body.get("parallel_tool_calls").is_none());
}

#[tokio::test]
async fn forwards_parallel_tool_calls_when_set() {
    // EVE-598: the operator setting reaches the OpenRouter wire and `decorate`
    // does not strip it.
    let mut config = base_config("openai/gpt-5-mini");
    config.parallel_tool_calls = Some(false);

    let body = capture_request_body(&config).await;

    assert_eq!(body["parallel_tool_calls"], false);
}

#[tokio::test]
async fn retries_after_openrouter_rate_limit_reset() {
    use futures::StreamExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    let reset_ms = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_secs()
        + 1)
        * 1000;
    let rate_limit_body = json!({
        "error": {
            "message": "Rate limit exceeded: free-models-per-min.",
            "code": 429,
            "metadata": {
                "headers": {
                    "X-RateLimit-Limit": "16",
                    "X-RateLimit-Remaining": "0",
                    "X-RateLimit-Reset": reset_ms.to_string()
                }
            }
        }
    });
    let success_body =
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\ndata: [DONE]\n\n";

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).set_body_json(rate_limit_body))
        .up_to_n_times(1)
        .expect(1)
        .named("OpenRouter first rate-limit response")
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(success_body),
        )
        .expect(1)
        .named("OpenRouter retry success response")
        .mount(&server)
        .await;

    let api_url = format!("{}/v1/responses", server.uri());
    let driver = provider(api_url);
    let messages = vec![LlmMessage::text(LlmMessageRole::User, "hello")];
    let mut stream = driver
        .chat_completion_stream(messages, &base_config("openai/gpt-4o-mini"))
        .await
        .expect("OpenRouter driver should retry after reset and start the stream");

    let mut text = String::new();
    while let Some(event) = stream.next().await {
        match event.expect("stream item") {
            everruns_provider::driver_registry::LlmStreamEvent::TextDelta(delta) => {
                text.push_str(&delta)
            }
            everruns_provider::driver_registry::LlmStreamEvent::Error(error) => {
                panic!("retry success stream should not emit an error: {error}")
            }
            _ => {}
        }
    }

    assert_eq!(text, "ok");
    let requests = server
        .received_requests()
        .await
        .expect("mock server recorded requests");
    assert_eq!(requests.len(), 2, "rate-limited request should be retried");
}

#[tokio::test]
async fn rejects_invalid_routing_before_dispatch() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let api_url = format!("{}/v1/responses", server.uri());
    let driver = provider(api_url);

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
    let driver = provider(api_url);

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

#[tokio::test]
async fn server_tools_are_appended_to_tools_array() {
    let mut config = base_config("openai/gpt-5-mini");
    config.openrouter_routing = Some(OpenRouterRoutingConfig {
        server_tools: vec![
            OpenRouterServerTool::with_parameters(
                OpenRouterServerToolKind::WebSearch,
                json!({ "max_results": 3 }),
            ),
            OpenRouterServerTool::new(OpenRouterServerToolKind::Datetime),
        ],
        ..Default::default()
    });

    let body = capture_request_body(&config).await;

    let tools = body["tools"]
        .as_array()
        .expect("tools array should be present");
    // Provider-executed server tools serialize as `openrouter:<name>` entries.
    let types: Vec<&str> = tools.iter().filter_map(|t| t["type"].as_str()).collect();
    assert!(types.contains(&"openrouter:web_search"));
    assert!(types.contains(&"openrouter:datetime"));

    let web = tools
        .iter()
        .find(|t| t["type"] == "openrouter:web_search")
        .expect("web_search entry present");
    assert_eq!(web["parameters"]["max_results"], 3);

    let datetime = tools
        .iter()
        .find(|t| t["type"] == "openrouter:datetime")
        .expect("datetime entry present");
    // Tools without parameters omit the field entirely.
    assert!(datetime.get("parameters").is_none());
}
