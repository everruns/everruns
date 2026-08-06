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
    BoxedChatDriver, ChatDriver, DiscoveredModel, DriverDescriptor, DriverId, DriverRegistry,
    LlmCallConfig, LlmMessage, LlmResponseStream,
};
use everruns_provider::error::Result;
use everruns_provider::openai_protocol::{
    apply_models_api_auth, models_url_for_api_url, normalize_api_url, url_host_eq,
};

/// Meta Model API Responses endpoint.
pub const META_DEFAULT_API_URL: &str = "https://api.meta.ai/v1/responses";
const META_API_HOST: &str = "api.meta.ai";

/// Meta Model API driver using the OpenAI-compatible Responses API.
#[derive(Clone)]
pub struct MetaChatDriver {
    inner: OpenResponsesProtocolChatDriver,
    uses_custom_url: bool,
}

impl MetaChatDriver {
    /// Create a driver targeting Meta's hosted Model API.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            inner: OpenResponsesProtocolChatDriver::with_base_url(api_key, META_DEFAULT_API_URL)
                .with_provider_type(DriverId::Meta),
            uses_custom_url: false,
        }
    }

    /// Create a driver with an explicit Responses API endpoint override.
    pub fn with_base_url(api_key: impl Into<String>, api_url: impl Into<String>) -> Self {
        let api_url = normalize_api_url(&api_url.into(), "/responses");
        Self {
            inner: OpenResponsesProtocolChatDriver::with_base_url(api_key, api_url)
                .with_provider_type(DriverId::Meta),
            uses_custom_url: true,
        }
    }

    /// The resolved Responses API URL.
    pub fn api_url(&self) -> &str {
        self.inner.api_url()
    }

    /// The provider id used for model-profile lookup.
    pub fn provider_type(&self) -> &DriverId {
        self.inner.provider_type()
    }
}

#[async_trait]
impl ChatDriver for MetaChatDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner.chat_completion_stream(messages, config).await
    }

    fn supports_stateful_responses(&self) -> bool {
        self.inner.supports_stateful_responses()
    }

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        self.inner.supports_parallel_tool_calls(model)
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        if self.uses_custom_url && !url_host_eq(self.api_url(), META_API_HOST) {
            return Ok(None);
        }

        let models_url = models_url_for_api_url(self.api_url());
        list_meta_models(self.inner.client(), self.inner.api_key(), &models_url).await
    }
}

impl std::fmt::Debug for MetaChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetaChatDriver")
            .field("api_url", &self.api_url())
            .field("api", &"Meta Model API Responses")
            .field("api_key", &"[REDACTED]")
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
    api_key: &str,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    fetch_models::<MetaModelsResponse, _>(
        apply_models_api_auth(client.get(models_url), models_url, api_key),
        "Failed to fetch Meta models",
        "Failed to parse Meta models response",
        &[],
        |response| {
            response
                .data
                .into_iter()
                .map(|model| DiscoveredModel {
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
        credential_schema: CredentialFormSchema::api_key(
            "Create an API key in the [Meta Model API dashboard](https://dev.meta.ai/).",
        ),
        ..DriverDescriptor::chat_only(DriverId::Meta, |config| {
            let api_key = config.api_key.as_deref().unwrap_or("");
            let driver = match config.base_url.as_deref() {
                Some(url) => MetaChatDriver::with_base_url(api_key, url),
                None => MetaChatDriver::new(api_key),
            };
            Box::new(driver) as BoxedChatDriver
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::driver_registry::{ProviderConfig, ServiceKind};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn defaults_to_meta_responses_api() {
        let driver = MetaChatDriver::new("test-key");
        assert_eq!(driver.api_url(), META_DEFAULT_API_URL);
        assert_eq!(driver.provider_type(), &DriverId::Meta);
        assert!(driver.supports_stateful_responses());
        assert!(driver.supports_parallel_tool_calls("muse-spark-1.2"));
    }

    #[test]
    fn base_url_is_normalized_to_responses() {
        let driver = MetaChatDriver::with_base_url("test-key", "https://api.meta.ai/v1");
        assert_eq!(driver.api_url(), META_DEFAULT_API_URL);
    }

    #[tokio::test]
    async fn custom_non_meta_host_skips_discovery() {
        let driver = MetaChatDriver::with_base_url("test-key", "https://proxy.example.com/v1");
        assert!(driver.list_models().await.unwrap().is_none());
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
                    "id": "muse-spark-1.2",
                    "object": "model",
                    "created": 1_785_888_000_i64,
                    "owned_by": "meta"
                }]
            })))
            .mount(&server)
            .await;

        let models = list_meta_models(
            &reqwest::Client::new(),
            "meta-secret",
            &format!("{}/v1/models", server.uri()),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "muse-spark-1.2");
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
        assert!(
            registry
                .create_chat_driver(&ProviderConfig::new(DriverId::Meta))
                .is_err()
        );
    }
}
