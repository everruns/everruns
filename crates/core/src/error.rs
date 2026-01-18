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

    /// Check if this is a request-too-large error (context length exceeded, token limits, etc.)
    ///
    /// Detects errors from various providers:
    /// - OpenAI: 429 with "Request too large" or "tokens" type, or "context_length_exceeded"
    /// - Anthropic: 413/400 with context length messages
    pub fn is_request_too_large(&self) -> bool {
        match self {
            AgentLoopError::Llm(msg) => {
                let msg_lower = msg.to_lowercase();

                // OpenAI patterns
                // - "Request too large for gpt-4"
                // - "context_length_exceeded"
                // - 429 with tokens-related message
                if msg_lower.contains("request too large")
                    || msg_lower.contains("context_length_exceeded")
                    || msg_lower.contains("maximum context length")
                    || msg_lower.contains("reduce the length")
                {
                    return true;
                }

                // Token limit patterns (common across providers)
                // - "tokens per min (tpm): limit X, requested Y"
                // - "input or output tokens must be reduced"
                if (msg_lower.contains("tokens") || msg_lower.contains("token"))
                    && (msg_lower.contains("limit")
                        || msg_lower.contains("exceeded")
                        || msg_lower.contains("must be reduced"))
                {
                    return true;
                }

                // Anthropic patterns
                // - "request size exceeded maximum"
                // - "prompt is too long"
                if msg_lower.contains("request size exceeded")
                    || msg_lower.contains("prompt is too long")
                {
                    return true;
                }

                false
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_request_too_large_openai_request_too_large() {
        let err = AgentLoopError::llm(
            r#"OpenAI API error (429 Too Many Requests): {"error":{"message":"Request too large for gpt-5-mini in organization org-xxx on tokens per min (TPM): Limit 500000, Requested 538772."}}"#,
        );
        assert!(err.is_request_too_large());
    }

    #[test]
    fn test_is_request_too_large_openai_context_length_exceeded() {
        let err = AgentLoopError::llm(
            r#"OpenAI API error (400): {"error":{"code":"context_length_exceeded","message":"This model's maximum context length is 128000 tokens."}}"#,
        );
        assert!(err.is_request_too_large());
    }

    #[test]
    fn test_is_request_too_large_token_limit() {
        let err = AgentLoopError::llm(
            "The input or output tokens must be reduced in order to run successfully.",
        );
        assert!(err.is_request_too_large());
    }

    #[test]
    fn test_is_request_too_large_anthropic_prompt_too_long() {
        let err = AgentLoopError::llm(
            r#"Anthropic API error (400): {"error":{"message":"prompt is too long: 250000 tokens > 200000 maximum"}}"#,
        );
        assert!(err.is_request_too_large());
    }

    #[test]
    fn test_is_request_too_large_anthropic_request_size() {
        let err = AgentLoopError::llm(
            r#"Anthropic API error (413): {"error":{"message":"request size exceeded maximum"}}"#,
        );
        assert!(err.is_request_too_large());
    }

    #[test]
    fn test_is_request_too_large_false_for_other_errors() {
        let err = AgentLoopError::llm("OpenAI API error (500): Internal server error");
        assert!(!err.is_request_too_large());

        let err = AgentLoopError::llm("Network connection failed");
        assert!(!err.is_request_too_large());

        let err = AgentLoopError::llm("Rate limit exceeded: too many requests");
        assert!(!err.is_request_too_large());
    }

    #[test]
    fn test_is_request_too_large_false_for_non_llm_errors() {
        let err = AgentLoopError::ToolExecution("Request too large".to_string());
        assert!(!err.is_request_too_large());

        let err = AgentLoopError::Cancelled;
        assert!(!err.is_request_too_large());
    }
}
