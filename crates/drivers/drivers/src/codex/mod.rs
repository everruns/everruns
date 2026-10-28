//! Legacy Codex backend protocol, sharing the Responses transport and parser.
//!
//! OAuth hosts assemble a provider with [`driver()`] (store-backed, rotating)
//! or [`driver::ephemeral`] (a grant held in memory); both serve chat and model
//! discovery from the same [`Provider`].
pub mod driver;
pub use crate::chatgpt::auth::{MemoryTokenStore, TokenRoute, TokenStore};
pub use driver::{CodexDriverBuilder, CodexDriverError, driver};

use async_trait::async_trait;
use everruns_contracts::{
    BearerAuth, ChatDriver, CompactRequest, CompactResponse, DiscoveredModel, DriverId,
    DriverRegistry, LlmCallConfig, LlmResponseStream, Message, ModelProfile,
    OpenResponsesProtocolChatDriver, Provider, ProviderEndpoint, ProviderKey, get_model_profile,
};
use everruns_contracts::{
    driver_registry::DriverDescriptor, error::Result,
    openresponses_protocol::OpenResponsesRequestExtension,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
pub const CODEX_DRIVER_ID: &str = "openai-codex";
pub const CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
/// Codex CLI version reported to the catalog. The backend hides models whose
/// `minimal_client_version` is newer, so hosts may raise it with
/// [`CodexChatDriver::with_client_version`] or the builder.
pub const CODEX_CLIENT_VERSION: &str = "0.164.0";
struct CodexRequest;
impl OpenResponsesRequestExtension for CodexRequest {
    fn supports_native_non_streaming(&self) -> bool {
        false
    }

    fn decorate(&self, body: &mut Value, _config: &LlmCallConfig) -> Result<()> {
        body["store"] = false.into();
        body["stream"] = true.into();
        if let Some(body) = body.as_object_mut() {
            // The Codex backend rejects Responses body metadata. Keep it in
            // the host config for observability and the session_id header.
            body.remove("metadata");
            body.remove("previous_response_id");
        }
        Ok(())
    }
    fn decorate_headers(
        &self,
        headers: &mut reqwest::header::HeaderMap,
        config: &LlmCallConfig,
    ) -> Result<()> {
        if let Some(session) = config.metadata.get("session_id") {
            let session = reqwest::header::HeaderValue::from_str(session)
                .map_err(|_| everruns_contracts::AgentLoopError::llm("Invalid Codex session ID"))?;
            headers.insert("session_id", session);
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct CodexChatDriver {
    inner: OpenResponsesProtocolChatDriver,
    unavailable_until: Arc<Mutex<HashMap<[u8; 32], Instant>>>,
    client_version: Arc<str>,
}
impl Default for CodexChatDriver {
    fn default() -> Self {
        Self::new()
    }
}
impl CodexChatDriver {
    pub fn new() -> Self {
        Self {
            inner: OpenResponsesProtocolChatDriver::new()
                .with_native_features(true, false)
                .with_hosted_tools(false)
                .with_stateful_responses(false)
                .with_prompt_cache_options(false)
                .with_required_completion(true)
                .with_request_extension(Arc::new(CodexRequest)),
            unavailable_until: Arc::new(Mutex::new(HashMap::new())),
            client_version: CODEX_CLIENT_VERSION.into(),
        }
    }
    /// Codex client version sent as `client_version` on catalog requests.
    pub fn with_client_version(mut self, version: impl AsRef<str>) -> Self {
        self.client_version = version.as_ref().into();
        self
    }
}
#[async_trait]
impl ChatDriver for CodexChatDriver {
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
    fn supports_parallel_tool_calls(&self, _model: &str) -> bool {
        true
    }
    fn effective_context_window(&self, model: &str) -> Option<usize> {
        model_profile(model)?.limits.map(|l| l.context as usize)
    }
    fn supports_compact(&self) -> bool {
        true
    }
    /// The Codex catalog, authenticated like chat: OAuth bearer plus the
    /// `chatgpt-account-id` and `originator` headers from the provider's auth.
    async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        let mut path = reqwest::Url::parse("http://localhost/models")
            .map_err(|_| everruns_contracts::AgentLoopError::llm("Invalid Codex catalog path"))?;
        path.query_pairs_mut()
            .append_pair("client_version", &self.client_version);
        let path = format!("models?{}", path.query().unwrap_or_default());
        crate::chatgpt::list_catalog(&self.inner.client(), endpoint, &path, "Codex")
            .await
            .map(Some)
    }
    async fn compact(
        &self,
        endpoint: &ProviderEndpoint,
        request: CompactRequest,
    ) -> Result<Option<CompactResponse>> {
        let url = endpoint.url("responses/compact").ok_or_else(|| {
            everruns_contracts::AgentLoopError::llm("Codex endpoint is not configured")
        })?;
        let resolved = endpoint.resolve("POST", url, &[]).await?;
        let account = resolved
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("chatgpt-account-id"))
            .or_else(|| {
                resolved
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            })
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        // A failed account must not disable compaction for another account.
        let key: [u8; 32] = Sha256::digest(format!("{}\0{account}", resolved.url)).into();
        {
            let mut cooldowns = self
                .unavailable_until
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            cooldowns.retain(|_, until| *until > Instant::now());
            if cooldowns.contains_key(&key) {
                return Ok(None);
            }
        }
        match self.inner.compact(endpoint, request).await {
            Err(error) if matches!(error.http_status(), Some(404 | 405)) => {
                self.unavailable_until
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(key, Instant::now() + Duration::from_secs(1800));
                Ok(None)
            }
            result => result.map(Some),
        }
    }
}
pub fn model_profile(model: &str) -> Option<ModelProfile> {
    get_model_profile(&DriverId::OpenAI, model)
}
pub fn provider(
    id: impl Into<everruns_contracts::ProviderKey>,
    auth: impl everruns_contracts::ProviderAuth + 'static,
) -> Provider {
    route_provider(id.into(), TokenRoute::Codex, None, None).auth(auth)
}
/// The one route-to-backend mapping: which chat driver and default base URL a
/// grant's route reaches. The builder and the descriptor both go through it,
/// so chat and discovery cannot target different backends.
pub(crate) fn route_provider(
    id: ProviderKey,
    route: TokenRoute,
    base_url: Option<&str>,
    client_version: Option<&str>,
) -> Provider {
    match route {
        TokenRoute::Codex => {
            let mut driver = CodexChatDriver::new();
            if let Some(version) = client_version {
                driver = driver.with_client_version(version);
            }
            Provider::new(id, driver).base_url(base_url.unwrap_or(CODEX_BASE_URL))
        }
        TokenRoute::ChatGptPlan => Provider::new(id, crate::chatgpt::ChatGptChatDriver::new())
            .base_url(base_url.unwrap_or(crate::chatgpt::oauth::RESOURCE)),
    }
}
pub fn descriptor() -> DriverDescriptor {
    let mut d = DriverDescriptor::chat_only(DriverId::external(CODEX_DRIVER_ID), |config| {
        // Dynamic config carries a host-resolved access token only. OAuth
        // hosts that rotate grants use `driver(store)` instead.
        route_provider(
            config.provider.clone(),
            TokenRoute::Codex,
            config.base_url.as_deref(),
            None,
        )
        .auth(BearerAuth::new(config.api_key.clone().unwrap_or_default()))
        .header("originator", "everruns")
        .header("openai-beta", "responses=experimental")
        .into_boxed_driver()
    });
    d.display_name = "Codex (legacy)".into();
    d
}
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_descriptor(descriptor());
}
