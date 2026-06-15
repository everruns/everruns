//! Error types for FetchKit

use thiserror::Error;

/// Errors that can occur during fetch operations
///
/// # Examples
///
/// ```
/// use fetchkit::FetchError;
///
/// let err = FetchError::MissingUrl;
/// assert_eq!(err.to_string(), "Missing required parameter: url");
/// ```
#[derive(Debug, Error)]
pub enum FetchError {
    /// URL is missing
    #[error("Missing required parameter: url")]
    MissingUrl,

    /// URL has invalid scheme
    #[error("Invalid URL: must start with http:// or https://")]
    InvalidUrlScheme,

    /// Invalid HTTP method
    #[error("Invalid method: must be GET or HEAD")]
    InvalidMethod,

    /// URL is blocked by policy (prefix list or DNS policy)
    #[error("Blocked URL: not allowed by policy")]
    BlockedUrl,

    /// Failed to build HTTP client
    #[error("Failed to create HTTP client")]
    ClientBuildError(#[source] reqwest::Error),

    /// Request timed out waiting for first byte
    #[error("Request timed out: server did not respond within 1 second")]
    FirstByteTimeout,

    /// Failed to connect to server
    #[error("Failed to connect to server")]
    ConnectError(#[source] reqwest::Error),

    /// Other request error
    #[error("Request failed: {0}")]
    RequestError(String),

    /// Fetcher-specific error
    #[error("Fetcher error: {0}")]
    FetcherError(String),

    /// File save failed
    #[error("Failed to save file: {0}")]
    SaveError(String),

    /// No FileSaver provided but save_to_file was requested
    #[error("File saving not available")]
    SaverNotAvailable,
}

impl FetchError {
    /// Create an error from a reqwest error
    ///
    // THREAT[TM-LEAK-001]: Sanitize reqwest errors to avoid leaking internal hostnames/IPs
    pub fn from_reqwest(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            FetchError::FirstByteTimeout
        } else if err.is_connect() {
            FetchError::ConnectError(err)
        } else if err.is_redirect() {
            FetchError::RequestError("redirect error".to_string())
        } else if err.is_body() {
            FetchError::RequestError("error reading response body".to_string())
        } else if err.is_decode() {
            FetchError::RequestError("error decoding response".to_string())
        } else {
            FetchError::RequestError("request failed".to_string())
        }
    }
}

/// Errors returned by the toolkit-library contract surface.
#[derive(Debug, Error)]
pub enum ToolError {
    /// Safe to surface to an LLM or end user.
    #[error("{0}")]
    UserFacing(String),
    /// Internal/operator-facing failure details.
    #[error("{0}")]
    Internal(String),
}

impl ToolError {
    /// Whether this error is safe to show to the LLM.
    pub fn is_user_facing(&self) -> bool {
        matches!(self, Self::UserFacing(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_messages() {
        assert_eq!(
            FetchError::MissingUrl.to_string(),
            "Missing required parameter: url"
        );
        assert_eq!(
            FetchError::InvalidUrlScheme.to_string(),
            "Invalid URL: must start with http:// or https://"
        );
        assert_eq!(
            FetchError::InvalidMethod.to_string(),
            "Invalid method: must be GET or HEAD"
        );
        assert_eq!(
            FetchError::BlockedUrl.to_string(),
            "Blocked URL: not allowed by policy"
        );
        assert_eq!(
            FetchError::FirstByteTimeout.to_string(),
            "Request timed out: server did not respond within 1 second"
        );
    }

    #[test]
    fn test_tool_error_classification() {
        assert!(ToolError::UserFacing("url is required".to_string()).is_user_facing());
        assert!(!ToolError::Internal("serde failure".to_string()).is_user_facing());
    }
}
