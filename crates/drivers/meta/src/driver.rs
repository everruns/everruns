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
    use everruns_provider::driver_registry::{ProviderConfig, ServiceKind};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn defaults_to_meta_responses_api() {
        let provider = provider("meta", "test-key");
        let driver = MetaChatDriver::new();
        assert_eq!(
            provider.endpoint().url("responses").unwrap(),
            META_DEFAULT_API_URL
        );
        assert!(driver.supports_stateful_responses());
        assert!(driver.supports_parallel_tool_calls("muse-spark-1.3"));
    }

    #[test]
    fn base_url_is_normalized_to_responses() {
        let provider = provider("meta", "test-key");
        assert_eq!(
            provider.endpoint().url("responses").unwrap(),
            META_DEFAULT_API_URL
        );
    }

    #[tokio::test]
    async fn custom_non_meta_host_skips_discovery() {
        let provider =
            Provider::new("proxy", MetaChatDriver::new()).base_url("https://proxy.example.com/v1");
        let driver = MetaChatDriver::new();
        assert!(
            driver
                .list_models(provider.endpoint())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn discovery_uses_bearer_auth_and_maps_models() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer meta-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [{
                    "id": "muse-spark-1.3",
                    "object": "model",
                    "created": 1_785_888_000_i64,
                    "owned_by": "meta"
                }]
            })))
            .mount(&server)
            .await;

        let provider = Provider::new("meta-test", MetaChatDriver::new())
            .base_url(format!("{}/v1", server.uri()))
            .auth(BearerAuth::new("meta-secret"));
        let models = list_meta_models(
            &reqwest::Client::new(),
            provider.endpoint(),
            &format!("{}/v1/models", server.uri()),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "muse-spark-1.3");
        assert_eq!(models[0].owned_by.as_deref(), Some("meta"));
        assert!(models[0].created_at.is_some());
    }

    #[test]
    fn registration_declares_chat_and_api_key() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);

        let descriptor = registry.descriptor(&DriverId::Meta).unwrap();
        assert_eq!(descriptor.display_name, "Meta Model API");
        assert_eq!(descriptor.services, vec![ServiceKind::Chat]);
        assert_eq!(descriptor.credential_schema.fields[0].name, "api_key");
        assert!(
            registry
                .create_chat_driver(&ProviderConfig::new(DriverId::Meta).with_api_key("test-key"))
                .is_ok()
        );
        // Missing credentials still construct a fail-closed driver so hosts
        // can inspect and repair configuration before any network operation.
        assert!(
            registry
                .create_chat_driver(&ProviderConfig::new(DriverId::Meta))
                .is_ok()
        );
    }
}
