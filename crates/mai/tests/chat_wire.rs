// Wire-level tests for the Microsoft MAI driver.
//
// These exercise the full path: `MaiChatDriver` builds an OpenAI-compatible
// Chat Completions request through the core protocol driver, and its attached
// auth provider sets the outbound auth header. A wiremock server captures the
// request so we can assert the auth scheme, and streams back an SSE response so
// we can confirm parsing.
//
// Two auth modes are covered:
//   1. Azure AI Foundry API key  -> `api-key` header.
//   2. Microsoft Entra ID OAuth  -> a token is minted from the (mocked) token
//      endpoint and applied as `Authorization: Bearer <token>`.

use everruns_core::DriverRegistry;
use everruns_core::driver_registry::{
    ChatDriver, DriverId, LlmCallConfig, LlmMessage, LlmMessageRole, LlmStreamEvent, ProviderConfig,
};
use everruns_mai::{EntraOAuthConfig, MaiAuth, MaiChatDriver, register_driver};
use futures::StreamExt;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config(model: &str) -> LlmCallConfig {
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

/// A minimal OpenAI-compatible streamed chat completion response.
fn sse_chat_response() -> String {
    [
        r#"data: {"id":"1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"pong"},"finish_reason":null}]}"#,
        "",
        r#"data: {"id":"1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#,
        "",
        "data: [DONE]",
        "",
        "",
    ]
    .join("\n")
}

async fn drain_text(mut stream: everruns_core::driver_registry::LlmResponseStream) -> String {
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        match event.expect("stream item should not be a transport error") {
            LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
            LlmStreamEvent::Error(e) => panic!("stream returned an error: {e}"),
            _ => {}
        }
    }
    text
}

#[tokio::test]
async fn api_key_auth_sends_api_key_header_and_streams() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/openai/v1/chat/completions"))
        .and(header("api-key", "foundry-secret"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(sse_chat_response(), "text/event-stream"),
        )
        .mount(&server)
        .await;

    let driver = MaiChatDriver::new(MaiAuth::ApiKey("foundry-secret".into()), server.uri());
    let messages = vec![LlmMessage::text(LlmMessageRole::User, "ping")];
    let stream = driver
        .chat_completion_stream(messages, &config("mai-code-1-flash"))
        .await
        .expect("MAI should accept the api-key authed request");

    assert_eq!(drain_text(stream).await, "pong");
}

#[tokio::test]
async fn entra_oauth_mints_token_and_sends_bearer() {
    let server = MockServer::start().await;

    // Token endpoint: client-credentials grant returns a bearer token.
    Mock::given(method("POST"))
        .and(path("/tenant-1/oauth2/v2.0/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .and(body_string_contains("client_id=client-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token_type": "Bearer",
            "access_token": "minted-token-xyz",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;

    // Chat endpoint: must receive the minted bearer token, not an api-key.
    Mock::given(method("POST"))
        .and(path("/openai/v1/chat/completions"))
        .and(header("authorization", "Bearer minted-token-xyz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(sse_chat_response(), "text/event-stream"),
        )
        .mount(&server)
        .await;

    let auth = MaiAuth::EntraOAuth(EntraOAuthConfig {
        tenant_id: "tenant-1".into(),
        client_id: "client-1".into(),
        client_secret: "secret-1".into(),
        scope: "https://cognitiveservices.azure.com/.default".into(),
        // Point the authority at the mock server so token minting is captured.
        authority: server.uri(),
    });

    let driver = MaiChatDriver::new(auth, server.uri());
    let messages = vec![LlmMessage::text(LlmMessageRole::User, "ping")];
    let stream = driver
        .chat_completion_stream(messages, &config("mai-code-1-flash"))
        .await
        .expect("MAI should accept the OAuth bearer authed request");

    assert_eq!(drain_text(stream).await, "pong");
}

/// The full server path: a provider is configured with an Entra OAuth **JSON
/// document** in its (encrypted) credential field, and the driver is built via
/// the registry exactly as model resolution / model sync do. This proves OAuth
/// works end-to-end without any provider-metadata plumbing.
#[tokio::test]
async fn registry_built_driver_uses_oauth_json_credential() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/tenant-9/oauth2/v2.0/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token_type": "Bearer",
            "access_token": "cred-doc-token",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/openai/v1/chat/completions"))
        .and(header("authorization", "Bearer cred-doc-token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(sse_chat_response(), "text/event-stream"),
        )
        .mount(&server)
        .await;

    // OAuth credentials carried as a JSON document in the credential field,
    // pointing `authority` at the mock token endpoint.
    let credential = serde_json::json!({
        "tenant_id": "tenant-9",
        "client_id": "client-9",
        "client_secret": "secret-9",
        "authority": server.uri(),
    })
    .to_string();

    let mut registry = DriverRegistry::new();
    register_driver(&mut registry);
    let driver = registry
        .create_chat_driver(
            &ProviderConfig::new(DriverId::Mai)
                .with_api_key(credential)
                .with_base_url(server.uri()),
        )
        .expect("registry should build a MAI driver from an OAuth JSON credential");

    let messages = vec![LlmMessage::text(LlmMessageRole::User, "ping")];
    let stream = driver
        .chat_completion_stream(messages, &config("mai-code-1-flash"))
        .await
        .expect("registry-built MAI driver should authenticate via OAuth");

    assert_eq!(drain_text(stream).await, "pong");
}
