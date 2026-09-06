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
    async fn disabled_service_rejects_streaming_and_nonstreaming_requests() {
        let service = DisabledUtilityLlmService;
        assert!(!service.is_configured());
        let completion = service
            .chat_completion(UtilityLlmRequest::user_text("summarize this"))
            .await
            .unwrap_err();
        let stream = service
            .chat_completion_stream(UtilityLlmRequest::user_text("summarize this"))
            .await
            .err()
            .expect("disabled stream must fail before returning a stream");
        assert_eq!(
            completion.to_string(),
            "LLM error: utility LLM service is disabled"
        );
        assert_eq!(
            stream.to_string(),
            "LLM error: utility LLM service is disabled"
        );
    }

    #[test]
    fn default_request_preserves_user_text_and_disables_agent_controls() {
        let (messages, config) = UtilityLlmRequest::user_text("summarize α")
            .into_driver_request()
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, LlmMessageRole::User);
        assert_eq!(messages[0].content_as_text(), "summarize α");
        assert_eq!(config.model, "gpt-5.5");
        assert_eq!(config.reasoning_effort, None);
        assert_eq!(config.temperature, None);
        assert_eq!(config.max_tokens, None);
        assert!(config.metadata.is_empty());
        assert!(config.tools.is_empty());
        assert!(config.tool_search.is_none());
        assert!(config.previous_response_id.is_none());
        assert!(config.provider_opaque_context.is_none());
        assert!(config.prompt_cache.is_none());
        assert!(config.openrouter_routing.is_none());
        assert_eq!(config.parallel_tool_calls, None);
        assert_eq!(config.volatile_suffix_len, 0);
        assert!(config.extra_headers.is_empty());
        assert!(config.speed.is_none());
        assert!(config.verbosity.is_none());
        assert!(config.cache_diagnostics.is_none());
    }

    #[test]
    fn configured_requests_preserve_messages_limits_metadata_and_reasoning() {
        for (effort, expected) in [
            (UtilityLlmReasoningEffort::Low, "low"),
            (UtilityLlmReasoningEffort::Medium, "medium"),
            (UtilityLlmReasoningEffort::High, "high"),
        ] {
            let (messages, config) = UtilityLlmRequest::new(vec![
                LlmMessage::text(LlmMessageRole::System, "Classify"),
                LlmMessage::text(LlmMessageRole::User, "Input α"),
            ])
            .with_reasoning_effort(effort)
            .with_temperature(0.25)
            .with_max_tokens(123)
            .with_metadata("source", "old")
            .with_metadata("request", "request_1")
            .with_metadata("source", "judge")
            .into_driver_request()
            .unwrap();
            assert_eq!(
                messages
                    .iter()
                    .map(|m| (m.role.clone(), m.content_as_text()))
                    .collect::<Vec<_>>(),
                [
                    (LlmMessageRole::System, "Classify".into()),
                    (LlmMessageRole::User, "Input α".into())
                ]
            );
            assert_eq!(config.reasoning_effort.map(|e| e.as_str()), Some(expected));
            assert_eq!(effort.as_str(), expected);
            assert_eq!(config.temperature, Some(0.25));
            assert_eq!(config.max_tokens, Some(123));
            assert_eq!(
                config.metadata,
                HashMap::from([
                    ("source".into(), "judge".into()),
                    ("request".into(), "request_1".into())
                ])
            );
            assert_eq!(config.model, "gpt-5.5");
        }
    }

    #[test]
    fn request_requires_messages() {
        let error = UtilityLlmRequest::new(vec![])
            .into_driver_request()
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "LLM error: utility LLM request must include at least one message"
        );
    }
}
