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

use everruns_provider::OpenAIProtocolChatDriver;
use everruns_provider::credential_schema::{CredentialFormSchema, FormField};
use everruns_provider::driver_helpers::fetch_models;
use everruns_provider::driver_registry::{
    ChatDriver, DiscoveredModel, DriverDescriptor, DriverId, DriverRegistry, LlmCallConfig,
    LlmMessage, LlmResponseStream,
};
use everruns_provider::error::Result;
use everruns_provider::model::{Modality, ModelLimits, ModelModalities, ModelProfile};
use everruns_provider::openai_protocol::{models_url_for_api_url, url_host_eq};
use everruns_provider::{BearerAuth, Provider, ProviderEndpoint};

/// Fireworks AI serverless inference endpoint (OpenAI-compatible Chat
/// Completions). The chat URL is the normalized `…/chat/completions` form.
pub const FIREWORKS_DEFAULT_API_URL: &str =
    "https://api.fireworks.ai/inference/v1/chat/completions";

/// Ready-to-use Fireworks provider assembly.
pub fn provider(
    id: impl Into<everruns_provider::ProviderKey>,
    api_key: impl Into<String>,
) -> Provider {
    Provider::new(id, FireworksChatDriver::new())
        .base_url("https://api.fireworks.ai/inference/v1")
        .auth(BearerAuth::new(api_key))
}

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
/// instances from the transitional descriptor catalog.
///
/// # Example
///
/// ```
/// use everruns_fireworks::provider;
///
/// let service = provider("fireworks", "fw-key");
/// assert_eq!(
///     service.endpoint().url("chat/completions").unwrap(),
///     "https://api.fireworks.ai/inference/v1/chat/completions",
/// );
/// ```
#[derive(Clone)]
pub struct FireworksChatDriver {
    inner: OpenAIProtocolChatDriver,
}

impl FireworksChatDriver {
    /// Create the Fireworks-compatible Chat Completions wire driver.
    pub fn new() -> Self {
        Self {
            inner: OpenAIProtocolChatDriver::new(),
        }
    }
}

#[async_trait]
impl ChatDriver for FireworksChatDriver {
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
        let Some(api_url) = endpoint.url("chat/completions") else {
            return Ok(None);
        };
        // Discovery only runs against Fireworks' own host. A custom proxy URL may
        // resolve to private infrastructure at request time (mirrors the
        // OpenRouter/MAI host gating).
        if !is_fireworks_api_url(&api_url) {
            return Ok(None);
        }

        let models_url = models_url_for_api_url(&api_url);
        list_fireworks_models(self.inner.client(), endpoint, &models_url).await
    }
}

impl std::fmt::Debug for FireworksChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FireworksChatDriver")
            .field("api", &"Fireworks AI (Chat Completions)")
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
    endpoint: &ProviderEndpoint,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let resolved = endpoint.resolve("GET", models_url, &[]).await?;
    let mut request = client.get(&resolved.url);
    for (name, value) in resolved.headers {
        request = request.header(name, value);
    }
    fetch_models::<FireworksModelsResponse, _>(
        request,
        "Failed to fetch Fireworks models",
        "Failed to parse Fireworks models response",
        &[],
        |models| {
            models
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
                        capabilities: vec!["chat".to_string()],
                        created_at,
                        display_name,
                        owned_by: m.owned_by,
                        discovered_profile,
                        model_id: m.id,
                    }
                })
                .collect()
        },
    )
    .await
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
            speed: None,
            verbosity: None,
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
/// use everruns_provider::DriverRegistry;
/// use everruns_fireworks::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// assert!(registry.has_driver(&everruns_provider::DriverId::Fireworks));
/// ```
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_descriptor(DriverDescriptor {
        display_name: "Fireworks AI".into(),
        credential_schema: fireworks_credential_schema(),
        ..DriverDescriptor::chat_only(DriverId::Fireworks, |config| {
            Provider::new(config.provider.clone(), FireworksChatDriver::new())
                .base_url(
                    config
                        .base_url
                        .as_deref()
                        .unwrap_or("https://api.fireworks.ai/inference/v1"),
                )
                .auth(BearerAuth::new(config.api_key.clone().unwrap_or_default()))
                .into_boxed_driver()
        })
    });
}

impl Default for FireworksChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::driver_registry::{LlmMessageRole, ProviderConfig, ServiceKind};
    use serde_json::{Value, json};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn config() -> LlmCallConfig {
        LlmCallConfig {
            model: "accounts/fireworks/models/example".into(),
            temperature: Some(0.25),
            max_tokens: Some(64),
            tools: vec![],
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            metadata: Default::default(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: Some(false),
            volatile_suffix_len: 0,
            extra_headers: vec![],
            cache_diagnostics: None,
        }
    }

    #[tokio::test]
    async fn direct_and_registered_providers_preserve_authenticated_chat_contract() {
        let direct = provider("fireworks", "synthetic-key");
        assert_eq!(
            direct.endpoint().url("chat/completions").as_deref(),
            Some("https://api.fireworks.ai/inference/v1/chat/completions")
        );
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);
        let descriptor = registry.descriptor(&DriverId::Fireworks).unwrap();
        assert_eq!(descriptor.display_name, "Fireworks AI");
        assert_eq!(descriptor.services, vec![ServiceKind::Chat]);
        assert_eq!(descriptor.credential_schema.fields.len(), 1);
        assert_eq!(descriptor.credential_schema.fields[0].name, "api_key");
        assert!(descriptor.credential_schema.fields[0].required);
        for suffix in ["/inference/v1", "/inference/v1/chat/completions"] {
            for registered in [false, true] {
                let server = MockServer::builder().start().await;
                Mock::given(method("POST")).and(path("/inference/v1/chat/completions")).and(header("authorization", "Bearer synthetic-key")).respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string("data: {\"id\":\"response-1\",\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2}}\n\ndata: [DONE]\n\n")).expect(1).mount(&server).await;
                let url = format!("{}{suffix}", server.uri());
                let messages = vec![
                    LlmMessage::text(LlmMessageRole::System, "rules"),
                    LlmMessage::text(LlmMessageRole::User, "question"),
                ];
                let result = if registered {
                    registry
                        .create_chat_driver(
                            &ProviderConfig::new(DriverId::Fireworks)
                                .with_api_key("synthetic-key")
                                .with_base_url(url),
                        )
                        .unwrap()
                        .chat_completion(&ProviderEndpoint::default(), messages, &config())
                        .await
                } else {
                    provider("direct", "synthetic-key")
                        .base_url(url)
                        .chat_completion(messages, &config())
                        .await
                }
                .unwrap();
                assert_eq!(result.text, "answer");
                assert!(result.tool_calls.is_none());
                assert!(result.reasoning.is_empty());
                assert_eq!(result.metadata.response_id.as_deref(), Some("response-1"));
                assert_eq!(result.metadata.finish_reason.as_deref(), Some("stop"));
                assert_eq!(
                    (
                        result.metadata.prompt_tokens,
                        result.metadata.completion_tokens,
                        result.metadata.total_tokens
                    ),
                    (Some(10), Some(2), Some(12))
                );
                let requests = server.received_requests().await.unwrap();
                assert_eq!(requests.len(), 1);
                assert_eq!(
                    requests[0].body_json::<Value>().unwrap(),
                    json!({"model":"accounts/fireworks/models/example","messages":[{"role":"system","content":"rules"},{"role":"user","content":"question"}],"temperature":0.25,"max_tokens":64,"parallel_tool_calls":false,"stream":true,"stream_options":{"include_usage":true}})
                );
            }
        }
        let keyless = registry
            .create_chat_driver(&ProviderConfig::new(DriverId::Fireworks))
            .unwrap();
        let error = keyless
            .chat_completion(&ProviderEndpoint::default(), vec![], &config())
            .await
            .unwrap_err();
        assert_eq!(
            error.llm_error_kind(),
            Some(everruns_provider::error::LlmErrorKind::Authentication)
        );
    }

    #[tokio::test]
    async fn discovery_host_gate_rejects_lookalikes_before_authentication() {
        use everruns_provider::runtime_provider::{ProviderAuth, ProviderAuthRequest};
        struct ForbiddenAuth;
        #[async_trait]
        impl ProviderAuth for ForbiddenAuth {
            async fn headers(&self, _: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>> {
                panic!("disallowed discovery must not resolve credentials")
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
        for allowed in [
            "https://api.fireworks.ai/inference/v1",
            "https://API.FIREWORKS.AI:443/custom?x=y",
        ] {
            assert!(is_fireworks_api_url(allowed), "{allowed}");
        }
        for url in [
            None,
            Some("https://proxy.example/v1"),
            Some("https://fireworks.ai/v1"),
            Some("https://api.fireworks.ai.evil.example/v1"),
            Some("https://api.fireworks.ai@evil.example/v1"),
            Some("https://evil.example/api.fireworks.ai"),
            Some("not a URL"),
        ] {
            let service = Provider::new("gate", FireworksChatDriver::new()).auth(ForbiddenAuth);
            let service = if let Some(url) = url {
                assert!(!is_fireworks_api_url(url), "{url}");
                service.base_url(url)
            } else {
                service
            };
            assert!(service.list_models().await.unwrap().is_none());
        }
    }

    fn expected_profile(
        name: &str,
        family: &str,
        image: bool,
        tools: bool,
        limits: Option<i32>,
    ) -> Value {
        let mut expected = json!({"name":name,"family":family,"attachment":image,"reasoning":false,"temperature":true,"tool_call":tools,"structured_output":true,"open_weights":true,"modalities":{"input":if image { vec!["text","image"] } else { vec!["text"] },"output":["text"]},"tool_search":false,"supports_phases":false});
        if let Some(limit) = limits {
            expected["limits"] = json!({"context":limit,"output":limit});
        }
        expected
    }

    #[tokio::test]
    async fn discovery_preserves_complete_catalog_profiles_and_numeric_boundaries() {
        let server = MockServer::builder().start().await;
        Mock::given(method("GET")).and(path("/inference/v1/models")).and(header("authorization", "Bearer synthetic-key")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
            {"id":"accounts/fireworks/models/vision","supports_chat":true,"supports_image_input":true,"supports_tools":true,"context_length":262144,"created":1700000000,"owned_by":"fireworks"},
            {"id":"plain","supports_chat":true,"supports_image_input":false,"supports_tools":false},
            {"id":"negative","supports_chat":true,"context_length":-5,"created":9223372036854775807_i64},
            {"id":"huge","supports_chat":true,"context_length":9223372036854775807_i64},
            {"id":"not-chat","supports_chat":false}, {"id":"unknown"}
        ]}))).expect(1).mount(&server).await;
        let service = provider("catalog", "synthetic-key");
        let models = list_fireworks_models(
            &reqwest::Client::new(),
            service.endpoint(),
            &format!("{}/inference/v1/models", server.uri()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(models.len(), 4);
        for (model, (id, name, image, tools, limit, created, owner)) in models.iter().zip([
            (
                "accounts/fireworks/models/vision",
                "vision",
                true,
                true,
                Some(262144),
                Some(1700000000),
                Some("fireworks"),
            ),
            ("plain", "plain", false, false, None, None, None),
            ("negative", "negative", false, false, Some(0), None, None),
            ("huge", "huge", false, false, Some(2147483647), None, None),
        ]) {
            assert_eq!(model.model_id, id);
            assert_eq!(model.display_name.as_deref(), Some(name));
            assert_eq!(model.capabilities, vec!["chat"]);
            assert_eq!(model.created_at.map(|time| time.timestamp()), created);
            assert_eq!(model.owned_by.as_deref(), owner);
            assert_eq!(
                serde_json::to_value(model.discovered_profile.as_ref().unwrap()).unwrap(),
                expected_profile(name, id, image, tools, limit)
            );
        }
    }
}
