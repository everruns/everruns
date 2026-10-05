//! Tool implementations for Browserless browser automation.
//!
//! Decision: 5 session-aware tools:
//!   1. browserless_screenshot - Take screenshot
//!   2. browserless_content   - Read DOM/HTML
//!   3. browserless_scrape    - Extract structured data via CSS selectors
//!   4. browserless_interact  - Click, type, navigate
//!   5. browserless_navigate  - Open URL and get page info
//!
//! When a CDP session is active (via browserless_open_browser), tools navigate within
//! the persistent browser, preserving login state and cookies across tool calls.
//! When no session exists, each call opens a one-shot guarded CDP browser.
//!
//! Decision (EVE-1189): every browser a tool drives is guarded, so Everruns
//! performs and checks each request it makes (`browser_egress.rs`). The REST
//! endpoints (`/screenshot`, `/content`, `/scrape`, `/function`) run browsers
//! with direct network access and remain only as a fallback for Browserless
//! cloud tokens that cannot open CDP sessions (`session_tools::acquire_browser`);
//! there the session ACL travels as Browserless rejection patterns.

use everruns_contracts::ToolResultImage;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::runtime::truncation_info::{TruncationInfo, TruncationReason};
use everruns_contracts::tool_types::ToolHints;

use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::debug;

use crate::cdp::CdpSession;
use crate::client::BrowserlessClient;
use crate::interaction_code::{build_interaction_code_with_policy, request_interception_prelude};
use crate::session_tools::{Acquired, acquire_browser};
use crate::state::{
    build_cookie_injection_code, extract_secret_refs, load_cookies, required_str,
    resolve_step_secrets, save_cookies, substitute_step_secrets,
};
#[cfg(test)]
use crate::validation::validate_browserless_url;
use crate::validation::{
    transport_reject_patterns, validate_browserless_navigation, validate_interaction_steps,
};

const MAX_HTML_BYTES: usize = 100_000;
const MAX_WAIT_MS: u64 = 120_000;

/// Attach the unified reading-tool truncation envelope (EVE-339) to a
/// `browserless_content` response.
///
/// `browserless_content` does not currently support in-place resume — there is
/// no `?offset` / `range` parameter on the REST endpoint, and CDP returns the
/// full DOM in one shot. The envelope reports `without_resume` when truncated
/// so LLM callers can pick a documented fallback (e.g. `browserless_scrape`
/// with narrower selectors).
fn attach_content_truncation(
    response: &mut Value,
    content_returned: &str,
    bytes_total: usize,
    was_truncated: bool,
) {
    let bytes_returned = content_returned.len();
    let info = if was_truncated {
        TruncationInfo::without_resume(bytes_returned, Some(bytes_total), TruncationReason::SizeCap)
    } else {
        TruncationInfo::not_truncated(bytes_returned)
    };
    info.attach(response);
}

/// Truncate HTML content if it exceeds MAX_HTML_BYTES. Safe for multi-byte UTF-8.
fn truncate_html(html: String) -> (String, bool) {
    let len = html.len();
    if len > MAX_HTML_BYTES {
        // Find a valid UTF-8 char boundary at or before MAX_HTML_BYTES
        let mut boundary = MAX_HTML_BYTES;
        while boundary > 0 && !html.is_char_boundary(boundary) {
            boundary -= 1;
        }
        let truncated = format!(
            "{}...\n\n[Truncated: {} total bytes]",
            &html[..boundary],
            len
        );
        (truncated, true)
    } else {
        (html, false)
    }
}

fn png_image_result(mut result: Value, base64: String, size_bytes: usize) -> ToolExecutionResult {
    if let Some(obj) = result.as_object_mut() {
        obj.insert("image_returned".to_string(), json!(true));
        obj.insert("format".to_string(), json!("png"));
        obj.insert("size_bytes".to_string(), json!(size_bytes));
    }
    ToolExecutionResult::success_with_images(
        result,
        vec![ToolResultImage {
            base64,
            media_type: "image/png".to_string(),
        }],
    )
}

#[cfg(test)]
fn validate_url(url: &str) -> Result<(), ToolExecutionResult> {
    validate_browserless_url(url)
}

fn validate_navigation(context: &ToolContext, url: &str) -> Result<(), ToolExecutionResult> {
    validate_browserless_navigation(context.network_access.as_ref(), url)
}

fn reject_patterns(context: &ToolContext) -> Vec<String> {
    context
        .network_access
        .as_ref()
        .map(transport_reject_patterns)
        .unwrap_or_default()
}

/// Navigate a guarded browser. Every request it makes, redirect hops
/// included, already passes the session ACL in `BrowserEgress`; the landing
/// check catches a URL the guard did not see (e.g. a client-side rewrite).
async fn begin_cdp_navigation(
    context: &ToolContext,
    session: &mut CdpSession,
    url: &str,
) -> Result<(), ToolExecutionResult> {
    session.navigate(url).await.map_err(|error| {
        ToolExecutionResult::tool_error(format!("CDP navigate failed: {error}"))
    })?;
    session
        .reset_if_landing_blocked(context.network_access.as_ref())
        .await
        .map_err(ToolExecutionResult::tool_error)?;
    Ok(())
}

/// Cap a wait/timeout value to MAX_WAIT_MS. // THREAT[TM-TOOL-016]
fn cap_wait_ms(ms: u64) -> u64 {
    ms.min(MAX_WAIT_MS)
}

/// THREAT: Never return DOM content when secret references were resolved in
/// interact steps. Typed secrets can be reflected into DOM/outerHTML.
fn should_suppress_interact_content(requested_content: bool, resolved_secret_count: usize) -> bool {
    requested_content && resolved_secret_count > 0
}

// ============================================================================
// BrowserlessScreenshotTool
// ============================================================================

pub struct BrowserlessScreenshotTool;

#[async_trait]
impl Tool for BrowserlessScreenshotTool {
    fn name(&self) -> &str {
        "browserless_screenshot"
    }

    fn description(&self) -> &str {
        "Take a screenshot of a web page. Returns a base64-encoded PNG image."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to screenshot"
                },
                "full_page": {
                    "type": "boolean",
                    "description": "Capture the full scrollable page (default: true)"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector to screenshot a specific element (optional)"
                },
                "wait_for_selector": {
                    "type": "string",
                    "description": "Wait for this CSS selector to appear before taking screenshot (optional)"
                },
                "wait_for_timeout": {
                    "type": "integer",
                    "description": "Wait this many milliseconds before taking screenshot (optional)"
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("browserless_screenshot requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let url = match required_str(&arguments, "url") {
            Ok(v) => v,
            Err(e) => return e,
        };
        if let Err(e) = validate_navigation(context, url) {
            return e;
        }

        let full_page = arguments
            .get("full_page")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // Guarded CDP browser: the persistent one, else a one-shot (EVE-1189)
        let acquired = match acquire_browser(context).await {
            Ok(acquired) => acquired,
            Err(e) => return e,
        };
        let api_token = match acquired {
            Acquired::RestFallback(api_token) => api_token,
            Acquired::Browser(mut session) => {
                let mode = session.mode();
                debug!("Using CDP session for screenshot");
                if let Err(e) = begin_cdp_navigation(context, &mut session, url).await {
                    session.release(context).await;
                    return e;
                }

                // Wait for selector if specified
                if let Some(sel) = arguments.get("wait_for_selector").and_then(|v| v.as_str()) {
                    let _ = session.wait_for_selector(sel, 30000).await;
                }
                if let Some(ms) = arguments.get("wait_for_timeout").and_then(|v| v.as_u64()) {
                    tokio::time::sleep(tokio::time::Duration::from_millis(cap_wait_ms(ms))).await;
                }

                let result = match arguments.get("selector").and_then(|v| v.as_str()) {
                    Some(selector) => session.screenshot_selector(selector).await,
                    None => session.screenshot(full_page).await,
                };
                session.release(context).await;

                return match result {
                    Ok(b64) => {
                        use base64::Engine;
                        let bytes = match base64::engine::general_purpose::STANDARD.decode(&b64) {
                            Ok(bytes) => bytes,
                            Err(e) => {
                                return ToolExecutionResult::tool_error(format!(
                                    "CDP screenshot returned invalid base64: {e}"
                                ));
                            }
                        };
                        png_image_result(
                            json!({
                                "url": url,
                                "session": mode
                            }),
                            b64,
                            bytes.len(),
                        )
                    }
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("CDP screenshot failed: {e}"))
                    }
                };
            }
        };

        // Fallback: REST API (Browserless cloud refused CDP for this token)

        let selector = arguments.get("selector").and_then(|v| v.as_str());
        let wait_for_selector = arguments.get("wait_for_selector").and_then(|v| v.as_str());
        let wait_for_timeout = arguments.get("wait_for_timeout").and_then(|v| v.as_u64());
        let stored_cookies = load_cookies(context).await;

        let client = BrowserlessClient::new(api_token);
        let patterns = reject_patterns(context);
        match client
            .screenshot_with_policy(
                url,
                full_page,
                selector,
                wait_for_selector,
                wait_for_timeout,
                &stored_cookies,
                &patterns,
            )
            .await
        {
            Ok(bytes) => {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                png_image_result(
                    json!({
                        "url": url,
                    }),
                    b64,
                    bytes.len(),
                )
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// BrowserlessContentTool
// ============================================================================

pub struct BrowserlessContentTool;

#[async_trait]
impl Tool for BrowserlessContentTool {
    fn name(&self) -> &str {
        "browserless_content"
    }

    fn description(&self) -> &str {
        "Get the fully rendered HTML content (DOM) of a web page, including JavaScript-rendered content."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to read"
                },
                "wait_for_selector": {
                    "type": "string",
                    "description": "Wait for this CSS selector to appear before reading content (optional)"
                },
                "wait_for_timeout": {
                    "type": "integer",
                    "description": "Wait this many milliseconds before reading content (optional)"
                },
                "best_attempt": {
                    "type": "boolean",
                    "description": "Continue even if async events fail or timeout (default: false)"
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("browserless_content requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let url = match required_str(&arguments, "url") {
            Ok(v) => v,
            Err(e) => return e,
        };
        if let Err(e) = validate_navigation(context, url) {
            return e;
        }

        // Guarded CDP browser: the persistent one, else a one-shot (EVE-1189)
        let acquired = match acquire_browser(context).await {
            Ok(acquired) => acquired,
            Err(e) => return e,
        };
        let api_token = match acquired {
            Acquired::RestFallback(api_token) => api_token,
            Acquired::Browser(mut session) => {
                let mode = session.mode();
                debug!("Using CDP session for content");
                if let Err(e) = begin_cdp_navigation(context, &mut session, url).await {
                    session.release(context).await;
                    return e;
                }

                if let Some(sel) = arguments.get("wait_for_selector").and_then(|v| v.as_str()) {
                    let _ = session.wait_for_selector(sel, 30000).await;
                }
                if let Some(ms) = arguments.get("wait_for_timeout").and_then(|v| v.as_u64()) {
                    tokio::time::sleep(tokio::time::Duration::from_millis(cap_wait_ms(ms))).await;
                }

                let result = session.get_content().await;
                session.release(context).await;

                return match result {
                    Ok(html) => {
                        let len = html.len();
                        let (content, was_truncated) = truncate_html(html);
                        let mut response = json!({
                            "url": url,
                            "content": content,
                            "size_bytes": len,
                            "truncated": was_truncated,
                            "session": mode
                        });
                        attach_content_truncation(&mut response, &content, len, was_truncated);
                        ToolExecutionResult::Success(response)
                    }
                    Err(e) => ToolExecutionResult::tool_error(format!("CDP content failed: {e}")),
                };
            }
        };

        // Fallback: REST API (Browserless cloud refused CDP for this token)

        let wait_for_selector = arguments.get("wait_for_selector").and_then(|v| v.as_str());
        let wait_for_timeout = arguments.get("wait_for_timeout").and_then(|v| v.as_u64());
        let best_attempt = arguments
            .get("best_attempt")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let stored_cookies = load_cookies(context).await;
        let client = BrowserlessClient::new(api_token);
        let patterns = reject_patterns(context);
        match client
            .content_with_policy(
                url,
                wait_for_selector,
                wait_for_timeout,
                best_attempt,
                &stored_cookies,
                &patterns,
            )
            .await
        {
            Ok(html) => {
                let len = html.len();
                let (content, was_truncated) = truncate_html(html);
                let mut response = json!({
                    "url": url,
                    "content": content,
                    "size_bytes": len,
                    "truncated": was_truncated
                });
                attach_content_truncation(&mut response, &content, len, was_truncated);
                ToolExecutionResult::Success(response)
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// BrowserlessScrapeTool
// ============================================================================

pub struct BrowserlessScrapeTool;

#[async_trait]
impl Tool for BrowserlessScrapeTool {
    fn name(&self) -> &str {
        "browserless_scrape"
    }

    fn description(&self) -> &str {
        "Extract structured data from a web page using CSS selectors. Returns JSON with matched elements."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to scrape"
                },
                "elements": {
                    "type": "array",
                    "description": "Array of element selectors to extract",
                    "items": {
                        "type": "object",
                        "properties": {
                            "selector": {
                                "type": "string",
                                "description": "CSS selector to match elements"
                            }
                        },
                        "required": ["selector"]
                    }
                },
                "wait_for_selector": {
                    "type": "string",
                    "description": "Wait for this CSS selector before scraping (optional)"
                },
                "wait_for_timeout": {
                    "type": "integer",
                    "description": "Wait this many milliseconds before scraping (optional)"
                }
            },
            "required": ["url", "elements"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("browserless_scrape requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let url = match required_str(&arguments, "url") {
            Ok(v) => v,
            Err(e) => return e,
        };
        if let Err(e) = validate_navigation(context, url) {
            return e;
        }

        let elements = match arguments.get("elements").and_then(|v| v.as_array()) {
            Some(arr) => arr.clone(),
            None => {
                return ToolExecutionResult::tool_error(
                    "Missing required parameter: elements (must be an array)",
                );
            }
        };

        let wait_for_selector = arguments.get("wait_for_selector").and_then(|v| v.as_str());
        let wait_for_timeout = arguments.get("wait_for_timeout").and_then(|v| v.as_u64());

        let api_token = match acquire_browser(context).await {
            Err(e) => return e,
            Ok(Acquired::RestFallback(api_token)) => api_token,
            Ok(Acquired::Browser(mut session)) => {
                if let Err(e) = begin_cdp_navigation(context, &mut session, url).await {
                    session.release(context).await;
                    return e;
                }
                if let Some(sel) = wait_for_selector {
                    let _ = session.wait_for_selector(sel, 30000).await;
                }
                if let Some(ms) = wait_for_timeout {
                    tokio::time::sleep(tokio::time::Duration::from_millis(cap_wait_ms(ms))).await;
                }
                let mode = session.mode();
                let result = session.scrape(&elements).await;
                session.release(context).await;
                return match result {
                    Ok(data) => ToolExecutionResult::Success(json!({
                        "url": url,
                        "data": data,
                        "session": mode
                    })),
                    Err(e) => ToolExecutionResult::tool_error(format!("CDP scrape failed: {e}")),
                };
            }
        };

        // Fallback: REST API (Browserless cloud refused CDP for this token)
        let stored_cookies = load_cookies(context).await;
        let client = BrowserlessClient::new(api_token);
        let patterns = reject_patterns(context);
        match client
            .scrape_with_policy(
                url,
                &elements,
                wait_for_selector,
                wait_for_timeout,
                &stored_cookies,
                &patterns,
            )
            .await
        {
            Ok(data) => ToolExecutionResult::Success(json!({
                "url": url,
                "data": data
            })),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// BrowserlessInteractTool
// ============================================================================

pub struct BrowserlessInteractTool;

#[async_trait]
impl Tool for BrowserlessInteractTool {
    fn name(&self) -> &str {
        "browserless_interact"
    }

    fn description(&self) -> &str {
        "Interact with a web page: navigate to a URL, then perform a sequence of actions \
         (click, type, keyboard, mouse, touch, scroll, wait). Returns the page title, \
         final URL, and optionally a screenshot or DOM content after all steps complete.\n\n\
         Secret references: Use ${{secrets.<name>}} in step 'value' fields to inject \
         session secrets without exposing them. Store secrets first with secret_store, \
         then reference them: {\"action\": \"type\", \"selector\": \"#password\", \
         \"value\": \"${{secrets.my_password}}\"}. Secrets are resolved server-side \
         and never returned to the agent."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The initial URL to navigate to"
                },
                "steps": {
                    "type": "array",
                    "description": "Ordered list of interaction steps to perform",
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": {
                                "type": "string",
                                "description": "Action type: click, type, keyboard, mouse_move, touch, scroll, wait, wait_for_selector, navigate",
                                "enum": ["click", "type", "keyboard", "mouse_move", "touch", "scroll", "wait", "wait_for_selector", "navigate"]
                            },
                            "selector": {
                                "type": "string",
                                "description": "CSS selector for the target element (for click, type, touch, wait_for_selector)"
                            },
                            "value": {
                                "type": "string",
                                "description": "Text to type (for type action), URL (for navigate), or scroll amount (for scroll). \
                                    Supports ${{secrets.<name>}} references to inject session secrets without exposing them to the agent."
                            },
                            "key": {
                                "type": "string",
                                "description": "Key to press (for keyboard action, e.g. 'Enter', 'Tab', 'Escape')"
                            },
                            "x": {
                                "type": "number",
                                "description": "X coordinate (for click with coordinates, mouse_move)"
                            },
                            "y": {
                                "type": "number",
                                "description": "Y coordinate (for click with coordinates, mouse_move)"
                            },
                            "wait_ms": {
                                "type": "integer",
                                "description": "Milliseconds to wait after this step (or duration for wait/wait_for_selector actions)"
                            }
                        },
                        "required": ["action"]
                    }
                },
                "return_screenshot": {
                    "type": "boolean",
                    "description": "If true, return a base64 screenshot after all steps. (default: false)"
                },
                "return_content": {
                    "type": "boolean",
                    "description": "If true, return DOM content after all steps. When both return_screenshot and return_content are true, both are included in the response. If neither is true, returns DOM content by default. (default: false)"
                }
            },
            "required": ["url", "steps"],
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
        ToolExecutionResult::tool_error("browserless_interact requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let url = match required_str(&arguments, "url") {
            Ok(v) => v,
            Err(e) => return e,
        };

        // THREAT: Block secret references in URL param to prevent exfiltration
        if !extract_secret_refs(url).is_empty() {
            return ToolExecutionResult::tool_error(
                "Secret references (${{secrets.*}}) are not allowed in the 'url' parameter.",
            );
        }

        if let Err(e) = validate_navigation(context, url) {
            return e;
        }

        let steps = match arguments.get("steps").and_then(|v| v.as_array()) {
            Some(arr) => arr.clone(),
            None => {
                return ToolExecutionResult::tool_error(
                    "Missing required parameter: steps (must be an array)",
                );
            }
        };

        // THREAT: Block secret references in navigate step URLs to prevent
        // secret exfiltration via URL (e.g. navigating to attacker.com/${{secrets.pw}})
        for step in &steps {
            if step.get("action").and_then(|v| v.as_str()) == Some("navigate")
                && let Some(val) = step.get("value").and_then(|v| v.as_str())
                && !extract_secret_refs(val).is_empty()
            {
                return ToolExecutionResult::tool_error(
                    "Secret references (${{secrets.*}}) are not allowed in 'navigate' step values.",
                );
            }
        }

        // Resolve ${{secrets.*}} references in step value fields
        let resolved_secrets = match resolve_step_secrets(context, &steps).await {
            Ok(r) => r,
            Err(e) => return e,
        };
        let steps = substitute_step_secrets(&steps, &resolved_secrets);

        // Validate interaction steps (including navigate URL validation) after substitution
        if let Err(e) = validate_interaction_steps(context.network_access.as_ref(), &steps) {
            return e;
        }

        let return_screenshot = arguments
            .get("return_screenshot")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let return_content = arguments
            .get("return_content")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        // If neither flag is set, default to returning content
        let (want_screenshot, requested_content) = match (return_screenshot, return_content) {
            (false, false) => (false, true),
            other => other,
        };
        let suppress_content_for_secrets =
            should_suppress_interact_content(requested_content, resolved_secrets.len());
        let want_content = requested_content && !suppress_content_for_secrets;

        // Guarded CDP browser: the persistent one, else a one-shot (EVE-1189)
        let acquired = match acquire_browser(context).await {
            Ok(acquired) => acquired,
            Err(e) => return e,
        };
        let api_token = match acquired {
            Acquired::RestFallback(api_token) => api_token,
            Acquired::Browser(mut session) => {
                let mode = session.mode();
                debug!("Using CDP session for interact");
                if let Err(e) = begin_cdp_navigation(context, &mut session, url).await {
                    session.release(context).await;
                    return e;
                }

                // Execute each step via CDP
                for step in &steps {
                    let action = step
                        .get("action")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let selector = step.get("selector").and_then(|v| v.as_str());
                    let value = step.get("value").and_then(|v| v.as_str());
                    let key = step.get("key").and_then(|v| v.as_str());
                    let x = step.get("x").and_then(|v| v.as_f64());
                    let y = step.get("y").and_then(|v| v.as_f64());
                    let wait_ms = step.get("wait_ms").and_then(|v| v.as_u64());

                    let step_result = match action {
                        "click" => {
                            if let Some(sel) = selector {
                                session.click_selector(sel).await
                            } else if let (Some(cx), Some(cy)) = (x, y) {
                                session.click_at(cx, cy).await
                            } else {
                                Err("click requires selector or x,y coordinates".to_string())
                            }
                        }
                        "type" => {
                            if let (Some(sel), Some(text)) = (selector, value) {
                                session.type_into_selector(sel, text).await
                            } else {
                                Err("type requires selector and value".to_string())
                            }
                        }
                        "keyboard" => {
                            if let Some(k) = key {
                                session.press_key(k).await
                            } else {
                                Err("keyboard requires key".to_string())
                            }
                        }
                        "mouse_move" => {
                            if let (Some(mx), Some(my)) = (x, y) {
                                session.mouse_move(mx, my).await
                            } else {
                                Err("mouse_move requires x and y".to_string())
                            }
                        }
                        "touch" => {
                            if let Some(sel) = selector {
                                session.tap_selector(sel).await
                            } else {
                                Err("touch requires selector".to_string())
                            }
                        }
                        "scroll" => {
                            let dy = step.get("value").and_then(|v| v.as_i64()).unwrap_or(500);
                            session.scroll(dy).await
                        }
                        "wait" => {
                            let ms = wait_ms.unwrap_or(1000);
                            tokio::time::sleep(tokio::time::Duration::from_millis(cap_wait_ms(ms)))
                                .await;
                            Ok(())
                        }
                        "wait_for_selector" => {
                            if let Some(sel) = selector {
                                let timeout = wait_ms.unwrap_or(10000);
                                session.wait_for_selector(sel, timeout).await
                            } else {
                                Err("wait_for_selector requires selector".to_string())
                            }
                        }
                        "navigate" => {
                            if let Some(nav_url) = value {
                                begin_cdp_navigation(context, &mut session, nav_url)
                                    .await
                                    .map_err(|error| match error {
                                        ToolExecutionResult::ToolError(message) => message,
                                        other => format!("{other:?}"),
                                    })
                            } else {
                                Err("navigate requires value (URL)".to_string())
                            }
                        }
                        other => Err(format!("Unknown action: {other}")),
                    };

                    if let Err(e) = step_result {
                        session.release(context).await;
                        return ToolExecutionResult::tool_error(format!(
                            "Interaction step '{action}' failed: {e}"
                        ));
                    }

                    // Per-step wait
                    if action != "wait"
                        && let Some(ms) = wait_ms
                    {
                        tokio::time::sleep(tokio::time::Duration::from_millis(cap_wait_ms(ms)))
                            .await;
                    }
                }

                // A click or keypress can navigate. The Fetch gate stops the request;
                // this catches a landing URL the gate did not see.
                if let Err(e) = session
                    .reset_if_landing_blocked(context.network_access.as_ref())
                    .await
                {
                    session.release(context).await;
                    return ToolExecutionResult::tool_error(e);
                }

                // Capture result
                let title = session.get_title().await.unwrap_or_default();
                let final_url = session.get_url().await.unwrap_or_default();
                let mut result = json!({
                    "title": title,
                    "url": final_url,
                    "session": mode
                });
                let mut images = Vec::new();
                if suppress_content_for_secrets {
                    result["content_redacted"] = json!(true);
                    result["content_redaction_reason"] = json!("secrets_used_in_steps");
                }
                if want_screenshot {
                    match session.screenshot(true).await {
                        Ok(b64) => {
                            use base64::Engine;
                            let bytes = match base64::engine::general_purpose::STANDARD.decode(&b64)
                            {
                                Ok(bytes) => bytes,
                                Err(e) => {
                                    session.release(context).await;
                                    return ToolExecutionResult::tool_error(format!(
                                        "CDP screenshot returned invalid base64: {e}"
                                    ));
                                }
                            };
                            result["screenshot_returned"] = json!(true);
                            result["screenshot_format"] = json!("png");
                            result["screenshot_size_bytes"] = json!(bytes.len());
                            images.push(ToolResultImage {
                                base64: b64,
                                media_type: "image/png".to_string(),
                            });
                        }
                        Err(e) => {
                            session.release(context).await;
                            return ToolExecutionResult::tool_error(format!(
                                "CDP screenshot failed: {e}"
                            ));
                        }
                    }
                }
                if want_content {
                    match session.get_content().await {
                        Ok(content) => {
                            let total = content.len();
                            let (content, was_truncated) = truncate_html(content);
                            result["content"] = json!(content);
                            result["truncated"] = json!(was_truncated);
                            attach_content_truncation(&mut result, &content, total, was_truncated);
                        }
                        Err(e) => {
                            session.release(context).await;
                            return ToolExecutionResult::tool_error(format!(
                                "CDP content failed: {e}"
                            ));
                        }
                    }
                }

                // A one-shot browser carries the session's cookies forward the way
                // the REST `/function` path does.
                session.save_cookies_if_one_shot(context).await;
                session.release(context).await;
                return if images.is_empty() {
                    ToolExecutionResult::Success(result)
                } else {
                    ToolExecutionResult::success_with_images(result, images)
                };
            }
        };

        // Fallback: REST API (Browserless cloud refused CDP for this token)

        let stored_cookies = load_cookies(context).await;
        let patterns = reject_patterns(context);
        let code = build_interaction_code_with_policy(
            url,
            &steps,
            want_screenshot,
            want_content,
            &stored_cookies,
            &patterns,
        );
        debug!("Generated interaction code ({} bytes)", code.len());

        let client = BrowserlessClient::new(api_token);
        match client.function(&code, None).await {
            Ok(result) => {
                if let Some(data_str) = result.get("data").and_then(|v| v.as_str()) {
                    match serde_json::from_str::<Value>(data_str) {
                        Ok(mut data) => {
                            let mut images = Vec::new();
                            let mut content_truncation: Option<(String, usize, bool)> = None;
                            // Extract and persist cookies (even empty — clears stale state)
                            if let Some(cookies) = data.get("__cookies").and_then(|v| v.as_array())
                                && let Err(e) = save_cookies(context, cookies).await
                            {
                                debug!("Failed to persist cookies: {e}");
                            }
                            if let Some(obj) = data.as_object_mut() {
                                obj.remove("__cookies");
                                if let Some(content_value) = obj.get("content").cloned()
                                    && let Some(content) = content_value.as_str()
                                {
                                    let total = content.len();
                                    let (content, was_truncated) =
                                        truncate_html(content.to_string());
                                    obj.insert("content".to_string(), json!(content.clone()));
                                    obj.insert("truncated".to_string(), json!(was_truncated));
                                    content_truncation = Some((content, total, was_truncated));
                                }
                                if let Some(screenshot_value) = obj.remove("screenshot")
                                    && let Some(b64) = screenshot_value.as_str()
                                {
                                    use base64::Engine;
                                    let bytes = match base64::engine::general_purpose::STANDARD
                                        .decode(b64)
                                    {
                                        Ok(bytes) => bytes,
                                        Err(e) => {
                                            return ToolExecutionResult::tool_error(format!(
                                                "REST screenshot returned invalid base64: {e}"
                                            ));
                                        }
                                    };
                                    obj.insert("screenshot_returned".to_string(), json!(true));
                                    obj.insert("screenshot_format".to_string(), json!("png"));
                                    obj.insert(
                                        "screenshot_size_bytes".to_string(),
                                        json!(bytes.len()),
                                    );
                                    images.push(ToolResultImage {
                                        base64: b64.to_string(),
                                        media_type: "image/png".to_string(),
                                    });
                                }
                                if suppress_content_for_secrets {
                                    obj.insert("content_redacted".to_string(), json!(true));
                                    obj.insert(
                                        "content_redaction_reason".to_string(),
                                        json!("secrets_used_in_steps"),
                                    );
                                }
                            }
                            if let Some((content, total, was_truncated)) = content_truncation {
                                attach_content_truncation(
                                    &mut data,
                                    content.as_str(),
                                    total,
                                    was_truncated,
                                );
                            }
                            if images.is_empty() {
                                ToolExecutionResult::Success(data)
                            } else {
                                ToolExecutionResult::success_with_images(data, images)
                            }
                        }
                        Err(_) => ToolExecutionResult::Success(json!({
                            "result": data_str
                        })),
                    }
                } else {
                    ToolExecutionResult::Success(result)
                }
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// BrowserlessNavigateTool
// ============================================================================

pub struct BrowserlessNavigateTool;

#[async_trait]
impl Tool for BrowserlessNavigateTool {
    fn name(&self) -> &str {
        "browserless_navigate"
    }

    fn description(&self) -> &str {
        "Navigate to a URL and return page metadata (title, final URL after redirects, \
         and a summary of the page content). Use this as a first step to explore a website."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to navigate to"
                },
                "wait_for_selector": {
                    "type": "string",
                    "description": "Wait for this CSS selector to appear (optional)"
                },
                "wait_for_timeout": {
                    "type": "integer",
                    "description": "Wait this many milliseconds after page load (optional)"
                }
            },
            "required": ["url"],
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
        ToolExecutionResult::tool_error("browserless_navigate requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let url = match required_str(&arguments, "url") {
            Ok(v) => v,
            Err(e) => return e,
        };
        if let Err(e) = validate_navigation(context, url) {
            return e;
        }

        // Guarded CDP browser: the persistent one, else a one-shot (EVE-1189)
        let acquired = match acquire_browser(context).await {
            Ok(acquired) => acquired,
            Err(e) => return e,
        };
        let api_token = match acquired {
            Acquired::RestFallback(api_token) => api_token,
            Acquired::Browser(mut session) => {
                let mode = session.mode();
                debug!("Using CDP session for navigate");
                if let Err(e) = begin_cdp_navigation(context, &mut session, url).await {
                    session.release(context).await;
                    return e;
                }

                if let Some(sel) = arguments.get("wait_for_selector").and_then(|v| v.as_str()) {
                    let _ = session.wait_for_selector(sel, 30000).await;
                }
                if let Some(ms) = arguments.get("wait_for_timeout").and_then(|v| v.as_u64()) {
                    tokio::time::sleep(tokio::time::Duration::from_millis(cap_wait_ms(ms))).await;
                }

                let result = session.get_page_info().await;
                session.release(context).await;

                return match result {
                    Ok(mut info) => {
                        info["session"] = json!(mode);
                        ToolExecutionResult::Success(info)
                    }
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("CDP get_page_info failed: {e}"))
                    }
                };
            }
        };

        // Fallback: REST API (Browserless cloud refused CDP for this token)

        let wait_for_selector = arguments.get("wait_for_selector").and_then(|v| v.as_str());
        let wait_for_timeout = arguments.get("wait_for_timeout").and_then(|v| v.as_u64());

        let stored_cookies = load_cookies(context).await;
        let patterns = reject_patterns(context);
        let mut code = String::new();
        code.push_str("export default async ({ page }) => {\n");
        code.push_str(&request_interception_prelude(&patterns));
        code.push_str(&build_cookie_injection_code(&stored_cookies));
        code.push_str(&format!(
            "  const response = await page.goto({}, {{ waitUntil: 'networkidle2', timeout: 30000 }});\n",
            serde_json::to_string(url).unwrap_or_else(|_| format!("\"{}\"", url))
        ));

        if let Some(sel) = wait_for_selector {
            code.push_str(&format!(
                "  await page.waitForSelector({}, {{ timeout: 30000 }});\n",
                serde_json::to_string(sel).unwrap()
            ));
        }
        if let Some(timeout) = wait_for_timeout {
            let capped = cap_wait_ms(timeout);
            code.push_str(&format!(
                "  await new Promise(r => setTimeout(r, {capped}));\n"
            ));
        }

        code.push_str(
            "  const title = await page.title();\n\
             \x20 const finalUrl = page.url();\n\
             \x20 const status = response ? response.status() : null;\n\
             \x20 const links = await page.$$eval('a[href]', els => els.slice(0, 50).map(a => ({ text: a.textContent?.trim()?.substring(0, 100), href: a.href })));\n\
             \x20 const headings = await page.$$eval('h1, h2, h3', els => els.slice(0, 20).map(h => ({ tag: h.tagName, text: h.textContent?.trim()?.substring(0, 200) })));\n\
             \x20 const meta = await page.$$eval('meta[name], meta[property]', els => els.slice(0, 20).map(m => ({ name: m.getAttribute('name') || m.getAttribute('property'), content: m.getAttribute('content')?.substring(0, 200) })));\n\
             \x20 return { data: JSON.stringify({ title, url: finalUrl, status, links, headings, meta }), type: 'application/json' };\n",
        );
        code.push_str("};\n");

        let client = BrowserlessClient::new(api_token);
        match client.function(&code, None).await {
            Ok(result) => {
                if let Some(data_str) = result.get("data").and_then(|v| v.as_str()) {
                    match serde_json::from_str::<Value>(data_str) {
                        Ok(data) => ToolExecutionResult::Success(data),
                        Err(_) => ToolExecutionResult::Success(json!({
                            "result": data_str
                        })),
                    }
                } else {
                    ToolExecutionResult::Success(result)
                }
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
