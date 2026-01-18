// Error types for the agent loop

use thiserror::Error;
use uuid::Uuid;

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
    AgentNotFound(Uuid),

    /// Session not found
    #[error("Session not found: {0}")]
    SessionNotFound(Uuid),

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
    pub fn agent_not_found(agent_id: Uuid) -> Self {
        AgentLoopError::AgentNotFound(agent_id)
    }

    /// Create a session not found error
    pub fn session_not_found(session_id: Uuid) -> Self {
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

    /// Check if this is a request-too-large error
    pub fn is_request_too_large(&self) -> bool {
        matches!(self, AgentLoopError::RequestTooLarge(_))
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
}
