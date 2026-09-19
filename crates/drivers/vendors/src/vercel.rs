// Vercel AI Gateway Chat Driver
//
// Vercel AI Gateway fronts many upstream providers and serves the Open
// Responses specification (https://openresponses.org) at `/v1/responses`, the
// same spec `OpenResponsesProtocolChatDriver` implements. So this driver adds
// only identity, auth, and model discovery; no wire work.
//
// Chat Completions is also available on the same base URL, but Open Responses
// is the richer surface and the one Everruns prefers for new drivers, so that
// is what this driver speaks.
//
// Model ids are namespaced by upstream provider (`anthropic/claude-opus-5`,
// `openai/gpt-6-astra`) and are passed through unchanged.

use async_trait::async_trait;
use chrono::TimeZone;
use serde::Deserialize;

use everruns_provider::OpenResponsesProtocolChatDriver;
use everruns_provider::credential_schema::CredentialFormSchema;
use everruns_provider::driver_helpers::fetch_models;
use everruns_provider::driver_registry::{
    ChatDriver, DiscoveredModel, DriverDescriptor, DriverId, DriverRegistry, LlmCallConfig,
    LlmResponse, LlmResponseStream, Message,
};
use everruns_provider::error::Result;
use everruns_provider::openai_protocol::{models_url_for_api_url, url_host_eq};
use everruns_provider::{BearerAuth, Provider, ProviderEndpoint};

/// Vercel AI Gateway base URL.
pub const VERCEL_AI_GATEWAY_DEFAULT_API_URL: &str = "https://ai-gateway.vercel.sh/v1";

/// Host serving the Vercel AI Gateway API.
pub const VERCEL_AI_GATEWAY_HOST: &str = "ai-gateway.vercel.sh";

/// Ready-to-use Vercel AI Gateway provider assembly.
///
/// `api_key` is an AI Gateway API key or a Vercel OIDC token; the gateway
/// accepts either in the same bearer header.
pub fn provider(
    id: impl Into<everruns_provider::ProviderKey>,
    api_key: impl Into<String>,
) -> Provider {
    Provider::new(id, VercelChatDriver::new())
        .base_url(VERCEL_AI_GATEWAY_DEFAULT_API_URL)
        .auth(BearerAuth::new(api_key))
}

/// Vercel AI Gateway driver using its Open Responses API.
#[derive(Clone)]
pub struct VercelChatDriver {
    inner: OpenResponsesProtocolChatDriver,
}

impl VercelChatDriver {
    /// Create the Vercel Open Responses wire driver.
    ///
    /// No protocol extensions are enabled: the gateway documents neither
    /// stateful continuation nor OpenAI's explicit prompt-cache controls, and
    /// sending fields an endpoint does not implement is how a working request
    /// starts failing after a vendor tightens validation.
    pub fn new() -> Self {
        Self {
            inner: OpenResponsesProtocolChatDriver::new(),
        }
    }
}

#[async_trait]
impl ChatDriver for VercelChatDriver {
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

    fn supports_stateful_responses(&self) -> bool {
        self.inner.supports_stateful_responses()
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
        let Some(api_url) = endpoint.url("responses") else {
            return Ok(None);
        };
        // Discovery only runs against Vercel's own host. A custom proxy URL may
        // resolve to private infrastructure at request time (mirrors the
        // OpenRouter/Meta/Fireworks host gating).
        if !url_host_eq(&api_url, VERCEL_AI_GATEWAY_HOST) {
            return Ok(None);
        }

        let models_url = models_url_for_api_url(&api_url);
        list_vercel_models(&self.inner.client(), endpoint, &models_url).await
    }
}

impl std::fmt::Debug for VercelChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VercelChatDriver")
            .field("api", &"Vercel AI Gateway (Open Responses)")
            .finish()
    }
}

impl Default for VercelChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Model discovery
// ============================================================================

/// Fetch the gateway's `/models` catalog. Vercel serves the OpenAI-standard
/// shape, so there is no richer metadata to parse: the profile registry fills
/// capabilities and limits by model id.
async fn list_vercel_models(
    client: &reqwest::Client,
    endpoint: &ProviderEndpoint,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let resolved = endpoint.resolve("GET", models_url, &[]).await?;
    let mut request = client.get(&resolved.url);
    for (name, value) in resolved.headers {
        request = request.header(name, value);
    }
    fetch_models::<VercelModelsResponse, _>(
        request,
        "Failed to fetch Vercel AI Gateway models",
        "Failed to parse Vercel AI Gateway models response",
        &[],
        |models| {
            models
                .data
                .into_iter()
                .map(|m| {
                    let created_at = m
                        .created
                        .and_then(|ts| chrono::Utc.timestamp_opt(ts, 0).single());
                    DiscoveredModel {
                        capabilities: vec!["chat".to_string()],
                        created_at,
                        display_name: None,
                        owned_by: m.owned_by,
                        discovered_profile: None,
                        model_id: m.id,
                    }
                })
                .collect()
        },
    )
    .await
}

/// Bare OpenAI-compatible `/models` list response.
#[derive(Debug, Deserialize)]
struct VercelModelsResponse {
    data: Vec<VercelModelInfo>,
}

/// One entry from the gateway's `/models` list.
#[derive(Debug, Deserialize)]
struct VercelModelInfo {
    id: String,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    owned_by: Option<String>,
}

/// This driver's descriptor: identity, services, and the credential schema
/// that declares its own environment variables.
pub fn descriptor() -> DriverDescriptor {
    DriverDescriptor {
        display_name: "Vercel AI Gateway".into(),
        // Matches the variable Vercel's own SDK and docs read.
        credential_schema: CredentialFormSchema::api_key(
            "AI_GATEWAY_API_KEY",
            "Create an AI Gateway API key in the [Vercel dashboard](https://vercel.com/d?to=%2F%5Bteam%5D%2F%7E%2Fai%2Fapi-keys). \
             A Vercel OIDC token works in the same field. Models are named \
             `provider/model` (`anthropic/claude-opus-5`, `openai/gpt-6-astra`).",
        ),
        base_url_env: None,
        ..DriverDescriptor::chat_only(DriverId::Vercel, |config| {
            let api_key = config.api_key.as_deref().unwrap_or("");
            Provider::new(config.provider.clone(), VercelChatDriver::new())
                .base_url(
                    config
                        .base_url
                        .as_deref()
                        .map(str::trim)
                        .filter(|url| !url.is_empty())
                        .unwrap_or(VERCEL_AI_GATEWAY_DEFAULT_API_URL),
                )
                .auth(BearerAuth::new(api_key))
                .into_boxed_driver()
        })
    }
}

/// Register the Vercel AI Gateway driver with the driver registry.
///
/// # Example
///
/// ```
/// use everruns_provider::DriverRegistry;
/// use everruns_drivers::vercel::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// assert!(registry.has_driver(&everruns_provider::DriverId::Vercel));
/// ```
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_descriptor(descriptor());
}

/// Build a provider from this driver's declared environment variables.
///
/// Standalone/CLI/dev only: server paths resolve credentials from storage and
/// must never read the environment.
pub fn from_env(
    id: impl Into<everruns_provider::ProviderKey>,
) -> std::result::Result<
    everruns_provider::Provider,
    everruns_provider::credential_provider::EnvCredentialError,
> {
    everruns_provider::credential_provider::provider_from_env(&descriptor(), id)
}
