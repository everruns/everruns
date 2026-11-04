//! Public provider setup stays independent of per-call model configuration.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[cfg(all(feature = "openai", feature = "mistral"))]
#[tokio::test]
async fn api_selection_controls_the_wire_endpoint_on_the_connection() {
    use everruns::{Mistral, MistralApi, OpenAI, OpenAIApi, Provider};
    use everruns_contracts::driver_registry::{LlmCallConfig, Message, MessageRole};
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };

    let server = MockServer::start().await;
    let base = format!("{}/v1", server.uri());
    let cases: Vec<(Provider, &str)> = vec![
        (
            OpenAI::new("synthetic-key").base_url(&base).into(),
            "responses",
        ),
        (
            OpenAI::new("synthetic-key")
                .api(OpenAIApi::ChatCompletions)
                .base_url(&base)
                .into(),
            "chat/completions",
        ),
        (
            Mistral::new("synthetic-key").base_url(&base).into(),
            "chat/completions",
        ),
        (
            Mistral::new("synthetic-key")
                .api(MistralApi::Responses)
                .base_url(&base)
                .into(),
            "responses",
        ),
    ];
    for (provider, endpoint) in cases {
        server.reset().await;
        let body = if endpoint == "responses" {
            format!(
                "data: {}\n\ndata: {}\n\n",
                json!({"type":"response.output_text.delta", "delta":"hello"}),
                json!({"type":"response.completed", "response":{"id":"r1","status":"completed","output":[]}})
            )
        } else {
            format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"id":"c1","choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":"stop"}]})
            )
        };
        Mock::given(method("POST"))
            .and(path(format!("/v1/{endpoint}")))
            .and(header("authorization", "Bearer synthetic-key"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .expect(1)
            .mount(&server)
            .await;
        let config = LlmCallConfig::new("test-model");
        let response = provider
            .chat_completion(vec![Message::text(MessageRole::User, "hi")], &config)
            .await
            .unwrap();
        assert_eq!(response.text, "hello");
        assert!(config.driver_options.is_empty());
        server.verify().await;
    }
}

#[cfg(feature = "openai")]
#[test]
fn openai_chat_selection_preserves_other_services() {
    use everruns::{OpenAI, OpenAIApi, Provider};
    use everruns_contracts::ServiceKind;
    for api in [OpenAIApi::Responses, OpenAIApi::ChatCompletions] {
        let provider: Provider = OpenAI::new("secret").api(api).into();
        for service in [
            ServiceKind::Chat,
            ServiceKind::Embeddings,
            ServiceKind::Decisions,
            ServiceKind::Realtime,
        ] {
            assert!(provider.supports_service(service));
        }
    }
}

#[cfg(all(
    feature = "meta",
    feature = "vercel",
    feature = "typesafe",
    feature = "cloudflare",
    feature = "mai",
    feature = "chatgpt",
    feature = "codex",
    feature = "openai"
))]
#[test]
fn every_new_wrapper_converts_without_network_and_redacts_credentials() {
    use everruns::providers::mai::MaiAuth;
    use everruns::{
        AzureOpenAI, ChatGpt, Cloudflare, Codex, Mai, Meta, Provider, TypeSafe, Vercel,
    };
    use everruns_contracts::{BearerAuth, ServiceKind};
    let configurations = [
        format!("{:?}", Meta::new("private-key")),
        format!("{:?}", Vercel::new("private-key")),
        format!("{:?}", TypeSafe::new("private-key")),
        format!(
            "{:?}",
            AzureOpenAI::new("https://resource/openai/v1", "private-key")
        ),
        format!("{:?}", Cloudflare::new("account", "private-key")),
        format!(
            "{:?}",
            Mai::new("https://resource", MaiAuth::ApiKey("private-key".into()))
        ),
        format!("{:?}", ChatGpt::new(BearerAuth::new("private-key"))),
        format!("{:?}", Codex::new(BearerAuth::new("private-key"))),
    ];
    assert!(
        configurations
            .iter()
            .all(|debug| !debug.contains("private-key"))
    );
    let chat: Vec<Provider> = vec![
        Meta::new("key").base_url("https://proxy/v1").into(),
        Vercel::new("key").base_url("https://proxy/v1").into(),
        AzureOpenAI::new("https://proxy/v1", "key").into(),
        Cloudflare::new("account", "key")
            .gateway("gateway")
            .base_url("https://proxy/v1")
            .into(),
        Mai::new("https://proxy", MaiAuth::ApiKey("key".into())).into(),
        ChatGpt::new(BearerAuth::new("key"))
            .base_url("https://proxy/v1")
            .into(),
        Codex::new(BearerAuth::new("key"))
            .base_url("https://proxy/v1")
            .into(),
    ];
    for provider in chat {
        assert!(provider.supports_service(ServiceKind::Chat));
        assert!(
            provider
                .endpoint()
                .base_url()
                .unwrap()
                .starts_with("https://proxy")
        );
    }
    let decisions: Provider = TypeSafe::new("key").into();
    assert!(decisions.supports_service(ServiceKind::Decisions));
    assert!(!decisions.supports_service(ServiceKind::Chat));
}
