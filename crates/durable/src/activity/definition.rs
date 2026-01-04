//! Activity trait definition

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::ActivityContext;

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

/// An activity is a unit of work that may fail and be retried
///
/// Activities are the building blocks of workflows. They represent
/// discrete operations that:
/// - Are executed by workers outside the workflow
/// - May take a long time to complete
/// - Can fail and be retried
/// - Can send heartbeats for liveness
///
/// # Example
///
/// ```ignore
/// use everruns_durable::prelude::*;
///
/// struct SendEmailActivity;
///
/// #[async_trait]
/// impl Activity for SendEmailActivity {
///     const TYPE: &'static str = "send_email";
///     type Input = SendEmailInput;
///     type Output = SendEmailOutput;
///
///     async fn execute(
///         &self,
///         ctx: &ActivityContext,
///         input: Self::Input,
///     ) -> Result<Self::Output, ActivityError> {
///         // Send email...
///         Ok(SendEmailOutput { message_id: "..." })
///     }
/// }
/// ```
#[async_trait]
pub trait Activity: Send + Sync + 'static {
    /// Unique type identifier for this activity
    ///
    /// This is used to look up the activity in the registry.
    const TYPE: &'static str;

    /// Input type for the activity
    type Input: Serialize + DeserializeOwned + Send;

    /// Output type for the activity
    type Output: Serialize + DeserializeOwned + Send;

    /// Execute the activity
    ///
    /// The context provides:
    /// - Attempt information
    /// - Heartbeat functionality
    /// - Cancellation token
    ///
    /// # Errors
    ///
    /// Return `ActivityError::retryable()` for transient failures that should be retried.
    /// Return `ActivityError::non_retryable()` for permanent failures.
    async fn execute(
        &self,
        ctx: &ActivityContext,
        input: Self::Input,
    ) -> Result<Self::Output, ActivityError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activity_error_retryable() {
        let error = ActivityError::retryable("timeout");
        assert!(error.retryable);
        assert_eq!(error.to_string(), "timeout");
    }

    #[test]
    fn test_activity_error_non_retryable() {
        let error = ActivityError::non_retryable("invalid input");
        assert!(!error.retryable);
    }

    #[test]
    fn test_activity_error_with_type() {
        let error = ActivityError::retryable("connection failed")
            .with_type("CONNECTION_ERROR");

        assert_eq!(error.error_type, Some("CONNECTION_ERROR".to_string()));
    }

    #[test]
    fn test_activity_error_serialization() {
        let error = ActivityError::retryable("test error")
            .with_type("TEST")
            .with_details(serde_json::json!({"key": "value"}));

        let json = serde_json::to_string(&error).unwrap();
        let parsed: ActivityError = serde_json::from_str(&json).unwrap();

        assert_eq!(error, parsed);
    }
}
