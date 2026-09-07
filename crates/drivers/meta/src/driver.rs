// Meta Model API Chat Driver
//
// Meta Model API exposes Muse models through an OpenAI-compatible Responses
// API. This driver tags the shared protocol implementation with DriverId::Meta
// so Muse profiles gate Meta-native features such as phases and tool search.

use async_trait::async_trait;
use chrono::TimeZone;
use serde::Deserialize;

use everruns_provider::OpenResponsesProtocolChatDriver;
use everruns_provider::credential_schema::CredentialFormSchema;
use everruns_provider::driver_helpers::fetch_models;
use everruns_provider::driver_registry::{
    ChatDriver, DiscoveredModel, DriverDescriptor, DriverId, DriverRegistry, LlmCallConfig,
    LlmMessage, LlmResponseStream,
};
use everruns_provider::error::Result;
use everruns_provider::openai_protocol::{models_url_for_api_url, url_host_eq};
use everruns_provider::{BearerAuth, Provider, ProviderEndpoint};

/// Meta Model API Responses endpoint.
pub const META_DEFAULT_API_URL: &str = "https://api.meta.ai/v1/responses";
const META_API_HOST: &str = "api.meta.ai";

/// Ready-to-use Meta Model API provider assembly.
pub fn provider(
    id: impl Into<everruns_provider::ProviderKey>,
    api_key: impl Into<String>,
) -> Provider {
    Provider::new(id, MetaChatDriver::new())
        .base_url("https://api.meta.ai/v1")
        .auth(BearerAuth::new(api_key))
}

/// Meta Model API driver using the OpenAI-compatible Responses API.
#[derive(Clone)]
pub struct MetaChatDriver {
    inner: OpenResponsesProtocolChatDriver,
}

impl MetaChatDriver {
    /// Create the Meta-compatible Responses wire driver.
    pub fn new() -> Self {
        Self {
            inner: OpenResponsesProtocolChatDriver::new()
                .with_native_features(true, true)
                .with_stateful_responses(true),
        }
    }
}

#[async_trait]
impl ChatDriver for MetaChatDriver {
    async fn chat_completion_stream(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner
            .chat_completion_stream(endpoint, messages, config)
            .await
    }

    fn supports_stateful_responses(&self) -> bool {
        self.inner.supports_stateful_responses()
    }

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        self.inner.supports_parallel_tool_calls(model)
    }

    async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        let Some(api_url) = endpoint.url("responses") else {
            return Ok(None);
        };
        if !url_host_eq(&api_url, META_API_HOST) {
            return Ok(None);
        }

        let models_url = models_url_for_api_url(&api_url);
        list_meta_models(self.inner.client(), endpoint, &models_url).await
    }
}

impl std::fmt::Debug for MetaChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetaChatDriver")
            .field("api", &"Meta Model API Responses")
            .finish()
    }
}

#[derive(Debug, Deserialize)]
struct MetaModelsResponse {
    data: Vec<MetaModelInfo>,
}

#[derive(Debug, Deserialize)]
struct MetaModelInfo {
    id: String,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    owned_by: Option<String>,
}

async fn list_meta_models(
    client: &reqwest::Client,
    endpoint: &ProviderEndpoint,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let resolved = endpoint.resolve("GET", models_url, &[]).await?;
    let mut request = client.get(&resolved.url);
    for (name, value) in resolved.headers {
        request = request.header(name, value);
    }
    fetch_models::<MetaModelsResponse, _>(
        request,
        "Failed to fetch Meta models",
        "Failed to parse Meta models response",
        &[],
        |response| {
            response
                .data
                .into_iter()
                .map(|model| DiscoveredModel {
                    capabilities: vec!["chat".to_string()],
                    created_at: model
                        .created
                        .and_then(|timestamp| chrono::Utc.timestamp_opt(timestamp, 0).single()),
                    display_name: None,
                    owned_by: model.owned_by,
                    model_id: model.id,
                    discovered_profile: None,
                })
                .collect()
        },
    )
    .await
}

/// Register Meta Model API as a chat provider.
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_descriptor(DriverDescriptor {
        display_name: "Meta Model API".into(),
        credential_schema: CredentialFormSchema::api_key(
            "Create an API key in the [Meta Model API dashboard](https://dev.meta.ai/).",
        ),
        ..DriverDescriptor::chat_only(DriverId::Meta, |config| {
            Provider::new(config.provider.clone(), MetaChatDriver::new())
                .base_url(
                    config
                        .base_url
                        .as_deref()
                        .unwrap_or("https://api.meta.ai/v1"),
                )
                .auth(BearerAuth::new(config.api_key.clone().unwrap_or_default()))
                .into_boxed_driver()
        })
    });
}

impl Default for MetaChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::driver_registry::{LlmMessageRole, ProviderConfig, ServiceKind};
    use serde_json::{Value, json};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    fn config() -> LlmCallConfig {
        LlmCallConfig {
            model: "muse-spark-1.3".into(),
            temperature: Some(0.25),
            max_tokens: Some(64),
            tools: vec![],
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            metadata: Default::default(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: Some(false),
            volatile_suffix_len: 0,
            extra_headers: vec![],
            cache_diagnostics: None,
        }
    }

    #[tokio::test]
    async fn direct_and_registered_providers_send_native_stateful_contract() {
        assert_eq!(
            provider("meta", "synthetic-key")
                .endpoint()
                .url("responses")
                .as_deref(),
            Some("https://api.meta.ai/v1/responses")
        );
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);
        let descriptor = registry.descriptor(&DriverId::Meta).unwrap();
        assert_eq!(descriptor.display_name, "Meta Model API");
        assert_eq!(descriptor.services, vec![ServiceKind::Chat]);
        assert_eq!(descriptor.credential_schema.fields.len(), 1);
        assert_eq!(descriptor.credential_schema.fields[0].name, "api_key");
        assert!(descriptor.credential_schema.fields[0].required);
        for registered in [false, true] {
            for continuation in [false, true] {
                let server = MockServer::builder().start().await;
                let terminal = json!({"type":"response.completed","sequence_number":2,"response":{"id":"meta-result","object":"response","created_at":0,"model":"muse-spark-1.3","status":"completed","output":[{"type":"message","id":"msg","status":"completed","role":"assistant","content":[],"phase":"final_answer"}],"usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}});
                let wire = format!(
                    "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}}\n\ndata: {terminal}\n\n"
                );
                Mock::given(method("POST"))
                    .and(path("/v1/responses"))
                    .and(header("authorization", "Bearer synthetic-key"))
                    .respond_with(
                        ResponseTemplate::new(200)
                            .insert_header("content-type", "text/event-stream")
                            .set_body_string(wire),
                    )
                    .expect(1)
                    .mount(&server)
                    .await;
                let mut config = config();
                config.previous_response_id = continuation.then(|| "prior-response".into());
                let mut prior = LlmMessage::text(LlmMessageRole::Assistant, "prior answer");
                prior.phase = Some(everruns_provider::execution_phase::ExecutionPhase::Commentary);
                let messages = vec![
                    LlmMessage::text(LlmMessageRole::System, "rules"),
                    LlmMessage::text(LlmMessageRole::User, "old question"),
                    prior,
                    LlmMessage::text(LlmMessageRole::User, "new question"),
                ];
                let url = format!(
                    "{}/v1{}",
                    server.uri(),
                    if registered { "/responses" } else { "" }
                );
                let response = if registered {
                    let driver = registry
                        .create_chat_driver(
                            &ProviderConfig::new(DriverId::Meta)
                                .with_api_key("synthetic-key")
                                .with_base_url(url),
                        )
                        .unwrap();
                    assert!(driver.supports_stateful_responses());
                    driver
                        .chat_completion(&ProviderEndpoint::default(), messages, &config)
                        .await
                } else {
                    provider("direct", "synthetic-key")
                        .base_url(url)
                        .chat_completion(messages, &config)
                        .await
                }
                .unwrap();
                assert_eq!(response.text, "answer");
                assert!(response.tool_calls.is_none());
                assert!(response.reasoning.is_empty());
                assert_eq!(
                    response.metadata.response_id.as_deref(),
                    Some("meta-result")
                );
                assert_eq!(response.metadata.phase.as_deref(), Some("final_answer"));
                assert_eq!(response.metadata.finish_reason.as_deref(), Some("stop"));
                assert_eq!(
                    (
                        response.metadata.prompt_tokens,
                        response.metadata.completion_tokens,
                        response.metadata.total_tokens
                    ),
                    (Some(10), Some(2), Some(12))
                );
                let input = if continuation {
                    json!([{"type":"message","role":"user","content":"new question"}])
                } else {
                    json!([{"type":"message","role":"user","content":"old question"},{"type":"message","role":"assistant","content":"prior answer","phase":"commentary"},{"type":"message","role":"user","content":"new question"}])
                };
                let mut expected = json!({"model":"muse-spark-1.3","instructions":"rules","input":input,"temperature":0.25,"max_output_tokens":64,"parallel_tool_calls":false,"stream":true});
                if continuation {
                    expected["previous_response_id"] = json!("prior-response");
                }
                let requests = server.received_requests().await.unwrap();
                assert_eq!(requests.len(), 1);
                assert_eq!(requests[0].body_json::<Value>().unwrap(), expected);
            }
        }
        let keyless = registry
            .create_chat_driver(&ProviderConfig::new(DriverId::Meta))
            .unwrap();
        let error = keyless
            .chat_completion(&ProviderEndpoint::default(), vec![], &config())
            .await
            .unwrap_err();
        assert_eq!(
            error.llm_error_kind(),
            Some(everruns_provider::error::LlmErrorKind::Authentication)
        );
    }
    #[tokio::test]
    async fn discovery_rejects_lookalike_hosts_before_resolving_credentials() {
        use everruns_provider::runtime_provider::{ProviderAuth, ProviderAuthRequest};
        struct ForbiddenAuth;
        #[async_trait]
        impl ProviderAuth for ForbiddenAuth {
            async fn headers(&self, _: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>> {
                panic!("disallowed discovery accessed credentials")
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
        for url in [
            None,
            Some("https://proxy.example/v1"),
            Some("https://meta.ai/v1"),
            Some("https://api.meta.ai.evil.example/v1"),
            Some("https://api.meta.ai@evil.example/v1"),
            Some("https://evil.example/api.meta.ai"),
            Some("not a URL"),
        ] {
            let service = Provider::new("gate", MetaChatDriver::new()).auth(ForbiddenAuth);
            let service = if let Some(url) = url {
                service.base_url(url)
            } else {
                service
            };
            assert!(service.list_models().await.unwrap().is_none(), "{url:?}");
        }
    }
    #[tokio::test]
    async fn discovery_preserves_complete_catalog_and_optional_metadata() {
        let server = MockServer::builder().start().await;
        Mock::given(method("GET")).and(path("/v1/models")).and(header("authorization","Bearer synthetic-key")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"muse-spark-1.3","created":0,"owned_by":"meta"},{"id":"future-model","created":9223372036854775807_i64,"owned_by":"extension"},{"id":"minimal"}]}))).expect(1).mount(&server).await;
        let service = provider("meta", "synthetic-key").base_url(format!("{}/v1", server.uri()));
        let models = list_meta_models(
            &reqwest::Client::new(),
            service.endpoint(),
            &format!("{}/v1/models", server.uri()),
        )
        .await
        .unwrap()
        .unwrap();
        let actual:Vec<_>=models.into_iter().map(|model|json!({"id":model.model_id,"name":model.display_name,"capabilities":model.capabilities,"created":model.created_at.map(|t|t.to_rfc3339()),"owner":model.owned_by,"profile":model.discovered_profile})).collect();
        assert_eq!(
            actual,
            vec![
                json!({"id":"muse-spark-1.3","name":null,"capabilities":["chat"],"created":"1970-01-01T00:00:00+00:00","owner":"meta","profile":null}),
                json!({"id":"future-model","name":null,"capabilities":["chat"],"created":null,"owner":"extension","profile":null}),
                json!({"id":"minimal","name":null,"capabilities":["chat"],"created":null,"owner":null,"profile":null})
            ]
        );
    }
}
