// Error types for the agent loop

use crate::typed_id::{AgentId, HarnessId, SessionId};
use thiserror::Error;

/// Result type alias for agent loop operations
pub type Result<T> = std::result::Result<T, AgentLoopError>;

/// Errors that can occur during agent loop execution
#[derive(Debug, Error)]
pub enum AgentLoopError {
    /// LLM provider error
    #[error("LLM error: {0}")]
    Llm(String),

    /// Request too large error (context length exceeded, token limits, etc.)
    /// Contains the original error message for logging
    #[error("Request too large: {0}")]
    RequestTooLarge(String),

    /// Model not available (404, model not found, access denied for model)
    /// Contains the model_id string that was requested
    #[error("Model not available: {0}")]
    ModelNotAvailable(String),

    /// Tool execution error
    #[error("Tool execution error: {0}")]
    ToolExecution(String),

    /// Message store error
    #[error("Message store error: {0}")]
    MessageStore(String),

    /// Event emission error
    #[error("Event emission error: {0}")]
    EventEmission(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Configuration(String),

    /// Loop terminated due to max iterations
    #[error("Max iterations ({0}) reached")]
    MaxIterationsReached(usize),

    /// Loop was cancelled
    #[error("Loop cancelled")]
    Cancelled,

    /// No messages to process
    #[error("No messages to process")]
    NoMessages,

    /// Agent not found
    #[error("Agent not found: {0}")]
    AgentNotFound(AgentId),

    /// Harness not found
    #[error("Harness not found: {0}")]
    HarnessNotFound(HarnessId),

    /// Session not found
    #[error("Session not found: {0}")]
    SessionNotFound(SessionId),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),

    /// Driver not registered for provider type
    #[error(
        "No driver registered for provider type '{0}'. Make sure the driver is registered at startup."
    )]
    DriverNotRegistered(String),
}

impl AgentLoopError {
    /// Create an LLM error
    pub fn llm(msg: impl Into<String>) -> Self {
        AgentLoopError::Llm(msg.into())
    }

    /// Create a tool execution error
    pub fn tool(msg: impl Into<String>) -> Self {
        AgentLoopError::ToolExecution(msg.into())
    }

    /// Create a message store error
    pub fn store(msg: impl Into<String>) -> Self {
        AgentLoopError::MessageStore(msg.into())
    }

    /// Create an event emission error
    pub fn event(msg: impl Into<String>) -> Self {
        AgentLoopError::EventEmission(msg.into())
    }

    /// Create a configuration error
    pub fn config(msg: impl Into<String>) -> Self {
        AgentLoopError::Configuration(msg.into())
    }

    /// Create an agent not found error
    pub fn agent_not_found(agent_id: AgentId) -> Self {
        AgentLoopError::AgentNotFound(agent_id)
    }

    /// Create a harness not found error
    pub fn harness_not_found(harness_id: HarnessId) -> Self {
        AgentLoopError::HarnessNotFound(harness_id)
    }

    /// Create a session not found error
    pub fn session_not_found(session_id: SessionId) -> Self {
        AgentLoopError::SessionNotFound(session_id)
    }

    /// Create a driver not registered error
    pub fn driver_not_registered(provider_type: impl Into<String>) -> Self {
        AgentLoopError::DriverNotRegistered(provider_type.into())
    }

    /// Create a request too large error
    pub fn request_too_large(msg: impl Into<String>) -> Self {
        AgentLoopError::RequestTooLarge(msg.into())
    }

    /// Create a model not available error
    pub fn model_not_available(model_id: impl Into<String>) -> Self {
        AgentLoopError::ModelNotAvailable(model_id.into())
    }

    /// Check if this is a request-too-large error
    pub fn is_request_too_large(&self) -> bool {
        matches!(self, AgentLoopError::RequestTooLarge(_))
    }

    /// Check if this is a model-not-available error
    pub fn is_model_not_available(&self) -> bool {
        matches!(self, AgentLoopError::ModelNotAvailable(_))
    }

    /// Get the model ID if this is a model-not-available error
    pub fn model_not_available_id(&self) -> Option<&str> {
        match self {
            AgentLoopError::ModelNotAvailable(id) => Some(id),
            _ => None,
        }
    }

    /// Check if this is a rate-limit error (HTTP 429 or rate-limit keywords)
    pub fn is_rate_limited(&self) -> bool {
        match self {
            AgentLoopError::Llm(msg) => {
                let msg_lower = msg.to_ascii_lowercase();
                msg_lower.contains("(429)")
                    || msg_lower.contains("rate limit")
                    || msg_lower.contains("too many requests")
            }
            _ => false,
        }
    }

    /// Check if this is an authentication/authorization error (HTTP 401/403)
    pub fn is_auth_error(&self) -> bool {
        match self {
            AgentLoopError::Llm(msg) => msg.contains("(401)") || msg.contains("(403)"),
            _ => false,
        }
    }

    /// Check if this is a server error (HTTP 5xx or transient provider issue)
    pub fn is_server_error(&self) -> bool {
        match self {
            AgentLoopError::Llm(msg) => {
                msg.contains("(500)")
                    || msg.contains("(502)")
                    || msg.contains("(503)")
                    || msg.contains("(504)")
                    || msg.contains("(529)")
            }
            _ => false,
        }
    }

    /// Check if this error is deterministic and should never be retried.
    ///
    /// Non-retryable errors reference data that is permanently gone (e.g. a
    /// deleted message, a missing agent). Retrying will never succeed and only
    /// burns attempts while keeping the workflow stuck.
    ///
    /// Note: the durable worker currently uses string-matching via
    /// `is_non_retryable_task_error` because task errors arrive as strings.
    /// This method provides the typed equivalent for callers that have access
    /// to a structured `AgentLoopError`.
    pub fn is_non_retryable(&self) -> bool {
        match self {
            // Missing data is permanent — the entity was deleted.
            AgentLoopError::AgentNotFound(_)
            | AgentLoopError::HarnessNotFound(_)
            | AgentLoopError::SessionNotFound(_)
            | AgentLoopError::NoMessages => true,

            // Config/driver errors won't self-heal within retries.
            AgentLoopError::Configuration(_) | AgentLoopError::DriverNotRegistered(_) => true,

            // MessageStore "not found" errors (deleted messages).
            AgentLoopError::MessageStore(msg) => msg.to_ascii_lowercase().contains("not found"),

            // Everything else is potentially transient.
            _ => false,
        }
    }

    /// Get user-facing error message based on error classification
    pub fn user_facing_message(&self) -> String {
        if let Some(model_id) = self.model_not_available_id() {
            format!(
                "The model `{}` is not available. It may have been removed, \
                 renamed, or your API key may not have access to it. \
                 Please select a different model.",
                model_id
            )
        } else if self.is_request_too_large() {
            "The conversation has become too long for the model to process. \
             Please start a new session or reduce the context size."
                .to_string()
        } else if self.is_rate_limited() {
            "Rate limited by the AI provider. Please wait a moment.".to_string()
        } else if self.is_auth_error() {
            "There is a misconfiguration with the AI provider. Please contact support.".to_string()
        } else if self.is_server_error() {
            "The AI provider is experiencing issues. Please try again shortly.".to_string()
        } else {
            "I encountered an error while processing your request. Please try again later."
                .to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_request_too_large_returns_true_for_typed_error() {
        let err = AgentLoopError::request_too_large("context length exceeded");
        assert!(err.is_request_too_large());
    }

    #[test]
    fn test_is_request_too_large_returns_false_for_llm_error() {
        let err = AgentLoopError::llm("OpenAI API error (500): Internal server error");
        assert!(!err.is_request_too_large());
    }

    #[test]
    fn test_is_request_too_large_returns_false_for_other_errors() {
        let err = AgentLoopError::ToolExecution("some error".to_string());
        assert!(!err.is_request_too_large());

        let err = AgentLoopError::Cancelled;
        assert!(!err.is_request_too_large());
    }

    #[test]
    fn test_request_too_large_error_preserves_message() {
        let original_msg = "OpenAI API error (429): Request too large for gpt-4";
        let err = AgentLoopError::request_too_large(original_msg);
        assert_eq!(
            err.to_string(),
            format!("Request too large: {}", original_msg)
        );
    }

    #[test]
    fn test_is_model_not_available_returns_true_for_typed_error() {
        let err = AgentLoopError::model_not_available("claude-sonnet-4-6-20260217");
        assert!(err.is_model_not_available());
        assert_eq!(
            err.model_not_available_id(),
            Some("claude-sonnet-4-6-20260217")
        );
    }

    #[test]
    fn test_is_model_not_available_returns_false_for_llm_error() {
        let err = AgentLoopError::llm("some error");
        assert!(!err.is_model_not_available());
        assert_eq!(err.model_not_available_id(), None);
    }

    #[test]
    fn test_model_not_available_error_display() {
        let err = AgentLoopError::model_not_available("gpt-99");
        assert_eq!(err.to_string(), "Model not available: gpt-99");
    }

    #[test]
    fn test_is_rate_limited_detects_429() {
        let err = AgentLoopError::llm("Anthropic API error (429): rate limit exceeded");
        assert!(err.is_rate_limited());
    }

    #[test]
    fn test_is_rate_limited_detects_rate_limit_keyword() {
        let err =
            AgentLoopError::llm("Rate limit exceeded (after 2 retries, last error: too many)");
        assert!(err.is_rate_limited());
    }

    #[test]
    fn test_is_rate_limited_false_for_server_error() {
        let err = AgentLoopError::llm("Anthropic API error (500): internal server error");
        assert!(!err.is_rate_limited());
    }

    #[test]
    fn test_is_auth_error_detects_401() {
        let err = AgentLoopError::llm("Anthropic API error (401): invalid api key");
        assert!(err.is_auth_error());
    }

    #[test]
    fn test_is_auth_error_detects_403() {
        let err = AgentLoopError::llm("OpenAI API error (403): forbidden");
        assert!(err.is_auth_error());
    }

    #[test]
    fn test_is_server_error_detects_500() {
        let err = AgentLoopError::llm("Anthropic API error (500): internal server error");
        assert!(err.is_server_error());
    }

    #[test]
    fn test_is_server_error_detects_503() {
        let err = AgentLoopError::llm("OpenAI API error (503): service unavailable");
        assert!(err.is_server_error());
    }

    #[test]
    fn test_user_facing_message_rate_limited() {
        let err = AgentLoopError::llm("Anthropic API error (429): rate limit exceeded");
        assert_eq!(
            err.user_facing_message(),
            "Rate limited by the AI provider. Please wait a moment."
        );
    }

    #[test]
    fn test_user_facing_message_auth_error() {
        let err = AgentLoopError::llm("Anthropic API error (401): invalid api key");
        assert_eq!(
            err.user_facing_message(),
            "There is a misconfiguration with the AI provider. Please contact support."
        );
    }

    #[test]
    fn test_user_facing_message_server_error() {
        let err = AgentLoopError::llm("Anthropic API error (500): internal server error");
        assert_eq!(
            err.user_facing_message(),
            "The AI provider is experiencing issues. Please try again shortly."
        );
    }

    #[test]
    fn test_user_facing_message_generic_fallback() {
        let err = AgentLoopError::llm("Failed to send request: connection refused");
        assert_eq!(
            err.user_facing_message(),
            "I encountered an error while processing your request. Please try again later."
        );
    }

    #[test]
    fn test_user_facing_message_model_not_available() {
        let err = AgentLoopError::model_not_available("gpt-99");
        assert!(err.user_facing_message().contains("gpt-99"));
        assert!(err.user_facing_message().contains("not available"));
    }

    #[test]
    fn test_user_facing_message_request_too_large() {
        let err = AgentLoopError::request_too_large("context length exceeded");
        assert!(err.user_facing_message().contains("too long"));
    }
}
