//! Runtime providers: service identity, endpoint, authentication, and wire driver.
//!
//! A [`ChatDriver`] implements a wire
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
    /// Endpoint for a protocol client outside a provider assembly.
    pub fn from_parts(base_url: impl Into<String>, auth: impl ProviderAuth + 'static) -> Self {
        Self {
            base_url: Some(base_url.into().trim_end_matches('/').to_string()),
            headers: Vec::new(),
            auth: Some(Arc::new(auth)),
        }
    }

    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    pub fn url(&self, path: &str) -> Option<String> {
        self.base_url.as_ref().map(|base| {
            let base = base.trim_end_matches('/');
            if path.is_empty() {
                return base.to_string();
            }
            if let Ok(mut url) = url::Url::parse(base) {
                // Operations can carry protocol query parameters (Gemini's
                // alt=sse). Keep them out of set_path, which escapes ? and #.
                let (path, fragment) = path
                    .split_once('#')
                    .map_or((path, None), |(p, f)| (p, Some(f)));
                let (path, query) = path
                    .split_once('?')
                    .map_or((path, None), |(p, q)| (p, Some(q)));
                let suffix = format!("/{}", path.trim_start_matches('/'));
                // Match actual path segments, never a hostname or suffix of
                // another segment; preserve query parameters while appending.
                let existing = url.path().trim_end_matches('/');
                if !existing.ends_with(&suffix) {
                    url.set_path(&format!("{existing}{suffix}"));
                }
                if let Some(query) = query {
                    let combined = match url.query() {
                        Some(existing) if !existing.is_empty() && !query.is_empty() => {
                            format!("{existing}&{query}")
                        }
                        Some(existing) if query.is_empty() => existing.to_string(),
                        _ => query.to_string(),
                    };
                    url.set_query(Some(&combined));
                }
                if let Some(fragment) = fragment {
                    url.set_fragment(Some(fragment));
                }
                url.to_string()
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
    driver: Option<Arc<dyn ChatDriver>>,
    decisions: Option<Arc<dyn crate::decision_driver::DecisionDriver>>,
    extractions: Option<Arc<dyn crate::extraction_driver::ExtractionDriver>>,
    embeddings: Option<Arc<dyn crate::driver_registry::EmbeddingsDriver>>,
    realtime: Option<Arc<dyn crate::voice::RealtimeDriver>>,
    endpoint: ProviderEndpoint,
    // Which driver kind this assembly speaks, for lookups that are about the
    // vendor rather than this configured instance (model profiles, catalog
    // enrichment). Optional because the runtime identity and the driver kind
    // are independent: a provider may be keyed `"my-openai"`. Unset falls back
    // to the key, which is the conventional case.
    driver_id: Option<crate::provider::DriverId>,
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
            driver: Some(driver),
            decisions: None,
            extractions: None,
            embeddings: None,
            realtime: None,
            endpoint: ProviderEndpoint::default(),
            driver_id: None,
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

    /// Declare the driver kind this provider speaks, when it differs from the
    /// runtime key. Used for model-profile lookups, never for routing.
    pub fn with_driver_id(mut self, driver_id: crate::provider::DriverId) -> Self {
        self.driver_id = Some(driver_id);
        self
    }

    pub fn id(&self) -> &ProviderKey {
        &self.id
    }

    /// The driver kind this provider speaks, falling back to the runtime key
    /// when nothing declared one.
    pub fn driver_id(&self) -> crate::provider::DriverId {
        self.driver_id
            .clone()
            .unwrap_or_else(|| crate::provider::DriverId::external(self.id.as_str()))
    }

    pub fn driver(&self) -> Result<&Arc<dyn ChatDriver>> {
        self.driver
            .as_ref()
            .ok_or_else(|| self.unsupported_service("chat"))
    }

    /// Whether this provider's driver implements native offline batches.
    /// This is a local capability query, not a check of account permissions,
    /// model eligibility or a custom endpoint's API compatibility.
    pub fn supports_batch(&self) -> bool {
        self.driver
            .as_ref()
            .and_then(|driver| driver.batch_driver())
            .is_some()
    }

    fn unsupported_service(&self, service: &str) -> crate::error::AgentLoopError {
        crate::error::AgentLoopError::Configuration(format!(
            "Provider '{}' does not implement the {service} service",
            self.id
        ))
    }

    /// Construct a provider whose services are supplied by typed builders.
    pub fn services(id: impl Into<ProviderKey>) -> Self {
        Self {
            id: id.into(),
            driver: None,
            decisions: None,
            extractions: None,
            embeddings: None,
            realtime: None,
            endpoint: ProviderEndpoint::default(),
            driver_id: None,
        }
    }

    pub fn with_decisions(
        mut self,
        driver: impl crate::decision_driver::DecisionDriver + 'static,
    ) -> Self {
        self.decisions = Some(Arc::new(driver));
        self
    }

    pub fn with_extractions(
        mut self,
        driver: impl crate::extraction_driver::ExtractionDriver + 'static,
    ) -> Self {
        self.extractions = Some(Arc::new(driver));
        self
    }

    pub fn with_embeddings(
        mut self,
        driver: impl crate::driver_registry::EmbeddingsDriver + 'static,
    ) -> Self {
        self.embeddings = Some(Arc::new(driver));
        self
    }

    /// Attach the speech-to-speech (realtime voice) service.
    pub fn with_realtime(mut self, driver: impl crate::voice::RealtimeDriver + 'static) -> Self {
        self.realtime = Some(Arc::new(driver));
        self
    }

    /// The realtime voice service with this provider's endpoint, or an error
    /// when the provider does not offer one.
    pub fn realtime(&self) -> Result<(Arc<dyn crate::voice::RealtimeDriver>, ProviderEndpoint)> {
        self.realtime
            .clone()
            .map(|driver| (driver, self.endpoint.clone()))
            .ok_or_else(|| self.unsupported_service("realtime"))
    }

    pub fn supports_service(&self, service: crate::ServiceKind) -> bool {
        match service {
            crate::ServiceKind::Chat => self.driver.is_some(),
            crate::ServiceKind::Decisions => self.decisions.is_some(),
            crate::ServiceKind::Extractions => self.extractions.is_some(),
            crate::ServiceKind::Embeddings => self.embeddings.is_some(),
            crate::ServiceKind::Realtime => self.realtime.is_some(),
            _ => false,
        }
    }

    pub async fn evaluate_decisions(
        &self,
        request: crate::decisions::DecisionRequest,
    ) -> Result<crate::decisions::DecisionOutcome> {
        if request.provider.as_ref().is_some_and(|key| key != &self.id) {
            return Err(crate::error::AgentLoopError::Configuration(
                "A bound provider cannot select another account".into(),
            ));
        }
        let driver = self
            .decisions
            .as_ref()
            .ok_or_else(|| self.unsupported_service("decisions"))?;
        driver.capabilities().check(driver.id(), &request)?;
        driver
            .evaluate(&self.endpoint, request)
            .await
            .map_err(|error| error.with_provider(self.id.as_str()))
    }

    pub async fn extract(
        &self,
        request: crate::extractions::ExtractionRequest,
    ) -> Result<crate::extractions::ExtractionOutcome> {
        if request.provider.as_ref().is_some_and(|key| key != &self.id) {
            return Err(crate::error::AgentLoopError::Configuration(
                "A bound provider cannot select another account".into(),
            ));
        }
        let driver = self
            .extractions
            .as_ref()
            .ok_or_else(|| self.unsupported_service("extractions"))?;
        driver.capabilities().check(driver.id(), &request)?;
        let outcome = driver
            .extract(&self.endpoint, request.clone())
            .await
            .map_err(|error| error.with_provider(self.id.as_str()))?;
        outcome
            .validate(&request)
            .map_err(|error| error.with_provider(self.id.as_str()))?;
        Ok(outcome)
    }

    pub async fn embed(
        &self,
        request: crate::driver_registry::EmbedRequest,
    ) -> std::result::Result<
        crate::driver_registry::EmbedResponse,
        crate::driver_registry::EmbeddingsDriverError,
    > {
        let driver = self.embeddings.as_ref().ok_or_else(|| {
            crate::driver_registry::EmbeddingsDriverError::Provider(
                self.unsupported_service("embeddings").to_string(),
            )
        })?;
        driver.embed(&self.endpoint, request).await
    }

    pub fn endpoint(&self) -> &ProviderEndpoint {
        &self.endpoint
    }

    /// A requested response format this driver cannot enforce is a
    /// configuration error, never a silently unconstrained reply (EVE-1116).
    fn check_response_format(&self, config: &crate::driver_registry::LlmCallConfig) -> Result<()> {
        if config.response_format.is_some()
            && !self.driver()?.supports_response_format(&config.model)
        {
            return Err(crate::error::AgentLoopError::Configuration(format!(
                "Structured output (response_format) is not supported by provider '{}' for model '{}'",
                self.id, config.model
            )));
        }
        Ok(())
    }

    pub async fn chat_completion_stream(
        &self,
        messages: Vec<crate::driver_registry::Message>,
        config: &crate::driver_registry::LlmCallConfig,
    ) -> Result<crate::driver_registry::LlmResponseStream> {
        self.check_response_format(config)?;
        let id = self.id.to_string();
        let limits = config.limits;
        // Establishing the stream is itself a round trip that can hang, so it
        // runs inside the budget, and what it spends is charged against what
        // the stream then gets.
        let (stream, spent) = crate::turn_collector::connect_within(
            &limits,
            self.driver()?
                .chat_completion_stream(&self.endpoint, messages, config),
        )
        .await
        .map_err(|error| error.with_provider(&id))?;
        let stream: crate::driver_registry::LlmResponseStream =
            Box::pin(stream.map(move |result| result.map_err(|error| error.with_provider(&id))));
        // The call's limits bind whoever consumes the stream, not just the
        // collected path: a caller rendering events itself is exactly the one
        // with no other way to bound a provider that stops sending.
        let stream = crate::turn_collector::limit_stream(stream, limits.after(spent));
        // Drivers use an empty `TextDelta` as filler for wire frames that carry
        // nothing (usage, role, keep-alive). The limits above still see those
        // frames; the caller never does, so every delta it gets has content.
        Ok(Box::pin(stream.filter(|event| {
            std::future::ready(!matches!(event, Ok(crate::driver_registry::LlmStreamEvent::TextDelta(delta)) if delta.is_empty()))
        })))
    }

    pub async fn chat_completion(
        &self,
        messages: Vec<crate::driver_registry::Message>,
        config: &crate::driver_registry::LlmCallConfig,
    ) -> Result<crate::driver_registry::LlmResponse> {
        self.check_response_format(config)?;
        self.driver()?
            .chat_completion(&self.endpoint, messages, config)
            .await
            .map_err(|error| error.with_provider(self.id.as_str()))
    }

    pub fn supports_native_non_streaming(&self) -> bool {
        self.driver
            .as_ref()
            .is_some_and(|driver| driver.supports_native_non_streaming())
    }

    pub async fn chat_completion_non_streaming(
        &self,
        messages: Vec<crate::driver_registry::Message>,
        config: &crate::driver_registry::LlmCallConfig,
    ) -> Result<crate::driver_registry::LlmResponse> {
        self.check_response_format(config)?;
        if config.limits.needs_incremental_events() {
            // Native JSON responses cannot enforce a byte cap before buffering
            // the body. Route bounded calls through the streaming collector so
            // every advertised limit is applied while bytes are arriving.
            let stream = self.chat_completion_stream(messages, config).await?;
            return Ok(
                crate::turn_collector::collect_turn(stream, &config.limits, |_| {})
                    .await?
                    .into_response(),
            );
        }
        let (response, _) = crate::turn_collector::connect_within(
            &config.limits,
            self.driver()?
                .chat_completion_non_streaming(&self.endpoint, messages, config),
        )
        .await
        .map_err(|error| error.with_provider(self.id.as_str()))?;
        Ok(response)
    }

    /// Every model the provider's catalogs list, coalesced by model id across
    /// chat, decision and extraction services. `Ok(None)` means no catalog.
    ///
    /// Decision discovery is additive: when the chat catalog answered, a
    /// failing decision catalog is logged and skipped, so one flaky endpoint
    /// never hides the chat models.
    pub async fn list_models(
        &self,
    ) -> Result<Option<Vec<crate::driver_registry::DiscoveredModel>>> {
        let chat = match &self.driver {
            Some(driver) => driver
                .list_models(&self.endpoint)
                .await
                .map_err(|error| error.with_provider(self.id.as_str()))?,
            None => None,
        };
        let mut catalog = chat;
        if let Some(decisions) = &self.decisions {
            match decisions.list_models(&self.endpoint).await {
                Ok(Some(models)) => catalog.get_or_insert_with(Vec::new).extend(models),
                Ok(None) => {}
                Err(error) if catalog.is_some() => {
                    tracing::warn!(provider = self.id.as_str(), %error, "Decision model discovery failed; retaining available catalog")
                }
                Err(error) => return Err(error.with_provider(self.id.as_str())),
            }
        }
        if let Some(extractions) = &self.extractions {
            match extractions.list_models(&self.endpoint).await {
                Ok(Some(models)) => catalog.get_or_insert_with(Vec::new).extend(models),
                Ok(None) => {}
                Err(error) if catalog.is_some() => {
                    tracing::warn!(provider = self.id.as_str(), %error, "Extraction model discovery failed; retaining available catalog")
                }
                Err(error) => return Err(error.with_provider(self.id.as_str())),
            }
        }
        Ok(catalog.map(|models| {
            // One provider/model identity may expose several services. Keep the
            // first (chat) profile and merge service tags instead of creating
            // duplicate database rows or replacing its richer chat metadata.
            let mut merged: Vec<crate::driver_registry::DiscoveredModel> = Vec::new();
            for model in models {
                if let Some(existing) = merged.iter_mut().find(|m| m.model_id == model.model_id) {
                    for capability in model.capabilities {
                        if !existing.capabilities.contains(&capability) {
                            existing.capabilities.push(capability);
                        }
                    }
                    existing.display_name = existing.display_name.take().or(model.display_name);
                    existing.created_at = existing.created_at.or(model.created_at);
                    existing.owned_by = existing.owned_by.take().or(model.owned_by);
                    existing.discovered_profile = existing
                        .discovered_profile
                        .take()
                        .or(model.discovered_profile);
                } else {
                    merged.push(model);
                }
            }
            merged
        }))
    }

    /// The provider's catalog, ready for display: discovered ids sorted
    /// newest-first and merged with the model-profile registry.
    ///
    /// `Ok(None)` means this provider offers no catalog — callers keep their
    /// curated suggestions rather than treating it as a failure. See
    /// [`list_models`](Self::list_models) for the driver's raw answer.
    pub async fn models(
        &self,
    ) -> Result<Option<Vec<crate::model_discovery::DiscoveredProviderModel>>> {
        let Some(models) = self.list_models().await? else {
            return Ok(None);
        };
        Ok(Some(crate::model_discovery::normalize_and_enrich(
            &self.driver_id(),
            models,
        )))
    }

    pub fn into_boxed_driver(self) -> BoxedChatDriver {
        Box::new(ProviderBoundDriver(self))
    }

    pub fn into_embeddings_driver(
        self,
    ) -> std::result::Result<
        crate::driver_registry::BoxedEmbeddingsDriver,
        crate::driver_registry::EmbeddingsDriverError,
    > {
        if self.embeddings.is_none() {
            return Err(crate::driver_registry::EmbeddingsDriverError::Provider(
                "Provider does not support embeddings".into(),
            ));
        }
        Ok(Box::new(ProviderOwnedEmbeddingsDriver(self)))
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
    // No batch hook: this legacy adapter hides its bound endpoint from the
    // caller. Exposing it would fingerprint the caller's unrelated endpoint in
    // persisted tickets. Batch clients must retain the RuntimeProvider instead.

    fn native_async_driver(
        &self,
        model: &str,
        tools: std::collections::BTreeMap<String, Option<serde_json::Value>>,
        continuation: Option<crate::native_async::Delivery>,
    ) -> Option<Arc<dyn ChatDriver>> {
        let driver = self
            .0
            .driver
            .as_ref()?
            .native_async_driver(model, tools, continuation)?;
        Some(Arc::new(ProviderBoundDriver(RuntimeProvider {
            id: self.0.id.clone(),
            endpoint: self.0.endpoint.clone(),
            driver: Some(driver),
            decisions: self.0.decisions.clone(),
            extractions: self.0.extractions.clone(),
            embeddings: self.0.embeddings.clone(),
            realtime: self.0.realtime.clone(),
            driver_id: self.0.driver_id.clone(),
        })))
    }
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        messages: Vec<crate::driver_registry::Message>,
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

    async fn credit_status(
        &self,
        _endpoint: &ProviderEndpoint,
    ) -> Result<Option<crate::provider_limit::CreditStatus>> {
        match &self.0.driver {
            Some(driver) => driver.credit_status(self.0.endpoint()).await,
            None => Ok(None),
        }
    }

    fn supports_native_non_streaming(&self) -> bool {
        self.0.supports_native_non_streaming()
    }

    async fn chat_completion_non_streaming(
        &self,
        _endpoint: &ProviderEndpoint,
        messages: Vec<crate::driver_registry::Message>,
        config: &crate::driver_registry::LlmCallConfig,
    ) -> Result<crate::driver_registry::LlmResponse> {
        self.0.chat_completion_non_streaming(messages, config).await
    }

    fn supports_compact(&self) -> bool {
        self.0
            .driver
            .as_ref()
            .map(|driver| driver.supports_compact())
            .unwrap_or(false)
    }

    fn supports_stateful_responses(&self) -> bool {
        self.0
            .driver
            .as_ref()
            .map(|driver| driver.supports_stateful_responses())
            .unwrap_or(false)
    }

    fn effective_context_window(&self, model: &str) -> Option<usize> {
        self.0
            .driver
            .as_ref()
            .map(|driver| driver.effective_context_window(model))
            .unwrap_or(None)
    }

    fn stream_stall_timeout(
        &self,
        config: &crate::driver_registry::LlmCallConfig,
    ) -> Option<std::time::Duration> {
        self.0
            .driver
            .as_ref()
            .and_then(|driver| driver.stream_stall_timeout(config))
    }

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        self.0
            .driver
            .as_ref()
            .map(|driver| driver.supports_parallel_tool_calls(model))
            .unwrap_or(false)
    }

    fn supports_response_format(&self, model: &str) -> bool {
        self.0
            .driver
            .as_ref()
            .map(|driver| driver.supports_response_format(model))
            .unwrap_or(false)
    }

    fn provider_managed_reduction_option(
        &self,
        _endpoint: &ProviderEndpoint,
        model: &str,
        budget_tokens: usize,
    ) -> Option<(String, serde_json::Value)> {
        self.0.driver.as_ref()?.provider_managed_reduction_option(
            self.0.endpoint(),
            model,
            budget_tokens,
        )
    }

    fn provider_managed_reduction_fallback_reason(
        &self,
        _endpoint: &ProviderEndpoint,
        config: &crate::driver_registry::LlmCallConfig,
    ) -> Option<&'static str> {
        self.0
            .driver
            .as_ref()?
            .provider_managed_reduction_fallback_reason(self.0.endpoint(), config)
    }

    fn validate_provider_opaque_context(
        &self,
        context: &crate::driver_registry::ProviderOpaqueContext,
    ) -> bool {
        self.0
            .driver
            .as_ref()
            .map(|driver| driver.validate_provider_opaque_context(context))
            .unwrap_or(false)
    }

    async fn compact(
        &self,
        _endpoint: &ProviderEndpoint,
        request: crate::compact::CompactRequest,
    ) -> Result<Option<crate::compact::CompactResponse>> {
        self.0
            .driver()?
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

#[async_trait]
impl crate::decisions::DecisionsService for RuntimeProvider {
    fn is_configured(&self) -> bool {
        self.decisions.is_some()
    }
    async fn evaluate(
        &self,
        request: crate::decisions::DecisionRequest,
    ) -> Result<crate::decisions::DecisionOutcome> {
        if request
            .provider
            .as_ref()
            .is_some_and(|provider| provider != &self.id)
        {
            return Err(crate::error::AgentLoopError::Configuration(
                "A bound provider cannot select another account".into(),
            ));
        }
        self.evaluate_decisions(request).await
    }
    fn name(&self) -> &'static str {
        "ProviderDecisions"
    }
}

struct ProviderOwnedEmbeddingsDriver(RuntimeProvider);
#[async_trait]
impl crate::driver_registry::EmbeddingsDriver for ProviderOwnedEmbeddingsDriver {
    async fn embed(
        &self,
        _endpoint: &ProviderEndpoint,
        request: crate::driver_registry::EmbedRequest,
    ) -> std::result::Result<
        crate::driver_registry::EmbedResponse,
        crate::driver_registry::EmbeddingsDriverError,
    > {
        self.0.embed(request).await
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "runtime_provider_service_tests.rs"]
mod service_tests;

#[async_trait]
impl crate::extractions::ExtractionsService for RuntimeProvider {
    fn is_configured(&self) -> bool {
        self.extractions.is_some()
    }
    async fn extract(
        &self,
        request: crate::extractions::ExtractionRequest,
    ) -> Result<crate::extractions::ExtractionOutcome> {
        RuntimeProvider::extract(self, request).await
    }
    fn name(&self) -> &'static str {
        "ProviderExtractions"
    }
}
