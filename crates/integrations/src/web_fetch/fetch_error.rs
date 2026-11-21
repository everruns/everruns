//! FetchKit error messages shown to the model.

use super::tools::ToolExecutionResult;
use fetchkit::FetchError;

/// Map a fetchkit error to a ToolExecutionResult.
pub(super) fn map_error(e: FetchError) -> ToolExecutionResult {
    let error_message = match e {
        FetchError::MissingUrl => "Missing required parameter: url".to_string(),
        FetchError::InvalidUrlScheme => {
            "Invalid URL: must start with http:// or https://".to_string()
        }
        FetchError::InvalidMethod => super::request::INVALID_METHOD.to_string(),
        FetchError::BlockedUrl => "URL is blocked by policy".to_string(),
        FetchError::ClientBuildError(_) => "Failed to create HTTP client".to_string(),
        FetchError::FirstByteTimeout => "Request timed out waiting for the server".to_string(),
        FetchError::ConnectError(_) => "Failed to connect to server".to_string(),
        // FetchKit flattens its custom transport's Connect variant to this
        // fixed RequestError message.
        FetchError::RequestError(msg) if msg == "failed to connect to server" => {
            "Failed to connect to server".to_string()
        }
        FetchError::RequestError(msg) => format!("Request failed: {msg}"),
        FetchError::FetcherError(msg) => format!("Fetch error: {msg}"),
        FetchError::SaveError(msg) => format!("Failed to save file: {msg}"),
        FetchError::SaverNotAvailable => "File saving not available".to_string(),
        FetchError::RenderNotAvailable => "Rendered fetch backend not available".to_string(),
    };
    ToolExecutionResult::tool_error(error_message)
}
