// Microsoft MAI Chat Driver
//
// Microsoft MAI models (e.g. MAI-Code-1-Flash) are served via Azure AI Foundry
// behind an OpenAI-compatible Chat Completions API. This driver wraps the core
// `OpenAIProtocolChatDriver` and supplies a pluggable, OAuth-capable auth layer
// (see `auth.rs`) through the driver's `AuthHeaderProvider` hook.
//
// Because authentication is handled by the auth provider, the underlying
// protocol driver's own `api_key` field is unused (constructed empty).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::TimeZone;
use serde::Deserialize;

use everruns_core::OpenAIProtocolChatDriver;
use everruns_core::credential_schema::{CredentialFormSchema, FieldType, FormField};
use everruns_core::driver_registry::{
    BoxedChatDriver, ChatDriver, DiscoveredModel, DriverConfig, DriverDescriptor, DriverId,
    DriverRegistry, LlmCallConfig, LlmMessage, LlmResponseStream,
};
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::openai_protocol::{
    AuthHeaderProvider, is_azure_openai_api_url, models_api_status_error, models_url_for_api_url,
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
        /// Retained so model discovery can authenticate the `/models` request
        /// with the same scheme (api-key or OAuth bearer) as chat requests.
        auth: Arc<dyn AuthHeaderProvider>,
    },
    Misconfigured(String),
}

impl MaiChatDriver {
    /// Create a driver from an explicit auth strategy and Azure AI Foundry
    /// endpoint. The endpoint is normalized to the chat-completions URL.
    pub fn new(auth: MaiAuth, endpoint: impl Into<String>) -> Self {
        let api_url = normalize_mai_url(&endpoint.into());
        let auth = auth.into_provider();
        let inner =
            OpenAIProtocolChatDriver::with_base_url("", &api_url).with_auth_provider(auth.clone());
        Self {
            state: DriverState::Ready {
                inner,
                api_url,
                auth,
            },
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

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        // MAI is OpenAI-compatible Chat Completions; the inner protocol driver
        // maps the preference onto the wire when the provider is configured.
        match self.ready() {
            Ok(inner) => inner.supports_parallel_tool_calls(model),
            Err(_) => false,
        }
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        let DriverState::Ready {
            inner,
            api_url,
            auth,
        } = &self.state
        else {
            // Misconfigured providers have nothing to discover.
            return Ok(None);
        };

        // Only run discovery against recognized Azure AI Foundry hosts. Custom
        // proxy URLs may resolve to private infrastructure, so they are skipped
        // (mirrors the OpenAI/Azure OpenAI driver gating).
        if !is_azure_openai_api_url(api_url) {
            return Ok(None);
        }

        list_foundry_models(
            inner.client(),
            auth.as_ref(),
            &models_url_for_api_url(api_url),
        )
        .await
    }
}

/// Fetch the Foundry `/models` catalog and map it to [`DiscoveredModel`]s.
///
/// Foundry's OpenAI-compatible `/models` endpoint is *bare* (id/created/owned_by
/// only — no capabilities, limits, or cost), exactly like OpenAI's. So
/// discovery returns `discovered_profile: None` and relies on the built-in model
/// profile registry to supply capabilities by matching the model id at sync
/// time. (Azure deployment names are operator-chosen; a deployment id that does
/// not match a known profile falls back to a minimal profile — the same caveat
/// as Azure OpenAI.)
///
/// The request is authenticated with the same `AuthHeaderProvider` used for chat
/// (`api-key` or an Entra ID OAuth bearer), so discovery works for both schemes.
async fn list_foundry_models(
    client: &reqwest::Client,
    auth: &dyn AuthHeaderProvider,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let (header_name, header_value) = auth.auth_header().await?;
    let response = client
        .get(models_url)
        .header(header_name, header_value)
        .send()
        .await
        .map_err(|e| AgentLoopError::llm(format!("Failed to fetch MAI models: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let _ = response.bytes().await; // drain body to allow connection reuse
        // Project-scoped Azure AI Foundry endpoints expose chat completions but
        // not a `/models` catalog: such an endpoint returns 404 for
        // `/openai/v1/models` while `/openai/v1/chat/completions` works. Treat a
        // missing/unimplemented listing endpoint as "discovery not supported"
        // (Ok(None)) rather than a hard error, so model sync degrades gracefully
        // instead of reporting a spurious failure. (Verified live against a
        // project endpoint.)
        if matches!(
            status,
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::NOT_IMPLEMENTED
        ) {
            return Ok(None);
        }
        return Err(models_api_status_error(status));
    }

    let models: FoundryModelsResponse = response
        .json()
        .await
        .map_err(|e| AgentLoopError::llm(format!("Failed to parse MAI models response: {e}")))?;

    let discovered = models
        .data
        .into_iter()
        .filter(FoundryModelInfo::is_chat_model)
        .map(|m| DiscoveredModel {
            created_at: m
                .created
                .and_then(|ts| chrono::Utc.timestamp_opt(ts, 0).single()),
            display_name: None,
            owned_by: m.owned_by,
            model_id: m.id,
            discovered_profile: None,
        })
        .collect();

    Ok(Some(discovered))
}

/// Bare Foundry/OpenAI-compatible `/models` list response.
#[derive(Debug, Deserialize)]
struct FoundryModelsResponse {
    data: Vec<FoundryModelInfo>,
}

/// One entry from the Foundry `/models` list. `created`/`owned_by` are optional
/// because Foundry variants do not always populate them.
#[derive(Debug, Deserialize)]
struct FoundryModelInfo {
    id: String,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    owned_by: Option<String>,
}

impl FoundryModelInfo {
    /// Whether this deployment is a chat/completion model. Foundry serves many
    /// model families (MAI, Llama, Phi, ...) whose ids do not share a prefix, so
    /// the filter is exclusion-based: drop obvious non-chat services (embeddings,
    /// speech, image, rerank) and keep everything else.
    fn is_chat_model(&self) -> bool {
        let id = self.id.to_ascii_lowercase();
        !(id.contains("embed")
            || id.contains("whisper")
            || id.starts_with("tts")
            || id.contains("-tts")
            || id.contains("text-to-speech")
            || id.contains("speech")
            || id.contains("dall-e")
            || id.contains("-image")
            || id.contains("image-")
            || id.contains("rerank"))
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
    } else if let Some(base) = trimmed.strip_suffix("/models") {
        // A models-listing URL was pasted as the endpoint; derive the sibling
        // chat endpoint by replacing `/models` rather than appending to it.
        format!("{base}/chat/completions")
    } else if trimmed.ends_with("/v1") || trimmed.ends_with("/openai/v1") {
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
                label: "API Key or Entra ID OAuth JSON".to_string(),
                field_type: FieldType::Password,
                required: true,
                placeholder: None,
                help_text: Some(
                    "An Azure AI Foundry resource key, or a Microsoft Entra ID OAuth JSON \
                     document: {\"tenant_id\":\"…\",\"client_id\":\"…\",\"client_secret\":\"…\"}."
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
             (client-credentials) by entering a JSON document with `tenant_id`, `client_id`, \
             and `client_secret` in the credential field."
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
    use everruns_core::driver_registry::{ProviderConfig, ProviderMetadata, ServiceKind};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn chat_model_filter_keeps_chat_excludes_embeddings_and_media() {
        let chat = |id: &str| FoundryModelInfo {
            id: id.to_string(),
            created: None,
            owned_by: None,
        };
        assert!(chat("mai-code-1-flash").is_chat_model());
        assert!(chat("mai-1-preview").is_chat_model());
        assert!(chat("Phi-4").is_chat_model());
        assert!(!chat("text-embedding-3-large").is_chat_model());
        assert!(!chat("whisper-large").is_chat_model());
        assert!(!chat("tts-1").is_chat_model());
        assert!(!chat("dall-e-3").is_chat_model());
        assert!(!chat("cohere-rerank-v3").is_chat_model());
    }

    #[tokio::test]
    async fn list_models_skips_non_azure_hosts() {
        // A custom/proxy (non-Foundry) host must not be probed for discovery.
        let driver = MaiChatDriver::new(MaiAuth::ApiKey("k".into()), "https://proxy.example.com");
        assert!(driver.list_models().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn discovery_fetch_authenticates_filters_and_maps() {
        // `list_foundry_models` is exercised directly so the Azure-host gate (which
        // a wiremock 127.0.0.1 host cannot satisfy) does not block the HTTP path.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/openai/v1/models"))
            .and(header("api-key", "foundry-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [
                    { "id": "mai-code-1-flash", "object": "model", "created": 1_700_000_000, "owned_by": "microsoft" },
                    { "id": "text-embedding-3-large", "object": "model" },
                ],
            })))
            .mount(&server)
            .await;

        let auth = MaiAuth::ApiKey("foundry-secret".into()).into_provider();
        let models_url = format!("{}/openai/v1/models", server.uri());
        let discovered = list_foundry_models(&reqwest::Client::new(), auth.as_ref(), &models_url)
            .await
            .expect("discovery request should succeed")
            .expect("discovery should return a model list");

        // Embedding model filtered out; chat model retained with bare metadata
        // (no discovered_profile — profiles come from the registry by id).
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].model_id, "mai-code-1-flash");
        assert_eq!(discovered[0].owned_by.as_deref(), Some("microsoft"));
        assert!(discovered[0].discovered_profile.is_none());
        assert!(discovered[0].created_at.is_some());
    }

    #[tokio::test]
    async fn discovery_treats_missing_models_endpoint_as_unsupported() {
        // Project-scoped Foundry endpoints 404 on /openai/v1/models while chat
        // works; discovery must degrade to Ok(None), not a hard error, so model
        // sync does not report a spurious failure. (Mirrors live behavior.)
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/openai/v1/models"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let auth = MaiAuth::ApiKey("k".into()).into_provider();
        let models_url = format!("{}/openai/v1/models", server.uri());
        let result = list_foundry_models(&reqwest::Client::new(), auth.as_ref(), &models_url)
            .await
            .expect("404 on /models should not be a hard error");
        assert!(
            result.is_none(),
            "missing /models endpoint should be Ok(None)"
        );
    }

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
    fn models_url_is_rewritten_to_chat_completions() {
        // A pasted models-listing URL must derive the sibling chat endpoint,
        // not append to `/models`.
        assert_eq!(
            normalize_mai_url("https://res.services.ai.azure.com/openai/v1/models"),
            "https://res.services.ai.azure.com/openai/v1/chat/completions"
        );
        assert_eq!(
            normalize_mai_url("https://res.services.ai.azure.com/api/projects/p/openai/v1/models/"),
            "https://res.services.ai.azure.com/api/projects/p/openai/v1/chat/completions"
        );
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
            parallel_tool_calls: None,
        }
    }
}
