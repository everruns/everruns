// Microsoft MAI Chat Driver
//
// Microsoft MAI models (e.g. MAI-Code-1-Flash) are served via Azure AI Foundry
// behind an OpenAI-compatible Chat Completions API. This driver wraps the core
// `OpenAIProtocolChatDriver`; the runtime provider owns the OAuth-capable auth
// layer from `auth.rs`.

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
use everruns_provider::openai_protocol::{is_azure_openai_api_url, models_url_for_api_url};
use everruns_provider::{Provider, ProviderEndpoint};

use crate::auth::{DEFAULT_ENTRA_AUTHORITY, DEFAULT_ENTRA_SCOPE, MaiAuth, failing_provider};

/// Ready-to-use Microsoft MAI provider assembly.
pub fn provider(
    id: impl Into<everruns_provider::ProviderKey>,
    base_url: impl Into<String>,
    auth: MaiAuth,
) -> Provider {
    Provider::new(id, MaiChatDriver::new())
        .base_url(mai_api_base_url(base_url.into()))
        .auth_arc(auth.into_provider())
}

fn mai_api_base_url(base_url: String) -> String {
    let base_url = base_url.trim_end_matches('/');
    if base_url.ends_with("/openai/v1") {
        base_url.to_string()
    } else {
        format!("{base_url}/openai/v1")
    }
}

/// Microsoft MAI chat driver (Azure AI Foundry, OpenAI-compatible).
///
/// Construct directly for programmatic use, or let [`register_driver`] build
/// instances from the transitional descriptor catalog.
///
/// # Example
///
/// ```
/// use everruns_mai::{MaiAuth, provider};
///
/// let service = provider(
///     "mai-prod",
///     "https://my-resource.services.ai.azure.com/openai/v1",
///     MaiAuth::ApiKey("foundry-key".into()),
/// );
/// assert_eq!(
///     service.endpoint().url("chat/completions").unwrap(),
///     "https://my-resource.services.ai.azure.com/openai/v1/chat/completions",
/// );
/// ```
pub struct MaiChatDriver {
    inner: OpenAIProtocolChatDriver,
}

impl MaiChatDriver {
    /// Create an Azure AI Foundry Chat Completions wire driver.
    pub fn new() -> Self {
        Self {
            inner: OpenAIProtocolChatDriver::new(),
        }
    }
}

#[async_trait]
impl ChatDriver for MaiChatDriver {
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
        // MAI is OpenAI-compatible Chat Completions; the inner protocol driver
        // maps the preference onto the wire when the provider is configured.
        self.inner.supports_parallel_tool_calls(model)
    }

    async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        let Some(api_url) = endpoint.url("chat/completions") else {
            return Ok(None);
        };

        // Only run discovery against recognized Azure AI Foundry hosts. Custom
        // proxy URLs may resolve to private infrastructure, so they are skipped
        // (mirrors the OpenAI/Azure OpenAI driver gating).
        if !is_azure_openai_api_url(&api_url) {
            return Ok(None);
        }

        list_foundry_models(
            self.inner.client(),
            endpoint,
            &models_url_for_api_url(&api_url),
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
/// The request is authenticated with the same runtime `ProviderAuth` used for chat
/// (`api-key` or an Entra ID OAuth bearer), so discovery works for both schemes.
async fn list_foundry_models(
    client: &reqwest::Client,
    endpoint: &ProviderEndpoint,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let resolved = endpoint.resolve("GET", models_url, &[]).await?;
    let mut request = client.get(&resolved.url);
    for (name, value) in resolved.headers {
        request = request.header(name, value);
    }
    // Project-scoped Azure AI Foundry endpoints expose chat completions but not a
    // `/models` catalog: such an endpoint returns 404 for `/openai/v1/models`
    // while `/openai/v1/chat/completions` works. Treat a missing/unimplemented
    // listing endpoint as "discovery not supported" (Ok(None)) rather than a hard
    // error, so model sync degrades gracefully instead of reporting a spurious
    // failure. (Verified live against a project endpoint.)
    fetch_models::<FoundryModelsResponse, _>(
        request,
        "Failed to fetch MAI models",
        "Failed to parse MAI models response",
        &[
            reqwest::StatusCode::NOT_FOUND,
            reqwest::StatusCode::NOT_IMPLEMENTED,
        ],
        |models| {
            models
                .data
                .into_iter()
                .filter(FoundryModelInfo::is_chat_model)
                .map(|m| DiscoveredModel {
                    capabilities: vec!["chat".to_string()],
                    created_at: m
                        .created
                        .and_then(|ts| chrono::Utc.timestamp_opt(ts, 0).single()),
                    display_name: None,
                    owned_by: m.owned_by,
                    model_id: m.id,
                    discovered_profile: None,
                })
                .collect()
        },
    )
    .await
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
            .field("api", &"Azure AI Foundry (Chat Completions)")
            .finish()
    }
}

/// Credential schema for the MAI driver: two mutually-exclusive credential
/// methods rendered as discrete fields — an Azure AI Foundry API key, or
/// first-class Microsoft Entra ID OAuth (client-credentials) fields. The
/// resource endpoint is the provider's first-class `base_url`, configured
/// separately rather than as a credential field.
fn mai_credential_schema() -> CredentialFormSchema {
    const API_KEY_GROUP: &str = "API key";
    const OAUTH_GROUP: &str = "Microsoft Entra ID OAuth";
    CredentialFormSchema {
        fields: vec![
            FormField::password("api_key", "Azure AI Foundry API Key")
                .required()
                .in_group(API_KEY_GROUP)
                .with_help("A resource key from your Azure AI Foundry deployment."),
            FormField::text("tenant_id", "Directory (tenant) ID")
                .required()
                .in_group(OAUTH_GROUP),
            FormField::text("client_id", "Application (client) ID")
                .required()
                .in_group(OAUTH_GROUP),
            FormField::password("client_secret", "Client secret")
                .required()
                .in_group(OAUTH_GROUP),
            FormField::text("scope", "Scope")
                .in_group(OAUTH_GROUP)
                .with_default(DEFAULT_ENTRA_SCOPE)
                .with_placeholder(DEFAULT_ENTRA_SCOPE)
                .with_help("Defaults to the Azure Cognitive Services scope."),
            FormField::text("authority", "Authority")
                .in_group(OAUTH_GROUP)
                .with_default(DEFAULT_ENTRA_AUTHORITY)
                .with_placeholder(DEFAULT_ENTRA_AUTHORITY)
                .with_help("Microsoft Entra authority host."),
        ],
        instructions_markdown:
            "Configure a Microsoft MAI deployment on [Azure AI Foundry](https://ai.azure.com), \
             then set the **Base URL** to your resource endpoint \
             (e.g. `https://<resource>.services.ai.azure.com`). Authenticate with the resource \
             **API key**, or with **Microsoft Entra ID OAuth** (client-credentials) by entering \
             the tenant, client id, and client secret."
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
/// use everruns_provider::DriverRegistry;
/// use everruns_mai::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// assert!(registry.has_driver(&everruns_provider::DriverId::Mai));
/// ```
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_descriptor(DriverDescriptor {
        display_name: "Microsoft MAI".into(),
        credential_schema: mai_credential_schema(),
        ..DriverDescriptor::chat_only(DriverId::Mai, |config| {
            let provider = Provider::new(config.provider.clone(), MaiChatDriver::new()).base_url(
                mai_api_base_url(config.base_url.clone().unwrap_or_default()),
            );
            match MaiAuth::from_driver_config(config) {
                Ok(auth) => provider.auth_arc(auth.into_provider()).into_boxed_driver(),
                Err(error) => provider
                    .auth_arc(failing_provider(error))
                    .into_boxed_driver(),
            }
        })
    });
}

impl Default for MaiChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::driver_registry::{ProviderConfig, ProviderMetadata, ServiceKind};
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
        let service = provider(
            "proxy",
            "https://proxy.example.com",
            MaiAuth::ApiKey("k".into()),
        );
        let driver = MaiChatDriver::new();
        assert!(
            driver
                .list_models(service.endpoint())
                .await
                .unwrap()
                .is_none()
        );
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

        let service = provider(
            "mai-test",
            format!("{}/openai/v1", server.uri()),
            MaiAuth::ApiKey("foundry-secret".into()),
        );
        let models_url = format!("{}/openai/v1/models", server.uri());
        let discovered =
            list_foundry_models(&reqwest::Client::new(), service.endpoint(), &models_url)
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

        let service = provider(
            "mai-test",
            format!("{}/openai/v1", server.uri()),
            MaiAuth::ApiKey("k".into()),
        );
        let models_url = format!("{}/openai/v1/models", server.uri());
        let result = list_foundry_models(&reqwest::Client::new(), service.endpoint(), &models_url)
            .await
            .expect("404 on /models should not be a hard error");
        assert!(
            result.is_none(),
            "missing /models endpoint should be Ok(None)"
        );
    }

    #[test]
    fn ready_provider_exposes_protocol_url() {
        let service = provider(
            "mai",
            "https://res.services.ai.azure.com/openai/v1",
            MaiAuth::ApiKey("k".into()),
        );
        assert_eq!(
            service.endpoint().url("chat/completions").as_deref(),
            Some("https://res.services.ai.azure.com/openai/v1/chat/completions")
        );
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
}
