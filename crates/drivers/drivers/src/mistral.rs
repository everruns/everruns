// Mistral AI Chat Driver
//
// Mistral's La Plateforme serves an OpenAI-compatible Chat Completions API at
// `https://api.mistral.ai/v1`, so this driver wraps `OpenAIProtocolChatDriver`
// and adds identity, auth, and model discovery.
//
// Two wire differences matter, and both are handled in the shared protocol
// rather than here, because any gateway relaying Mistral's native shape hits
// them too:
// - With `reasoning_effort: "high"` the message `content` is an array of typed
//   chunks (`thinking` chunks, then `text`) instead of a string, streamed and
//   non-streamed alike.
// - A plain answer carries `"tool_calls": null`.
//
// Reasoning on Mistral Large 4 is a toggle: the API takes only `none` and
// `high` and answers 400 to any other grade. A graded effort carried over from
// another model (an agent's `medium` default, a session that switched models)
// would fail the turn, so a grade the model's profile does not offer is
// snapped to its highest offered grade, which is how the model reasons at all.
//
// Discovery reads Mistral's `/models`, which advertises capabilities
// (`completion_chat`, `function_calling`, `vision`, `reasoning`) and the
// context window. Every alias is listed as its own entry carrying the same
// canonical `name`, so only the canonical entry is kept.

use async_trait::async_trait;
use chrono::TimeZone;
use serde::Deserialize;

use everruns_contracts::OpenAIProtocolChatDriver;
use everruns_contracts::credential_schema::CredentialFormSchema;
use everruns_contracts::driver_helpers::fetch_models;
use everruns_contracts::driver_registry::{
    ChatDriver, DiscoveredModel, DriverDescriptor, DriverId, DriverRegistry, LlmCallConfig,
    LlmResponse, LlmResponseStream, Message,
};
use everruns_contracts::error::Result;
use everruns_contracts::model::{Modality, ModelLimits, ModelModalities, ModelProfile};
use everruns_contracts::openai_protocol::{models_url_for_api_url, url_host_eq};
use everruns_contracts::{BearerAuth, Provider, ProviderEndpoint};

/// Mistral La Plateforme API base URL.
pub const MISTRAL_DEFAULT_API_URL: &str = "https://api.mistral.ai/v1";

/// Host serving Mistral's API. Discovery only runs against this host.
pub const MISTRAL_API_HOST: &str = "api.mistral.ai";

/// Ready-to-use Mistral provider assembly.
///
/// # Example
///
/// ```
/// use everruns_drivers::mistral::provider;
///
/// let service = provider("mistral", "mistral-key");
/// assert_eq!(
///     service.endpoint().url("chat/completions").unwrap(),
///     "https://api.mistral.ai/v1/chat/completions",
/// );
/// ```
pub fn provider(
    id: impl Into<everruns_contracts::ProviderKey>,
    api_key: impl Into<String>,
) -> Provider {
    Provider::new(id, MistralChatDriver::new())
        .base_url(MISTRAL_DEFAULT_API_URL)
        .auth(BearerAuth::new(api_key))
}

/// Mistral AI chat driver (OpenAI-compatible Chat Completions).
#[derive(Clone)]
pub struct MistralChatDriver {
    inner: OpenAIProtocolChatDriver,
}

impl MistralChatDriver {
    /// Create the Mistral Chat Completions wire driver.
    pub fn new() -> Self {
        Self {
            inner: OpenAIProtocolChatDriver::new(),
        }
    }
}

/// `config` with a reasoning grade the model does not offer snapped to the
/// highest grade it does. Unprofiled models pass through untouched: there is
/// nothing to check the grade against.
fn supported_effort(config: &LlmCallConfig) -> std::borrow::Cow<'_, LlmCallConfig> {
    let Some(requested) = config.reasoning_effort.filter(|e| e.requests_reasoning()) else {
        return std::borrow::Cow::Borrowed(config);
    };
    let offered =
        everruns_contracts::model_profiles::get_model_profile(&DriverId::Mistral, &config.model)
            .and_then(|profile| profile.reasoning_effort)
            .map(|efforts| efforts.values);
    let Some(offered) = offered else {
        return std::borrow::Cow::Borrowed(config);
    };
    if offered.iter().any(|value| value.value == requested) {
        return std::borrow::Cow::Borrowed(config);
    }
    let mut snapped = config.clone();
    snapped.reasoning_effort = offered.last().map(|value| value.value);
    std::borrow::Cow::Owned(snapped)
}

impl Default for MistralChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChatDriver for MistralChatDriver {
    async fn chat_completion_stream(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner
            .chat_completion_stream(endpoint, messages, &supported_effort(config))
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
            .chat_completion_non_streaming(endpoint, messages, &supported_effort(config))
            .await
    }

    async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        let Some(api_url) = endpoint.url("chat/completions") else {
            return Ok(None);
        };
        // Discovery only runs against Mistral's own host. A custom proxy URL may
        // resolve to private infrastructure at request time (mirrors the
        // OpenRouter/Fireworks/Vercel host gating).
        if !url_host_eq(&api_url, MISTRAL_API_HOST) {
            return Ok(None);
        }
        let models_url = models_url_for_api_url(&api_url);
        list_mistral_models(&self.inner.client(), endpoint, &models_url).await
    }
}

impl std::fmt::Debug for MistralChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MistralChatDriver")
            .field("api", &"Mistral AI (Chat Completions)")
            .finish()
    }
}

// ============================================================================
// Model discovery
// ============================================================================

async fn list_mistral_models(
    client: &reqwest::Client,
    endpoint: &ProviderEndpoint,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let resolved = endpoint.resolve("GET", models_url, &[]).await?;
    let mut request = client.get(&resolved.url);
    for (name, value) in resolved.headers {
        request = request.header(name, value);
    }
    fetch_models::<MistralModelsResponse, _>(
        request,
        "Failed to fetch Mistral models",
        "Failed to parse Mistral models response",
        &[],
        |models| {
            models
                .data
                .into_iter()
                .filter(MistralModelInfo::is_canonical_chat_model)
                .map(|m| {
                    let discovered_profile = Some(m.to_discovered_profile());
                    let created_at = m
                        .created
                        .and_then(|ts| chrono::Utc.timestamp_opt(ts, 0).single());
                    DiscoveredModel {
                        capabilities: vec!["chat".to_string()],
                        created_at,
                        display_name: None,
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

#[derive(Debug, Deserialize)]
struct MistralModelsResponse {
    data: Vec<MistralModelInfo>,
}

/// One entry from Mistral's `/models` list.
#[derive(Debug, Deserialize)]
struct MistralModelInfo {
    id: String,
    /// The canonical model this id serves. Aliases repeat it.
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    owned_by: Option<String>,
    #[serde(default)]
    capabilities: MistralCapabilities,
    #[serde(default)]
    max_context_length: Option<i64>,
    /// Set once Mistral schedules the model's retirement.
    #[serde(default)]
    deprecation: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct MistralCapabilities {
    #[serde(default)]
    completion_chat: bool,
    #[serde(default)]
    function_calling: bool,
    #[serde(default)]
    vision: bool,
    #[serde(default)]
    reasoning: bool,
}

impl MistralModelInfo {
    /// Chat models only (embeddings, OCR, moderation and speech share the
    /// list), one entry per model: an alias repeats its canonical `name`, so
    /// keeping `id == name` keeps exactly one. Models already scheduled for
    /// retirement are left out of new pickers.
    fn is_canonical_chat_model(&self) -> bool {
        let canonical = self.name.as_deref().is_none_or(|name| name == self.id);
        self.capabilities.completion_chat && canonical && self.deprecation.is_none()
    }

    fn to_discovered_profile(&self) -> ModelProfile {
        let mut input = vec![Modality::Text];
        if self.capabilities.vision {
            input.push(Modality::Image);
        }
        let limits = self.max_context_length.map(|ctx| {
            let context = ctx.clamp(0, i32::MAX as i64) as i32;
            ModelLimits {
                context,
                input: None,
                // Mistral advertises no separate output cap; the context window
                // is the theoretical maximum rather than a misleading `0`.
                output: context,
                max_media: None,
            }
        });
        ModelProfile {
            name: self.id.clone(),
            family: self.id.clone(),
            description: None,
            release_date: None,
            last_updated: None,
            attachment: self.capabilities.vision,
            reasoning: self.capabilities.reasoning,
            temperature: true,
            knowledge: None,
            tool_call: self.capabilities.function_calling,
            // Mistral serves `response_format: json_schema` on its chat models.
            structured_output: true,
            // The catalog does not say which weights are published.
            open_weights: false,
            cost: None,
            limits,
            modalities: Some(ModelModalities {
                input,
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
        }
    }
}

/// This driver's descriptor: identity, services, and the credential schema
/// that declares its own environment variables.
pub fn descriptor() -> DriverDescriptor {
    DriverDescriptor {
        display_name: "Mistral AI".into(),
        // The variable Mistral's own SDKs read.
        credential_schema: CredentialFormSchema::api_key(
            "MISTRAL_API_KEY",
            "Create an API key in [Mistral Studio](https://console.mistral.ai/api-keys). \
             Available models are discovered automatically on sync.",
        ),
        base_url_env: Some("MISTRAL_BASE_URL".into()),
        ..DriverDescriptor::chat_only(DriverId::Mistral, |config| {
            Provider::new(config.provider.clone(), MistralChatDriver::new())
                .base_url(
                    config
                        .base_url
                        .as_deref()
                        .map(str::trim)
                        .filter(|url| !url.is_empty())
                        .unwrap_or(MISTRAL_DEFAULT_API_URL),
                )
                .auth(BearerAuth::new(config.api_key.clone().unwrap_or_default()))
                .into_boxed_driver()
        })
    }
}

/// Register the Mistral driver with the driver registry.
///
/// # Example
///
/// ```
/// use everruns_contracts::DriverRegistry;
/// use everruns_drivers::mistral::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// assert!(registry.has_driver(&everruns_contracts::DriverId::Mistral));
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
    use everruns_contracts::driver_registry::{
        LlmStreamEvent, MessageRole, ProviderConfig, ServiceKind,
    };
    use everruns_contracts::model::ReasoningEffort;
    use futures::StreamExt;
    use serde_json::{Value, json};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Verbatim shape of a `mistral-large-4` reasoning stream: thinking
    /// chunks, then a last chunk with an empty thinking chunk beside the
    /// answer, the finish reason and usage.
    const REASONING_STREAM: &str = concat!(
        "data: {\"id\":\"r1\",\"object\":\"chat.completion.chunk\",\"model\":\"mistral-large-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"r1\",\"object\":\"chat.completion.chunk\",\"model\":\"mistral-large-4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":[{\"type\":\"thinking\",\"thinking\":[{\"type\":\"text\",\"text\":\"17 x 23\"}],\"closed\":true}]},\"finish_reason\":null}],\"p\":\"abc\"}\n\n",
        "data: {\"id\":\"r1\",\"object\":\"chat.completion.chunk\",\"model\":\"mistral-large-4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":[{\"type\":\"thinking\",\"thinking\":[{\"type\":\"text\",\"text\":\" = 391\"}],\"closed\":true}]},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"r1\",\"object\":\"chat.completion.chunk\",\"model\":\"mistral-large-4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":[{\"type\":\"thinking\",\"thinking\":[]},{\"type\":\"text\",\"text\":\"391\"}]},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":24,\"total_tokens\":151,\"completion_tokens\":127,\"prompt_tokens_details\":{\"cached_tokens\":0}}}\n\n",
        "data: [DONE]\n\n",
    );

    async fn mock_chat(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer synthetic-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("connection", "close")
                    .set_body_raw(REASONING_STREAM, "text/event-stream"),
            )
            .expect(1)
            .mount(server)
            .await;
    }

    async fn drain(mut stream: LlmResponseStream) -> (String, String, Option<String>) {
        let (mut text, mut reasoning, mut finish) = (String::new(), String::new(), None);
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
                LlmStreamEvent::ReasoningDelta { delta, .. } => reasoning.push_str(&delta),
                LlmStreamEvent::Done(metadata) => finish = metadata.finish_reason,
                LlmStreamEvent::Error(error) => panic!("stream error: {error}"),
                _ => {}
            }
        }
        (text, reasoning, finish)
    }

    fn reasoning_config() -> LlmCallConfig {
        let mut config = LlmCallConfig::new("mistral-large-4");
        config.reasoning_effort = Some(ReasoningEffort::High);
        config.max_tokens = Some(2000);
        config
    }

    #[test]
    fn descriptor_declares_the_mistral_api_key() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);
        let descriptor = registry.descriptor(&DriverId::Mistral).unwrap();
        assert_eq!(descriptor.display_name, "Mistral AI");
        assert_eq!(descriptor.services, vec![ServiceKind::Chat]);
        assert_eq!(
            descriptor.declared_env_vars(),
            vec!["MISTRAL_API_KEY", "MISTRAL_BASE_URL"]
        );
    }

    /// Direct and registered providers both reach `/v1/chat/completions` with
    /// bearer auth, send `reasoning_effort` and `max_tokens` as Mistral names
    /// them, and split the thinking chunks from the answer.
    #[tokio::test]
    async fn reasoning_stream_splits_thinking_from_the_answer() {
        for registered in [false, true] {
            let server = MockServer::start().await;
            mock_chat(&server).await;
            let base_url = format!("{}/v1", server.uri());
            let messages = vec![Message::text(MessageRole::User, "17*23?")];
            let stream = if registered {
                let mut registry = DriverRegistry::new();
                register_driver(&mut registry);
                registry
                    .create_chat_driver(
                        &ProviderConfig::new(DriverId::Mistral)
                            .with_api_key("synthetic-key")
                            .with_base_url(base_url),
                    )
                    .unwrap()
                    .chat_completion_stream(
                        &ProviderEndpoint::default(),
                        messages,
                        &reasoning_config(),
                    )
                    .await
            } else {
                provider("mistral", "synthetic-key")
                    .base_url(base_url)
                    .chat_completion_stream(messages, &reasoning_config())
                    .await
            }
            .unwrap();

            let (text, reasoning, finish) = drain(stream).await;
            assert_eq!(text, "391");
            assert_eq!(reasoning, "17 x 23 = 391");
            assert_eq!(finish.as_deref(), Some("stop"));

            let requests = server.received_requests().await.unwrap();
            let body = requests[0].body_json::<Value>().unwrap();
            assert_eq!(body["model"], json!("mistral-large-4"));
            assert_eq!(body["reasoning_effort"], json!("high"));
            assert_eq!(body["max_tokens"], json!(2000));
            assert!(body.get("max_completion_tokens").is_none());
        }
    }

    /// A graded effort Mistral Large 4 does not take becomes its one "on"
    /// grade instead of a 400; an unprofiled model's grade passes through.
    #[test]
    fn unsupported_grades_snap_to_the_offered_one() {
        for (model, requested, sent) in [
            (
                "mistral-large-4",
                ReasoningEffort::Low,
                Some(ReasoningEffort::High),
            ),
            (
                "mistral-large-4",
                ReasoningEffort::Medium,
                Some(ReasoningEffort::High),
            ),
            (
                "mistral-large-4-0",
                ReasoningEffort::Xhigh,
                Some(ReasoningEffort::High),
            ),
            (
                "mistral-large-4",
                ReasoningEffort::High,
                Some(ReasoningEffort::High),
            ),
            (
                "mistral-large-4",
                ReasoningEffort::None,
                Some(ReasoningEffort::None),
            ),
            (
                "magistral-medium-latest",
                ReasoningEffort::Medium,
                Some(ReasoningEffort::Medium),
            ),
        ] {
            let mut config = LlmCallConfig::new(model);
            config.reasoning_effort = Some(requested);
            assert_eq!(
                supported_effort(&config).reasoning_effort,
                sent,
                "{model} {requested:?}"
            );
        }
    }

    /// Effort "none" is Mistral's default and is not sent.
    #[tokio::test]
    async fn reasoning_off_omits_the_effort_field() {
        let server = MockServer::start().await;
        mock_chat(&server).await;
        let mut config = reasoning_config();
        config.reasoning_effort = Some(ReasoningEffort::None);
        let stream = provider("mistral", "synthetic-key")
            .base_url(format!("{}/v1", server.uri()))
            .chat_completion_stream(vec![Message::text(MessageRole::User, "hi")], &config)
            .await
            .unwrap();
        drain(stream).await;
        let requests = server.received_requests().await.unwrap();
        assert!(
            requests[0]
                .body_json::<Value>()
                .unwrap()
                .get("reasoning_effort")
                .is_none()
        );
    }

    /// Discovery keeps one entry per chat model (aliases repeat the canonical
    /// `name`), drops non-chat services and scheduled retirements, and maps
    /// the advertised capabilities. Entries are trimmed from Mistral's live
    /// `/v1/models`.
    #[tokio::test]
    async fn discovery_keeps_canonical_chat_models_with_their_capabilities() {
        let server = MockServer::start().await;
        let chat = |id: &str, name: &str, vision: bool, reasoning: bool| {
            json!({"id": id, "object": "model", "created": 1791305626, "owned_by": "mistralai",
                   "name": name, "max_context_length": 524288, "deprecation": null,
                   "capabilities": {"completion_chat": true, "function_calling": true,
                                    "vision": vision, "reasoning": reasoning}})
        };
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer synthetic-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"object": "list", "data": [
                    chat("mistral-large-4", "mistral-large-4", true, true),
                    chat("mistral-large-4-0", "mistral-large-4", true, true),
                    chat("ministral-8b-2512", "ministral-8b-2512", false, false),
                    {"id": "mistral-embed", "name": "mistral-embed-2312",
                     "capabilities": {"completion_chat": false}},
                    {"id": "old-model", "name": "old-model", "deprecation": "2026-11-30T12:00:00Z",
                     "capabilities": {"completion_chat": true}},
                ]}),
            ))
            .expect(1)
            .mount(&server)
            .await;

        let service = provider("mistral", "synthetic-key");
        let models = list_mistral_models(
            &reqwest::Client::new(),
            service.endpoint(),
            &format!("{}/v1/models", server.uri()),
        )
        .await
        .unwrap()
        .unwrap();
        let ids: Vec<_> = models.iter().map(|m| m.model_id.as_str()).collect();
        assert_eq!(ids, ["mistral-large-4", "ministral-8b-2512"]);

        let large = models[0].discovered_profile.as_ref().unwrap();
        assert!(large.reasoning && large.tool_call && large.attachment);
        assert_eq!(
            large.modalities.as_ref().unwrap().input,
            vec![Modality::Text, Modality::Image]
        );
        assert_eq!(large.limits.as_ref().unwrap().context, 524_288);
        assert_eq!(models[0].owned_by.as_deref(), Some("mistralai"));
        let small = models[1].discovered_profile.as_ref().unwrap();
        assert!(!small.reasoning && !small.attachment);
    }

    /// A base URL that is not Mistral's own host is never probed.
    #[tokio::test]
    async fn discovery_declines_a_host_that_is_not_mistral() {
        for url in [
            "https://proxy.example/v1",
            "https://mistral.ai/v1",
            "https://api.mistral.ai.evil.example/v1",
        ] {
            let service = provider("gate", "synthetic-key").base_url(url);
            assert!(
                MistralChatDriver::new()
                    .list_models(service.endpoint())
                    .await
                    .unwrap()
                    .is_none(),
                "{url}"
            );
        }
    }
}
