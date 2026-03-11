//! Browserless API key resolution and parameter helpers.

use everruns_core::tools::ToolExecutionResult;
use everruns_core::traits::ToolContext;

use serde_json::Value;
use tracing::error;

// ============================================================================
// API Key Resolution
// ============================================================================

/// Resolve Browserless API token via user connection (Settings > Connections > Browserless).
pub async fn get_api_token(context: &ToolContext) -> Result<String, ToolExecutionResult> {
    if let Some(resolver) = context.connection_resolver.as_ref() {
        match resolver
            .get_connection_token(context.session_id, "browserless")
            .await
        {
            Ok(Some(key)) => return Ok(key),
            Ok(None) => {} // fall through to error
            Err(e) => {
                error!("Failed to resolve Browserless user connection: {e}");
            }
        }
    }

    Err(ToolExecutionResult::tool_error(
        "Browserless API token not configured.\n\n\
         Set up your API token in **Settings > Connections > Browserless**.\n\n\
         Get your token at https://cloud.browserless.io/ under API Keys.",
    ))
}

/// Extract a required string parameter from tool arguments.
pub fn required_str<'a>(args: &'a Value, name: &str) -> Result<&'a str, ToolExecutionResult> {
    args.get(name).and_then(|v| v.as_str()).ok_or_else(|| {
        ToolExecutionResult::tool_error(format!("Missing required parameter: {name}"))
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_required_str_present() {
        let args = serde_json::json!({"url": "https://example.com"});
        let result = required_str(&args, "url");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "https://example.com");
    }

    #[test]
    fn test_required_str_missing() {
        let args = serde_json::json!({"other": "value"});
        let result = required_str(&args, "url");
        assert!(result.is_err());
    }

    #[test]
    fn test_required_str_null_value() {
        let args = serde_json::json!({"url": null});
        let result = required_str(&args, "url");
        assert!(result.is_err());
    }

    #[test]
    fn test_required_str_non_string_value() {
        let args = serde_json::json!({"url": 42});
        let result = required_str(&args, "url");
        assert!(result.is_err());
    }
}
