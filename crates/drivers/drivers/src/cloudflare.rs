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
use chrono::{NaiveDateTime, TimeZone, Utc};
use serde::Deserialize;

use everruns_contracts::OpenAIProtocolChatDriver;
use everruns_contracts::credential_schema::{CredentialFormSchema, FormField};
use everruns_contracts::driver_helpers::fetch_models;
use everruns_contracts::driver_registry::{
    ChatDriver, DiscoveredModel, DriverConfig, DriverDescriptor, DriverId, DriverRegistry,
    LlmCallConfig, LlmResponse, LlmResponseStream, Message,
};
use everruns_contracts::error::Result;
use everruns_contracts::model::{Modality, ModelLimits, ModelModalities, ModelProfile};
use everruns_contracts::openai_protocol::url_host_eq;
use everruns_contracts::{Provider, ProviderAuth, ProviderAuthRequest, ProviderEndpoint};

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
    id: impl Into<everruns_contracts::ProviderKey>,
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

    /// The Workers AI half of the catalog, from `ai/models/search`.
    ///
    /// That endpoint enumerates Workers AI (`@cf/`) models only — the
    /// third-party models the gateway can route to are never listed, because
    /// which ones an account can reach depends on what it is able to bill. So
    /// this is a partial catalog by construction, and the models it omits still
    /// have to be added by id.
    ///
    /// Returning the partial list still beats `None`: the `@cf/` ids are the
    /// awkward ones to type from memory, and they arrive with the context
    /// window and tool-calling support the gateway advertises.
    async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        let Some(models_url) = models_search_url(endpoint) else {
            return Ok(None);
        };
        // Discovery only runs against Cloudflare's own host; a custom proxy URL
        // may resolve to private infrastructure at request time. Mirrors the
        // Vercel/OpenRouter/Meta/Fireworks gating.
        if !url_host_eq(&models_url, CLOUDFLARE_API_HOST) {
            return Ok(None);
        }

        let resolved = endpoint.resolve("GET", &models_url, &[]).await?;
        let mut request = self.inner.client().get(&resolved.url);
        for (name, value) in resolved.headers {
            request = request.header(name, value);
        }
        fetch_models::<CloudflareModelsResponse, _>(
            request,
            "Failed to fetch Cloudflare Workers AI models",
            "Failed to parse Cloudflare Workers AI models response",
            &[],
            |models| {
                models
                    .result
                    .into_iter()
                    // Only the text-generation task serves chat completions.
                    // The rest of the catalog is embeddings, speech, and image
                    // work this driver cannot route.
                    .filter(|m| {
                        m.task
                            .as_ref()
                            .is_some_and(|t| t.name == CF_TEXT_GENERATION)
                    })
                    .map(|m| m.into_discovered())
                    .collect()
            },
        )
        .await
    }
}

/// The Workers AI task whose models serve chat completions.
const CF_TEXT_GENERATION: &str = "Text Generation";

/// Derive the model-search URL from the chat base URL.
///
/// The chat base stops at `/ai/v1` because the protocol driver appends
/// `chat/completions`; the model catalog is a sibling of `v1`, at
/// `/ai/models/search`. Deriving it rather than rebuilding from the account id
/// keeps a configured base URL (a proxy) in control of where discovery points,
/// which is what the host gate below is then checking.
fn models_search_url(endpoint: &ProviderEndpoint) -> Option<String> {
    let base = endpoint.base_url()?;
    let trimmed = base.trim_end_matches('/');
    let root = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
    Some(format!("{root}/models/search"))
}

/// `ai/models/search` response.
///
/// `result_info.total_count` is not a reliable page count: an account that gets
/// 69 models in one page is told the total is 321, and page 2 comes back empty.
/// So this reads the single page the endpoint serves instead of paginating on a
/// number that does not describe this result.
#[derive(Debug, Deserialize)]
struct CloudflareModelsResponse {
    result: Vec<CloudflareModel>,
}

#[derive(Debug, Deserialize)]
struct CloudflareModel {
    name: String,
    description: Option<String>,
    created_at: Option<String>,
    task: Option<CloudflareTask>,
    #[serde(default)]
    properties: Vec<CloudflareProperty>,
}

#[derive(Debug, Deserialize)]
struct CloudflareTask {
    name: String,
}

/// A property is a loosely typed name/value pair: `value` is a string for the
/// flags this driver reads and an array for pricing, so it stays untyped here
/// and each reader interprets its own.
#[derive(Debug, Deserialize)]
struct CloudflareProperty {
    property_id: String,
    value: serde_json::Value,
}

impl CloudflareModel {
    fn property(&self, id: &str) -> Option<&serde_json::Value> {
        self.properties
            .iter()
            .find(|p| p.property_id == id)
            .map(|p| &p.value)
    }

    fn flag(&self, id: &str) -> bool {
        self.property(id)
            .and_then(|v| v.as_str())
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }

    fn into_discovered(self) -> DiscoveredModel {
        let context = self
            .property("context_window")
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse::<i32>().ok());
        let tool_call = self.flag("function_calling");

        // `2024-12-06 17:09:18.338`: no zone, no `T`. Cloudflare serves this
        // field in UTC.
        let created_at = self.created_at.as_deref().and_then(|raw| {
            NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f")
                .ok()
                .and_then(|naive| Utc.from_local_datetime(&naive).single())
        });

        // `@cf/meta/llama-3.3-70b-instruct-fp8-fast` -> `meta`.
        let owned_by = self
            .name
            .strip_prefix("@cf/")
            .and_then(|rest| rest.split('/').next())
            .map(str::to_string);

        let profile = ModelProfile {
            name: short_model_name(&self.name),
            family: self.name.clone(),
            description: self.description.clone(),
            release_date: None,
            last_updated: None,
            attachment: false,
            // The catalog advertises no reasoning flag; leave it off rather
            // than guess.
            reasoning: false,
            temperature: true,
            knowledge: None,
            tool_call,
            structured_output: false,
            // Workers AI serves open-weight models.
            open_weights: true,
            cost: None,
            limits: context.map(|context| ModelLimits {
                context,
                input: None,
                output: context,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
            supports_server_compaction: false,
            decisions: None,
        };

        DiscoveredModel {
            capabilities: vec!["chat".to_string()],
            created_at,
            display_name: None,
            owned_by,
            discovered_profile: Some(profile),
            model_id: self.name,
        }
    }
}

/// Last path segment of a namespaced id, for display.
fn short_model_name(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_string()
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
             the account id it belongs to. Workers AI models (`@cf/author/model`) are \
             discovered automatically and also need a gateway name; third-party models \
             (`openai/gpt-6-luna`, `anthropic/claude-opus-5`) are not listed by the API, \
             so add those by id."
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
/// use everruns_contracts::DriverRegistry;
/// use everruns_drivers::cloudflare::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// assert!(registry.has_driver(&everruns_contracts::DriverId::Cloudflare));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The catalog is a sibling of `v1`, not a path under it: the chat base
    /// stops at `/ai/v1` only because the protocol driver appends
    /// `chat/completions`.
    #[test]
    fn models_search_is_a_sibling_of_the_api_version() {
        let endpoint = provider("cf", "acct123", "token", None).endpoint().clone();
        assert_eq!(
            models_search_url(&endpoint).as_deref(),
            Some("https://api.cloudflare.com/client/v4/accounts/acct123/ai/models/search")
        );
    }

    /// A base URL that already lacks the version suffix must not lose a segment.
    #[test]
    fn models_search_tolerates_a_base_without_the_version_suffix() {
        let endpoint = provider("cf", "acct123", "token", None)
            .base_url("https://proxy.example/ai")
            .endpoint()
            .clone();
        assert_eq!(
            models_search_url(&endpoint).as_deref(),
            Some("https://proxy.example/ai/models/search")
        );
    }

    fn model(json: serde_json::Value) -> CloudflareModel {
        serde_json::from_value(json).expect("fixture should deserialize")
    }

    /// The advertised properties are what make a discovered model useful: a
    /// context window and tool-calling support the caller would otherwise have
    /// to look up by hand.
    #[test]
    fn properties_map_into_the_discovered_profile() {
        let discovered = model(serde_json::json!({
            "name": "@cf/meta/llama-3.3-70b-instruct-fp8-fast",
            "description": "Llama 3.3 70B",
            "created_at": "2024-12-06 17:09:18.338",
            "task": { "name": "Text Generation" },
            "properties": [
                { "property_id": "context_window", "value": "24000" },
                { "property_id": "function_calling", "value": "true" },
                { "property_id": "price", "value": [{ "unit": "per M input tokens", "price": 0.293 }] }
            ]
        }))
        .into_discovered();

        assert_eq!(
            discovered.model_id,
            "@cf/meta/llama-3.3-70b-instruct-fp8-fast"
        );
        // `@cf/<owner>/<model>` — the owner is the segment after the namespace.
        assert_eq!(discovered.owned_by.as_deref(), Some("meta"));
        assert_eq!(discovered.capabilities, vec!["chat".to_string()]);
        assert!(
            discovered.created_at.is_some(),
            "the timestamp should parse"
        );

        let profile = discovered
            .discovered_profile
            .expect("a discovered model carries its profile");
        assert!(profile.tool_call);
        assert_eq!(profile.limits.map(|l| l.context), Some(24_000));
        assert_eq!(profile.name, "llama-3.3-70b-instruct-fp8-fast");
    }

    /// A model that advertises nothing still discovers, just without a profile
    /// claim the catalog never made.
    #[test]
    fn absent_properties_claim_nothing() {
        let discovered = model(serde_json::json!({
            "name": "@cf/qwen/qwen1.5-0.5b-chat",
            "task": { "name": "Text Generation" },
            "properties": []
        }))
        .into_discovered();

        let profile = discovered
            .discovered_profile
            .expect("a discovered model carries its profile");
        assert!(!profile.tool_call, "absence is not tool support");
        assert!(profile.limits.is_none(), "absence is not a context window");
        assert!(discovered.created_at.is_none());
    }
}
