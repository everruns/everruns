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
        list_openrouter_models(self.inner.client(), endpoint, &models_url).await
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

    #[test]
    fn test_openrouter_driver_defaults_to_responses_api() {
        let driver = OpenRouterChatDriver::new();
        let service = provider("openrouter", "test-key");
        assert!(format!("{:?}", driver).contains("OpenRouterChatDriver"));
        assert_eq!(
            service.endpoint().url("responses").as_deref(),
            Some("https://openrouter.ai/api/v1/responses")
        );
    }

    #[test]
    fn test_openrouter_driver_with_base_url_marks_custom_url() {
        let service =
            provider("openrouter", "test-key").base_url("https://openrouter.ai/api/v1/responses");
        assert_eq!(
            service.endpoint().url("responses").as_deref(),
            Some("https://openrouter.ai/api/v1/responses")
        );
    }

    #[tokio::test]
    async fn test_openrouter_custom_non_openrouter_host_skips_model_listing() {
        let service = Provider::new("custom", OpenRouterChatDriver::new())
            .base_url("https://custom.api.example/v1");
        let driver = OpenRouterChatDriver::new();

        let discovered = driver
            .list_models(service.endpoint())
            .await
            .expect("custom non-OpenRouter discovery should be skipped");

        assert!(discovered.is_none());
    }

    #[test]
    fn recognizes_openrouter_host() {
        // OpenRouter is reached via the Open Responses driver with a custom base
        // URL; discovery must run so capability profiles (reasoning) are derived.
        assert!(is_openrouter_api_url(
            "https://openrouter.ai/api/v1/responses"
        ));
        assert!(!is_openrouter_api_url("https://example.com/v1/responses"));
    }

    #[test]
    fn openrouter_models_url_is_derived_from_responses_url() {
        assert_eq!(
            models_url_for_api_url("https://openrouter.ai/api/v1/responses"),
            "https://openrouter.ai/api/v1/models"
        );
    }

    #[test]
    fn test_register_driver() {
        let mut registry = DriverRegistry::new();
        assert!(!registry.has_driver(&DriverId::OpenRouter));

        register_driver(&mut registry);

        assert!(registry.has_driver(&DriverId::OpenRouter));

        let openrouter_config = ProviderConfig::new(DriverId::OpenRouter).with_api_key("test-key");
        let openrouter_driver = registry.create_chat_driver(&openrouter_config);
        assert!(openrouter_driver.is_ok());
    }

    #[test]
    fn registered_descriptor_declares_chat_service_and_credentials() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);

        let descriptor = registry.descriptor(&DriverId::OpenRouter).unwrap();
        assert_eq!(descriptor.services, vec![ServiceKind::Chat]);
        assert_eq!(descriptor.credential_schema.fields[0].name, "api_key");
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
}
