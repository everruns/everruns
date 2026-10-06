//! Workflow error type

use serde::Serialize;

/// Error type for workflow failures
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct WorkflowError {
    /// Error message
    pub message: String,

    /// Error code for programmatic handling
    pub code: Option<String>,

    /// Whether this error is retryable
    pub retryable: bool,
}

impl WorkflowError {
    /// Create a new workflow error
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
            retryable: false,
        }
    }

    /// Create a retryable error
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
            retryable: true,
        }
    }

    /// Set the error code
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for WorkflowError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_error_display() {
        let error = WorkflowError::new("something went wrong");
        assert_eq!(error.to_string(), "something went wrong");
    }

    #[test]
    fn test_workflow_error_with_code() {
        let error = WorkflowError::new("not found").with_code("NOT_FOUND");
        assert_eq!(error.code, Some("NOT_FOUND".to_string()));
    }

    #[test]
    fn test_workflow_error_retryable() {
        let error = WorkflowError::retryable("temporary failure");
        assert!(error.retryable);
    }
}
