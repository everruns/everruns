#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// Wire-level tests for the gateway drivers in `everruns-drivers`.
//
// These exercise the full path for each vendor: the driver builds a request
// through the shared protocol driver, its attached auth sets the outbound
// headers, and a wiremock server captures the request so the URL shape and
// auth scheme can be asserted before an SSE response is streamed back.
//
// Each vendor's block is behind its own feature, so the suite tells the truth
// about whatever feature set it was compiled with.

#[cfg(any(feature = "cloudflare", feature = "vercel"))]
use everruns_provider::driver_registry::{
    ChatDriver, DriverRegistry, LlmCallConfig, LlmStreamEvent, Message, MessageRole,
    ProviderConfig, ServiceKind,
};
#[cfg(any(feature = "cloudflare", feature = "vercel"))]
use futures::StreamExt;

#[cfg(any(feature = "cloudflare", feature = "vercel"))]
async fn drain_text(mut stream: everruns_provider::driver_registry::LlmResponseStream) -> String {
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

#[cfg(feature = "cloudflare")]
mod cloudflare {
    use super::*;
    use everruns_drivers::cloudflare::{
        CLOUDFLARE_AI_GATEWAY_HOST, CloudflareChatDriver, CloudflareGatewayAuth, descriptor,
        gateway_base_url, register_driver,
    };
    use everruns_provider::{DriverId, Provider};
    use wiremock::matchers::{header, header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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

    #[test]
    fn base_url_embeds_the_account_and_gateway_and_stops_at_compat() {
        assert_eq!(
            gateway_base_url("acct123", "my-gateway"),
            format!("https://{CLOUDFLARE_AI_GATEWAY_HOST}/v1/acct123/my-gateway/compat")
        );
        // A blank gateway name falls back to the one Cloudflare creates, and
        // stray whitespace or slashes never produce an empty path segment.
        assert_eq!(
            gateway_base_url("  acct123 ", "  "),
            format!("https://{CLOUDFLARE_AI_GATEWAY_HOST}/v1/acct123/default/compat")
        );
        assert_eq!(
            gateway_base_url("/acct123/", "/gw/"),
            format!("https://{CLOUDFLARE_AI_GATEWAY_HOST}/v1/acct123/gw/compat")
        );
    }

    #[test]
    fn descriptor_declares_the_gateway_token_the_two_ids_and_the_optional_upstream_key() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);
        let descriptor = registry.descriptor(&DriverId::Cloudflare).unwrap();
        assert_eq!(descriptor.display_name, "Cloudflare AI Gateway");
        assert_eq!(descriptor.services, vec![ServiceKind::Chat]);

        let required: Vec<_> = descriptor
            .credential_schema
            .fields
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(required, vec!["api_key", "account_id", "gateway_id"]);
        let optional: Vec<_> = descriptor
            .credential_schema
            .fields
            .iter()
            .filter(|f| !f.required)
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(optional, vec!["provider_api_key"]);
    }

    /// The gateway's own token goes in `cf-aig-authorization`, and nothing is
    /// sent in `Authorization` when the gateway holds the provider keys.
    #[tokio::test]
    async fn gateway_token_alone_authenticates_and_streams() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/acct123/my-gateway/compat/chat/completions"))
            .and(header("cf-aig-authorization", "Bearer gateway-secret"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(sse_chat_response(), "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let provider = Provider::new("cloudflare-test", CloudflareChatDriver::new())
            .base_url(format!("{}/v1/acct123/my-gateway/compat", server.uri()))
            .auth(CloudflareGatewayAuth::new(
                Some("gateway-secret".into()),
                None,
            ));
        let stream = provider
            .chat_completion_stream(
                vec![Message::text(MessageRole::User, "ping")],
                &LlmCallConfig::new("openai/gpt-5.2"),
            )
            .await
            .expect("the gateway should accept a cf-aig-authorization request");

        assert_eq!(drain_text(stream).await, "pong");
    }

    /// A gateway that stores no provider keys needs both headers.
    #[tokio::test]
    async fn upstream_key_is_sent_alongside_the_gateway_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/acct123/my-gateway/compat/chat/completions"))
            .and(header("cf-aig-authorization", "Bearer gateway-secret"))
            .and(header("authorization", "Bearer upstream-secret"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(sse_chat_response(), "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let provider = Provider::new("cloudflare-test", CloudflareChatDriver::new())
            .base_url(format!("{}/v1/acct123/my-gateway/compat", server.uri()))
            .auth(
                CloudflareGatewayAuth::new(Some("gateway-secret".into()), None)
                    .with_provider_api_key("upstream-secret"),
            );
        let stream = provider
            .chat_completion_stream(
                vec![Message::text(MessageRole::User, "ping")],
                &LlmCallConfig::new("openai/gpt-5.2"),
            )
            .await
            .expect("the gateway should accept a dual-header request");

        assert_eq!(drain_text(stream).await, "pong");
    }

    /// The registered driver derives its URL from the credential fields, so an
    /// operator never hand-assembles a URL whose shape is fixed.
    #[tokio::test]
    async fn registered_driver_derives_the_url_from_the_credential_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/acct123/my-gateway/compat/chat/completions"))
            .and(header("cf-aig-authorization", "Bearer gateway-secret"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(sse_chat_response(), "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);
        // An explicit base URL wins over the derived one, which is how a
        // `/compat` proxy in front of the gateway is configured — and is what
        // lets this test point at wiremock.
        let driver = registry
            .create_chat_driver(
                &ProviderConfig::new(DriverId::Cloudflare)
                    .with_api_key(
                        everruns_provider::credential_schema::assemble_credential_document(
                            &[
                                ("api_key".to_string(), "gateway-secret".to_string()),
                                ("account_id".to_string(), "acct123".to_string()),
                                ("gateway_id".to_string(), "my-gateway".to_string()),
                            ]
                            .into_iter()
                            .collect(),
                        )
                        .unwrap(),
                    )
                    .with_base_url(format!("{}/v1/acct123/my-gateway/compat", server.uri())),
            )
            .expect("the Cloudflare driver should be constructible from its credential document");

        let stream = driver
            .chat_completion_stream(
                &everruns_provider::ProviderEndpoint::default(),
                vec![Message::text(MessageRole::User, "ping")],
                &LlmCallConfig::new("openai/gpt-5.2"),
            )
            .await
            .expect("the registered driver should reach the gateway");

        assert_eq!(drain_text(stream).await, "pong");
    }

    /// `/compat` serves no catalog, so discovery declines rather than
    /// reporting an empty one as the truth.
    #[tokio::test]
    async fn discovery_declines_because_compat_serves_no_catalog() {
        let server = MockServer::start().await;
        // Any request at all would be wrong: assert none is made.
        Mock::given(header_exists("cf-aig-authorization"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let endpoint = Provider::new("cloudflare-test", CloudflareChatDriver::new())
            .base_url(format!("{}/v1/acct123/my-gateway/compat", server.uri()))
            .auth(CloudflareGatewayAuth::new(
                Some("gateway-secret".into()),
                None,
            ))
            .endpoint()
            .clone();

        assert!(
            CloudflareChatDriver::new()
                .list_models(&endpoint)
                .await
                .unwrap()
                .is_none()
        );
    }

    /// Credentials must never reach a log through `{:?}`.
    #[test]
    fn debug_redacts_both_tokens() {
        let rendered = format!(
            "{:?}",
            CloudflareGatewayAuth::new(Some("gateway-secret".into()), None)
                .with_provider_api_key("upstream-secret")
        );
        assert!(!rendered.contains("gateway-secret"), "{rendered}");
        assert!(!rendered.contains("upstream-secret"), "{rendered}");
    }

    #[test]
    fn descriptor_is_chat_only() {
        assert_eq!(descriptor().services, vec![ServiceKind::Chat]);
    }
}

#[cfg(feature = "vercel")]
mod vercel {
    use super::*;
    use everruns_drivers::vercel::{
        VERCEL_AI_GATEWAY_DEFAULT_API_URL, VercelChatDriver, descriptor, provider, register_driver,
    };
    use everruns_provider::DriverId;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A minimal Open Responses streamed response.
    fn sse_responses() -> String {
        let terminal = r#"{"type":"response.completed","sequence_number":2,"response":{"id":"vercel-result","object":"response","created_at":0,"model":"anthropic/claude-opus-5","status":"completed","output":[{"type":"message","id":"msg","status":"completed","role":"assistant","content":[]}],"usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}}"#;
        format!(
            "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"pong\"}}\n\ndata: {terminal}\n\n"
        )
    }

    #[test]
    fn provider_targets_the_documented_responses_endpoint() {
        assert_eq!(
            provider("vercel", "synthetic-key")
                .endpoint()
                .url("responses")
                .as_deref(),
            Some("https://ai-gateway.vercel.sh/v1/responses")
        );
        assert_eq!(
            VERCEL_AI_GATEWAY_DEFAULT_API_URL,
            "https://ai-gateway.vercel.sh/v1"
        );
    }

    #[test]
    fn descriptor_declares_the_gateway_api_key() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);
        let descriptor = registry.descriptor(&DriverId::Vercel).unwrap();
        assert_eq!(descriptor.display_name, "Vercel AI Gateway");
        assert_eq!(descriptor.services, vec![ServiceKind::Chat]);
        assert_eq!(descriptor.credential_schema.fields.len(), 1);
        assert_eq!(descriptor.credential_schema.fields[0].name, "api_key");
        assert!(descriptor.credential_schema.fields[0].required);
    }

    /// The gateway takes an API key or an OIDC token in the same bearer header.
    #[tokio::test]
    async fn bearer_auth_reaches_responses_and_streams() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .and(header("authorization", "Bearer synthetic-key"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(sse_responses(), "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let stream = provider("vercel-test", "synthetic-key")
            .base_url(format!("{}/v1", server.uri()))
            .chat_completion_stream(
                vec![Message::text(MessageRole::User, "ping")],
                &LlmCallConfig::new("anthropic/claude-opus-5"),
            )
            .await
            .expect("the gateway should accept a bearer-authed Open Responses request");

        assert_eq!(drain_text(stream).await, "pong");
    }

    #[tokio::test]
    async fn registered_driver_streams_over_open_responses() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .and(header("authorization", "Bearer synthetic-key"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(sse_responses(), "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);
        let driver = registry
            .create_chat_driver(
                &ProviderConfig::new(DriverId::Vercel)
                    .with_api_key("synthetic-key")
                    .with_base_url(format!("{}/v1", server.uri())),
            )
            .expect("the Vercel driver should be constructible");

        let stream = driver
            .chat_completion_stream(
                &everruns_provider::ProviderEndpoint::default(),
                vec![Message::text(MessageRole::User, "ping")],
                &LlmCallConfig::new("anthropic/claude-opus-5"),
            )
            .await
            .expect("the registered driver should reach the gateway");

        assert_eq!(drain_text(stream).await, "pong");
    }

    /// Discovery is gated on Vercel's own host: a custom proxy URL may resolve
    /// to private infrastructure, so it is declined rather than probed.
    #[tokio::test]
    async fn discovery_declines_a_host_that_is_not_the_gateway() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [{"id": "anthropic/claude-opus-5", "object": "model"}],
            })))
            .expect(0)
            .mount(&server)
            .await;

        let endpoint = provider("vercel-test", "synthetic-key")
            .base_url(format!("{}/v1", server.uri()))
            .endpoint()
            .clone();

        assert!(
            VercelChatDriver::new()
                .list_models(&endpoint)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn descriptor_is_chat_only() {
        assert_eq!(descriptor().services, vec![ServiceKind::Chat]);
    }
}
