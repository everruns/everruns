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

    fn supported_auth(&self) -> everruns_contracts::driver_registry::SupportedAuth {
        everruns_contracts::driver_registry::SupportedAuth::OAuth
    }

    async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<everruns_contracts::DiscoveredModel>>> {
        // Chat and discovery share one rotating OAuth bearer. Fail fast when
        // the caller attached a key-based provider instead.
        ensure_oauth(endpoint)?;
        let url = endpoint.url("models").ok_or_else(|| {
            everruns_contracts::AgentLoopError::llm("codex provider has no base URL")
        })?;
        // Chat goes to `{resource}/codex/...`; the catalog lives at the OAuth
        // resource root, so drop a trailing `/codex` segment when present.
        // Custom bases without that suffix are used as-is.
        let url = match url.strip_suffix("/codex/models") {
            Some(root) => format!("{root}/models"),
            None => url,
        };
        let resolved = endpoint.resolve("GET", url, &[]).await?;
        let mut request = reqwest::Client::new().get(&resolved.url);
        for (name, value) in &resolved.headers {
            request = request.header(name, value);
        }
        let response = request
            .send()
            .await
            .map_err(anyhow::Error::from)?
            .error_for_status()
            .map_err(anyhow::Error::from)?;
        let body: serde_json::Value = response.json().await.map_err(anyhow::Error::from)?;
        // Accept the ChatGPT shape (`{"models": [{"slug", ...}]}`) and the
        // OpenAI shape (`{"data": [{"id", ...}]}`); skip unlisted entries.
        let items = body
            .get("models")
            .or_else(|| body.get("data"))
            .and_then(serde_json::Value::as_array)
            .map(|models| models.iter().filter_map(codex_catalog_model).collect())
            .ok_or_else(|| {
                everruns_contracts::AgentLoopError::llm("Codex returned no model catalog")
            })?;
        Ok(Some(items))
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

// ---------------------------------------------------------------------------
// Store-backed OAuth construction: generic discovery for every provider.
// ---------------------------------------------------------------------------
// Clients build Codex through this typed API instead of parsing
// `metadata.extra` for `flow` / `auth_source` / raw tokens:
// `codex::driver(store)` (or [`ephemeral`] for tests and one-shot grants).
// Key drivers keep their own constructors (`openai::provider(id, key)`); no
// global `Auth` enum exists in contracts, so `AnthropicKey + OAuth` cannot
// even be named, let alone compile.

use crate::chatgpt::CodexAuth;
use crate::chatgpt::auth::{RotatingAuth, TokenRoute, TokenStore};

/// In-memory [`TokenStore`]: seeds tests and one-shot (ephemeral) grants.
///
/// File-backed stores stay client side (yolop): this API only ever takes
/// `Arc<dyn TokenStore>`, never a file path.
pub struct MemoryTokenStore {
    state: std::sync::Arc<tokio::sync::Mutex<Option<CodexAuth>>>,
    /// Refresh exclusion only. Held across `load` by [`RotatingAuth`], so it
    /// must never guard the token state itself (that would self-deadlock the
    /// same task); file stores use a lockfile for the same reason.
    lease: std::sync::Arc<tokio::sync::Mutex<()>>,
}

impl MemoryTokenStore {
    /// Empty store; starts signed out.
    pub fn new() -> Self {
        Self {
            state: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            lease: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Store preloaded with one grant; backs [`ephemeral`].
    pub fn seeded(auth: CodexAuth) -> Self {
        Self {
            state: std::sync::Arc::new(tokio::sync::Mutex::new(Some(auth))),
            lease: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

impl Default for MemoryTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl TokenStore for MemoryTokenStore {
    async fn lock(&self) -> anyhow::Result<Box<dyn Send>> {
        Ok(Box::new(self.lease.clone().lock_owned().await))
    }

    async fn load(&self) -> anyhow::Result<Option<CodexAuth>> {
        Ok(self.state.lock().await.clone())
    }

    async fn save(&self, auth: CodexAuth) -> anyhow::Result<()> {
        *self.state.lock().await = Some(auth);
        Ok(())
    }
}

/// One-shot OAuth grant as a [`TokenStore`]: tests and short-lived grants.
/// No file I/O, no global `Auth` enum.
pub fn ephemeral(auth: CodexAuth) -> Arc<dyn TokenStore> {
    Arc::new(MemoryTokenStore::seeded(auth))
}

/// Typed entry point for Codex OAuth drivers: `codex::driver(store)` with a
/// caller-owned [`TokenStore`], not a file path and not a key string.
pub fn driver(store: Arc<dyn TokenStore>) -> CodexBuilder {
    CodexBuilder::new(store)
}

/// Fluent Codex construction. One [`Provider`] serves both chat and
/// `list_models` with the same rotating OAuth bearer auth.
pub struct CodexBuilder {
    store: Arc<dyn TokenStore>,
    route: TokenRoute,
    originator: String,
    base_url: String,
}

impl CodexBuilder {
    /// Start from a caller-owned token store; default route is [`TokenRoute::Codex`].
    pub fn new(store: Arc<dyn TokenStore>) -> Self {
        Self {
            store,
            route: TokenRoute::Codex,
            originator: "codex_cli_rs".to_owned(),
            base_url: CODEX_BASE_URL.to_owned(),
        }
    }

    /// OAuth route: [`TokenRoute::Codex`] (default) or
    /// [`TokenRoute::ChatGptPlan`]. Typed, so no `flow` strings leak to clients.
    pub fn route(mut self, route: TokenRoute) -> Self {
        self.route = route;
        self
    }

    /// `originator` header value surfaced to the API.
    pub fn originator(mut self, originator: impl Into<String>) -> Self {
        self.originator = originator.into();
        self
    }

    /// Chat base URL (default [`CODEX_BASE_URL`]). `list_models` strips a
    /// trailing `/codex` so the catalog resolves at the OAuth resource root.
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Which route this builder mints tokens for (test visibility).
    pub fn selected_route(&self) -> TokenRoute {
        self.route
    }

    /// Build one [`Provider`] for both chat and `list_models`.
    pub fn build(self) -> Provider {
        let auth = RotatingAuth::new(self.store, self.route).with_originator(self.originator);
        Provider::new(CODEX_DRIVER_ID, CodexChatDriver::new())
            .base_url(self.base_url)
            .auth(auth)
    }
}

/// Reject a non-OAuth provider for the Codex driver before any network call.
///
/// Key drivers keep working (their `supported_auth()` is `Any`); Codex
/// requires the rotating [`RotatingAuth`] the builder above installs.
pub fn ensure_oauth(endpoint: &ProviderEndpoint) -> Result<()> {
    if endpoint.auth::<RotatingAuth>().is_some() {
        Ok(())
    } else {
        Err(everruns_contracts::AgentLoopError::llm_kind(
            everruns_contracts::LlmErrorKind::UnsupportedAuth,
            "codex driver requires OAuth token-store auth: build it with codex::driver(store), not an API key",
        ))
    }
}

/// Map one catalog entry (`slug` or OpenAI-style `id`) to a
/// [`DiscoveredModel`]; `None` skips unlisted plan entries.
fn codex_catalog_model(item: &serde_json::Value) -> Option<everruns_contracts::DiscoveredModel> {
    if item
        .get("visibility")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|visibility| visibility != "list")
    {
        return None;
    }
    let slug = item
        .get("slug")
        .or_else(|| item.get("id"))
        .and_then(serde_json::Value::as_str)?;
    let display_name = item
        .get("display_name")
        .or_else(|| item.get("name"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| Some(slug.to_owned()));
    Some(everruns_contracts::DiscoveredModel {
        model_id: slug.to_owned(),
        display_name,
        capabilities: vec!["chat".to_owned()],
        created_at: None,
        owned_by: None,
        discovered_profile: None,
    })
}

#[cfg(test)]
mod codex_oauth_tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn grant_json() -> serde_json::Value {
        serde_json::json!({
            "access_token": "test-access-token",
            "refresh_token": null,
            "expires_at": 4_000_000_000_000i64,
            "account_id": "acct-test",
            "email": null,
            "client_id": null,
            "open_source": null
        })
    }

    fn test_grant() -> CodexAuth {
        serde_json::from_value(grant_json()).expect("test grant deserializes")
    }

    #[test]
    fn builder_defaults_to_codex_route() {
        let builder = CodexBuilder::new(Arc::new(MemoryTokenStore::new()));
        assert_eq!(builder.selected_route(), TokenRoute::Codex);
    }

    #[test]
    fn builder_route_override_selects_chatgpt_plan() {
        let builder = driver(Arc::new(MemoryTokenStore::new())).route(TokenRoute::ChatGptPlan);
        assert_eq!(builder.selected_route(), TokenRoute::ChatGptPlan);
    }

    #[tokio::test]
    async fn ephemeral_store_round_trips_grant() {
        let store = ephemeral(test_grant());
        let snapshot = serde_json::to_value(store.load().await.expect("load works"))
            .expect("grant serializes");
        assert_eq!(snapshot["access_token"], "test-access-token");
        // Same builder serves chat and list_models: the built provider
        // carries rotating OAuth auth, so the descriptor check passes.
        let provider = driver(store).build();
        ensure_oauth(provider.endpoint()).expect("oauth provider accepted");
    }

    #[tokio::test]
    async fn codex_list_models_uses_oauth_bearer() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header("authorization", "Bearer test-access-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [
                    {"slug": "gpt-6.1-sol", "display_name": "Sol", "visibility": "list"},
                    {"slug": "hidden-model", "visibility": "unlisted"}
                ]
            })))
            .mount(&server)
            .await;
        let provider = driver(ephemeral(test_grant()))
            .base_url(server.uri())
            .build();
        let models = CodexChatDriver::new()
            .list_models(provider.endpoint())
            .await
            .expect("listing works")
            .expect("codex supports listing");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "gpt-6.1-sol");
    }

    #[test]
    fn key_auth_is_rejected_for_codex() {
        let provider = Provider::new(CODEX_DRIVER_ID, CodexChatDriver::new())
            .base_url("https://example.invalid")
            .auth(BearerAuth::new("sk-test"));
        let err = ensure_oauth(provider.endpoint()).expect_err("key auth must be rejected");
        assert!(matches!(
            err.llm_error_kind(),
            Some(everruns_contracts::LlmErrorKind::UnsupportedAuth)
        ));
    }
}
