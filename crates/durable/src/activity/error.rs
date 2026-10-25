//! Activity error type

use serde::{Deserialize, Serialize};

/// Error type for activity failures
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityError {
    /// Error message
    pub message: String,

    /// Error type/code for programmatic handling
    pub error_type: Option<String>,

    /// Whether this error is retryable
    ///
    /// Non-retryable errors will immediately fail the activity
    /// without further retry attempts.
    pub retryable: bool,

    /// Additional error details (for debugging)
    pub details: Option<serde_json::Value>,
}

impl ActivityError {
    /// Create a new retryable error
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            error_type: None,
            retryable: true,
            details: None,
        }
    }

    /// Create a non-retryable error
    pub fn non_retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            error_type: None,
            retryable: false,
            details: None,
        }
    }

    /// Set the error type
    pub fn with_type(mut self, error_type: impl Into<String>) -> Self {
        self.error_type = Some(error_type.into());
        self
    }

    /// Add error details
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl std::fmt::Display for ActivityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ActivityError {}

impl From<anyhow::Error> for ActivityError {
    fn from(err: anyhow::Error) -> Self {
        Self::retryable(err.to_string())
    }
}
