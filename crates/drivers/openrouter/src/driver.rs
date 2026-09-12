// OpenRouter Chat Driver
//
// OpenRouter exposes an OpenAI-compatible Responses API, so this driver wraps
// the core `OpenResponsesProtocolChatDriver` and tags it with
// `DriverId::OpenRouter` for model-profile lookup. The richer `/models`
// discovery metadata OpenRouter advertises is parsed here into capability
// profiles (notably reasoning support, which gates the UI's effort selector).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::TimeZone;

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

use crate::request_ext::OpenRouterRequestExtension;
use crate::types::OpenRouterModelsResponse;

/// Ready-to-use OpenRouter provider assembly.
pub fn provider(
    id: impl Into<everruns_provider::ProviderKey>,
    api_key: impl Into<String>,
) -> Provider {
    Provider::new(id, OpenRouterChatDriver::new())
        .base_url("https://openrouter.ai/api/v1")
        .auth(BearerAuth::new(api_key))
}

// ============================================================================
// OpenRouter Chat Driver (OpenAI-compatible Responses API)
// ============================================================================

/// OpenRouter driver using its OpenAI-compatible Responses API.
#[derive(Clone)]
pub struct OpenRouterChatDriver {
    inner: OpenResponsesProtocolChatDriver,
}

impl OpenRouterChatDriver {
    /// Create the OpenRouter Responses wire driver.
    pub fn new() -> Self {
        Self {
            inner: OpenResponsesProtocolChatDriver::new()
                .with_request_extension(Arc::new(OpenRouterRequestExtension)),
        }
    }
}

#[async_trait]
impl ChatDriver for OpenRouterChatDriver {
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
        // OpenRouter discovery is only safe against OpenRouter's own host.
        // Custom proxy URLs may resolve to private infrastructure at request time.
        if !is_openrouter_api_url(&api_url) {
            return Ok(None);
        }

        let models_url = models_url_for_api_url(&api_url);
        list_openrouter_models(&self.inner.client(), endpoint, &models_url).await
    }
}

impl std::fmt::Debug for OpenRouterChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenRouterChatDriver")
            .field("api", &"OpenRouter Responses")
            .finish()
    }
}

// ============================================================================
// Model discovery
// ============================================================================

/// Fetch and filter OpenRouter models, building capability profiles from the
/// `supported_parameters` metadata OpenRouter advertises.
async fn list_openrouter_models(
    client: &reqwest::Client,
    endpoint: &ProviderEndpoint,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let resolved = endpoint.resolve("GET", models_url, &[]).await?;
    let mut request = client.get(&resolved.url);
    for (name, value) in resolved.headers {
        request = request.header(name, value);
    }
    fetch_models::<OpenRouterModelsResponse, _>(
        request,
        "Failed to fetch models",
        "Failed to parse models response",
        &[],
        |models_response| {
            models_response
                .data
                .into_iter()
                .filter(|m| m.is_chat_model())
                .map(|m| {
                    let profile = m.to_discovered_profile();
                    DiscoveredModel {
                        capabilities: vec!["chat".to_string()],
                        created_at: m
                            .created
                            .and_then(|ts| chrono::Utc.timestamp_opt(ts, 0).single()),
                        display_name: m.name.clone(),
                        owned_by: m.id.split('/').next().map(str::to_owned),
                        model_id: m.id,
                        discovered_profile: Some(profile),
                    }
                })
                .collect()
        },
    )
    .await
}

/// OpenRouter exposes an OpenAI-compatible `/models` endpoint with richer
/// metadata; recognize its host so discovery (and capability profiling) runs.
fn is_openrouter_api_url(api_url: &str) -> bool {
    url_host_eq(api_url, "openrouter.ai")
}

// ============================================================================
// Driver Registration
// ============================================================================

/// Register the OpenRouter driver with the driver registry.
///
/// This registers `DriverId::OpenRouter` (OpenRouter Responses API).
///
/// # Example
///
/// ```ignore
/// use everruns_provider::DriverRegistry;
/// use everruns_openrouter::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// ```
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_descriptor(DriverDescriptor {
        display_name: "OpenRouter".into(),
        credential_schema: CredentialFormSchema::api_key(
            "Create an API key at [openrouter.ai/settings/keys](https://openrouter.ai/settings/keys), \
             or use \"Connect with OpenRouter\" to authorize one without leaving the app.",
        ),
        // OpenRouter supports a one-click PKCE flow that hands back a
        // user-controlled API key, so an admin can connect without minting and
        // pasting a key manually. The key is stored like any other credential.
        oauth: Some(everruns_provider::DriverOAuthConfig::openrouter()),
        ..DriverDescriptor::chat_only(DriverId::OpenRouter, |config| {
            Provider::new(config.provider.clone(), OpenRouterChatDriver::new())
                .base_url(config.base_url.as_deref().unwrap_or("https://openrouter.ai/api/v1"))
                .auth(BearerAuth::new(config.api_key.clone().unwrap_or_default()))
                .into_boxed_driver()
        })
    });
}

impl Default for OpenRouterChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::driver_registry::{DriverId, ProviderConfig, ServiceKind};

    fn base_config(model: &str) -> LlmCallConfig {
        LlmCallConfig {
            speed: None,
            verbosity: None,
            model: model.to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            reasoning_state: None,
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            driver_options: Default::default(),
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        }
    }

    #[tokio::test]
    async fn direct_and_registered_providers_send_complete_authenticated_requests() {
        use everruns_provider::driver_registry::LlmMessageRole;
        use serde_json::{Value, json};
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);
        let descriptor = registry.descriptor(&DriverId::OpenRouter).unwrap();
        assert_eq!(descriptor.services, vec![ServiceKind::Chat]);
        assert_eq!(descriptor.credential_schema.fields[0].name, "api_key");
        assert_eq!(
            provider("default", "synthetic-key")
                .endpoint()
                .url("responses")
                .as_deref(),
            Some("https://openrouter.ai/api/v1/responses")
        );
        for suffix in ["/api/v1", "/api/v1/responses"] {
            for registered in [false, true] {
                let server = MockServer::builder().start().await;
                Mock::given(method("POST")).and(path("/api/v1/responses")).and(query_param("route","custom")).and(header("authorization","Bearer synthetic-key"))
                    .respond_with(ResponseTemplate::new(200).insert_header("content-type","text/event-stream").set_body_string("data: {\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}\n\ndata: {\"type\":\"response.completed\",\"sequence_number\":2,\"response\":{\"id\":\"resp-router\",\"object\":\"response\",\"created_at\":0,\"model\":\"vendor/model\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":10,\"output_tokens\":2,\"total_tokens\":12}}}\n\n"))
                    .expect(1).mount(&server).await;
                let url = format!("{}{suffix}?route=custom", server.uri());
                let mut config = base_config("vendor/model");
                config.parallel_tool_calls = Some(false);
                let messages = vec![LlmMessage::text(LlmMessageRole::User, "hello")];
                let response = if registered {
                    registry
                        .create_chat_driver(
                            &ProviderConfig::new(DriverId::OpenRouter)
                                .with_api_key("synthetic-key")
                                .with_base_url(url),
                        )
                        .unwrap()
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
                assert_eq!(
                    response.metadata.response_id.as_deref(),
                    Some("resp-router")
                );
                assert_eq!(
                    (
                        response.metadata.prompt_tokens,
                        response.metadata.completion_tokens,
                        response.metadata.total_tokens
                    ),
                    (Some(10), Some(2), Some(12))
                );
                assert!(response.tool_calls.is_none());
                assert!(response.reasoning.is_empty());
                let requests = server.received_requests().await.unwrap();
                assert_eq!(requests.len(), 1);
                assert_eq!(
                    requests[0].body_json::<Value>().unwrap(),
                    json!({"model":"vendor/model","input":[{"type":"message","role":"user","content":"hello"}],"stream":true,"parallel_tool_calls":false,"reasoning":{"exclude":true}})
                );
            }
        }
        let server = MockServer::builder().start().await;
        let driver = registry
            .create_chat_driver(
                &ProviderConfig::new(DriverId::OpenRouter).with_base_url(server.uri()),
            )
            .unwrap();
        let error = driver
            .chat_completion(
                &ProviderEndpoint::default(),
                vec![],
                &base_config("vendor/model"),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "LLM error: API key is required. Configure the API key in provider settings."
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn discovery_preserves_the_approved_origin_before_resolving_credentials() {
        use everruns_provider::runtime_provider::{ProviderAuth, ProviderAuthRequest};
        struct Probe(&'static str);
        #[async_trait]
        impl ProviderAuth for Probe {
            async fn headers(
                &self,
                request: ProviderAuthRequest<'_>,
            ) -> Result<Vec<(String, String)>> {
                assert_eq!(request.method, "GET");
                assert_eq!(
                    request.url, self.0,
                    "discovery must keep credentials on the approved origin"
                );
                Err(everruns_provider::error::AgentLoopError::config(
                    "probe stops before network",
                ))
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
        for base in [
            None,
            Some("not a URL"),
            Some("https://custom.example/v1"),
            Some("https://openrouter.ai.evil.example/api/v1"),
            Some("https://openrouter.ai@evil.example/api/v1"),
            Some("https://evil.example/openrouter.ai"),
        ] {
            let service = Provider::new("gate", OpenRouterChatDriver::new())
                .auth(Probe("auth must not be accessed"));
            let service = if let Some(base) = base {
                service.base_url(base)
            } else {
                service
            };
            assert!(service.list_models().await.unwrap().is_none(), "{base:?}");
        }
        for (base, expected) in [
            (
                "https://openrouter.ai/api/v1",
                "https://openrouter.ai/api/v1/models",
            ),
            (
                "https://openrouter.ai/api/v1/responses",
                "https://openrouter.ai/api/v1/models",
            ),
            (
                "https://openrouter.ai/api/v1/responses?route=custom",
                "https://openrouter.ai/api/v1/models?route=custom",
            ),
            (
                "https://openrouter.ai/custom",
                "https://openrouter.ai/custom/models",
            ),
        ] {
            let service = Provider::new("probe", OpenRouterChatDriver::new())
                .base_url(base)
                .auth(Probe(expected));
            assert!(
                service
                    .list_models()
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("probe stops before network")
            );
        }
    }

    #[test]
    fn registered_descriptor_declares_oauth_connect_flow() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);

        let oauth = registry
            .descriptor(&DriverId::OpenRouter)
            .unwrap()
            .oauth
            .as_ref()
            .expect("OpenRouter declares an OAuth connect flow");
        assert_eq!(
            oauth.flow,
            everruns_provider::DriverOAuthFlow::OpenRouterPkce
        );
        assert_eq!(oauth.authorize_url, "https://openrouter.ai/auth");
        assert_eq!(oauth.token_url, "https://openrouter.ai/api/v1/auth/keys");
    }
    #[tokio::test]
    async fn catalog_fetch_preserves_query_auth_and_model_mapping() {
        use serde_json::{Value, json};
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::builder().start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/models"))
            .and(query_param("route", "custom"))
            .and(header("authorization", "Bearer synthetic-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
                {"id":"vendor/chat","name":"Chat","created":0,"supported_parameters":["reasoning"]},
                {"id":"minimal","created":9223372036854775807_i64},
                {"id":"vendor/image","architecture":{"output_modalities":["image"]}}
            ]})))
            .expect(1)
            .mount(&server)
            .await;
        let service = provider("catalog", "synthetic-key")
            .base_url(format!("{}/api/v1?route=custom", server.uri()));
        let url = models_url_for_api_url(&service.endpoint().url("responses").unwrap());
        let models = list_openrouter_models(&reqwest::Client::new(), service.endpoint(), &url)
            .await
            .unwrap()
            .unwrap();
        let actual:Vec<Value>=models.into_iter().map(|m| {
            let profile=m.discovered_profile.expect("catalog model has a derived profile");
            json!({"id":m.model_id,"name":m.display_name,"owner":m.owned_by,"created":m.created_at.map(|t|t.to_rfc3339()),"capabilities":m.capabilities,"profile_name":profile.name,"profile_family":profile.family,"reasoning":profile.reasoning})
        }).collect();
        assert_eq!(
            actual,
            vec![
                json!({"id":"vendor/chat","name":"Chat","owner":"vendor","created":"1970-01-01T00:00:00+00:00","capabilities":["chat"],"profile_name":"Chat","profile_family":"vendor/chat","reasoning":true}),
                json!({"id":"minimal","name":null,"owner":"minimal","created":null,"capabilities":["chat"],"profile_name":"minimal","profile_family":"minimal","reasoning":false})
            ]
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}
