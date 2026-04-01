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
/// Uses a single left-to-right pass to avoid order-dependent or nested substitution.
pub fn substitute_secrets(s: &str, resolved: &std::collections::HashMap<String, String>) -> String {
    let mut result = String::with_capacity(s.len());
    let mut pos = 0;

    while let Some(rel_start) = s[pos..].find(SECRET_REF_PREFIX) {
        let start = pos + rel_start;
        result.push_str(&s[pos..start]);

        let name_start = start + SECRET_REF_PREFIX.len();
        let Some(rel_end) = s[name_start..].find(SECRET_REF_SUFFIX) else {
            // No closing suffix — push the rest verbatim
            result.push_str(&s[start..]);
            return result;
        };
        let name_end = name_start + rel_end;
        let name = &s[name_start..name_end];

        if let Some(value) = resolved.get(name) {
            result.push_str(value);
        } else {
            // Unknown secret — leave placeholder as-is
            result.push_str(&s[start..name_end + SECRET_REF_SUFFIX.len()]);
        }

        pos = name_end + SECRET_REF_SUFFIX.len();
    }

    result.push_str(&s[pos..]);
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
// Cookie Persistence (REST-mode)
// ============================================================================

/// Session secret key for persisted browser cookies.
/// Uses namespaced prefix to avoid collisions with user-set secrets.
const COOKIES_SECRET_KEY: &str = "browserless_internal:cookies";

/// Save cookies to session storage as an encrypted secret.
/// Cookies are a JSON array of Puppeteer cookie objects.
pub async fn save_cookies(context: &ToolContext, cookies: &[Value]) -> Result<(), String> {
    let storage = match context.storage_store.as_ref() {
        Some(s) => s,
        None => return Ok(()), // silently skip if no storage
    };
    let json_str =
        serde_json::to_string(cookies).map_err(|e| format!("Failed to serialize cookies: {e}"))?;
    storage
        .set_secret(context.session_id, COOKIES_SECRET_KEY, &json_str)
        .await
        .map_err(|e| format!("Failed to save cookies: {e}"))?;
    debug!("Saved {} cookies to session storage", cookies.len());
    Ok(())
}

/// Load stored cookies from session storage. Returns empty vec if none stored.
pub async fn load_cookies(context: &ToolContext) -> Vec<Value> {
    let storage = match context.storage_store.as_ref() {
        Some(s) => s,
        None => return Vec::new(),
    };
    match storage
        .get_secret(context.session_id, COOKIES_SECRET_KEY)
        .await
    {
        Ok(Some(json_str)) => match serde_json::from_str::<Vec<Value>>(&json_str) {
            Ok(cookies) => {
                debug!("Loaded {} stored cookies", cookies.len());
                cookies
            }
            Err(e) => {
                debug!("Failed to parse stored cookies: {e}; deleting corrupted entry");
                let _ = storage
                    .delete_secret(context.session_id, COOKIES_SECRET_KEY)
                    .await;
                Vec::new()
            }
        },
        _ => Vec::new(),
    }
}

/// Delete stored cookies from session storage.
pub async fn delete_cookies(context: &ToolContext) {
    let storage = match context.storage_store.as_ref() {
        Some(s) => s,
        None => return,
    };
    let _ = storage
        .delete_secret(context.session_id, COOKIES_SECRET_KEY)
        .await;
    debug!("Deleted stored cookies");
}

/// Generate Puppeteer code to inject stored cookies before navigation.
/// Returns empty string if no cookies are available.
pub fn build_cookie_injection_code(cookies: &[Value]) -> String {
    if cookies.is_empty() {
        return String::new();
    }
    let cookies_json = serde_json::to_string(cookies).unwrap_or_else(|_| "[]".to_string());
    format!("  await page.setCookie(...{cookies_json});\n")
}

/// Generate Puppeteer code to extract cookies after page interactions.
pub fn build_cookie_extraction_code() -> &'static str {
    "  const __cookies = await page.cookies();\n"
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

    /// Nested `${{secrets.*}}` inside a resolved value must NOT be recursively expanded.
    /// Single-pass left-to-right substitution guarantees deterministic behavior.
    #[test]
    fn test_substitute_secrets_nested_placeholder_in_value() {
        let mut resolved = std::collections::HashMap::new();
        resolved.insert("inner".to_string(), "INNER_VALUE".to_string());
        resolved.insert(
            "outer".to_string(),
            "prefix ${{secrets.inner}} suffix".to_string(),
        );

        let result = substitute_secrets("${{secrets.outer}}", &resolved);
        // Only the top-level placeholder is substituted; nested placeholder stays as-is
        assert_eq!(result, "prefix ${{secrets.inner}} suffix");
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

    // ====================================================================
    // Cookie persistence tests
    // ====================================================================

    #[test]
    fn test_build_cookie_injection_code_empty() {
        assert_eq!(build_cookie_injection_code(&[]), "");
    }

    #[test]
    fn test_build_cookie_injection_code_with_cookies() {
        let cookies = vec![serde_json::json!({
            "name": "sid",
            "value": "abc",
            "domain": "example.com"
        })];
        let code = build_cookie_injection_code(&cookies);
        assert!(code.contains("setCookie"), "should use page.setCookie");
        assert!(code.contains("sid"), "should include cookie name");
    }

    #[test]
    fn test_build_cookie_extraction_code() {
        let code = build_cookie_extraction_code();
        assert!(code.contains("page.cookies()"));
        assert!(code.contains("__cookies"));
    }
}
