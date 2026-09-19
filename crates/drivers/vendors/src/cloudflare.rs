// Cloudflare AI Gateway Chat Driver
//
// Cloudflare AI Gateway fronts many upstream providers behind one
// OpenAI-compatible Chat Completions endpoint, so this driver wraps the shared
// `OpenAIProtocolChatDriver` and adds only Cloudflare's URL shape and auth.
//
// Two things are unlike a direct vendor:
//
//   1. The endpoint embeds the account and gateway ids
//      (`/v1/{account_id}/{gateway_id}/compat`), so there is no vendor-wide
//      default base URL. The ids are credential-form fields and the base URL
//      is derived from them, rather than asking an operator to hand-assemble
//      a URL that has a fixed shape.
//   2. The gateway takes its own token in `cf-aig-authorization`, separately
//      from the upstream provider's key in `Authorization`. A gateway holding
//      stored provider keys needs only the former; one without needs both.
//
// Model ids are namespaced by upstream provider (`openai/gpt-5.2`,
// `workers-ai/@cf/meta/llama-3.3-70b-instruct-fp8-fast`) and are passed
// through unchanged.

use async_trait::async_trait;

use everruns_provider::OpenAIProtocolChatDriver;
use everruns_provider::credential_schema::{CredentialFormSchema, FormField};
use everruns_provider::driver_registry::{
    ChatDriver, DiscoveredModel, DriverConfig, DriverDescriptor, DriverId, DriverRegistry,
    LlmCallConfig, LlmResponse, LlmResponseStream, Message,
};
use everruns_provider::error::Result;
use everruns_provider::{Provider, ProviderAuth, ProviderAuthRequest, ProviderEndpoint};

/// Host serving every Cloudflare AI Gateway endpoint.
pub const CLOUDFLARE_AI_GATEWAY_HOST: &str = "gateway.ai.cloudflare.com";

/// Gateway name Cloudflare creates by default.
pub const DEFAULT_GATEWAY_ID: &str = "default";

/// The OpenAI-compatible base URL for one gateway.
///
/// The protocol driver appends `chat/completions`, so this stops at `/compat`.
/// Both ids are trimmed; neither is escaped, because Cloudflare account ids are
/// hex and gateway names are slugs.
pub fn gateway_base_url(account_id: &str, gateway_id: &str) -> String {
    let account_id = account_id.trim().trim_matches('/');
    let gateway_id = match gateway_id.trim().trim_matches('/') {
        "" => DEFAULT_GATEWAY_ID,
        id => id,
    };
    format!("https://{CLOUDFLARE_AI_GATEWAY_HOST}/v1/{account_id}/{gateway_id}/compat")
}

/// Whether `api_url` points at Cloudflare's AI Gateway host.
pub fn is_cloudflare_gateway_url(api_url: &str) -> bool {
    everruns_provider::openai_protocol::url_host_eq(api_url, CLOUDFLARE_AI_GATEWAY_HOST)
}

/// Ready-to-use Cloudflare AI Gateway provider assembly.
///
/// For a gateway that holds its own upstream provider keys. A gateway without
/// them also needs [`CloudflareGatewayAuth::with_provider_api_key`].
pub fn provider(
    id: impl Into<everruns_provider::ProviderKey>,
    account_id: &str,
    gateway_id: &str,
    gateway_token: impl Into<String>,
) -> Provider {
    Provider::new(id, CloudflareChatDriver::new())
        .base_url(gateway_base_url(account_id, gateway_id))
        .auth(CloudflareGatewayAuth::new(Some(gateway_token.into()), None))
}

/// Cloudflare AI Gateway driver using the OpenAI-compatible Chat Completions
/// endpoint.
#[derive(Clone)]
pub struct CloudflareChatDriver {
    inner: OpenAIProtocolChatDriver,
}

impl CloudflareChatDriver {
    /// Create the Cloudflare-compatible Chat Completions wire driver.
    pub fn new() -> Self {
        Self {
            inner: OpenAIProtocolChatDriver::new(),
        }
    }
}

#[async_trait]
impl ChatDriver for CloudflareChatDriver {
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

    /// No catalog: the `/compat` surface serves no `/models` endpoint, and the
    /// set of reachable models is whichever upstreams the gateway is
    /// configured for. Returning `None` lets the caller fall back rather than
    /// reporting an empty catalog as the truth.
    async fn list_models(
        &self,
        _endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        Ok(None)
    }
}

impl std::fmt::Debug for CloudflareChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudflareChatDriver")
            .field("api", &"Cloudflare AI Gateway (Chat Completions)")
            .finish()
    }
}

impl Default for CloudflareChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Authentication for a Cloudflare AI Gateway request.
///
/// Emits `cf-aig-authorization` for the gateway itself and `Authorization` for
/// the upstream provider. Either may be absent: an unauthenticated gateway
/// needs no gateway token, and a gateway holding stored provider keys needs no
/// provider key. Both absent is a misconfiguration the gateway rejects, which
/// is a clearer failure than one this driver invents.
///
/// `Debug` redacts both tokens so they never leak through `{:?}`.
#[derive(Clone)]
pub struct CloudflareGatewayAuth {
    gateway_token: Option<String>,
    provider_api_key: Option<String>,
}

impl CloudflareGatewayAuth {
    /// Build from the two tokens directly.
    pub fn new(gateway_token: Option<String>, provider_api_key: Option<String>) -> Self {
        let clean = |value: Option<String>| {
            value
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        Self {
            gateway_token: clean(gateway_token),
            provider_api_key: clean(provider_api_key),
        }
    }

    /// Add the upstream provider key, for a gateway that stores none.
    pub fn with_provider_api_key(mut self, provider_api_key: impl Into<String>) -> Self {
        let key = provider_api_key.into().trim().to_string();
        self.provider_api_key = (!key.is_empty()).then_some(key);
        self
    }

    /// Read both tokens from a resolved credential document.
    pub fn from_driver_config(config: &DriverConfig) -> Self {
        Self::new(
            config.credentials.get("api_key").cloned(),
            config.credentials.get("provider_api_key").cloned(),
        )
    }
}

#[async_trait]
impl ProviderAuth for CloudflareGatewayAuth {
    async fn headers(&self, _request: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>> {
        let mut headers = Vec::with_capacity(2);
        if let Some(token) = &self.gateway_token {
            headers.push((
                "cf-aig-authorization".to_string(),
                format!("Bearer {token}"),
            ));
        }
        if let Some(key) = &self.provider_api_key {
            headers.push(("authorization".to_string(), format!("Bearer {key}")));
        }
        Ok(headers)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl std::fmt::Debug for CloudflareGatewayAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudflareGatewayAuth")
            .field("gateway_token", &self.gateway_token.as_ref().map(|_| "***"))
            .field(
                "provider_api_key",
                &self.provider_api_key.as_ref().map(|_| "***"),
            )
            .finish()
    }
}

/// Credential schema: the gateway token plus the two ids its URL is built
/// from, and the optional upstream provider key.
///
/// The ids are credential fields rather than a hand-written base URL because
/// the URL's shape is fixed and Cloudflare's own SDK examples read the account
/// id from `CLOUDFLARE_ACCOUNT_ID`. An explicit base URL still wins when one is
/// configured, which is what a `/compat` proxy in front of the gateway needs.
fn cloudflare_credential_schema() -> CredentialFormSchema {
    CredentialFormSchema {
        fields: vec![
            FormField::password("api_key", "AI Gateway Token")
                .required()
                .with_help(
                    "Sent as `cf-aig-authorization`. Create one on the gateway's \
                     settings page with the AI Gateway Run permission.",
                )
                .env("CLOUDFLARE_API_TOKEN"),
            FormField::text("account_id", "Account ID")
                .required()
                .with_help("The Cloudflare account that owns the gateway.")
                .env("CLOUDFLARE_ACCOUNT_ID"),
            FormField::text("gateway_id", "Gateway name")
                .required()
                .with_default(DEFAULT_GATEWAY_ID)
                .with_placeholder(DEFAULT_GATEWAY_ID)
                .with_help("The gateway's name, as it appears in its URL.")
                .env("CLOUDFLARE_AI_GATEWAY_ID"),
            FormField::password("provider_api_key", "Upstream provider key")
                .with_help(
                    "Only for a gateway that stores no provider keys. Sent as \
                     `Authorization` and used by whichever upstream the model id names.",
                ),
        ],
        instructions_markdown:
            "Create a gateway in the [Cloudflare dashboard](https://dash.cloudflare.com/?to=/:account/ai/ai-gateway), \
             then enter its account id and name. Models are named \
             `provider/model` (`openai/gpt-5.2`, `anthropic/claude-4-5-sonnet`, \
             `workers-ai/@cf/meta/llama-3.3-70b-instruct-fp8-fast`)."
                .to_string(),
    }
}

/// This driver's descriptor: identity, services, and the credential schema
/// that declares its own environment variables.
pub fn descriptor() -> DriverDescriptor {
    DriverDescriptor {
        display_name: "Cloudflare AI Gateway".into(),
        credential_schema: cloudflare_credential_schema(),
        // No vendor default: the base URL is derived from the account and
        // gateway ids below unless one is configured explicitly.
        base_url_env: None,
        ..DriverDescriptor::chat_only(DriverId::Cloudflare, |config| {
            let base_url = config
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    gateway_base_url(
                        config
                            .credentials
                            .get("account_id")
                            .map_or("", String::as_str),
                        config
                            .credentials
                            .get("gateway_id")
                            .map_or(DEFAULT_GATEWAY_ID, String::as_str),
                    )
                });
            Provider::new(config.provider.clone(), CloudflareChatDriver::new())
                .base_url(base_url)
                .auth(CloudflareGatewayAuth::from_driver_config(config))
                .into_boxed_driver()
        })
    }
}

/// Register the Cloudflare AI Gateway driver with the driver registry.
///
/// # Example
///
/// ```
/// use everruns_provider::DriverRegistry;
/// use everruns_drivers::cloudflare::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// assert!(registry.has_driver(&everruns_provider::DriverId::Cloudflare));
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
