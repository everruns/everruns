//! Browserless API key resolution, browser session state, and parameter helpers.
//!
//! Decision: API token ONLY comes from user connection (Settings > Connections > Browserless).
//!   Never stored in session secrets. Session state only tracks the WS endpoint.

use everruns_core::tools::ToolExecutionResult;
use everruns_core::traits::ToolContext;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, error};

// ============================================================================
// Constants
// ============================================================================

/// Session storage key for browser session state.
const BROWSER_SESSION_KEY: &str = "browserless_browser_session";

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

// ============================================================================
// Browser Session State (CDP persistent sessions)
// ============================================================================

/// State for an active CDP browser session.
/// Only stores the WS endpoint — the API token is always resolved from the
/// connection provider, never stored in session state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserSessionState {
    /// The WebSocket endpoint to reconnect to.
    pub ws_endpoint: String,
    /// When this session was created.
    pub created_at: String,
    /// Last reconnect time.
    pub last_active_at: String,
}

impl BrowserSessionState {
    pub fn new(ws_endpoint: String) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            ws_endpoint,
            created_at: now.clone(),
            last_active_at: now,
        }
    }

    /// Build the full reconnect URL (endpoint + token query param).
    /// Token is passed in — never stored in session state.
    pub fn reconnect_url(&self, api_token: &str) -> String {
        let sep = if self.ws_endpoint.contains('?') {
            "&"
        } else {
            "?"
        };
        format!("{}{}token={}", self.ws_endpoint, sep, api_token)
    }
}

/// Save browser session state to session storage (plain key-value, not secret).
pub async fn save_browser_session(
    context: &ToolContext,
    state: &BrowserSessionState,
) -> Result<(), ToolExecutionResult> {
    let storage = context.storage_store.as_ref().ok_or_else(|| {
        ToolExecutionResult::tool_error(
            "Session storage not available. The 'session_storage' capability may be required.",
        )
    })?;

    let json_str = serde_json::to_string(state).map_err(|e| {
        ToolExecutionResult::tool_error(format!("Failed to serialize browser session: {e}"))
    })?;

    storage
        .set_value(context.session_id, BROWSER_SESSION_KEY, &json_str)
        .await
        .map_err(|e| {
            ToolExecutionResult::tool_error(format!("Failed to save browser session: {e}"))
        })?;

    debug!("Saved browser session state");
    Ok(())
}

/// Load browser session state from session storage. Returns None if no active session.
pub async fn get_browser_session(
    context: &ToolContext,
) -> Result<Option<BrowserSessionState>, ToolExecutionResult> {
    let storage = match context.storage_store.as_ref() {
        Some(s) => s,
        None => return Ok(None),
    };

    let json_str = storage
        .get_value(context.session_id, BROWSER_SESSION_KEY)
        .await
        .map_err(|e| {
            ToolExecutionResult::tool_error(format!("Failed to load browser session: {e}"))
        })?;

    match json_str {
        Some(s) => {
            let state: BrowserSessionState = serde_json::from_str(&s).map_err(|e| {
                ToolExecutionResult::tool_error(format!(
                    "Failed to deserialize browser session: {e}"
                ))
            })?;
            Ok(Some(state))
        }
        None => Ok(None),
    }
}

/// Delete browser session state from session storage.
pub async fn delete_browser_session(context: &ToolContext) -> Result<(), ToolExecutionResult> {
    let storage = match context.storage_store.as_ref() {
        Some(s) => s,
        None => return Ok(()),
    };

    storage
        .delete_value(context.session_id, BROWSER_SESSION_KEY)
        .await
        .map_err(|e| {
            ToolExecutionResult::tool_error(format!("Failed to delete browser session: {e}"))
        })?;

    debug!("Deleted browser session state");
    Ok(())
}

// ============================================================================
// Parameter Helpers
// ============================================================================

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

    #[test]
    fn test_browser_session_state_new() {
        let state = BrowserSessionState::new("wss://example.com/browser/abc123".to_string());
        assert_eq!(state.ws_endpoint, "wss://example.com/browser/abc123");
        assert!(!state.created_at.is_empty());
        assert!(!state.last_active_at.is_empty());
    }

    #[test]
    fn test_browser_session_reconnect_url_no_query() {
        let state = BrowserSessionState::new("wss://example.com/browser/abc123".to_string());
        assert_eq!(
            state.reconnect_url("my_token"),
            "wss://example.com/browser/abc123?token=my_token"
        );
    }

    #[test]
    fn test_browser_session_reconnect_url_with_query() {
        let state =
            BrowserSessionState::new("wss://example.com/browser/abc123?param=1".to_string());
        assert_eq!(
            state.reconnect_url("my_token"),
            "wss://example.com/browser/abc123?param=1&token=my_token"
        );
    }

    #[test]
    fn test_browser_session_serialization_roundtrip() {
        let state = BrowserSessionState::new("wss://example.com/browser/abc123".to_string());
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: BrowserSessionState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.ws_endpoint, state.ws_endpoint);
        assert_eq!(deserialized.created_at, state.created_at);
        // No api_token in serialized state
        assert!(!json.contains("api_token"));
    }
}
