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
         Get your token at https://www.browserless.io/account/home under API Keys.",
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

/// Stable leased-resource identifier for a browser session.
///
/// Browserless reconnect endpoints may carry ephemeral query parameters.
/// The leased-resource key strips the query string so touch operations keep
/// updating the same logical browser session resource.
pub fn browser_session_external_id(ws_endpoint: &str) -> String {
    ws_endpoint
        .split('?')
        .next()
        .unwrap_or(ws_endpoint)
        .to_string()
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
// Secret Reference Resolution
// ============================================================================

/// Pattern: `${{secrets.name}}` where `name` is a session secret key.
/// Double-brace avoids collision with JS template literals.
const SECRET_REF_PREFIX: &str = "${{secrets.";
const SECRET_REF_SUFFIX: &str = "}}";

/// Extract all secret names referenced in `${{secrets.<name>}}` patterns within a string.
pub fn extract_secret_refs(s: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut search_from = 0;
    while let Some(start) = s[search_from..].find(SECRET_REF_PREFIX) {
        let abs_start = search_from + start + SECRET_REF_PREFIX.len();
        if let Some(end) = s[abs_start..].find(SECRET_REF_SUFFIX) {
            let name = &s[abs_start..abs_start + end];
            if !name.is_empty() && !refs.contains(&name.to_string()) {
                refs.push(name.to_string());
            }
            search_from = abs_start + end + SECRET_REF_SUFFIX.len();
        } else {
            break;
        }
    }
    refs
}

/// Replace all `${{secrets.<name>}}` patterns in `s` with resolved values.
pub fn substitute_secrets(s: &str, resolved: &std::collections::HashMap<String, String>) -> String {
    let mut result = s.to_string();
    for (name, value) in resolved {
        let pattern = format!("{SECRET_REF_PREFIX}{name}{SECRET_REF_SUFFIX}");
        result = result.replace(&pattern, value);
    }
    result
}

/// Resolve all `${{secrets.<name>}}` references found in interaction step `value` fields.
/// Returns a map of secret_name → plaintext_value.
/// Fails if any referenced secret is missing or session storage is unavailable.
pub async fn resolve_step_secrets(
    context: &ToolContext,
    steps: &[Value],
) -> Result<std::collections::HashMap<String, String>, ToolExecutionResult> {
    // Collect all unique secret names from step value fields
    let mut secret_names: Vec<String> = Vec::new();
    for step in steps {
        if let Some(val) = step.get("value").and_then(|v| v.as_str()) {
            for name in extract_secret_refs(val) {
                if !secret_names.contains(&name) {
                    secret_names.push(name);
                }
            }
        }
    }

    if secret_names.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let storage = context.storage_store.as_ref().ok_or_else(|| {
        ToolExecutionResult::tool_error(
            "Session storage not available. Secret references require the 'session_storage' capability.",
        )
    })?;

    let mut resolved = std::collections::HashMap::new();
    for name in &secret_names {
        match storage.get_secret(context.session_id, name).await {
            Ok(Some(value)) => {
                resolved.insert(name.clone(), value);
            }
            Ok(None) => {
                return Err(ToolExecutionResult::tool_error(format!(
                    "Secret '{name}' not found. Store it first with: secret_store set {name} <value>"
                )));
            }
            Err(e) => {
                return Err(ToolExecutionResult::tool_error(format!(
                    "Failed to resolve secret '{name}': {e}"
                )));
            }
        }
    }

    Ok(resolved)
}

/// Apply secret substitution to a cloned list of steps, replacing `${{secrets.*}}`
/// in `value` fields with resolved plaintext. Returns new steps (originals unchanged).
pub fn substitute_step_secrets(
    steps: &[Value],
    resolved: &std::collections::HashMap<String, String>,
) -> Vec<Value> {
    if resolved.is_empty() {
        return steps.to_vec();
    }
    steps
        .iter()
        .map(|step| {
            let mut step = step.clone();
            if let Some(val) = step.get("value").and_then(|v| v.as_str()) {
                let substituted = substitute_secrets(val, resolved);
                if substituted != val {
                    step.as_object_mut()
                        .unwrap()
                        .insert("value".to_string(), Value::String(substituted));
                }
            }
            step
        })
        .collect()
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

    /// Structural verification: session state must contain exactly 3 fields
    /// and never include any secret/token/credential data.
    #[test]
    fn test_browser_session_state_no_secrets_structural() {
        let state = BrowserSessionState::new("wss://example.com/browser/abc123".to_string());
        let json: serde_json::Value = serde_json::to_value(&state).unwrap();
        let obj = json.as_object().unwrap();

        // Exactly 3 fields: ws_endpoint, created_at, last_active_at
        assert_eq!(obj.len(), 3, "Session state must have exactly 3 fields");
        assert!(obj.contains_key("ws_endpoint"));
        assert!(obj.contains_key("created_at"));
        assert!(obj.contains_key("last_active_at"));

        // No field should contain secret-like names
        for key in obj.keys() {
            assert!(
                !key.contains("token")
                    && !key.contains("secret")
                    && !key.contains("password")
                    && !key.contains("credential")
                    && !key.contains("key"),
                "Session state must not contain secret-like field: {key}"
            );
        }
    }

    // ====================================================================
    // Secret reference tests
    // ====================================================================

    #[test]
    fn test_extract_secret_refs_none() {
        assert!(extract_secret_refs("plain text").is_empty());
        assert!(extract_secret_refs("").is_empty());
    }

    #[test]
    fn test_extract_secret_refs_single() {
        let refs = extract_secret_refs("${{secrets.my_password}}");
        assert_eq!(refs, vec!["my_password"]);
    }

    #[test]
    fn test_extract_secret_refs_multiple() {
        let refs = extract_secret_refs("user=${{secrets.user}} pass=${{secrets.pass}}");
        assert_eq!(refs, vec!["user", "pass"]);
    }

    #[test]
    fn test_extract_secret_refs_deduplicates() {
        let refs = extract_secret_refs("${{secrets.token}} and again ${{secrets.token}}");
        assert_eq!(refs, vec!["token"]);
    }

    #[test]
    fn test_extract_secret_refs_ignores_empty_name() {
        assert!(extract_secret_refs("${{secrets.}}").is_empty());
    }

    #[test]
    fn test_extract_secret_refs_ignores_unclosed() {
        assert!(extract_secret_refs("${{secrets.name").is_empty());
    }

    #[test]
    fn test_substitute_secrets_basic() {
        let mut resolved = std::collections::HashMap::new();
        resolved.insert("pw".to_string(), "hunter2".to_string());
        assert_eq!(substitute_secrets("${{secrets.pw}}", &resolved), "hunter2");
    }

    #[test]
    fn test_substitute_secrets_mixed() {
        let mut resolved = std::collections::HashMap::new();
        resolved.insert("token".to_string(), "abc123".to_string());
        assert_eq!(
            substitute_secrets("Bearer ${{secrets.token}}", &resolved),
            "Bearer abc123"
        );
    }

    #[test]
    fn test_substitute_secrets_no_match() {
        let resolved = std::collections::HashMap::new();
        assert_eq!(
            substitute_secrets("no refs here", &resolved),
            "no refs here"
        );
    }

    #[test]
    fn test_substitute_step_secrets_replaces_value_only() {
        let steps = vec![
            serde_json::json!({
                "action": "type",
                "selector": "#password",
                "value": "${{secrets.pw}}"
            }),
            serde_json::json!({
                "action": "click",
                "selector": "#submit"
            }),
        ];
        let mut resolved = std::collections::HashMap::new();
        resolved.insert("pw".to_string(), "hunter2".to_string());

        let result = substitute_step_secrets(&steps, &resolved);
        assert_eq!(result[0]["value"], "hunter2");
        assert_eq!(result[0]["selector"], "#password"); // untouched
        assert_eq!(result[1]["selector"], "#submit"); // untouched
    }

    #[test]
    fn test_substitute_step_secrets_empty_resolved() {
        let steps = vec![serde_json::json!({"action": "click", "selector": "#btn"})];
        let resolved = std::collections::HashMap::new();
        let result = substitute_step_secrets(&steps, &resolved);
        assert_eq!(result, steps);
    }
}
