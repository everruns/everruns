// Fireworks AI Chat Driver
//
// Fireworks AI serves open models (Llama, Qwen, DeepSeek, GLM, Kimi, gpt-oss,
// ...) behind an OpenAI-compatible Chat Completions API. This driver wraps the
// core `OpenAIProtocolChatDriver` (which authenticates non-Azure hosts with a
// bearer token by default) and adds Fireworks-specific model discovery: the
// `/models` endpoint advertises rich capability metadata that is parsed into
// `ModelProfile`s at sync time.

use async_trait::async_trait;
use chrono::TimeZone;
use serde::Deserialize;

use everruns_core::OpenAIProtocolChatDriver;
use everruns_core::credential_schema::{CredentialFormSchema, FormField};
use everruns_core::driver_registry::{
    BoxedChatDriver, ChatDriver, DiscoveredModel, DriverConfig, DriverDescriptor, DriverId,
    DriverRegistry, LlmCallConfig, LlmMessage, LlmResponseStream,
};
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::model::{Modality, ModelLimits, ModelModalities, ModelProfile};
use everruns_core::openai_protocol::{
    apply_models_api_auth, models_api_status_error, models_url_for_api_url, normalize_api_url,
    url_host_eq,
};

/// Fireworks AI serverless inference endpoint (OpenAI-compatible Chat
/// Completions). The chat URL is the normalized `…/chat/completions` form.
pub const FIREWORKS_DEFAULT_API_URL: &str =
    "https://api.fireworks.ai/inference/v1/chat/completions";

/// Canonical Fireworks API host. Discovery only runs against this host.
const FIREWORKS_HOST: &str = "api.fireworks.ai";

/// Whether `api_url` points at Fireworks' hosted API (`api.fireworks.ai`).
///
/// Host-based (not prefix-based) so it tolerates ports and trailing paths.
pub fn is_fireworks_api_url(api_url: &str) -> bool {
    url_host_eq(api_url, FIREWORKS_HOST)
}

// ============================================================================
// Fireworks Chat Driver (OpenAI-compatible Chat Completions API)
// ============================================================================

/// Fireworks AI chat driver.
///
/// Construct directly for programmatic use, or let [`register_driver`] build
/// instances from a [`DriverConfig`].
///
/// # Example
///
/// ```
/// use everruns_fireworks::FireworksChatDriver;
///
/// let driver = FireworksChatDriver::new("fw-key");
/// assert_eq!(
///     driver.api_url(),
///     "https://api.fireworks.ai/inference/v1/chat/completions",
/// );
/// ```
#[derive(Clone)]
pub struct FireworksChatDriver {
    inner: OpenAIProtocolChatDriver,
    api_url: String,
    /// Whether constructed with an explicit base URL override via
    /// [`with_base_url`]. Discovery is skipped for custom (non-Fireworks) hosts.
    uses_custom_url: bool,
}

impl FireworksChatDriver {
    /// Create a driver with the default Fireworks serverless endpoint.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            inner: OpenAIProtocolChatDriver::with_base_url(api_key, FIREWORKS_DEFAULT_API_URL),
            api_url: FIREWORKS_DEFAULT_API_URL.to_string(),
            uses_custom_url: false,
        }
    }

    /// Create a driver with an explicit API URL override (self-hosted proxy or a
    /// custom Fireworks endpoint). The URL is normalized to a
    /// `…/chat/completions` endpoint.
    pub fn with_base_url(api_key: impl Into<String>, api_url: impl Into<String>) -> Self {
        let api_url = normalize_api_url(&api_url.into(), "/chat/completions");
        Self {
            inner: OpenAIProtocolChatDriver::with_base_url(api_key, &api_url),
            api_url,
            uses_custom_url: true,
        }
    }

    /// Build a driver from a registry [`DriverConfig`]. The API key is required
    /// (enforced by the registry); `base_url` overrides the default endpoint.
    fn from_driver_config(config: &DriverConfig) -> Self {
        // The registry guarantees an api_key for Fireworks before calling the
        // factory; default to empty so the infallible factory contract holds.
        let api_key = config.api_key.clone().unwrap_or_default();
        match config.base_url.as_deref().filter(|u| !u.is_empty()) {
            Some(base_url) => Self::with_base_url(api_key, base_url),
            None => Self::new(api_key),
        }
    }

    /// The resolved chat-completions API URL.
    pub fn api_url(&self) -> &str {
        &self.api_url
    }
}

#[async_trait]
impl ChatDriver for FireworksChatDriver {
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
        // Discovery only runs against Fireworks' own host. A custom proxy URL may
        // resolve to private infrastructure at request time (mirrors the
        // OpenRouter/MAI host gating).
        if self.uses_custom_url && !is_fireworks_api_url(&self.api_url) {
            return Ok(None);
        }

        let models_url = models_url_for_api_url(&self.api_url);
        list_fireworks_models(self.inner.client(), self.inner.api_key(), &models_url).await
    }
}

impl std::fmt::Debug for FireworksChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FireworksChatDriver")
            .field("api_url", &self.api_url)
            .field("api", &"Fireworks AI (Chat Completions)")
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// Model discovery
// ============================================================================

/// Fetch the Fireworks `/models` catalog and map chat models to
/// [`DiscoveredModel`]s, building capability profiles from the rich metadata
/// Fireworks advertises (`supports_chat`, `supports_tools`,
/// `supports_image_input`, `context_length`).
async fn list_fireworks_models(
    client: &reqwest::Client,
    api_key: &str,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let response = apply_models_api_auth(client.get(models_url), models_url, api_key)
        .send()
        .await
        .map_err(|e| AgentLoopError::llm(format!("Failed to fetch Fireworks models: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let _ = response.bytes().await; // drain body to allow connection reuse
        return Err(models_api_status_error(status));
    }

    let models: FireworksModelsResponse = response.json().await.map_err(|e| {
        AgentLoopError::llm(format!("Failed to parse Fireworks models response: {e}"))
    })?;

    let discovered = models
        .data
        .into_iter()
        .filter(FireworksModelInfo::is_chat_model)
        .map(|m| {
            // Build the profile (borrows `m`) before moving owned fields out.
            let discovered_profile = Some(m.to_discovered_profile());
            let created_at = m
                .created
                .and_then(|ts| chrono::Utc.timestamp_opt(ts, 0).single());
            let display_name = Some(short_model_name(&m.id));
            DiscoveredModel {
                created_at,
                display_name,
                owned_by: m.owned_by,
                discovered_profile,
                model_id: m.id,
            }
        })
        .collect();

    Ok(Some(discovered))
}

/// Bare Fireworks/OpenAI-compatible `/models` list response.
#[derive(Debug, Deserialize)]
struct FireworksModelsResponse {
    data: Vec<FireworksModelInfo>,
}

/// One entry from the Fireworks `/models` list. Beyond the OpenAI-standard
/// `id`/`created`/`owned_by`, Fireworks advertises capability and limit fields.
#[derive(Debug, Deserialize)]
struct FireworksModelInfo {
    id: String,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    owned_by: Option<String>,
    /// Whether the model is served via the chat-completions API.
    #[serde(default)]
    supports_chat: Option<bool>,
    /// Whether the model accepts image inputs (vision).
    #[serde(default)]
    supports_image_input: Option<bool>,
    /// Whether the model supports tool/function calling.
    #[serde(default)]
    supports_tools: Option<bool>,
    /// Maximum context window in tokens.
    #[serde(default)]
    context_length: Option<i64>,
}

impl FireworksModelInfo {
    /// Whether this model is a chat/completion model. Fireworks marks chat
    /// models with `supports_chat: true`; non-chat services (e.g. image
    /// generation like `flux-1-schnell-fp8`) report `false`/absent and are
    /// excluded so they never reach chat pickers.
    fn is_chat_model(&self) -> bool {
        self.supports_chat.unwrap_or(false)
    }

    /// Build a [`ModelProfile`] from Fireworks' advertised metadata.
    fn to_discovered_profile(&self) -> ModelProfile {
        let image_input = self.supports_image_input.unwrap_or(false);

        let modalities = {
            let mut input = vec![Modality::Text];
            if image_input {
                input.push(Modality::Image);
            }
            Some(ModelModalities {
                input,
                output: vec![Modality::Text],
            })
        };

        let limits = self.context_length.map(|ctx| {
            let context = clamp_i64_to_i32(ctx);
            ModelLimits {
                context,
                input: None,
                // Fireworks does not advertise a separate output cap. Fall back
                // to the context window (the model's theoretical max output)
                // rather than emitting a misleading `0`.
                output: context,
                max_media: None,
            }
        });

        ModelProfile {
            name: short_model_name(&self.id),
            family: self.id.clone(),
            description: None,
            release_date: None,
            last_updated: None,
            attachment: image_input,
            // Fireworks `/models` does not advertise reasoning support; leave it
            // off rather than guess. Operators can correct the profile if needed.
            reasoning: false,
            temperature: true,
            knowledge: None,
            tool_call: self.supports_tools.unwrap_or(false),
            // Fireworks broadly supports JSON / structured output across its
            // chat models.
            structured_output: true,
            // Fireworks exclusively serves open-weight models.
            open_weights: true,
            cost: None,
            limits,
            modalities,
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }
    }
}

/// Derive a short, human-readable name from a Fireworks model id.
///
/// Fireworks ids are namespaced (`accounts/fireworks/models/llama-v3p1-70b`);
/// the last path segment is the readable model name.
fn short_model_name(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_string()
}

/// Clamp an `i64` token count into the `i32` range used by [`ModelLimits`].
/// Fireworks context windows can exceed `i32::MAX` once expressed in tokens for
/// very-large-context models, so saturate rather than wrap.
fn clamp_i64_to_i32(value: i64) -> i32 {
    value.clamp(0, i32::MAX as i64) as i32
}

/// Credential schema for the Fireworks driver: a single API key. The optional
/// endpoint override is the provider's first-class `base_url`, not a credential
/// field, so it is configured separately rather than stored in the credential
/// document.
fn fireworks_credential_schema() -> CredentialFormSchema {
    CredentialFormSchema {
        fields: vec![
            FormField::password("api_key", "API Key")
                .required()
                .with_placeholder("fw_...")
                .with_help(
                    "Your Fireworks AI API key. Create one at https://fireworks.ai under \
                     Account → API Keys.",
                ),
        ],
        instructions_markdown:
            "Configure a [Fireworks AI](https://fireworks.ai) provider with your API key. \
             Fireworks serves open models (Llama, Qwen, DeepSeek, GLM, Kimi, gpt-oss, ...) \
             via an OpenAI-compatible Chat Completions API. Available models are discovered \
             automatically on sync."
                .to_string(),
    }
}

/// Register the Fireworks AI driver with the driver registry.
///
/// Registers [`DriverId::Fireworks`], a chat-only driver backed by Fireworks'
/// OpenAI-compatible Chat Completions API.
///
/// # Example
///
/// ```
/// use everruns_core::DriverRegistry;
/// use everruns_fireworks::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// assert!(registry.has_driver(&everruns_core::DriverId::Fireworks));
/// ```
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_descriptor(DriverDescriptor {
        credential_schema: fireworks_credential_schema(),
        ..DriverDescriptor::chat_only(DriverId::Fireworks, |config| {
            Box::new(FireworksChatDriver::from_driver_config(config)) as BoxedChatDriver
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::driver_registry::{ProviderConfig, ServiceKind};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn default_driver_uses_fireworks_endpoint() {
        let driver = FireworksChatDriver::new("fw-key");
        assert_eq!(driver.api_url(), FIREWORKS_DEFAULT_API_URL);
        assert!(!driver.uses_custom_url);
    }

    #[test]
    fn with_base_url_normalizes_to_chat_completions() {
        let driver =
            FireworksChatDriver::with_base_url("fw-key", "https://api.fireworks.ai/inference/v1");
        assert_eq!(driver.api_url(), FIREWORKS_DEFAULT_API_URL);
        assert!(driver.uses_custom_url);
    }

    #[test]
    fn with_base_url_preserves_full_chat_completions_url() {
        let driver = FireworksChatDriver::with_base_url("fw-key", FIREWORKS_DEFAULT_API_URL);
        assert_eq!(driver.api_url(), FIREWORKS_DEFAULT_API_URL);
    }

    #[test]
    fn is_fireworks_api_url_matches_host_only() {
        assert!(is_fireworks_api_url(
            "https://api.fireworks.ai/inference/v1/chat/completions"
        ));
        assert!(is_fireworks_api_url(
            "https://api.fireworks.ai/inference/v1"
        ));
        assert!(!is_fireworks_api_url("https://proxy.example.com/v1"));
        assert!(!is_fireworks_api_url("https://fireworks.ai/v1"));
    }

    #[test]
    fn short_model_name_takes_last_segment() {
        assert_eq!(
            short_model_name("accounts/fireworks/models/llama-v3p1-70b-instruct"),
            "llama-v3p1-70b-instruct"
        );
        assert_eq!(short_model_name("gpt-oss-120b"), "gpt-oss-120b");
    }

    #[test]
    fn chat_model_filter_uses_supports_chat() {
        let model = |id: &str, chat: Option<bool>| FireworksModelInfo {
            id: id.to_string(),
            created: None,
            owned_by: None,
            supports_chat: chat,
            supports_image_input: None,
            supports_tools: None,
            context_length: None,
        };
        assert!(model("gpt-oss-120b", Some(true)).is_chat_model());
        assert!(!model("flux-1-schnell-fp8", Some(false)).is_chat_model());
        // Absent flag defaults to non-chat (conservative).
        assert!(!model("mystery", None).is_chat_model());
    }

    #[test]
    fn profile_maps_capabilities_and_limits() {
        let model = FireworksModelInfo {
            id: "accounts/fireworks/models/kimi-k2p6".to_string(),
            created: None,
            owned_by: Some("fireworks".to_string()),
            supports_chat: Some(true),
            supports_image_input: Some(true),
            supports_tools: Some(true),
            context_length: Some(262_144),
        };
        let profile = model.to_discovered_profile();
        assert_eq!(profile.name, "kimi-k2p6");
        assert!(profile.tool_call);
        assert!(profile.attachment);
        assert!(profile.open_weights);
        let limits = profile.limits.expect("limits present");
        assert_eq!(limits.context, 262_144);
        assert_eq!(limits.output, 262_144);
        let modalities = profile.modalities.expect("modalities present");
        assert!(modalities.input.contains(&Modality::Image));
    }

    #[test]
    fn profile_without_context_has_no_limits() {
        let model = FireworksModelInfo {
            id: "accounts/fireworks/models/text-only".to_string(),
            created: None,
            owned_by: None,
            supports_chat: Some(true),
            supports_image_input: Some(false),
            supports_tools: Some(false),
            context_length: None,
        };
        let profile = model.to_discovered_profile();
        assert!(profile.limits.is_none());
        assert!(!profile.tool_call);
        assert!(!profile.attachment);
    }

    #[test]
    fn clamp_saturates_above_i32_max() {
        assert_eq!(clamp_i64_to_i32(10), 10);
        assert_eq!(clamp_i64_to_i32(-5), 0);
        assert_eq!(clamp_i64_to_i32(i64::MAX), i32::MAX);
    }

    #[tokio::test]
    async fn list_models_skips_custom_non_fireworks_host() {
        let driver = FireworksChatDriver::with_base_url("fw-key", "https://proxy.example.com/v1");
        assert!(driver.list_models().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn discovery_fetch_authenticates_filters_and_maps() {
        // `list_fireworks_models` is exercised directly so the Fireworks-host
        // gate (which a wiremock 127.0.0.1 host cannot satisfy) does not block
        // the HTTP path.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/inference/v1/models"))
            .and(header("authorization", "Bearer fw-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [
                    {
                        "id": "accounts/fireworks/models/gpt-oss-120b",
                        "object": "model",
                        "created": 1_700_000_000,
                        "owned_by": "fireworks",
                        "supports_chat": true,
                        "supports_tools": true,
                        "supports_image_input": false,
                        "context_length": 131_072
                    },
                    {
                        "id": "accounts/fireworks/models/flux-1-schnell-fp8",
                        "object": "model",
                        "supports_chat": false
                    },
                ],
            })))
            .mount(&server)
            .await;

        let models_url = format!("{}/inference/v1/models", server.uri());
        let discovered = list_fireworks_models(&reqwest::Client::new(), "fw-secret", &models_url)
            .await
            .expect("discovery request should succeed")
            .expect("discovery should return a model list");

        // Image model filtered out; chat model retained with a discovered profile.
        assert_eq!(discovered.len(), 1);
        assert_eq!(
            discovered[0].model_id,
            "accounts/fireworks/models/gpt-oss-120b"
        );
        assert_eq!(discovered[0].display_name.as_deref(), Some("gpt-oss-120b"));
        assert_eq!(discovered[0].owned_by.as_deref(), Some("fireworks"));
        let profile = discovered[0]
            .discovered_profile
            .as_ref()
            .expect("profile present");
        assert!(profile.tool_call);
        assert_eq!(profile.limits.as_ref().unwrap().context, 131_072);
        assert!(discovered[0].created_at.is_some());
    }

    #[test]
    fn register_driver_registers_fireworks() {
        let mut registry = DriverRegistry::new();
        assert!(!registry.has_driver(&DriverId::Fireworks));
        register_driver(&mut registry);
        assert!(registry.has_driver(&DriverId::Fireworks));

        let descriptor = registry.descriptor(&DriverId::Fireworks).unwrap();
        assert_eq!(descriptor.services, vec![ServiceKind::Chat]);
        assert_eq!(descriptor.display_name, "Fireworks AI");
        // Credential schema: required api_key + optional base_url.
        assert_eq!(descriptor.credential_schema.fields.len(), 2);
        assert_eq!(descriptor.credential_schema.fields[0].name, "api_key");
        assert!(descriptor.credential_schema.fields[0].required);

        // A provider with an api key builds a usable chat driver.
        let config = ProviderConfig::new(DriverId::Fireworks).with_api_key("fw-key");
        assert!(registry.create_chat_driver(&config).is_ok());

        // Without an api key the registry rejects it (Fireworks is not exempt).
        let no_key = ProviderConfig::new(DriverId::Fireworks);
        assert!(registry.create_chat_driver(&no_key).is_err());
    }
}
