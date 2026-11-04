//! Groq Chat Completions and native Files/Batches lifecycle.

use async_trait::async_trait;

use everruns_contracts::OpenAIProtocolChatDriver;
use everruns_contracts::credential_schema::CredentialFormSchema;
use everruns_contracts::driver_helpers::fetch_models;
use everruns_contracts::driver_registry::{
    ChatDriver, DiscoveredModel, DriverDescriptor, DriverId, DriverRegistry, LlmCallConfig,
    LlmResponse, LlmResponseStream, Message,
};
use everruns_contracts::error::Result;
use everruns_contracts::openai_protocol::{models_url_for_api_url, url_host_eq};
use everruns_contracts::{BearerAuth, Provider, ProviderEndpoint};

/// Groq API base URL.
pub const GROQ_DEFAULT_API_URL: &str = "https://api.groq.com/openai/v1";

/// Host serving Groq's API. Discovery only runs against this host.
pub const GROQ_API_HOST: &str = "api.groq.com";

/// Ready-to-use Groq provider assembly.
///
/// # Example
///
/// ```
/// use everruns_drivers::groq::provider;
///
/// let service = provider("groq", "groq-key");
/// assert_eq!(
///     service.endpoint().url("chat/completions").unwrap(),
///     "https://api.groq.com/openai/v1/chat/completions",
/// );
/// ```
pub fn provider(
    id: impl Into<everruns_contracts::ProviderKey>,
    api_key: impl Into<String>,
) -> Provider {
    Provider::new(id, GroqChatDriver::new())
        .base_url(GROQ_DEFAULT_API_URL)
        .auth(BearerAuth::new(api_key))
}

/// Groq AI chat driver (OpenAI-compatible Chat Completions).
#[derive(Clone)]
pub struct GroqChatDriver {
    pub(crate) inner: OpenAIProtocolChatDriver,
}

impl GroqChatDriver {
    /// Create the Groq Chat Completions wire driver.
    pub fn new() -> Self {
        Self {
            inner: OpenAIProtocolChatDriver::new(),
        }
    }
}

impl Default for GroqChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChatDriver for GroqChatDriver {
    #[cfg(not(target_arch = "wasm32"))]
    fn batch_driver(&self) -> Option<&dyn everruns_contracts::batch::BatchDriver> {
        Some(self)
    }

    async fn chat_completion_stream(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner
            .chat_completion_stream(endpoint, messages, config)
            .await
    }

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        self.inner.supports_parallel_tool_calls(model)
    }

    fn supports_native_non_streaming(&self) -> bool {
        self.inner.supports_native_non_streaming()
    }

    async fn chat_completion_non_streaming(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        self.inner
            .chat_completion_non_streaming(endpoint, messages, config)
            .await
    }

    async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        let Some(api_url) = endpoint.url("chat/completions") else {
            return Ok(None);
        };
        // Discovery only runs against Groq's own host. A custom proxy URL may
        // resolve to private infrastructure at request time (mirrors the
        // OpenRouter/Fireworks/Vercel host gating).
        if !url_host_eq(&api_url, GROQ_API_HOST) {
            return Ok(None);
        }
        let models_url = models_url_for_api_url(&api_url);
        let resolved = endpoint.resolve("GET", models_url, &[]).await?;
        let mut request = self.inner.client().get(&resolved.url);
        for (name, value) in resolved.headers {
            request = request.header(name, value);
        }
        fetch_models::<GroqModelsResponse, _>(
            request,
            "Failed to fetch Groq models",
            "Failed to parse Groq models",
            &[],
            |m| {
                m.data
                    .into_iter()
                    .filter(|m| m.active)
                    .map(|m| DiscoveredModel {
                        model_id: m.id,
                        capabilities: vec!["chat".into()],
                        created_at: None,
                        display_name: None,
                        owned_by: m.owned_by,
                        discovered_profile: None,
                    })
                    .collect()
            },
        )
        .await
    }
}

impl std::fmt::Debug for GroqChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroqChatDriver")
            .field("api", &"Groq AI (Chat Completions)")
            .finish()
    }
}

#[derive(serde::Deserialize)]
struct GroqModelsResponse {
    data: Vec<GroqModelInfo>,
}
#[derive(serde::Deserialize)]
struct GroqModelInfo {
    id: String,
    #[serde(default)]
    owned_by: Option<String>,
    #[serde(default = "active")]
    active: bool,
}
fn active() -> bool {
    true
}
/// This driver's descriptor: identity, services, and the credential schema
/// that declares its own environment variables.
pub fn descriptor() -> DriverDescriptor {
    DriverDescriptor {
        display_name: "Groq AI".into(),
        // The variable Groq's own SDKs read.
        credential_schema: CredentialFormSchema::api_key(
            "GROQ_API_KEY",
            "Create an API key in [Groq Console](https://console.groq.com/keys). \
             Available models are discovered automatically on sync.",
        ),
        base_url_env: Some("GROQ_BASE_URL".into()),
        ..DriverDescriptor::chat_only(DriverId::Groq, |config| {
            Provider::new(config.provider.clone(), GroqChatDriver::new())
                .base_url(
                    config
                        .base_url
                        .as_deref()
                        .map(str::trim)
                        .filter(|url| !url.is_empty())
                        .unwrap_or(GROQ_DEFAULT_API_URL),
                )
                .auth(BearerAuth::new(config.api_key.clone().unwrap_or_default()))
                .into_boxed_driver()
        })
    }
}

/// Register the Groq driver with the driver registry.
///
/// # Example
///
/// ```
/// use everruns_contracts::DriverRegistry;
/// use everruns_drivers::groq::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// assert!(registry.has_driver(&everruns_contracts::DriverId::Groq));
/// ```
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_descriptor(descriptor());
}

/// Build a provider from this driver's declared environment variables.
///
/// Standalone/CLI/dev only: server paths resolve credentials from storage and
/// must never read the environment.
pub fn from_env(
    id: impl Into<everruns_contracts::ProviderKey>,
) -> std::result::Result<
    everruns_contracts::Provider,
    everruns_contracts::credential_provider::EnvCredentialError,
> {
    everruns_contracts::credential_provider::provider_from_env(&descriptor(), id)
}
