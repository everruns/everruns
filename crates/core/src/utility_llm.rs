//! System utility LLM service.
//!
//! This is a host-owned service for capability internals, not an agent-visible
//! model provider. It is configured once per deployment and deliberately keeps
//! the model fixed so call sites cannot turn it into a user-selectable model.

use crate::{AgentLoopError, LlmCallConfig, LlmMessage, LlmResponse, LlmResponseStream, Result};
use async_trait::async_trait;
use std::collections::HashMap;

pub const UTILITY_LLM_MODEL: &str = "gpt-5.5";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UtilityLlmReasoningEffort {
    Low,
    Medium,
    High,
}

impl From<UtilityLlmReasoningEffort> for everruns_provider::model::ReasoningEffort {
    fn from(value: UtilityLlmReasoningEffort) -> Self {
        match value {
            UtilityLlmReasoningEffort::Low => Self::Low,
            UtilityLlmReasoningEffort::Medium => Self::Medium,
            UtilityLlmReasoningEffort::High => Self::High,
        }
    }
}

impl UtilityLlmReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UtilityLlmRequest {
    pub messages: Vec<LlmMessage>,
    pub reasoning_effort: Option<UtilityLlmReasoningEffort>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub metadata: HashMap<String, String>,
}

impl UtilityLlmRequest {
    pub fn new(messages: Vec<LlmMessage>) -> Self {
        Self {
            messages,
            reasoning_effort: None,
            temperature: None,
            max_tokens: None,
            metadata: HashMap::new(),
        }
    }

    pub fn user_text(prompt: impl Into<String>) -> Self {
        Self::new(vec![LlmMessage::text(
            crate::LlmMessageRole::User,
            prompt.into(),
        )])
    }

    pub fn with_reasoning_effort(mut self, effort: UtilityLlmReasoningEffort) -> Self {
        self.reasoning_effort = Some(effort);
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Convert the host-neutral request into the provider-driver inputs.
    pub fn into_driver_request(self) -> Result<(Vec<LlmMessage>, LlmCallConfig)> {
        if self.messages.is_empty() {
            return Err(AgentLoopError::llm(
                "utility LLM request must include at least one message",
            ));
        }

        let config = LlmCallConfig {
            speed: None,
            verbosity: None,
            model: UTILITY_LLM_MODEL.to_string(),
            temperature: self.temperature,
            max_tokens: self.max_tokens,
            tools: Vec::new(),
            reasoning_effort: self.reasoning_effort.map(Into::into),
            metadata: self.metadata,
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
        Ok((self.messages, config))
    }
}

#[async_trait]
pub trait UtilityLlmService: Send + Sync {
    fn is_configured(&self) -> bool;

    async fn chat_completion(&self, request: UtilityLlmRequest) -> Result<LlmResponse>;

    async fn chat_completion_stream(&self, request: UtilityLlmRequest)
    -> Result<LlmResponseStream>;

    fn name(&self) -> &'static str {
        "UtilityLlmService"
    }
}

#[derive(Debug, Clone, Default)]
pub struct DisabledUtilityLlmService;

#[async_trait]
impl UtilityLlmService for DisabledUtilityLlmService {
    fn is_configured(&self) -> bool {
        false
    }

    async fn chat_completion(&self, _request: UtilityLlmRequest) -> Result<LlmResponse> {
        Err(AgentLoopError::llm("utility LLM service is disabled"))
    }

    async fn chat_completion_stream(
        &self,
        _request: UtilityLlmRequest,
    ) -> Result<LlmResponseStream> {
        Err(AgentLoopError::llm("utility LLM service is disabled"))
    }

    fn name(&self) -> &'static str {
        "DisabledUtilityLlmService"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LlmMessageRole;

    #[tokio::test]
    async fn disabled_service_reports_not_configured() {
        let service = DisabledUtilityLlmService;

        assert!(!service.is_configured());
        let error = service
            .chat_completion(UtilityLlmRequest::user_text("summarize this"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("disabled"));
    }

    #[test]
    fn request_builds_hardcoded_model_without_reasoning_by_default() {
        let request = UtilityLlmRequest::user_text("summarize this");
        let (messages, config) = request.into_driver_request().unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(config.model, UTILITY_LLM_MODEL);
        assert_eq!(config.reasoning_effort, None);
        assert!(config.tools.is_empty());
        assert!(config.tool_search.is_none());
    }

    #[test]
    fn request_accepts_supported_reasoning_efforts() {
        for (effort, expected) in [
            (UtilityLlmReasoningEffort::Low, "low"),
            (UtilityLlmReasoningEffort::Medium, "medium"),
            (UtilityLlmReasoningEffort::High, "high"),
        ] {
            let (_, config) = UtilityLlmRequest::new(vec![LlmMessage::text(
                LlmMessageRole::User,
                "classify this",
            )])
            .with_reasoning_effort(effort)
            .into_driver_request()
            .unwrap();

            assert_eq!(config.reasoning_effort.map(|e| e.as_str()), Some(expected));
        }
    }

    #[test]
    fn request_requires_messages() {
        let error = UtilityLlmRequest::new(vec![])
            .into_driver_request()
            .unwrap_err();

        assert!(error.to_string().contains("at least one message"));
    }
}
