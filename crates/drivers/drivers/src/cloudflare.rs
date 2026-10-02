// Cloudflare AI Gateway Chat Driver
//
// Cloudflare's AI REST API fronts many upstream providers and its own Workers
// AI models behind one account-scoped endpoint, and serves Chat Completions
// at `/ai/v1/chat/completions`. So this driver wraps
// `OpenAIProtocolChatDriver` and adds only identity, auth, and the gateway
// header.
//
// Chat Completions rather than one of the three other formats the REST API
// offers, each ruled out by measurement against the live API:
//
//   - `/ai/v1/responses` (Open Responses, which Everruns prefers elsewhere)
//     rejects Workers AI models: a `@cf/` model answers HTTP 400 asking for
//     `prompt` or `messages`, because the gateway does not translate them into
//     the Responses shape. Cloudflare's own endpoint table says "model
//     dependent" for that column. Workers AI models are the free ones, so that
//     is most of the reason to point at Cloudflare at all.
//   - `/ai/run`, the native envelope, covers every model and modality and does
//     return OpenAI-shaped `tool_calls` — but it cannot stream. With
//     `stream: true` it answers `content-type: application/json` and a
//     `{"result":{}}` body: no events, HTTP 200, no error. A driver must
//     produce an incremental stream, and failing silently is worse than
//     failing loudly.
//   - `/ai/v1/messages` (Anthropic Messages) excludes Workers AI outright.
//
// Three things are unlike a direct vendor:
//
//   1. The endpoint embeds the account id
//      (`/client/v4/accounts/{account_id}/ai/v1`), so there is no vendor-wide
//      default base URL. The account id is a credential-form field and the
//      base URL is derived from it, rather than asking an operator to
//      hand-assemble a URL whose shape is fixed.
//   2. The gateway is selected by the `cf-aig-gateway-id` header rather than
//      by the URL. It is optional for third-party models, which fall back to
//      the account's default gateway, and required for Workers AI (`@cf/`)
//      models.
//   3. No upstream provider keys are involved: Cloudflare authenticates and
//      bills the whole call against the account's own API token. Third-party
//      models therefore need a funded account; Workers AI models draw on the
//      account's own allocation.
//
// Not the `gateway.ai.cloudflare.com/.../compat` endpoint: Cloudflare
// deprecated that for single-model calls in favour of this one, and it keeps
// it only for dynamic routes (`dynamic/{route}`), which this driver does not
// target. The two also authenticate differently — `/compat` takes
// `cf-aig-authorization`, this surface takes a plain bearer token — so a
// base-URL override cannot carry a caller from one to the other.
//
// Model ids are namespaced (`openai/gpt-6-luna`, `anthropic/claude-opus-5`,
// `@cf/meta/llama-3.3-70b-instruct-fp8-fast`) and are passed through
// unchanged.

use async_trait::async_trait;

use everruns_provider::OpenAIProtocolChatDriver;
use everruns_provider::credential_schema::{CredentialFormSchema, FormField};
use everruns_provider::driver_registry::{
    ChatDriver, DiscoveredModel, DriverConfig, DriverDescriptor, DriverId, DriverRegistry,
    LlmCallConfig, LlmResponse, LlmResponseStream, Message,
};
use everruns_provider::error::Result;
use everruns_provider::{Provider, ProviderAuth, ProviderAuthRequest, ProviderEndpoint};

/// Host serving Cloudflare's REST API.
pub const CLOUDFLARE_API_HOST: &str = "api.cloudflare.com";

/// The AI REST API base URL for one account.
///
/// The protocol driver appends `chat/completions`, so this stops at `/ai/v1`. The
/// account id is trimmed but not escaped, because Cloudflare account ids are
/// hex.
///
/// THREAT[TM-API-013]: the account id is operator input reaching a request
/// URL, but it lands in the path of a fixed scheme and host. The authority is
/// already closed by the first `/`, so no value — slashes, `@`, `?`, `#` —
/// moves the request off `api.cloudflare.com`. An explicitly configured base
/// URL can, and that path still goes through `validate_provider_base_url` at
/// provider create/update plus the shared client's SSRF-guarding resolver.
pub fn account_base_url(account_id: &str) -> String {
    let account_id = account_id.trim().trim_matches('/');
    format!("https://{CLOUDFLARE_API_HOST}/client/v4/accounts/{account_id}/ai/v1")
}

/// Ready-to-use Cloudflare provider assembly.
///
/// `api_token` needs the Account > Workers AI > Read permission. Pass
/// `gateway_id` to pin a gateway; `None` uses the account's default, which
/// Workers AI models do not accept.
pub fn provider(
    id: impl Into<everruns_provider::ProviderKey>,
    account_id: &str,
    api_token: impl Into<String>,
    gateway_id: Option<&str>,
) -> Provider {
    Provider::new(id, CloudflareChatDriver::new())
        .base_url(account_base_url(account_id))
        .auth(CloudflareAuth::new(
            api_token.into(),
            gateway_id.map(str::to_string),
        ))
}

/// Cloudflare driver using the AI REST API's Chat Completions surface.
#[derive(Clone)]
pub struct CloudflareChatDriver {
    inner: OpenAIProtocolChatDriver,
}

impl CloudflareChatDriver {
    /// Create the Cloudflare Chat Completions wire driver.
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

    /// No catalog: the AI REST API serves no `/models` listing, and the set of
    /// reachable models spans whichever upstreams the account can bill plus
    /// Workers AI. Returning `None` lets the caller fall back rather than
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
            .field("api", &"Cloudflare AI REST API (Chat Completions)")
            .finish()
    }
}

impl Default for CloudflareChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Authentication and gateway selection for a Cloudflare AI REST request.
///
/// Emits `Authorization` with the account's API token, and `cf-aig-gateway-id`
/// when a gateway is pinned. The gateway id is routing rather than
/// authentication, but [`ProviderAuth`] is the per-request header seam on
/// [`Provider`], and the two always travel together.
///
/// `Debug` redacts the token so it never leaks through `{:?}`.
#[derive(Clone)]
pub struct CloudflareAuth {
    api_token: String,
    gateway_id: Option<String>,
}

impl CloudflareAuth {
    /// Build from the token and an optional gateway id.
    pub fn new(api_token: impl Into<String>, gateway_id: Option<String>) -> Self {
        Self {
            api_token: api_token.into().trim().to_string(),
            gateway_id: gateway_id
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty()),
        }
    }

    /// Read the token and gateway id from a resolved credential document.
    pub fn from_driver_config(config: &DriverConfig) -> Self {
        Self::new(
            config
                .credentials
                .get("api_key")
                .cloned()
                .unwrap_or_default(),
            config.credentials.get("gateway_id").cloned(),
        )
    }
}

#[async_trait]
impl ProviderAuth for CloudflareAuth {
    async fn headers(&self, _request: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>> {
        let mut headers = vec![(
            "authorization".to_string(),
            format!("Bearer {}", self.api_token),
        )];
        if let Some(gateway_id) = &self.gateway_id {
            headers.push(("cf-aig-gateway-id".to_string(), gateway_id.clone()));
        }
        Ok(headers)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl std::fmt::Debug for CloudflareAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudflareAuth")
            .field("api_token", &"***")
            .field("gateway_id", &self.gateway_id)
            .finish()
    }
}

/// Credential schema: the account token, the account id its URL is built from,
/// and the optional gateway to pin.
///
/// The account id is a credential field rather than a hand-written base URL
/// because the URL's shape is fixed and Cloudflare's own examples read the id
/// from `CLOUDFLARE_ACCOUNT_ID`. An explicit base URL still wins when one is
/// configured.
fn cloudflare_credential_schema() -> CredentialFormSchema {
    CredentialFormSchema {
        fields: vec![
            FormField::password("api_key", "API Token")
                .required()
                .with_help(
                    "A Cloudflare API token with the Account > Workers AI > Read \
                     permission. A token holding only AI Gateway permissions is \
                     rejected: those cover gateway configuration, not inference.",
                )
                .env("CLOUDFLARE_API_TOKEN"),
            FormField::text("account_id", "Account ID")
                .required()
                .with_help("The Cloudflare account billed for these calls.")
                .env("CLOUDFLARE_ACCOUNT_ID"),
            FormField::text("gateway_id", "Gateway name")
                .with_help(
                    "Optional: calls route through the account's default \
                     gateway when it is unset. Cloudflare documents it as \
                     required for Workers AI (`@cf/`) models, though they are \
                     served without it on an account that has a default.",
                )
                .env("CLOUDFLARE_AI_GATEWAY_ID"),
        ],
        instructions_markdown:
            "Create an API token with the **Account > Workers AI > Read** permission in the \
             [Cloudflare dashboard](https://dash.cloudflare.com/profile/api-tokens), and enter \
             the account id it belongs to. Models are named `provider/model` \
             (`openai/gpt-6-luna`, `anthropic/claude-opus-5`) or, for Workers AI, \
             `@cf/author/model` — those also need a gateway name."
                .to_string(),
    }
}

/// This driver's descriptor: identity, services, and the credential schema
/// that declares its own environment variables.
pub fn descriptor() -> DriverDescriptor {
    DriverDescriptor {
        display_name: "Cloudflare AI Gateway".into(),
        // No vendor default: the base URL is derived from the account id below
        // unless one is configured explicitly.
        base_url_env: None,
        credential_schema: cloudflare_credential_schema(),
        ..DriverDescriptor::chat_only(DriverId::Cloudflare, |config| {
            let base_url = config
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    account_base_url(
                        config
                            .credentials
                            .get("account_id")
                            .map_or("", String::as_str),
                    )
                });
            Provider::new(config.provider.clone(), CloudflareChatDriver::new())
                .base_url(base_url)
                .auth(CloudflareAuth::from_driver_config(config))
                .into_boxed_driver()
        })
    }
}

/// Register the Cloudflare driver with the driver registry.
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
