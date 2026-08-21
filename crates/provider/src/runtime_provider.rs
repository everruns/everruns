//! Runtime providers: service identity, endpoint, authentication, and wire driver.
//!
//! A [`ChatDriver`](crate::driver_registry::ChatDriver) implements a wire
//! protocol. A `Provider` is a configured service that speaks that protocol.
//! Keeping credentials and endpoints here lets one driver serve any number of
//! services without adding vendor branches to the runtime.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::driver_registry::{BoxedChatDriver, ChatDriver};
use crate::error::Result;

/// Open, normalized identity used by model specifications to select a provider.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderKey(String);

impl ProviderKey {
    pub fn new(id: impl AsRef<str>) -> Self {
        Self(id.as_ref().trim().to_ascii_lowercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ProviderKey {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ProviderKey {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl Serialize for ProviderKey {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ProviderKey {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

/// Immutable request material available to an authentication implementation.
///
/// `body` is the exact serialized payload. This makes the contract suitable
/// for request signatures such as AWS SigV4 as well as ordinary header auth.
pub struct ProviderAuthRequest<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: &'a [(String, String)],
    pub body: &'a [u8],
}

/// Resolves authentication for each outbound provider request.
#[async_trait]
pub trait ProviderAuth: Send + Sync {
    async fn headers(&self, request: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>>;
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Static `Authorization: Bearer …` authentication.
pub struct BearerAuth {
    key: String,
}

impl BearerAuth {
    pub fn new(key: impl Into<String>) -> Self {
        Self { key: key.into() }
    }
}

#[async_trait]
impl ProviderAuth for BearerAuth {
    async fn headers(&self, _request: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>> {
        Ok(vec![(
            "authorization".to_string(),
            format!("Bearer {}", self.key),
        )])
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Static authentication carried in one named header.
pub struct StaticHeaderAuth {
    name: String,
    value: String,
}

impl StaticHeaderAuth {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into().to_ascii_lowercase(),
            value: value.into(),
        }
    }
}

#[async_trait]
impl ProviderAuth for StaticHeaderAuth {
    async fn headers(&self, _request: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>> {
        Ok(vec![(self.name.clone(), self.value.clone())])
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Endpoint and authentication policy handed to a wire driver.
///
/// This runtime value is intentionally not serializable. Its `Debug` output
/// exposes only header names and whether authentication is configured.
#[derive(Clone, Default)]
pub struct ProviderEndpoint {
    base_url: Option<String>,
    headers: Vec<(String, String)>,
    auth: Option<Arc<dyn ProviderAuth>>,
}

impl ProviderEndpoint {
    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    pub fn url(&self, path: &str) -> Option<String> {
        self.base_url.as_ref().map(|base| {
            let base = base.trim_end_matches('/');
            if path.is_empty() || base.ends_with(path) {
                base.to_string()
            } else {
                format!("{base}/{}", path.trim_start_matches('/'))
            }
        })
    }

    pub async fn resolve(
        &self,
        method: &str,
        url: impl Into<String>,
        body: &[u8],
    ) -> Result<ResolvedProviderRequest> {
        let url = url.into();
        let mut headers = self.headers.clone();
        if let Some(auth) = &self.auth {
            let auth_headers = auth
                .headers(ProviderAuthRequest {
                    method,
                    url: &url,
                    headers: &headers,
                    body,
                })
                .await?;
            for (name, value) in auth_headers {
                headers.retain(|(existing, _)| !existing.eq_ignore_ascii_case(&name));
                headers.push((name.to_ascii_lowercase(), value));
            }
        }
        Ok(ResolvedProviderRequest { url, headers })
    }

    /// Access a provider-owned authentication implementation needed by a
    /// protocol whose signing stack cannot be represented as header strings.
    pub fn auth<T: ProviderAuth + 'static>(&self) -> Option<&T> {
        self.auth.as_deref()?.as_any().downcast_ref()
    }
}

impl fmt::Debug for ProviderEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderEndpoint")
            .field("base_url", &self.base_url.as_ref().map(|_| "<configured>"))
            .field("auth", &self.auth.as_ref().map(|_| "<configured>"))
            .field(
                "headers",
                &self
                    .headers
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Fully resolved outbound request metadata.
#[derive(Clone)]
pub struct ResolvedProviderRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl fmt::Debug for ResolvedProviderRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedProviderRequest")
            .field("url", &redacted_url(&self.url))
            .field(
                "headers",
                &self
                    .headers
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

fn redacted_url(value: &str) -> String {
    let Ok(mut url) = url::Url::parse(value) else {
        return "<configured>".to_string();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

/// Runtime service assembly over one reusable wire-protocol driver.
#[derive(Clone)]
pub struct RuntimeProvider {
    id: ProviderKey,
    driver: Arc<dyn ChatDriver>,
    endpoint: ProviderEndpoint,
}

/// Public application-facing name for a runtime provider.
pub type Provider = RuntimeProvider;

impl RuntimeProvider {
    pub fn new(id: impl Into<ProviderKey>, driver: impl ChatDriver + 'static) -> Self {
        Self::from_driver(id, Arc::new(driver))
    }

    pub fn from_driver(id: impl Into<ProviderKey>, driver: Arc<dyn ChatDriver>) -> Self {
        Self {
            id: id.into(),
            driver,
            endpoint: ProviderEndpoint::default(),
        }
    }

    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.endpoint.base_url = Some(url.into().trim_end_matches('/').to_string());
        self
    }

    pub fn auth(mut self, auth: impl ProviderAuth + 'static) -> Self {
        self.endpoint.auth = Some(Arc::new(auth));
        self
    }

    pub fn auth_arc(mut self, auth: Arc<dyn ProviderAuth>) -> Self {
        self.endpoint.auth = Some(auth);
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.endpoint
            .headers
            .push((name.into().to_ascii_lowercase(), value.into()));
        self
    }

    pub fn id(&self) -> &ProviderKey {
        &self.id
    }

    pub fn driver(&self) -> &Arc<dyn ChatDriver> {
        &self.driver
    }

    pub fn endpoint(&self) -> &ProviderEndpoint {
        &self.endpoint
    }

    pub async fn chat_completion_stream(
        &self,
        messages: Vec<crate::driver_registry::LlmMessage>,
        config: &crate::driver_registry::LlmCallConfig,
    ) -> Result<crate::driver_registry::LlmResponseStream> {
        let id = self.id.to_string();
        let stream = self
            .driver
            .chat_completion_stream(&self.endpoint, messages, config)
            .await
            .map_err(|error| error.with_provider(&id))?;
        Ok(Box::pin(stream.map(move |result| {
            result.map_err(|error| error.with_provider(&id))
        })))
    }

    pub async fn chat_completion(
        &self,
        messages: Vec<crate::driver_registry::LlmMessage>,
        config: &crate::driver_registry::LlmCallConfig,
    ) -> Result<crate::driver_registry::LlmResponse> {
        self.driver
            .chat_completion(&self.endpoint, messages, config)
            .await
            .map_err(|error| error.with_provider(self.id.as_str()))
    }

    pub async fn list_models(
        &self,
    ) -> Result<Option<Vec<crate::driver_registry::DiscoveredModel>>> {
        self.driver
            .list_models(&self.endpoint)
            .await
            .map_err(|error| error.with_provider(self.id.as_str()))
    }

    pub fn into_boxed_driver(self) -> BoxedChatDriver {
        Box::new(ProviderBoundDriver(self))
    }

    pub fn bind_embeddings(
        self,
        driver: crate::driver_registry::BoxedEmbeddingsDriver,
    ) -> crate::driver_registry::BoxedEmbeddingsDriver {
        Box::new(ProviderBoundEmbeddingsDriver {
            id: self.id,
            endpoint: self.endpoint,
            driver,
        })
    }
}

impl fmt::Debug for RuntimeProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Provider")
            .field("id", &self.id)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

struct ProviderBoundDriver(RuntimeProvider);

struct ProviderBoundEmbeddingsDriver {
    id: ProviderKey,
    endpoint: ProviderEndpoint,
    driver: crate::driver_registry::BoxedEmbeddingsDriver,
}

#[async_trait]
impl crate::driver_registry::EmbeddingsDriver for ProviderBoundEmbeddingsDriver {
    async fn embed(
        &self,
        _endpoint: &ProviderEndpoint,
        request: crate::driver_registry::EmbedRequest,
    ) -> std::result::Result<
        crate::driver_registry::EmbedResponse,
        crate::driver_registry::EmbeddingsDriverError,
    > {
        self.driver
            .embed(&self.endpoint, request)
            .await
            .map_err(|error| {
                crate::driver_registry::EmbeddingsDriverError::Provider(format!(
                    "provider '{}': {error}",
                    self.id
                ))
            })
    }
}

#[async_trait]
impl ChatDriver for ProviderBoundDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        messages: Vec<crate::driver_registry::LlmMessage>,
        config: &crate::driver_registry::LlmCallConfig,
    ) -> Result<crate::driver_registry::LlmResponseStream> {
        self.0.chat_completion_stream(messages, config).await
    }

    async fn list_models(
        &self,
        _endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<crate::driver_registry::DiscoveredModel>>> {
        self.0.list_models().await
    }

    fn supports_compact(&self) -> bool {
        self.0.driver.supports_compact()
    }

    fn supports_stateful_responses(&self) -> bool {
        self.0.driver.supports_stateful_responses()
    }

    fn effective_context_window(&self, model: &str) -> Option<usize> {
        self.0.driver.effective_context_window(model)
    }

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        self.0.driver.supports_parallel_tool_calls(model)
    }

    async fn compact(
        &self,
        _endpoint: &ProviderEndpoint,
        request: crate::compact::CompactRequest,
    ) -> Result<Option<crate::compact::CompactResponse>> {
        self.0
            .driver
            .compact(self.0.endpoint(), request)
            .await
            .map_err(|error| error.with_provider(self.0.id.as_str()))
    }
}

/// Runtime provider instances keyed by their open service identity.
#[derive(Clone, Default)]
pub struct RuntimeProviderRegistry {
    providers: HashMap<ProviderKey, Arc<RuntimeProvider>>,
}

/// Public registry name for runtime provider instances.
pub type ProviderRegistry = RuntimeProviderRegistry;

impl RuntimeProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, provider: RuntimeProvider) -> Result<()> {
        if self.providers.contains_key(provider.id()) {
            return Err(crate::error::AgentLoopError::Configuration(format!(
                "provider '{}' is already registered; use replace() to overwrite intentionally",
                provider.id()
            )));
        }
        self.providers
            .insert(provider.id.clone(), Arc::new(provider));
        Ok(())
    }

    pub fn replace(&mut self, provider: RuntimeProvider) -> Option<Arc<RuntimeProvider>> {
        self.providers
            .insert(provider.id.clone(), Arc::new(provider))
    }

    pub fn get(&self, id: &ProviderKey) -> Option<Arc<RuntimeProvider>> {
        self.providers.get(id).cloned()
    }

    pub fn ids(&self) -> Vec<String> {
        let mut ids = self
            .providers
            .keys()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }
}

impl fmt::Debug for RuntimeProviderRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderRegistry")
            .field("providers", &self.ids())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_key_deserialization_is_canonical() {
        let key: ProviderKey = serde_json::from_str(r#"" Gateway-PROD ""#).unwrap();
        assert_eq!(key.as_str(), "gateway-prod");
        assert_eq!(serde_json::to_string(&key).unwrap(), r#""gateway-prod""#);
    }

    #[test]
    fn debug_redacts_auth_and_header_values() {
        struct Noop;
        #[async_trait]
        impl ChatDriver for Noop {
            async fn chat_completion_stream(
                &self,
                _endpoint: &ProviderEndpoint,
                _messages: Vec<crate::driver_registry::LlmMessage>,
                _config: &crate::driver_registry::LlmCallConfig,
            ) -> Result<crate::driver_registry::LlmResponseStream> {
                unreachable!()
            }
        }

        let provider = RuntimeProvider::new("Gateway", Noop)
            .base_url("https://example.test/")
            .header("x-secret", "hidden-service-value")
            .auth(BearerAuth::new("hidden-key"));
        let debug = format!("{provider:?}");
        assert!(debug.contains("gateway"));
        assert!(debug.contains("x-secret"));
        assert!(!debug.contains("hidden-service-value"));
        assert!(!debug.contains("hidden-key"));
    }

    #[test]
    fn duplicate_registration_is_explicit() {
        struct Noop;
        #[async_trait]
        impl ChatDriver for Noop {
            async fn chat_completion_stream(
                &self,
                _endpoint: &ProviderEndpoint,
                _messages: Vec<crate::driver_registry::LlmMessage>,
                _config: &crate::driver_registry::LlmCallConfig,
            ) -> Result<crate::driver_registry::LlmResponseStream> {
                unreachable!()
            }
        }
        let mut registry = RuntimeProviderRegistry::new();
        registry.register(RuntimeProvider::new("a", Noop)).unwrap();
        assert!(registry.register(RuntimeProvider::new("A", Noop)).is_err());
        assert_eq!(registry.ids(), vec!["a"]);
    }

    #[tokio::test]
    async fn one_protocol_serves_distinct_provider_identities() {
        struct Noop;
        #[async_trait]
        impl ChatDriver for Noop {
            async fn chat_completion_stream(
                &self,
                _endpoint: &ProviderEndpoint,
                _messages: Vec<crate::driver_registry::LlmMessage>,
                _config: &crate::driver_registry::LlmCallConfig,
            ) -> Result<crate::driver_registry::LlmResponseStream> {
                unreachable!()
            }
        }

        let protocol: Arc<dyn ChatDriver> = Arc::new(Noop);
        let first = Provider::from_driver("first", protocol.clone())
            .base_url("https://first.example/v1")
            .header("x-service", "first")
            .auth(BearerAuth::new("first-key"));
        let second = Provider::from_driver("second", protocol.clone())
            .base_url("https://second.example/v1")
            .header("x-service", "second")
            .auth(BearerAuth::new("second-key"));

        assert!(Arc::ptr_eq(first.driver(), second.driver()));
        let first_request = first
            .endpoint()
            .resolve("POST", first.endpoint().url("chat").unwrap(), b"{}")
            .await
            .unwrap();
        let second_request = second
            .endpoint()
            .resolve("POST", second.endpoint().url("chat").unwrap(), b"{}")
            .await
            .unwrap();
        assert_ne!(first_request.url, second_request.url);
        assert_ne!(first_request.headers, second_request.headers);
    }

    #[tokio::test]
    async fn refreshable_auth_is_resolved_for_each_request() {
        struct Rotating(std::sync::atomic::AtomicUsize);
        #[async_trait]
        impl ProviderAuth for Rotating {
            async fn headers(
                &self,
                request: ProviderAuthRequest<'_>,
            ) -> Result<Vec<(String, String)>> {
                let token = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                Ok(vec![
                    ("authorization".into(), format!("Bearer token-{token}")),
                    (
                        "x-signed-body".into(),
                        String::from_utf8_lossy(request.body).into_owned(),
                    ),
                ])
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }

        let endpoint = ProviderEndpoint {
            base_url: Some("https://service.example".into()),
            headers: Vec::new(),
            auth: Some(Arc::new(Rotating(std::sync::atomic::AtomicUsize::new(0)))),
        };
        let first = endpoint
            .resolve("POST", "https://service.example/chat", b"one")
            .await
            .unwrap();
        let second = endpoint
            .resolve("POST", "https://service.example/chat", b"two")
            .await
            .unwrap();
        assert_eq!(first.headers[0].1, "Bearer token-1");
        assert_eq!(second.headers[0].1, "Bearer token-2");
        assert_eq!(first.headers[1].1, "one");
        assert_eq!(second.headers[1].1, "two");
    }

    #[tokio::test]
    async fn provider_identity_prefixes_start_and_stream_errors() {
        struct Failing {
            fail_to_start: bool,
        }
        #[async_trait]
        impl ChatDriver for Failing {
            async fn chat_completion_stream(
                &self,
                _endpoint: &ProviderEndpoint,
                _messages: Vec<crate::LlmMessage>,
                _config: &crate::LlmCallConfig,
            ) -> Result<crate::LlmResponseStream> {
                if self.fail_to_start {
                    return Err(crate::AgentLoopError::llm("request failed"));
                }
                Ok(Box::pin(futures::stream::once(async {
                    Err(crate::AgentLoopError::llm("stream failed"))
                })))
            }
        }

        let config = crate::LlmCallConfig {
            model: "model".into(),
            temperature: None,
            max_tokens: None,
            tools: Vec::new(),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        };
        let start = Provider::new(
            "customer-gateway",
            Failing {
                fail_to_start: true,
            },
        );
        let error = match start.chat_completion_stream(Vec::new(), &config).await {
            Ok(_) => panic!("the test driver should fail before returning a stream"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("provider 'customer-gateway'"));

        let stream = Provider::new(
            "customer-gateway",
            Failing {
                fail_to_start: false,
            },
        );
        let error = stream
            .chat_completion_stream(Vec::new(), &config)
            .await
            .unwrap()
            .next()
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("provider 'customer-gateway'"));
    }
}
