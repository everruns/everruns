//! Legacy Codex backend protocol, sharing the Responses transport and parser.
use async_trait::async_trait;
use everruns_contracts::{
    BearerAuth, ChatDriver, CompactRequest, CompactResponse, DriverId, DriverRegistry,
    LlmCallConfig, LlmResponseStream, Message, ModelProfile, OpenResponsesProtocolChatDriver,
    Provider, ProviderEndpoint, get_model_profile,
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
struct CodexRequest;
impl OpenResponsesRequestExtension for CodexRequest {
    fn decorate(&self, body: &mut Value, _config: &LlmCallConfig) -> Result<()> {
        body["store"] = false.into();
        body["stream"] = true.into();
        if let Some(body) = body.as_object_mut() {
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
        }
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
    Provider::new(id, CodexChatDriver::new())
        .base_url(CODEX_BASE_URL)
        .auth(auth)
}
pub fn descriptor() -> DriverDescriptor {
    let mut d = DriverDescriptor::chat_only(DriverId::external(CODEX_DRIVER_ID), |config| {
        Provider::new(config.provider.clone(), CodexChatDriver::new())
            .base_url(config.base_url.as_deref().unwrap_or(CODEX_BASE_URL))
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
