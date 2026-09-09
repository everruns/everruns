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
    let Ok(mut url) = reqwest::Url::parse(&base_url) else {
        return base_url;
    };
    // Only normalize the path; query parameters may carry required API routing/version values.
    let path = url.path().trim_end_matches('/');
    let path = path.strip_suffix("/chat/completions").unwrap_or(path);
    let path = if path.ends_with("/openai/v1") {
        path.to_string()
    } else {
        format!("{path}/openai/v1")
    };
    url.set_path(&path);
    url.to_string()
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
            &self.inner.client(),
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
    use everruns_provider::driver_registry::{
        LlmMessageRole, ProviderConfig, ProviderMetadata, ServiceKind,
    };
    use serde_json::{Value, json};
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    fn config() -> LlmCallConfig {
        LlmCallConfig {
            model: "mai-code-1-flash".into(),
            temperature: Some(0.25),
            max_tokens: Some(64),
            tools: vec![],
            reasoning_effort: None,
            reasoning_state: None,
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
    async fn direct_and_registered_chat_preserve_endpoint_components_and_auth() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);
        let descriptor = registry.descriptor(&DriverId::Mai).unwrap();
        assert_eq!(descriptor.display_name, "Microsoft MAI");
        assert_eq!(descriptor.services, vec![ServiceKind::Chat]);
        assert_eq!(
            provider(
                "mai",
                "https://res.services.ai.azure.com/openai/v1",
                MaiAuth::ApiKey("key".into())
            )
            .endpoint()
            .url("chat/completions")
            .as_deref(),
            Some("https://res.services.ai.azure.com/openai/v1/chat/completions")
        );
        for suffix in [
            "/project",
            "/project/openai/v1/",
            "/project/openai/v1/chat/completions",
        ] {
            for registered in [false, true] {
                let server = MockServer::builder().start().await;
                Mock::given(method("POST")).and(path("/project/openai/v1/chat/completions")).and(query_param("api-version","preview")).and(query_param("route","a b")).and(header("api-key","synthetic-key")).respond_with(ResponseTemplate::new(200).insert_header("content-type","text/event-stream").set_body_string("data: {\"id\":\"mai-response\",\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2}}\n\ndata: [DONE]\n\n")).expect(1).mount(&server).await;
                let url = format!("{}{suffix}?api-version=preview&route=a%20b", server.uri());
                let messages = vec![
                    LlmMessage::text(LlmMessageRole::System, "rules"),
                    LlmMessage::text(LlmMessageRole::User, "question"),
                ];
                let response = if registered {
                    registry
                        .create_chat_driver(
                            &ProviderConfig::new(DriverId::Mai)
                                .with_api_key("synthetic-key")
                                .with_base_url(url),
                        )
                        .unwrap()
                        .chat_completion(&ProviderEndpoint::default(), messages, &config())
                        .await
                } else {
                    provider("direct", url, MaiAuth::ApiKey("synthetic-key".into()))
                        .chat_completion(messages, &config())
                        .await
                }
                .unwrap();
                assert_eq!(response.text, "answer");
                assert!(response.tool_calls.is_none());
                assert!(response.reasoning.is_empty());
                assert_eq!(
                    response.metadata.response_id.as_deref(),
                    Some("mai-response")
                );
                assert_eq!(response.metadata.finish_reason.as_deref(), Some("stop"));
                assert_eq!(
                    (
                        response.metadata.prompt_tokens,
                        response.metadata.completion_tokens,
                        response.metadata.total_tokens
                    ),
                    (Some(10), Some(2), Some(12))
                );
                let requests = server.received_requests().await.unwrap();
                assert_eq!(requests.len(), 1);
                assert!(requests[0].headers.get("authorization").is_none());
                assert_eq!(
                    requests[0].body_json::<Value>().unwrap(),
                    json!({"model":"mai-code-1-flash","messages":[{"role":"system","content":"rules"},{"role":"user","content":"question"}],"temperature":0.25,"max_tokens":64,"parallel_tool_calls":false,"stream":true,"stream_options":{"include_usage":true}})
                );
            }
        }
        let server = MockServer::builder().start().await;
        for (config, expected) in [
            (
                ProviderConfig::new(DriverId::Mai),
                "Provider credentials are required",
            ),
            (
                ProviderConfig::new(DriverId::Mai)
                    .with_api_key("fallback-key")
                    .with_metadata(ProviderMetadata {
                        extra: Some(json!({"tenant_id":"tenant","client_id":"client"})),
                        ..Default::default()
                    }),
                "Client secret (Microsoft Entra ID OAuth) is required",
            ),
        ] {
            let driver = registry
                .create_chat_driver(&config.with_base_url(server.uri()))
                .unwrap();
            let error = driver
                .chat_completion(&ProviderEndpoint::default(), vec![], &self::config())
                .await
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
        let oauth = registry.create_chat_driver(
            &ProviderConfig::new(DriverId::Mai).with_base_url(server.uri()).with_metadata(ProviderMetadata {
                extra: Some(json!({"tenant_id":"tenant","client_id":"client","client_secret":"secret"})),
                ..Default::default()
            })
        ).unwrap();
        // A valid OAuth-only configuration passes the credential gate for unsupported discovery without minting a token.
        assert!(
            oauth
                .list_models(&ProviderEndpoint::default())
                .await
                .unwrap()
                .is_none()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn discovery_rejects_non_azure_and_lookalike_hosts_before_auth() {
        use everruns_provider::runtime_provider::{ProviderAuth, ProviderAuthRequest};
        struct ForbiddenAuth;
        #[async_trait]
        impl ProviderAuth for ForbiddenAuth {
            async fn headers(&self, _: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>> {
                panic!("disallowed discovery accessed credentials")
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
        for url in [
            None,
            Some("https://proxy.example"),
            Some("https://res.services.ai.azure.com.evil.example"),
            Some("https://res.openai.azure.com@evil.example"),
            Some("https://evil.example/res.openai.azure.com"),
            Some("not a URL"),
        ] {
            let service = Provider::new("gate", MaiChatDriver::new()).auth(ForbiddenAuth);
            let service = if let Some(url) = url {
                service.base_url(url)
            } else {
                service
            };
            assert!(service.list_models().await.unwrap().is_none(), "{url:?}");
        }
    }
    #[tokio::test]
    async fn discovery_filters_full_catalog_and_preserves_optional_metadata() {
        let server = MockServer::builder().start().await;
        let mut data = vec![
            json!({"id":"mai-code-1-flash","created":0,"owned_by":"microsoft"}),
            json!({"id":"mai-1-preview","created":9223372036854775807_i64}),
            json!({"id":"Phi-4"}),
        ];
        data.extend(
            [
                "text-embedding-3-large",
                "WHISPER-large",
                "tts-1",
                "voice-tts",
                "text-to-speech",
                "speech-model",
                "dall-e-3",
                "model-image",
                "image-model",
                "cohere-rerank-v3",
            ]
            .map(|id| json!({"id":id})),
        );
        Mock::given(method("GET"))
            .and(path("/openai/v1/models"))
            .and(header("api-key", "synthetic-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":data})))
            .expect(1)
            .mount(&server)
            .await;
        let service = provider("mai", server.uri(), MaiAuth::ApiKey("synthetic-key".into()));
        let models = list_foundry_models(
            &reqwest::Client::new(),
            service.endpoint(),
            &format!("{}/openai/v1/models", server.uri()),
        )
        .await
        .unwrap()
        .unwrap();
        let actual:Vec<_>=models.into_iter().map(|m|json!({"id":m.model_id,"name":m.display_name,"created":m.created_at.map(|t|t.to_rfc3339()),"owner":m.owned_by,"capabilities":m.capabilities,"profile":m.discovered_profile})).collect();
        assert_eq!(
            actual,
            vec![
                json!({"id":"mai-code-1-flash","name":null,"created":"1970-01-01T00:00:00+00:00","owner":"microsoft","capabilities":["chat"],"profile":null}),
                json!({"id":"mai-1-preview","name":null,"created":null,"owner":null,"capabilities":["chat"],"profile":null}),
                json!({"id":"Phi-4","name":null,"created":null,"owner":null,"capabilities":["chat"],"profile":null})
            ]
        );
    }
    #[tokio::test]
    async fn discovery_fallback_is_limited_to_missing_or_unimplemented_catalogs() {
        for status in [404, 501, 401, 500] {
            let server = MockServer::builder().start().await;
            Mock::given(method("GET"))
                .and(path("/openai/v1/models"))
                .and(header("api-key", "synthetic-key"))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&server)
                .await;
            let service = provider("mai", server.uri(), MaiAuth::ApiKey("synthetic-key".into()));
            let result = list_foundry_models(
                &reqwest::Client::new(),
                service.endpoint(),
                &format!("{}/openai/v1/models", server.uri()),
            )
            .await;
            if matches!(status, 404 | 501) {
                assert!(result.unwrap().is_none());
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains(&status.to_string())
                );
            }
        }
    }
}
