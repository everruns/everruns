//! Deployment-owned utility LLM wiring.
//!
//! The utility service is selected by which deployment key is present:
//! `UTILITY_OPENROUTER_API_KEY` picks OpenRouter, `UTILITY_OPENAI_API_KEY`
//! picks OpenAI, neither disables the service. `UTILITY_LLM_MODEL` overrides
//! the model for whichever backend was selected.

use async_trait::async_trait;
use everruns_core::{
    DisabledUtilityLlmService, UTILITY_LLM_MODEL, UtilityLlmRequest, UtilityLlmService,
};
use everruns_provider::driver_registry::{LlmResponse, LlmResponseStream};
use everruns_provider::error::Result;
use everruns_provider::{BearerAuth, OpenResponsesProtocolChatDriver, Provider};
use std::sync::Arc;

/// Environment variable used by the deployment-owned utility OpenAI client.
pub const UTILITY_OPENAI_API_KEY_ENV: &str = "UTILITY_OPENAI_API_KEY";

/// Environment variable used by the deployment-owned utility OpenRouter client.
pub const UTILITY_OPENROUTER_API_KEY_ENV: &str = "UTILITY_OPENROUTER_API_KEY";

/// Environment variable overriding the utility model for the selected backend.
pub const UTILITY_LLM_MODEL_ENV: &str = "UTILITY_LLM_MODEL";

/// Base URL of the OpenAI API. OpenRouter's own default stays owned by
/// `everruns-openrouter`, so it is not duplicated here.
const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

/// Default utility model when OpenRouter is the backend. OpenRouter model ids
/// are namespaced by upstream provider, so the OpenAI default does not carry
/// over unchanged.
pub const UTILITY_OPENROUTER_LLM_MODEL: &str = "openai/gpt-6-luna";

/// Which deployment-owned backend serves utility model calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UtilityLlmBackend {
    /// Called directly against the OpenAI API.
    OpenAi,
    /// Routed through the OpenRouter gateway.
    OpenRouter,
}

impl UtilityLlmBackend {
    /// Default model for this backend when the deployment sets no override.
    pub fn default_model(self) -> &'static str {
        match self {
            Self::OpenAi => UTILITY_LLM_MODEL,
            Self::OpenRouter => UTILITY_OPENROUTER_LLM_MODEL,
        }
    }

    fn service_name(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAiUtilityLlmService",
            Self::OpenRouter => "OpenRouterUtilityLlmService",
        }
    }
}

/// Provider-backed implementation of core's provider-neutral utility LLM
/// service.
#[derive(Clone)]
pub struct ProviderUtilityLlmService {
    provider: Provider,
    backend: UtilityLlmBackend,
    model: String,
}

impl std::fmt::Debug for ProviderUtilityLlmService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderUtilityLlmService")
            .field("backend", &self.backend)
            .field("model", &self.model)
            .field("configured", &true)
            .finish()
    }
}

impl ProviderUtilityLlmService {
    /// Construct the OpenAI-backed service with a deployment-owned key.
    pub fn openai(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::openai_at(api_key, model, None)
    }

    /// Construct the OpenRouter-backed service with a deployment-owned key.
    pub fn openrouter(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::openrouter_at(api_key, model, None)
    }

    /// `base_url` overrides the backend's endpoint; only tests pass one, so
    /// the shipped construction is the one they exercise.
    fn openai_at(
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: Option<&str>,
    ) -> Self {
        // THREAT[TM-LLM-021]: Utility LLM credentials remain deployment-owned
        // and never become agent- or session-configurable.
        Self {
            provider: Provider::new("utility-openai", OpenResponsesProtocolChatDriver::new())
                .base_url(base_url.unwrap_or(OPENAI_BASE_URL))
                .auth(BearerAuth::new(api_key)),
            backend: UtilityLlmBackend::OpenAi,
            model: model.into(),
        }
    }

    fn openrouter_at(
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: Option<&str>,
    ) -> Self {
        // THREAT[TM-LLM-021]: same deployment-owned credential contract as the
        // OpenAI backend; only the gateway in front of the model changes.
        let mut provider = everruns_openrouter::provider("utility-openrouter", api_key);
        if let Some(base_url) = base_url {
            provider = provider.base_url(base_url);
        }
        Self {
            provider,
            backend: UtilityLlmBackend::OpenRouter,
            model: model.into(),
        }
    }

    /// Backend serving this instance.
    pub fn backend(&self) -> UtilityLlmBackend {
        self.backend
    }

    /// Model this instance calls.
    pub fn model(&self) -> &str {
        &self.model
    }
}

#[async_trait]
impl UtilityLlmService for ProviderUtilityLlmService {
    fn is_configured(&self) -> bool {
        true
    }

    async fn chat_completion(&self, request: UtilityLlmRequest) -> Result<LlmResponse> {
        let (messages, config) = request.into_driver_request(&self.model)?;
        self.provider
            .chat_completion_non_streaming(messages, &config)
            .await
    }

    async fn chat_completion_stream(
        &self,
        request: UtilityLlmRequest,
    ) -> Result<LlmResponseStream> {
        let (messages, config) = request.into_driver_request(&self.model)?;
        self.provider
            .chat_completion_stream(messages, &config)
            .await
    }

    fn name(&self) -> &'static str {
        self.backend.service_name()
    }
}

/// Deployment startup configuration for the concrete utility LLM service.
#[derive(Clone, PartialEq, Eq)]
pub enum SystemUtilityLlmConfig {
    /// Utility model calls are unavailable.
    Disabled,
    /// Enable the utility model on `backend` with a system-owned API key.
    Enabled {
        backend: UtilityLlmBackend,
        api_key: String,
        model: String,
    },
}

impl std::fmt::Debug for SystemUtilityLlmConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.debug_struct("SystemUtilityLlmConfig::Disabled").finish(),
            Self::Enabled { backend, model, .. } => f
                .debug_struct("SystemUtilityLlmConfig::Enabled")
                .field("backend", backend)
                .field("model", model)
                .field("api_key", &"<redacted>")
                .finish(),
        }
    }
}

impl SystemUtilityLlmConfig {
    /// Resolve utility LLM configuration from the process environment.
    pub fn from_env() -> Self {
        Self::resolve(
            env_value(UTILITY_OPENROUTER_API_KEY_ENV),
            env_value(UTILITY_OPENAI_API_KEY_ENV),
            env_value(UTILITY_LLM_MODEL_ENV),
        )
    }

    /// Key presence selects the backend, so the choice stays a deployment
    /// secret decision rather than another variable to keep in sync. Split out
    /// from `from_env` so the precedence is testable without mutating the
    /// process environment.
    fn resolve(
        openrouter_api_key: Option<String>,
        openai_api_key: Option<String>,
        model_override: Option<String>,
    ) -> Self {
        let (backend, api_key) = match (openrouter_api_key, openai_api_key) {
            (Some(api_key), openai) => {
                if openai.is_some() {
                    tracing::warn!(
                        "both {UTILITY_OPENROUTER_API_KEY_ENV} and {UTILITY_OPENAI_API_KEY_ENV} \
                         are set; using OpenRouter for the utility LLM"
                    );
                }
                (UtilityLlmBackend::OpenRouter, api_key)
            }
            (None, Some(api_key)) => (UtilityLlmBackend::OpenAi, api_key),
            (None, None) => return Self::Disabled,
        };
        Self::Enabled {
            backend,
            api_key,
            model: model_override.unwrap_or_else(|| backend.default_model().to_string()),
        }
    }

    /// Materialize the configured service behind core's neutral trait.
    pub fn into_service(self) -> Arc<dyn UtilityLlmService> {
        match self {
            Self::Disabled => Arc::new(DisabledUtilityLlmService),
            Self::Enabled {
                backend: UtilityLlmBackend::OpenAi,
                api_key,
                model,
            } => Arc::new(ProviderUtilityLlmService::openai(api_key, model)),
            Self::Enabled {
                backend: UtilityLlmBackend::OpenRouter,
                api_key,
                model,
            } => Arc::new(ProviderUtilityLlmService::openrouter(api_key, model)),
        }
    }
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_config_debug_redacts_api_key() {
        let debug = format!(
            "{:?}",
            SystemUtilityLlmConfig::Enabled {
                backend: UtilityLlmBackend::OpenAi,
                api_key: "sk-secret-value".to_string(),
                model: UTILITY_LLM_MODEL.to_string(),
            }
        );
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("sk-secret-value"));
    }

    #[test]
    fn missing_keys_disable_the_service() {
        assert_eq!(
            SystemUtilityLlmConfig::resolve(None, None, Some("some-model".into())),
            SystemUtilityLlmConfig::Disabled
        );
    }

    #[test]
    fn each_key_selects_its_backend_and_default_model() {
        assert_eq!(
            SystemUtilityLlmConfig::resolve(None, Some("sk-openai".into()), None),
            SystemUtilityLlmConfig::Enabled {
                backend: UtilityLlmBackend::OpenAi,
                api_key: "sk-openai".into(),
                model: UTILITY_LLM_MODEL.into(),
            }
        );
        assert_eq!(
            SystemUtilityLlmConfig::resolve(Some("sk-or".into()), None, None),
            SystemUtilityLlmConfig::Enabled {
                backend: UtilityLlmBackend::OpenRouter,
                api_key: "sk-or".into(),
                model: UTILITY_OPENROUTER_LLM_MODEL.into(),
            }
        );
    }

    #[test]
    fn openrouter_key_wins_when_both_are_set() {
        assert_eq!(
            SystemUtilityLlmConfig::resolve(Some("sk-or".into()), Some("sk-openai".into()), None),
            SystemUtilityLlmConfig::Enabled {
                backend: UtilityLlmBackend::OpenRouter,
                api_key: "sk-or".into(),
                model: UTILITY_OPENROUTER_LLM_MODEL.into(),
            }
        );
    }

    #[test]
    fn model_override_applies_to_either_backend() {
        for key in [
            (Some("sk-or".to_string()), None),
            (None, Some("sk-openai".to_string())),
        ] {
            let config = SystemUtilityLlmConfig::resolve(key.0, key.1, Some("custom-model".into()));
            let SystemUtilityLlmConfig::Enabled { model, .. } = config else {
                panic!("a configured key must enable the service");
            };
            assert_eq!(model, "custom-model");
        }
    }

    /// One SSE body both backends can answer with: the Open Responses shape
    /// the shipped driver parses.
    fn completed_sse_body(model: &str) -> String {
        format!(
            "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}}\n\n\
             data: {{\"type\":\"response.completed\",\"sequence_number\":2,\"response\":\
             {{\"id\":\"resp-utility\",\"object\":\"response\",\"created_at\":0,\
             \"model\":\"{model}\",\"status\":\"completed\",\"output\":[],\
             \"usage\":{{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}}}}\n\n"
        )
    }

    /// The deployment's model has to reach the wire, on whichever backend the
    /// keys selected — the whole point of `UTILITY_LLM_MODEL`.
    #[tokio::test]
    async fn each_backend_sends_the_deployment_model_with_its_own_credential() {
        use everruns_core::UtilityLlmRequest;
        use serde_json::Value;
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        for (backend, model, mock_path) in [
            (
                UtilityLlmBackend::OpenAi,
                "custom-openai-model",
                "/responses",
            ),
            (
                UtilityLlmBackend::OpenRouter,
                "vendor/custom-model",
                "/api/v1/responses",
            ),
        ] {
            let server = MockServer::builder().start().await;
            Mock::given(method("POST"))
                .and(path(mock_path))
                .and(header("authorization", "Bearer sk-deployment-owned"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "text/event-stream")
                        .set_body_string(completed_sse_body(model)),
                )
                .expect(1)
                .mount(&server)
                .await;

            let service = match backend {
                UtilityLlmBackend::OpenAi => ProviderUtilityLlmService::openai_at(
                    "sk-deployment-owned",
                    model,
                    Some(&server.uri()),
                ),
                UtilityLlmBackend::OpenRouter => ProviderUtilityLlmService::openrouter_at(
                    "sk-deployment-owned",
                    model,
                    Some(&format!("{}/api/v1", server.uri())),
                ),
            };

            let response = service
                .chat_completion(UtilityLlmRequest::user_text("summarize"))
                .await
                .expect("the mocked backend answers a completed response");
            assert_eq!(response.text, "ok");

            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1);
            let body = requests[0].body_json::<Value>().unwrap();
            assert_eq!(body["model"], model);
            // No tools, no tool search, no previous response id: a utility
            // request stays the bounded internal call TM-LLM-021 describes.
            assert!(body.get("tools").is_none());
            assert!(body.get("previous_response_id").is_none());
            if backend == UtilityLlmBackend::OpenRouter {
                // Proof the OpenRouter driver is in the path rather than a
                // bare base-URL swap: only it excludes provider reasoning.
                assert_eq!(body["reasoning"]["exclude"], Value::Bool(true));
            }
        }
    }

    #[test]
    fn service_reports_its_backend_model_and_name() {
        let openai = ProviderUtilityLlmService::openai("sk-openai", UTILITY_LLM_MODEL);
        assert!(openai.is_configured());
        assert_eq!(openai.backend(), UtilityLlmBackend::OpenAi);
        assert_eq!(openai.model(), UTILITY_LLM_MODEL);
        assert_eq!(openai.name(), "OpenAiUtilityLlmService");

        let openrouter =
            ProviderUtilityLlmService::openrouter("sk-or", UTILITY_OPENROUTER_LLM_MODEL);
        assert_eq!(openrouter.backend(), UtilityLlmBackend::OpenRouter);
        assert_eq!(openrouter.model(), UTILITY_OPENROUTER_LLM_MODEL);
        assert_eq!(openrouter.name(), "OpenRouterUtilityLlmService");
        assert!(!format!("{openrouter:?}").contains("sk-or"));
    }
}
