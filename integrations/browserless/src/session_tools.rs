//! CDP session management tools: open_browser and close_browser.
//!
//! Decision: Persistent browser sessions via CDP WebSocket + Browserless.reconnect.
//! Decision: Session state stores only WS endpoint (no secrets). API token always
//!   resolved from user connection at call time.
//! Decision: Each tool call reconnects → does work → calls reconnect → disconnects.
//!   No long-lived WebSocket connections. The browser stays alive on Browserless servers.

use everruns_contracts::runtime::UpsertLeasedResource;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::tool_types::ToolHints;

use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::browser_egress::BrowserEgress;
use crate::cdp::{CdpSession, DEFAULT_RECONNECT_TIMEOUT_MS, is_cdp_refused};
use crate::state::{
    BrowserSessionState, browser_session_external_id, delete_browser_session, delete_cookies,
    get_api_token, get_browser_session, load_cookies, save_browser_session, save_cookies,
};
use crate::validation::validate_browserless_navigation;

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

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("browserless_open_browser requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        if let Some(url) = arguments
            .get("url")
            .and_then(|value| value.as_str())
            .filter(|url| !url.is_empty())
            && let Err(error) =
                validate_browserless_navigation(context.network_access.as_ref(), url)
        {
            return error;
        }

        context
            .emit_progress("browserless_open_browser", "Resolving connection…")
            .await;

        let api_token = match get_api_token(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };

        // Check if there's already an active session
        if let Ok(Some(existing)) = get_browser_session(context).await {
            if let Ok(url) = existing.validated_reconnect_url(&api_token) {
                match CdpSession::connect(
                    &url,
                    BrowserEgress::for_context(context),
                    existing.browser_context_id.as_deref(),
                )
                .await
                {
                    Ok(mut session) => {
                        let landing_blocked = session
                            .reset_if_landing_blocked(context.network_access.as_ref())
                            .await
                            .err();
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
                                state.browser_context_id =
                                    Some(session.browser_context_id().to_string());
                                state.last_active_at = chrono::Utc::now().to_rfc3339();
                                if let Err(e) = save_browser_session(context, &state).await {
                                    warn!("Failed to update session state: {e:?}");
                                }
                                if let Err(e) = upsert_browser_session_lease(context, &state).await
                                {
                                    session.disconnect().await;
                                    return e;
                                }
                                session.disconnect().await;

                                if let Some(message) = landing_blocked {
                                    return ToolExecutionResult::tool_error(message);
                                }
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
            } else {
                // Stored session state was invalid/tampered. Discard and open a new session.
                let _ = delete_browser_session(context).await;
            }
        }

        // Open a new browser session
        context
            .emit_progress("browserless_open_browser", "Connecting to browser…")
            .await;

        let ws_url = crate::browser_session_url(&crate::browserless_ws_base(), &api_token);

        let mut session =
            match CdpSession::connect(&ws_url, BrowserEgress::for_context(context), None).await {
                Ok(s) => s,
                Err(e) => return ToolExecutionResult::tool_error(e),
            };

        // Navigate to initial URL if provided
        if let Some(url) = arguments
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|u| !u.is_empty())
        {
            // Emit progress with host only to avoid leaking query params/tokens
            let host = url
                .split("//")
                .nth(1)
                .unwrap_or(url)
                .split('/')
                .next()
                .unwrap_or("site")
                .split('?')
                .next()
                .unwrap_or("site")
                .split('#')
                .next()
                .unwrap_or("site");
            context
                .emit_progress(
                    "browserless_open_browser",
                    &format!("Navigating to {host}…"),
                )
                .await;
            if let Err(e) = session.navigate(url).await {
                session.disconnect().await;
                return ToolExecutionResult::tool_error(format!(
                    "Failed to navigate to {url}: {e}"
                ));
            }
            if let Err(error) = session
                .reset_if_landing_blocked(context.network_access.as_ref())
                .await
            {
                session.disconnect().await;
                return ToolExecutionResult::tool_error(error);
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

        // Save state (WS endpoint and guarded context id, no token)
        let mut state = BrowserSessionState::new(ws_endpoint);
        state.browser_context_id = Some(session.browser_context_id().to_string());
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

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_destructive(true)
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
        match session_state.validated_reconnect_url(&api_token) {
            Ok(url) => match CdpSession::connect_to_close(&url).await {
                Ok(session) => {
                    session.disconnect().await;
                    debug!("Browser session closed (disconnected without reconnect)");
                }
                Err(e) => {
                    debug!("Browser session already expired: {e}");
                }
            },
            Err(_) => {
                debug!("Discarded invalid stored browser session endpoint before close");
            }
        }

        if let Err(e) = delete_browser_session(context).await {
            return e;
        }
        if let Err(e) = release_browser_session_lease(context, &session_state).await {
            return e;
        }
        // Clear persisted cookies when browser session is closed
        delete_cookies(context).await;

        ToolExecutionResult::Success(json!({
            "status": "closed",
            "message": "Browser session closed. Resources and cookies have been released."
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
    let url = match state.validated_reconnect_url(&api_token) {
        Ok(url) => url,
        Err(_) => {
            debug!("Discarded invalid stored browser session endpoint before reconnect");
            let _ = delete_browser_session(context).await;
            return None;
        }
    };

    match CdpSession::connect(
        &url,
        BrowserEgress::for_context(context),
        state.browser_context_id.as_deref(),
    )
    .await
    {
        Ok(session) => Some(session),
        Err(e) => {
            debug!("CDP reconnect failed (session may have expired): {e}");
            let _ = delete_browser_session(context).await;
            None
        }
    }
}

/// A guarded browser one tool call drives.
///
/// `persistent` is the session's `browserless_open_browser` browser, kept
/// alive after the call. Otherwise it is a one-shot browser Browserless
/// destroys when this connection closes, which replaces the REST endpoints
/// whose browsers had direct network access (EVE-1189).
pub struct ToolBrowser {
    session: CdpSession,
    persistent: bool,
}

/// How a tool reaches a browser for one call.
pub enum Acquired {
    Browser(ToolBrowser),
    /// Browserless cloud refused CDP for this token; the REST endpoints run
    /// the call in Browserless' own network without the Everruns guard.
    RestFallback(String),
}

impl std::ops::Deref for ToolBrowser {
    type Target = CdpSession;

    fn deref(&self) -> &CdpSession {
        &self.session
    }
}

impl std::ops::DerefMut for ToolBrowser {
    fn deref_mut(&mut self) -> &mut CdpSession {
        &mut self.session
    }
}

impl ToolBrowser {
    /// `"cdp"` for the persistent browser, `"one_shot"` otherwise.
    pub fn mode(&self) -> &'static str {
        if self.persistent { "cdp" } else { "one_shot" }
    }

    /// Persist a one-shot browser's cookies for the next call. The persistent
    /// browser keeps its own cookie jar.
    pub async fn save_cookies_if_one_shot(&mut self, context: &ToolContext) {
        if self.persistent {
            return;
        }
        match self.session.context_cookies().await {
            Ok(cookies) => {
                if let Err(e) = save_cookies(context, &cookies).await {
                    debug!("Failed to persist cookies: {e}");
                }
            }
            Err(e) => debug!("Failed to read browser cookies: {e}"),
        }
    }

    /// Keep the persistent browser alive, then disconnect. A one-shot browser
    /// is destroyed by the disconnect.
    pub async fn release(mut self, context: &ToolContext) {
        if self.persistent {
            keep_session_alive(context, &mut self.session).await;
        }
        self.session.disconnect().await;
    }
}

/// THREAT[TM-TOOL-015][TM-TOOL-056]: Unguarded REST is used only when
/// Browserless cloud refuses CDP for the token. Cloud browsers run on
/// Browserless infrastructure, outside Everruns and operator networks. A
/// self-hosted Browserless (`BROWSERLESS_API_BASE` on another host) never
/// falls back, because its browser sits inside the operator's network.
fn unguarded_rest_allowed() -> bool {
    reqwest::Url::parse(&crate::browserless_api_base())
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| host.ends_with(".browserless.io"))
}

/// The session's persistent browser, or a one-shot guarded browser.
///
/// Stored cookies are seeded into a one-shot browser, matching the REST path.
pub async fn acquire_browser(context: &ToolContext) -> Result<Acquired, ToolExecutionResult> {
    if let Some(session) = try_get_cdp_session(context).await {
        return Ok(Acquired::Browser(ToolBrowser {
            session,
            persistent: true,
        }));
    }

    let api_token = get_api_token(context).await?;
    let ws_url = crate::browser_session_url(&crate::browserless_ws_base(), &api_token);
    match CdpSession::connect(&ws_url, BrowserEgress::for_context(context), None).await {
        Ok(mut session) => {
            let cookies = load_cookies(context).await;
            if let Err(e) = session.set_context_cookies(&cookies).await {
                debug!("Failed to seed stored cookies: {e}");
            }
            Ok(Acquired::Browser(ToolBrowser {
                session,
                persistent: false,
            }))
        }
        Err(e) if is_cdp_refused(&e) && unguarded_rest_allowed() => {
            debug!("Browserless refused CDP for this token; using REST endpoints");
            Ok(Acquired::RestFallback(api_token))
        }
        Err(e) => Err(ToolExecutionResult::tool_error(format!(
            "Could not open a guarded Browserless browser: {e}"
        ))),
    }
}

/// After using a CDP session, call reconnect to keep the browser alive
/// and update the stored endpoint.
pub async fn keep_session_alive(context: &ToolContext, session: &mut CdpSession) {
    match session.reconnect(DEFAULT_RECONNECT_TIMEOUT_MS).await {
        Ok(new_endpoint) => {
            if let Ok(Some(mut state)) = get_browser_session(context).await {
                state.ws_endpoint = new_endpoint;
                state.browser_context_id = Some(session.browser_context_id().to_string());
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

/// Record `ws_endpoint` as the session's persistent browser, creating the
/// session state and lease when none exists yet. Used by callers that opened a
/// browser implicitly (the `computer` tool) rather than through
/// `browserless_open_browser`.
pub async fn register_session_endpoint(
    context: &ToolContext,
    ws_endpoint: String,
    browser_context_id: &str,
) -> Result<(), ToolExecutionResult> {
    let mut state = match get_browser_session(context).await {
        Ok(Some(mut state)) => {
            state.ws_endpoint = ws_endpoint;
            state.last_active_at = chrono::Utc::now().to_rfc3339();
            state
        }
        _ => BrowserSessionState::new(ws_endpoint),
    };
    state.browser_context_id = Some(browser_context_id.to_string());
    save_browser_session(context, &state).await?;
    upsert_browser_session_lease(context, &state).await
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
