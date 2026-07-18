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
    BoxedChatDriver, ChatDriver, DiscoveredModel, DriverDescriptor, DriverId, DriverRegistry,
    LlmCallConfig, LlmMessage, LlmResponseStream,
};
use everruns_provider::error::Result;
use everruns_provider::openai_protocol::{
    apply_models_api_auth, models_url_for_api_url, normalize_api_url, url_host_eq,
};

use crate::request_ext::OpenRouterRequestExtension;
use crate::types::OpenRouterModelsResponse;

const OPENROUTER_RESPONSES_URL: &str = "https://openrouter.ai/api/v1/responses";

// ============================================================================
// OpenRouter Chat Driver (OpenAI-compatible Responses API)
// ============================================================================

/// OpenRouter driver using its OpenAI-compatible Responses API.
#[derive(Clone)]
pub struct OpenRouterChatDriver {
    inner: OpenResponsesProtocolChatDriver,
    /// Whether constructed with an explicit base URL override via [`with_base_url`].
    uses_custom_url: bool,
}

impl OpenRouterChatDriver {
    /// Create a new OpenRouter driver with the default Responses API endpoint.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            inner: OpenResponsesProtocolChatDriver::with_base_url(
                api_key,
                OPENROUTER_RESPONSES_URL,
            )
            .with_provider_type(DriverId::OpenRouter)
            .with_request_extension(Arc::new(OpenRouterRequestExtension)),
            uses_custom_url: false,
        }
    }

    /// Create a new OpenRouter driver with an explicit API URL override.
    pub fn with_base_url(api_key: impl Into<String>, api_url: impl Into<String>) -> Self {
        let api_url = normalize_api_url(&api_url.into(), "/responses");
        Self {
            inner: OpenResponsesProtocolChatDriver::with_base_url(api_key, api_url)
                .with_provider_type(DriverId::OpenRouter)
                .with_request_extension(Arc::new(OpenRouterRequestExtension)),
            uses_custom_url: true,
        }
    }

    /// Get the API URL.
    pub fn api_url(&self) -> &str {
        self.inner.api_url()
    }

    /// Get the provider type used for model profile lookup.
    pub fn provider_type(&self) -> &DriverId {
        self.inner.provider_type()
    }

    /// Check if using a custom base URL.
    pub fn uses_custom_url(&self) -> bool {
        self.uses_custom_url
    }
}

#[async_trait]
impl ChatDriver for OpenRouterChatDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner.chat_completion_stream(messages, config).await
    }

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        self.inner.supports_parallel_tool_calls(model)
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        // OpenRouter discovery is only safe against OpenRouter's own host.
        // Custom proxy URLs may resolve to private infrastructure at request time.
        if self.uses_custom_url && !is_openrouter_api_url(self.api_url()) {
            return Ok(None);
        }

        let models_url = models_url_for_api_url(self.api_url());
        list_openrouter_models(self.inner.client(), self.inner.api_key(), &models_url).await
    }
}

impl std::fmt::Debug for OpenRouterChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenRouterChatDriver")
            .field("api_url", &self.api_url())
            .field("api", &"OpenRouter Responses")
            .field("api_key", &"[REDACTED]")
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
    api_key: &str,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    fetch_models::<OpenRouterModelsResponse, _>(
        apply_models_api_auth(client.get(models_url), models_url, api_key),
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
        credential_schema: CredentialFormSchema::api_key(
            "Create an API key at [openrouter.ai/settings/keys](https://openrouter.ai/settings/keys), \
             or use \"Connect with OpenRouter\" to authorize one without leaving the app.",
        ),
        // OpenRouter supports a one-click PKCE flow that hands back a
        // user-controlled API key, so an admin can connect without minting and
        // pasting a key manually. The key is stored like any other credential.
        oauth: Some(everruns_provider::DriverOAuthConfig::openrouter()),
        ..DriverDescriptor::chat_only(DriverId::OpenRouter, |config| {
            let api_key = config.api_key.as_deref().unwrap_or("");
            let driver = match config.base_url.as_deref() {
                Some(url) => OpenRouterChatDriver::with_base_url(api_key, url),
                None => OpenRouterChatDriver::new(api_key),
            };
            Box::new(driver) as BoxedChatDriver
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::driver_registry::{DriverId, ProviderConfig, ServiceKind};

    #[test]
    fn test_openrouter_driver_defaults_to_responses_api() {
        let driver = OpenRouterChatDriver::new("test-key");
        assert!(format!("{:?}", driver).contains("OpenRouterChatDriver"));
        assert_eq!(driver.api_url(), "https://openrouter.ai/api/v1/responses");
        assert_eq!(driver.provider_type(), &DriverId::OpenRouter);
    }

    #[test]
    fn test_openrouter_driver_with_base_url_marks_custom_url() {
        let driver = OpenRouterChatDriver::with_base_url(
            "test-key",
            "https://openrouter.ai/api/v1/responses",
        );
        assert_eq!(driver.api_url(), "https://openrouter.ai/api/v1/responses");
        assert!(driver.uses_custom_url());
    }

    #[tokio::test]
    async fn test_openrouter_custom_non_openrouter_host_skips_model_listing() {
        let driver = OpenRouterChatDriver::with_base_url(
            "test-key",
            "https://custom.api.example/v1/responses",
        );

        let discovered = driver
            .list_models()
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
