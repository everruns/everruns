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
        CLOUDFLARE_API_HOST, CloudflareAuth, CloudflareChatDriver, account_base_url, descriptor,
        provider, register_driver,
    };
    use everruns_provider::DriverId;
    use wiremock::matchers::{header, header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A minimal OpenAI-compatible streamed chat completion response.
    fn sse_responses() -> String {
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
    fn base_url_embeds_the_account_and_stops_at_the_api_version() {
        assert_eq!(
            account_base_url("acct123"),
            format!("https://{CLOUDFLARE_API_HOST}/client/v4/accounts/acct123/ai/v1")
        );
        // Stray whitespace or slashes never produce an empty path segment.
        assert_eq!(
            account_base_url("  /acct123/ "),
            format!("https://{CLOUDFLARE_API_HOST}/client/v4/accounts/acct123/ai/v1")
        );
        assert_eq!(
            provider("cloudflare", "acct123", "token", None)
                .endpoint()
                .url("chat/completions")
                .as_deref(),
            Some("https://api.cloudflare.com/client/v4/accounts/acct123/ai/v1/chat/completions")
        );
    }

    #[test]
    fn descriptor_requires_the_token_and_account_and_leaves_the_gateway_optional() {
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
        assert_eq!(required, vec!["api_key", "account_id"]);
        // Third-party models fall back to the account's default gateway, so
        // pinning one is optional.
        let optional: Vec<_> = descriptor
            .credential_schema
            .fields
            .iter()
            .filter(|f| !f.required)
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(optional, vec!["gateway_id"]);
    }

    /// The account token is a plain bearer token on this surface, not the
    /// `cf-aig-authorization` the deprecated `/compat` endpoint takes.
    #[tokio::test]
    async fn bearer_auth_reaches_chat_completions_and_streams() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/client/v4/accounts/acct123/ai/v1/chat/completions"))
            .and(header("authorization", "Bearer cf-token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(sse_responses(), "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let stream = provider("cloudflare-test", "acct123", "cf-token", None)
            .base_url(format!("{}/client/v4/accounts/acct123/ai/v1", server.uri()))
            .chat_completion_stream(
                vec![Message::text(MessageRole::User, "ping")],
                &LlmCallConfig::new("openai/gpt-6-luna"),
            )
            .await
            .expect("the AI REST API should accept a bearer-authed Chat Completions request");

        assert_eq!(drain_text(stream).await, "pong");
    }

    /// A pinned gateway rides the `cf-aig-gateway-id` header, which is what
    /// Workers AI (`@cf/`) models require.
    #[tokio::test]
    async fn a_pinned_gateway_is_sent_as_a_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/client/v4/accounts/acct123/ai/v1/chat/completions"))
            .and(header("authorization", "Bearer cf-token"))
            .and(header("cf-aig-gateway-id", "my-gateway"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(sse_responses(), "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let stream = provider("cloudflare-test", "acct123", "cf-token", Some("my-gateway"))
            .base_url(format!("{}/client/v4/accounts/acct123/ai/v1", server.uri()))
            .chat_completion_stream(
                vec![Message::text(MessageRole::User, "ping")],
                &LlmCallConfig::new("@cf/meta/llama-3.3-70b-instruct-fp8-fast"),
            )
            .await
            .expect("a pinned gateway should reach the AI REST API");

        assert_eq!(drain_text(stream).await, "pong");
    }

    /// No gateway configured means no header at all, so the account's default
    /// gateway applies rather than one named "".
    #[tokio::test]
    async fn no_gateway_sends_no_gateway_header() {
        let server = MockServer::start().await;
        Mock::given(header_exists("cf-aig-gateway-id"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/client/v4/accounts/acct123/ai/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(sse_responses(), "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let stream = provider("cloudflare-test", "acct123", "cf-token", Some("   "))
            .base_url(format!("{}/client/v4/accounts/acct123/ai/v1", server.uri()))
            .chat_completion_stream(
                vec![Message::text(MessageRole::User, "ping")],
                &LlmCallConfig::new("openai/gpt-6-luna"),
            )
            .await
            .expect("an unpinned gateway should still reach the AI REST API");

        assert_eq!(drain_text(stream).await, "pong");
    }

    /// The registered driver derives its URL from the credential fields, so an
    /// operator never hand-assembles a URL whose shape is fixed.
    #[tokio::test]
    async fn registered_driver_derives_the_url_from_the_credential_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/client/v4/accounts/acct123/ai/v1/chat/completions"))
            .and(header("authorization", "Bearer cf-token"))
            .and(header("cf-aig-gateway-id", "my-gateway"))
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
                &ProviderConfig::new(DriverId::Cloudflare)
                    .with_api_key(
                        everruns_provider::credential_schema::assemble_credential_document(
                            &[
                                ("api_key".to_string(), "cf-token".to_string()),
                                ("account_id".to_string(), "acct123".to_string()),
                                ("gateway_id".to_string(), "my-gateway".to_string()),
                            ]
                            .into_iter()
                            .collect(),
                        )
                        .unwrap(),
                    )
                    // An explicit base URL wins over the derived one, which is
                    // what lets this test point at wiremock.
                    .with_base_url(format!("{}/client/v4/accounts/acct123/ai/v1", server.uri())),
            )
            .expect("the Cloudflare driver should be constructible from its credential document");

        let stream = driver
            .chat_completion_stream(
                &everruns_provider::ProviderEndpoint::default(),
                vec![Message::text(MessageRole::User, "ping")],
                &LlmCallConfig::new("openai/gpt-6-luna"),
            )
            .await
            .expect("the registered driver should reach the AI REST API");

        assert_eq!(drain_text(stream).await, "pong");
    }

    /// Discovery never probes a host that is not Cloudflare's: a proxy base URL
    /// would otherwise receive the account's bearer token.
    #[tokio::test]
    async fn discovery_declines_a_foreign_host() {
        let server = MockServer::start().await;
        // Any request at all would be wrong: assert none is made.
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        // `provider()` derives the api.cloudflare.com URL; overriding the base
        // is the operator path this gate exists for.
        let endpoint = provider("cloudflare-test", "acct123", "cf-token", None)
            .base_url(format!("{}/ai/v1", server.uri()))
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
    fn debug_redacts_the_token() {
        let rendered = format!("{:?}", CloudflareAuth::new("cf-token", Some("gw".into())));
        assert!(!rendered.contains("cf-token"), "{rendered}");
        // The gateway id is routing, not a secret, so it stays legible.
        assert!(rendered.contains("gw"), "{rendered}");
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
