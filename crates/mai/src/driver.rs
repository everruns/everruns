// Microsoft MAI Chat Driver
//
// Microsoft MAI models (e.g. MAI-Code-1-Flash) are served via Azure AI Foundry
// behind an OpenAI-compatible Chat Completions API. This driver wraps the core
// `OpenAIProtocolChatDriver` and supplies a pluggable, OAuth-capable auth layer
// (see `auth.rs`) through the driver's `AuthHeaderProvider` hook.
//
// Because authentication is handled by the auth provider, the underlying
// protocol driver's own `api_key` field is unused (constructed empty).

use async_trait::async_trait;

use everruns_core::OpenAIProtocolChatDriver;
use everruns_core::credential_schema::{CredentialFormSchema, FieldType, FormField};
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::llm_driver_registry::{
    BoxedChatDriver, ChatDriver, DiscoveredModel, DriverConfig, DriverDescriptor, DriverId,
    DriverRegistry, LlmCallConfig, LlmMessage, LlmResponseStream,
};

use crate::auth::MaiAuth;

/// Microsoft MAI chat driver (Azure AI Foundry, OpenAI-compatible).
///
/// Construct directly for programmatic use, or let [`register_driver`] build
/// instances from a [`DriverConfig`].
///
/// # Example
///
/// ```
/// use everruns_mai::{MaiAuth, MaiChatDriver};
///
/// let driver = MaiChatDriver::new(
///     MaiAuth::ApiKey("foundry-key".into()),
///     "https://my-resource.services.ai.azure.com",
/// );
/// assert_eq!(
///     driver.api_url(),
///     "https://my-resource.services.ai.azure.com/openai/v1/chat/completions",
/// );
/// ```
pub struct MaiChatDriver {
    /// Ready driver, or a captured configuration error surfaced at call time.
    state: DriverState,
}

enum DriverState {
    Ready {
        inner: OpenAIProtocolChatDriver,
        api_url: String,
    },
    Misconfigured(String),
}

impl MaiChatDriver {
    /// Create a driver from an explicit auth strategy and Azure AI Foundry
    /// endpoint. The endpoint is normalized to the chat-completions URL.
    pub fn new(auth: MaiAuth, endpoint: impl Into<String>) -> Self {
        let api_url = normalize_mai_url(&endpoint.into());
        let inner = OpenAIProtocolChatDriver::with_base_url("", &api_url)
            .with_auth_provider(auth.into_provider());
        Self {
            state: DriverState::Ready { inner, api_url },
        }
    }

    /// Build a driver from a registry [`DriverConfig`], resolving auth from the
    /// api key / OAuth metadata and the endpoint from `base_url`.
    ///
    /// Configuration errors (no endpoint, no auth) are captured and surfaced
    /// when the driver is first called, so the infallible factory contract is
    /// preserved while still producing a clear error.
    fn from_driver_config(config: &DriverConfig) -> Self {
        let Some(endpoint) = config.base_url.as_deref().filter(|u| !u.is_empty()) else {
            return Self {
                state: DriverState::Misconfigured(
                    "Microsoft MAI provider requires a base URL: set it to your Azure AI \
                     Foundry resource endpoint (e.g. https://<resource>.services.ai.azure.com)."
                        .to_string(),
                ),
            };
        };

        match MaiAuth::from_driver_config(config) {
            Ok(auth) => Self::new(auth, endpoint),
            Err(err) => Self {
                state: DriverState::Misconfigured(err.to_string()),
            },
        }
    }

    /// The resolved chat-completions API URL, or `None` when misconfigured.
    pub fn api_url(&self) -> &str {
        match &self.state {
            DriverState::Ready { api_url, .. } => api_url,
            DriverState::Misconfigured(_) => "",
        }
    }

    fn ready(&self) -> Result<&OpenAIProtocolChatDriver> {
        match &self.state {
            DriverState::Ready { inner, .. } => Ok(inner),
            DriverState::Misconfigured(err) => Err(AgentLoopError::llm(err.clone())),
        }
    }
}

#[async_trait]
impl ChatDriver for MaiChatDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.ready()?.chat_completion_stream(messages, config).await
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        // MAI deployments on Azure AI Foundry are resource-specific and do not
        // expose a reliable public `/models` catalog for discovery. Model ids
        // are well-known and carried by the built-in model profile registry, so
        // discovery is intentionally skipped.
        Ok(None)
    }
}

impl std::fmt::Debug for MaiChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaiChatDriver")
            .field("api_url", &self.api_url())
            .field("api", &"Azure AI Foundry (Chat Completions)")
            .finish()
    }
}

/// Normalize an Azure AI Foundry endpoint to its chat-completions URL.
///
/// Accepts a bare resource host, a `/openai/v1` or `/models` base, or a full
/// `/chat/completions` URL, and always returns a `/chat/completions` endpoint.
/// A bare host gets Foundry's v1 OpenAI-compatible path appended.
fn normalize_mai_url(endpoint: &str) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1")
        || trimmed.ends_with("/openai/v1")
        || trimmed.ends_with("/models")
    {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/openai/v1/chat/completions")
    }
}

/// Credential schema for the MAI driver: an Azure AI Foundry API key plus an
/// optional resource endpoint. Entra ID OAuth is configured through provider
/// metadata rather than the credential form.
fn mai_credential_schema() -> CredentialFormSchema {
    CredentialFormSchema {
        fields: vec![
            FormField {
                name: "api_key".to_string(),
                label: "Azure AI Foundry API Key".to_string(),
                field_type: FieldType::Password,
                required: false,
                placeholder: None,
                help_text: Some(
                    "Leave blank to authenticate with Microsoft Entra ID (OAuth) credentials \
                     configured in provider metadata."
                        .to_string(),
                ),
            },
            FormField {
                name: "base_url".to_string(),
                label: "Resource Endpoint".to_string(),
                field_type: FieldType::Url,
                required: true,
                placeholder: Some("https://<resource>.services.ai.azure.com".to_string()),
                help_text: Some("Your Azure AI Foundry resource endpoint.".to_string()),
            },
        ],
        instructions_markdown:
            "Configure a Microsoft MAI deployment on [Azure AI Foundry](https://ai.azure.com). \
             Authenticate with the resource API key, or with Microsoft Entra ID OAuth \
             (client-credentials) by supplying `tenant_id`, `client_id`, and `client_secret` \
             in the provider metadata."
                .to_string(),
    }
}

/// Register the Microsoft MAI driver with the driver registry.
///
/// Registers [`DriverId::Mai`], a chat-only driver backed by Azure AI Foundry's
/// OpenAI-compatible Chat Completions API.
///
/// # Example
///
/// ```
/// use everruns_core::DriverRegistry;
/// use everruns_mai::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// assert!(registry.has_driver(&everruns_core::DriverId::Mai));
/// ```
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_descriptor(DriverDescriptor {
        credential_schema: mai_credential_schema(),
        ..DriverDescriptor::chat_only(DriverId::Mai, |config| {
            Box::new(MaiChatDriver::from_driver_config(config)) as BoxedChatDriver
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::llm_driver_registry::{ProviderConfig, ProviderMetadata, ServiceKind};

    #[test]
    fn normalizes_bare_foundry_host() {
        assert_eq!(
            normalize_mai_url("https://res.services.ai.azure.com"),
            "https://res.services.ai.azure.com/openai/v1/chat/completions"
        );
    }

    #[test]
    fn normalizes_trailing_slash_and_v1() {
        assert_eq!(
            normalize_mai_url("https://res.services.ai.azure.com/openai/v1/"),
            "https://res.services.ai.azure.com/openai/v1/chat/completions"
        );
    }

    #[test]
    fn preserves_full_chat_completions_url() {
        let url = "https://res.services.ai.azure.com/openai/v1/chat/completions";
        assert_eq!(normalize_mai_url(url), url);
    }

    #[test]
    fn ready_driver_exposes_normalized_url() {
        let driver = MaiChatDriver::new(
            MaiAuth::ApiKey("k".into()),
            "https://res.services.ai.azure.com",
        );
        assert_eq!(
            driver.api_url(),
            "https://res.services.ai.azure.com/openai/v1/chat/completions"
        );
    }

    #[tokio::test]
    async fn misconfigured_without_base_url_errors_at_call_time() {
        let config = DriverConfig {
            provider_type: DriverId::Mai,
            api_key: Some("k".into()),
            base_url: None,
            metadata: ProviderMetadata::default(),
        };
        let driver = MaiChatDriver::from_driver_config(&config);
        let err = match driver.chat_completion_stream(vec![], &dummy_config()).await {
            Ok(_) => panic!("expected configuration error"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("requires a base URL"));
    }

    #[tokio::test]
    async fn misconfigured_without_auth_errors_at_call_time() {
        let config = DriverConfig {
            provider_type: DriverId::Mai,
            api_key: None,
            base_url: Some("https://res.services.ai.azure.com".into()),
            metadata: ProviderMetadata::default(),
        };
        let driver = MaiChatDriver::from_driver_config(&config);
        let err = match driver.chat_completion_stream(vec![], &dummy_config()).await {
            Ok(_) => panic!("expected configuration error"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("not authenticated"));
    }

    #[test]
    fn register_driver_registers_mai() {
        let mut registry = DriverRegistry::new();
        assert!(!registry.has_driver(&DriverId::Mai));
        register_driver(&mut registry);
        assert!(registry.has_driver(&DriverId::Mai));

        let descriptor = registry.descriptor(&DriverId::Mai).unwrap();
        assert_eq!(descriptor.services, vec![ServiceKind::Chat]);
        assert_eq!(descriptor.display_name, "Microsoft MAI");

        // A provider with an api key and endpoint builds a usable chat driver
        // (Mai is exempt from the registry's mandatory-api-key check, so OAuth
        // works too).
        let config = ProviderConfig::new(DriverId::Mai)
            .with_api_key("k")
            .with_base_url("https://res.services.ai.azure.com");
        assert!(registry.create_chat_driver(&config).is_ok());
    }

    #[test]
    fn oauth_provider_builds_without_api_key() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);

        let config = ProviderConfig::new(DriverId::Mai)
            .with_base_url("https://res.services.ai.azure.com")
            .with_metadata(ProviderMetadata {
                extra: Some(serde_json::json!({
                    "tenant_id": "t",
                    "client_id": "c",
                    "client_secret": "s",
                })),
                ..Default::default()
            });
        // No api_key, but OAuth metadata present — must construct successfully.
        assert!(registry.create_chat_driver(&config).is_ok());
    }

    fn dummy_config() -> LlmCallConfig {
        LlmCallConfig {
            model: "mai-code-1-flash".into(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            metadata: Default::default(),
            previous_response_id: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
        }
    }
}
