//! Deployment-owned OpenAI utility LLM wiring.

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

/// OpenAI-backed implementation of core's provider-neutral utility LLM service.
#[derive(Clone)]
pub struct OpenAiUtilityLlmService {
    provider: Provider,
}

impl std::fmt::Debug for OpenAiUtilityLlmService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiUtilityLlmService")
            .field("model", &UTILITY_LLM_MODEL)
            .field("configured", &true)
            .finish()
    }
}

impl OpenAiUtilityLlmService {
    /// Construct the fixed-model utility service with a deployment-owned key.
    pub fn new(api_key: impl Into<String>) -> Self {
        // THREAT[TM-LLM-021]: Utility LLM credentials remain deployment-owned
        // and never become agent- or session-configurable.
        Self {
            provider: Provider::new("utility-openai", OpenResponsesProtocolChatDriver::new())
                .base_url("https://api.openai.com/v1")
                .auth(BearerAuth::new(api_key)),
        }
    }
}

#[async_trait]
impl UtilityLlmService for OpenAiUtilityLlmService {
    fn is_configured(&self) -> bool {
        true
    }

    async fn chat_completion(&self, request: UtilityLlmRequest) -> Result<LlmResponse> {
        let (messages, config) = request.into_driver_request()?;
        self.provider.chat_completion(messages, &config).await
    }

    async fn chat_completion_stream(
        &self,
        request: UtilityLlmRequest,
    ) -> Result<LlmResponseStream> {
        let (messages, config) = request.into_driver_request()?;
        self.provider
            .chat_completion_stream(messages, &config)
            .await
    }

    fn name(&self) -> &'static str {
        "OpenAiUtilityLlmService"
    }
}

/// Deployment startup configuration for the concrete utility LLM service.
#[derive(Clone, PartialEq, Eq)]
pub enum SystemUtilityLlmConfig {
    /// Utility model calls are unavailable.
    Disabled,
    /// Enable the fixed OpenAI utility model with a system-owned API key.
    OpenAi { api_key: String },
}

impl std::fmt::Debug for SystemUtilityLlmConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.debug_struct("SystemUtilityLlmConfig::Disabled").finish(),
            Self::OpenAi { .. } => f
                .debug_struct("SystemUtilityLlmConfig::OpenAi")
                .field("api_key", &"<redacted>")
                .finish(),
        }
    }
}

impl SystemUtilityLlmConfig {
    /// Resolve utility LLM configuration from the process environment.
    pub fn from_env() -> Self {
        match std::env::var(UTILITY_OPENAI_API_KEY_ENV)
            .ok()
            .filter(|value| !value.is_empty())
        {
            Some(api_key) => Self::OpenAi { api_key },
            None => Self::Disabled,
        }
    }

    /// Materialize the configured service behind core's neutral trait.
    pub fn into_service(self) -> Arc<dyn UtilityLlmService> {
        match self {
            Self::Disabled => Arc::new(DisabledUtilityLlmService),
            Self::OpenAi { api_key } => Arc::new(OpenAiUtilityLlmService::new(api_key)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_config_debug_redacts_api_key() {
        let debug = format!(
            "{:?}",
            SystemUtilityLlmConfig::OpenAi {
                api_key: "sk-secret-value".to_string(),
            }
        );
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("sk-secret-value"));
    }
}
