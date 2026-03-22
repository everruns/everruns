//! CDP session management tools: open_browser and close_browser.
//!
//! Decision: Persistent browser sessions via CDP WebSocket + Browserless.reconnect.
//! Decision: Session state stores only WS endpoint (no secrets). API token always
//!   resolved from user connection at call time.
//! Decision: Each tool call reconnects → does work → calls reconnect → disconnects.
//!   No long-lived WebSocket connections. The browser stays alive on Browserless servers.

use everruns_core::UpsertLeasedResource;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;

use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::cdp::{CdpSession, DEFAULT_RECONNECT_TIMEOUT_MS};
use crate::state::{
    BrowserSessionState, browser_session_external_id, delete_browser_session, get_api_token,
    get_browser_session, save_browser_session,
};
use crate::validation::validate_browserless_url;

const BROWSER_SESSION_LEASE_DURATION_SECONDS: u32 = 20 * 60;

async fn upsert_browser_session_lease(
    context: &ToolContext,
    state: &BrowserSessionState,
) -> Result<(), ToolExecutionResult> {
    let Some(store) = context.leased_resource_store.as_ref() else {
        return Ok(());
    };

    let owner_user_id = if let Some(resolver) = context.connection_resolver.as_ref() {
        resolver
            .get_connection_user(context.session_id, "browserless")
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    store
        .upsert_resource(UpsertLeasedResource {
            session_id: context.session_id,
            provider: "browserless".to_string(),
            resource_type: "browser_session".to_string(),
            external_id: browser_session_external_id(&state.ws_endpoint),
            display_name: Some("Persistent browser session".to_string()),
            owner_user_id,
            lease_duration_seconds: BROWSER_SESSION_LEASE_DURATION_SECONDS,
            // THREAT[TM-API-015]: leased-resource metadata is exposed via API/UI.
            // Persist only the tokenless reconnect endpoint and timestamps here;
            // the Browserless API token is resolved from the user connection
            // during cleanup instead of being stored with the lease.
            metadata: json!({
                "ws_endpoint": state.ws_endpoint,
                "created_at": state.created_at,
                "last_active_at": state.last_active_at,
            }),
        })
        .await
        .map_err(|e| {
            ToolExecutionResult::internal_error_msg(format!(
                "Failed to update Browserless browser lease: {e}"
            ))
        })?;

    Ok(())
}

async fn release_browser_session_lease(
    context: &ToolContext,
    state: &BrowserSessionState,
) -> Result<(), ToolExecutionResult> {
    let Some(store) = context.leased_resource_store.as_ref() else {
        return Ok(());
    };

    store
        .release_resource(
            context.session_id,
            "browserless",
            "browser_session",
            &browser_session_external_id(&state.ws_endpoint),
        )
        .await
        .map_err(|e| {
            ToolExecutionResult::internal_error_msg(format!(
                "Failed to release Browserless browser lease: {e}"
            ))
        })?;

    Ok(())
}

// ============================================================================
// BrowserlessOpenBrowserTool
// ============================================================================

pub struct BrowserlessOpenBrowserTool;

#[async_trait]
impl Tool for BrowserlessOpenBrowserTool {
    fn name(&self) -> &str {
        "browserless_open_browser"
    }

    fn description(&self) -> &str {
        "Open a persistent browser session. The browser stays alive between tool calls, \
         allowing multi-step workflows (e.g., login once, then navigate multiple pages). \
         Use browserless_close_browser when done to release the browser."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Optional initial URL to navigate to after opening the browser"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "How long the browser stays alive between tool calls, in milliseconds (default: 60000 = 60 seconds)"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("browserless_open_browser requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        context
            .emit_progress("browserless_open_browser", "Resolving connection…")
            .await;

        let api_token = match get_api_token(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };

        // Check if there's already an active session
        if let Ok(Some(existing)) = get_browser_session(context).await {
            let url = existing.reconnect_url(&api_token);
            match CdpSession::connect(&url).await {
                Ok(mut session) => {
                    let title = session.get_title().await.unwrap_or_default();
                    let current_url = session.get_url().await.unwrap_or_default();

                    let timeout_ms = arguments
                        .get("timeout_ms")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(DEFAULT_RECONNECT_TIMEOUT_MS);

                    match session.reconnect(timeout_ms).await {
                        Ok(new_endpoint) => {
                            let mut state = existing;
                            state.ws_endpoint = new_endpoint;
                            state.last_active_at = chrono::Utc::now().to_rfc3339();
                            if let Err(e) = save_browser_session(context, &state).await {
                                warn!("Failed to update session state: {e:?}");
                            }
                            if let Err(e) = upsert_browser_session_lease(context, &state).await {
                                session.disconnect().await;
                                return e;
                            }
                            session.disconnect().await;

                            return ToolExecutionResult::Success(json!({
                                "status": "already_open",
                                "message": "Browser session is already active.",
                                "title": title,
                                "url": current_url
                            }));
                        }
                        Err(_) => {
                            session.disconnect().await;
                            let _ = delete_browser_session(context).await;
                        }
                    }
                }
                Err(_) => {
                    let _ = delete_browser_session(context).await;
                }
            }
        }

        // Open a new browser session
        context
            .emit_progress("browserless_open_browser", "Connecting to browser…")
            .await;

        let ws_url = format!("{}?token={}", crate::BROWSERLESS_WS_BASE, api_token);

        let mut session = match CdpSession::connect(&ws_url).await {
            Ok(s) => s,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        // Navigate to initial URL if provided
        if let Some(url) = arguments
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|u| !u.is_empty())
        {
            context
                .emit_progress("browserless_open_browser", &format!("Navigating to {url}…"))
                .await;
            if let Err(e) = validate_browserless_url(url) {
                session.disconnect().await;
                return e;
            }
            if let Err(e) = session.navigate(url).await {
                session.disconnect().await;
                return ToolExecutionResult::tool_error(format!(
                    "Failed to navigate to {url}: {e}"
                ));
            }
        }

        let title = session.get_title().await.unwrap_or_default();
        let current_url = session.get_url().await.unwrap_or_default();

        let timeout_ms = arguments
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_RECONNECT_TIMEOUT_MS);

        context
            .emit_progress(
                "browserless_open_browser",
                "Registering session for persistence…",
            )
            .await;

        let ws_endpoint = match session.reconnect(timeout_ms).await {
            Ok(endpoint) => endpoint,
            Err(e) => {
                session.disconnect().await;
                return ToolExecutionResult::tool_error(format!(
                    "Failed to register browser for reconnection: {e}"
                ));
            }
        };

        // Save state (only WS endpoint, no token)
        let state = BrowserSessionState::new(ws_endpoint);
        if let Err(e) = save_browser_session(context, &state).await {
            session.disconnect().await;
            return e;
        }
        if let Err(e) = upsert_browser_session_lease(context, &state).await {
            session.disconnect().await;
            return e;
        }

        session.disconnect().await;
        debug!("Browser session opened and registered for reconnection");

        ToolExecutionResult::Success(json!({
            "status": "opened",
            "message": "Browser session opened. It will stay alive between tool calls. Use browserless_close_browser when done.",
            "title": title,
            "url": current_url,
            "timeout_ms": timeout_ms
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// BrowserlessCloseBrowserTool
// ============================================================================

pub struct BrowserlessCloseBrowserTool;

#[async_trait]
impl Tool for BrowserlessCloseBrowserTool {
    fn name(&self) -> &str {
        "browserless_close_browser"
    }

    fn description(&self) -> &str {
        "Close the persistent browser session opened by browserless_open_browser. \
         This releases the browser resources on Browserless servers."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("browserless_close_browser requires context.")
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let session_state = match get_browser_session(context).await {
            Ok(Some(state)) => state,
            Ok(None) => {
                return ToolExecutionResult::Success(json!({
                    "status": "no_session",
                    "message": "No active browser session to close."
                }));
            }
            Err(e) => return e,
        };

        // Resolve token from connection to build reconnect URL
        let api_token = match get_api_token(context).await {
            Ok(v) => v,
            Err(e) => {
                // Can't resolve token — just clean up state, browser will expire on its own
                let _ = delete_browser_session(context).await;
                return e;
            }
        };

        // Reconnect and close (don't call reconnect — browser will be destroyed)
        let url = session_state.reconnect_url(&api_token);
        match CdpSession::connect(&url).await {
            Ok(session) => {
                session.disconnect().await;
                debug!("Browser session closed (disconnected without reconnect)");
            }
            Err(e) => {
                debug!("Browser session already expired: {e}");
            }
        }

        if let Err(e) = delete_browser_session(context).await {
            return e;
        }
        if let Err(e) = release_browser_session_lease(context, &session_state).await {
            return e;
        }

        ToolExecutionResult::Success(json!({
            "status": "closed",
            "message": "Browser session closed. Resources have been released."
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// CDP Session Helpers (for use by other tools)
// ============================================================================

/// Try to get an active CDP session for the current context.
/// Resolves API token from connection provider. Returns None if no session exists
/// or if reconnection fails.
pub async fn try_get_cdp_session(context: &ToolContext) -> Option<CdpSession> {
    let state = get_browser_session(context).await.ok()??;

    // Resolve token from connection — never from session state
    let api_token = get_api_token(context).await.ok()?;
    let url = state.reconnect_url(&api_token);

    match CdpSession::connect(&url).await {
        Ok(session) => Some(session),
        Err(e) => {
            debug!("CDP reconnect failed (session may have expired): {e}");
            let _ = delete_browser_session(context).await;
            None
        }
    }
}

/// After using a CDP session, call reconnect to keep the browser alive
/// and update the stored endpoint.
pub async fn keep_session_alive(context: &ToolContext, session: &mut CdpSession) {
    match session.reconnect(DEFAULT_RECONNECT_TIMEOUT_MS).await {
        Ok(new_endpoint) => {
            if let Ok(Some(mut state)) = get_browser_session(context).await {
                state.ws_endpoint = new_endpoint;
                state.last_active_at = chrono::Utc::now().to_rfc3339();
                let _ = save_browser_session(context, &state).await;
                let _ = upsert_browser_session_lease(context, &state).await;
            }
        }
        Err(e) => {
            warn!("Failed to keep browser session alive: {e}");
            let _ = delete_browser_session(context).await;
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_browser_metadata() {
        let tool = BrowserlessOpenBrowserTool;
        assert_eq!(tool.name(), "browserless_open_browser");
        assert!(tool.requires_context());
        let schema = tool.parameters_schema();
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn test_close_browser_metadata() {
        let tool = BrowserlessCloseBrowserTool;
        assert_eq!(tool.name(), "browserless_close_browser");
        assert!(tool.requires_context());
        let schema = tool.parameters_schema();
        assert_eq!(schema["additionalProperties"], false);
    }

    #[tokio::test]
    async fn test_open_browser_no_context() {
        let tool = BrowserlessOpenBrowserTool;
        let result = tool.execute(json!({})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            other => panic!("Expected ToolError, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_close_browser_no_context() {
        let tool = BrowserlessCloseBrowserTool;
        let result = tool.execute(json!({})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            other => panic!("Expected ToolError, got: {other:?}"),
        }
    }
}
