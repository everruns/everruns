//! Tool implementations for Browserless browser automation.
//!
//! Decision: 5 REST tools + session-aware behavior:
//!   1. browserless_screenshot - Take screenshot (CDP if session active, REST otherwise)
//!   2. browserless_content   - Read DOM/HTML (CDP if session active, REST otherwise)
//!   3. browserless_scrape    - Extract structured data via CSS selectors (REST only)
//!   4. browserless_interact  - Click, type, navigate (CDP if session active, REST otherwise)
//!   5. browserless_navigate  - Open URL and get page info (CDP if session active, REST otherwise)
//!
//! When a CDP session is active (via browserless_open_browser), tools navigate within
//! the persistent browser, preserving login state and cookies across tool calls.
//! When no session exists, each tool call uses a fresh browser via REST API.

use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;

use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::debug;

use crate::client::BrowserlessClient;
use crate::session_tools::{keep_session_alive, try_get_cdp_session};
use crate::state::{get_api_token, required_str};
use crate::validation::{validate_browserless_url, validate_interaction_steps};

const MAX_HTML_BYTES: usize = 100_000;
const MAX_WAIT_MS: u64 = 120_000;

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

fn validate_url(url: &str) -> Result<(), ToolExecutionResult> {
    validate_browserless_url(url)
}

/// Cap a wait/timeout value to MAX_WAIT_MS. // THREAT[TM-TOOL-016]
fn cap_wait_ms(ms: u64) -> u64 {
    ms.min(MAX_WAIT_MS)
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
        if let Err(e) = validate_url(url) {
            return e;
        }

        let full_page = arguments
            .get("full_page")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // Try CDP session first
        if let Some(mut session) = try_get_cdp_session(context).await {
            debug!("Using CDP session for screenshot");
            if let Err(e) = session.navigate(url).await {
                keep_session_alive(context, &mut session).await;
                session.disconnect().await;
                return ToolExecutionResult::tool_error(format!("CDP navigate failed: {e}"));
            }

            // Wait for selector if specified
            if let Some(sel) = arguments.get("wait_for_selector").and_then(|v| v.as_str()) {
                let _ = session.wait_for_selector(sel, 30000).await;
            }
            if let Some(ms) = arguments.get("wait_for_timeout").and_then(|v| v.as_u64()) {
                tokio::time::sleep(tokio::time::Duration::from_millis(cap_wait_ms(ms))).await;
            }

            let result = session.screenshot(full_page).await;
            keep_session_alive(context, &mut session).await;
            session.disconnect().await;

            return match result {
                Ok(b64) => {
                    use base64::Engine;
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(&b64)
                        .unwrap_or_default();
                    ToolExecutionResult::Success(json!({
                        "url": url,
                        "format": "png",
                        "size_bytes": bytes.len(),
                        "image_base64": b64,
                        "session": "cdp"
                    }))
                }
                Err(e) => ToolExecutionResult::tool_error(format!("CDP screenshot failed: {e}")),
            };
        }

        // Fallback: REST API
        let api_token = match get_api_token(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };

        let selector = arguments.get("selector").and_then(|v| v.as_str());
        let wait_for_selector = arguments.get("wait_for_selector").and_then(|v| v.as_str());
        let wait_for_timeout = arguments.get("wait_for_timeout").and_then(|v| v.as_u64());

        let client = BrowserlessClient::new(api_token);
        match client
            .screenshot(
                url,
                full_page,
                selector,
                wait_for_selector,
                wait_for_timeout,
            )
            .await
        {
            Ok(bytes) => {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                ToolExecutionResult::Success(json!({
                    "url": url,
                    "format": "png",
                    "size_bytes": bytes.len(),
                    "image_base64": b64
                }))
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
        if let Err(e) = validate_url(url) {
            return e;
        }

        // Try CDP session first
        if let Some(mut session) = try_get_cdp_session(context).await {
            debug!("Using CDP session for content");
            if let Err(e) = session.navigate(url).await {
                keep_session_alive(context, &mut session).await;
                session.disconnect().await;
                return ToolExecutionResult::tool_error(format!("CDP navigate failed: {e}"));
            }

            if let Some(sel) = arguments.get("wait_for_selector").and_then(|v| v.as_str()) {
                let _ = session.wait_for_selector(sel, 30000).await;
            }
            if let Some(ms) = arguments.get("wait_for_timeout").and_then(|v| v.as_u64()) {
                tokio::time::sleep(tokio::time::Duration::from_millis(cap_wait_ms(ms))).await;
            }

            let result = session.get_content().await;
            keep_session_alive(context, &mut session).await;
            session.disconnect().await;

            return match result {
                Ok(html) => {
                    let len = html.len();
                    let (content, was_truncated) = truncate_html(html);
                    ToolExecutionResult::Success(json!({
                        "url": url,
                        "content": content,
                        "size_bytes": len,
                        "truncated": was_truncated,
                        "session": "cdp"
                    }))
                }
                Err(e) => ToolExecutionResult::tool_error(format!("CDP content failed: {e}")),
            };
        }

        // Fallback: REST API
        let api_token = match get_api_token(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };

        let wait_for_selector = arguments.get("wait_for_selector").and_then(|v| v.as_str());
        let wait_for_timeout = arguments.get("wait_for_timeout").and_then(|v| v.as_u64());
        let best_attempt = arguments
            .get("best_attempt")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let client = BrowserlessClient::new(api_token);
        match client
            .content(url, wait_for_selector, wait_for_timeout, best_attempt)
            .await
        {
            Ok(html) => {
                let len = html.len();
                let (content, was_truncated) = truncate_html(html);
                ToolExecutionResult::Success(json!({
                    "url": url,
                    "content": content,
                    "size_bytes": len,
                    "truncated": was_truncated
                }))
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
        if let Err(e) = validate_url(url) {
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

        let api_token = match get_api_token(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };

        let wait_for_selector = arguments.get("wait_for_selector").and_then(|v| v.as_str());
        let wait_for_timeout = arguments.get("wait_for_timeout").and_then(|v| v.as_u64());

        let client = BrowserlessClient::new(api_token);
        match client
            .scrape(url, &elements, wait_for_selector, wait_for_timeout)
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

/// Build Puppeteer function code from a list of interaction steps.
/// Each step is executed sequentially in a fresh browser session.
fn build_interaction_code(url: &str, steps: &[Value], return_screenshot: bool) -> String {
    let mut code = String::new();
    code.push_str("export default async ({ page }) => {\n");
    code.push_str(&format!(
        "  await page.goto({}, {{ waitUntil: 'networkidle2', timeout: 30000 }});\n",
        serde_json::to_string(url).unwrap_or_else(|_| format!("\"{}\"", url))
    ));

    for step in steps {
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

        match action {
            "click" => {
                if let Some(sel) = selector {
                    code.push_str(&format!(
                        "  await page.waitForSelector({sel_js}, {{ timeout: 10000 }});\n\
                         \x20 await page.click({sel_js});\n",
                        sel_js = serde_json::to_string(sel).unwrap()
                    ));
                } else if let (Some(cx), Some(cy)) = (x, y) {
                    code.push_str(&format!("  await page.mouse.click({cx}, {cy});\n"));
                }
            }
            "type" => {
                if let (Some(sel), Some(text)) = (selector, value) {
                    code.push_str(&format!(
                        "  await page.waitForSelector({sel_js}, {{ timeout: 10000 }});\n\
                         \x20 await page.type({sel_js}, {val_js});\n",
                        sel_js = serde_json::to_string(sel).unwrap(),
                        val_js = serde_json::to_string(text).unwrap()
                    ));
                }
            }
            "keyboard" => {
                if let Some(k) = key {
                    code.push_str(&format!(
                        "  await page.keyboard.press({key_js});\n",
                        key_js = serde_json::to_string(k).unwrap()
                    ));
                }
            }
            "mouse_move" => {
                if let (Some(mx), Some(my)) = (x, y) {
                    code.push_str(&format!("  await page.mouse.move({mx}, {my});\n"));
                }
            }
            "touch" => {
                if let Some(sel) = selector {
                    code.push_str(&format!(
                        "  await page.waitForSelector({sel_js}, {{ timeout: 10000 }});\n\
                         \x20 await page.tap({sel_js});\n",
                        sel_js = serde_json::to_string(sel).unwrap()
                    ));
                }
            }
            "scroll" => {
                let scroll_y = step.get("value").and_then(|v| v.as_i64()).unwrap_or(500);
                code.push_str(&format!(
                    "  await page.evaluate(() => window.scrollBy(0, {scroll_y}));\n"
                ));
            }
            "wait" => {
                let ms = cap_wait_ms(wait_ms.unwrap_or(1000));
                code.push_str(&format!("  await new Promise(r => setTimeout(r, {ms}));\n"));
            }
            "wait_for_selector" => {
                if let Some(sel) = selector {
                    let timeout = cap_wait_ms(wait_ms.unwrap_or(10000));
                    code.push_str(&format!(
                        "  await page.waitForSelector({sel_js}, {{ timeout: {timeout} }});\n",
                        sel_js = serde_json::to_string(sel).unwrap()
                    ));
                }
            }
            "navigate" => {
                if let Some(nav_url) = value {
                    code.push_str(&format!(
                        "  await page.goto({url_js}, {{ waitUntil: 'networkidle2', timeout: 30000 }});\n",
                        url_js = serde_json::to_string(nav_url).unwrap()
                    ));
                }
            }
            other => {
                debug!("Unknown interaction action: {other}");
            }
        }

        // Optional per-step wait
        if action != "wait"
            && let Some(ms) = wait_ms
        {
            let capped = cap_wait_ms(ms);
            code.push_str(&format!(
                "  await new Promise(r => setTimeout(r, {capped}));\n"
            ));
        }
    }

    // Capture final state
    code.push_str("  const title = await page.title();\n");
    code.push_str("  const url = page.url();\n");

    if return_screenshot {
        code.push_str(
            "  const screenshot = await page.screenshot({ encoding: 'base64', fullPage: true });\n",
        );
        code.push_str(
            "  return { data: JSON.stringify({ title, url, screenshot }), type: 'application/json' };\n",
        );
    } else {
        code.push_str("  const content = await page.content();\n");
        code.push_str(
            "  return { data: JSON.stringify({ title, url, content }), type: 'application/json' };\n",
        );
    }

    code.push_str("};\n");
    code
}

#[async_trait]
impl Tool for BrowserlessInteractTool {
    fn name(&self) -> &str {
        "browserless_interact"
    }

    fn description(&self) -> &str {
        "Interact with a web page: navigate to a URL, then perform a sequence of actions \
         (click, type, keyboard, mouse, touch, scroll, wait). Returns the page title, \
         final URL, and optionally a screenshot or DOM content after all steps complete."
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
                                "description": "Text to type (for type action), URL (for navigate), or scroll amount (for scroll)"
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
                    "description": "If true, return a base64 screenshot after all steps. If false, return DOM content. (default: false)"
                }
            },
            "required": ["url", "steps"],
            "additionalProperties": false
        })
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
        if let Err(e) = validate_url(url) {
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
        if let Err(e) = validate_interaction_steps(&steps) {
            return e;
        }

        let return_screenshot = arguments
            .get("return_screenshot")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Try CDP session first
        if let Some(mut session) = try_get_cdp_session(context).await {
            debug!("Using CDP session for interact");
            if let Err(e) = session.navigate(url).await {
                keep_session_alive(context, &mut session).await;
                session.disconnect().await;
                return ToolExecutionResult::tool_error(format!("CDP navigate failed: {e}"));
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
                            session.navigate(nav_url).await.map(|_| ())
                        } else {
                            Err("navigate requires value (URL)".to_string())
                        }
                    }
                    other => Err(format!("Unknown action: {other}")),
                };

                if let Err(e) = step_result {
                    keep_session_alive(context, &mut session).await;
                    session.disconnect().await;
                    return ToolExecutionResult::tool_error(format!(
                        "Interaction step '{action}' failed: {e}"
                    ));
                }

                // Per-step wait
                if action != "wait"
                    && let Some(ms) = wait_ms
                {
                    tokio::time::sleep(tokio::time::Duration::from_millis(cap_wait_ms(ms))).await;
                }
            }

            // Capture result
            let title = session.get_title().await.unwrap_or_default();
            let final_url = session.get_url().await.unwrap_or_default();
            let result = if return_screenshot {
                match session.screenshot(true).await {
                    Ok(b64) => json!({
                        "title": title,
                        "url": final_url,
                        "screenshot": b64,
                        "session": "cdp"
                    }),
                    Err(e) => {
                        keep_session_alive(context, &mut session).await;
                        session.disconnect().await;
                        return ToolExecutionResult::tool_error(format!(
                            "CDP screenshot failed: {e}"
                        ));
                    }
                }
            } else {
                match session.get_content().await {
                    Ok(content) => json!({
                        "title": title,
                        "url": final_url,
                        "content": content,
                        "session": "cdp"
                    }),
                    Err(e) => {
                        keep_session_alive(context, &mut session).await;
                        session.disconnect().await;
                        return ToolExecutionResult::tool_error(format!("CDP content failed: {e}"));
                    }
                }
            };

            keep_session_alive(context, &mut session).await;
            session.disconnect().await;
            return ToolExecutionResult::Success(result);
        }

        // Fallback: REST API
        let api_token = match get_api_token(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };

        let code = build_interaction_code(url, &steps, return_screenshot);
        debug!("Generated interaction code ({} bytes)", code.len());

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
        if let Err(e) = validate_url(url) {
            return e;
        }

        // Try CDP session first
        if let Some(mut session) = try_get_cdp_session(context).await {
            debug!("Using CDP session for navigate");
            if let Err(e) = session.navigate(url).await {
                keep_session_alive(context, &mut session).await;
                session.disconnect().await;
                return ToolExecutionResult::tool_error(format!("CDP navigate failed: {e}"));
            }

            if let Some(sel) = arguments.get("wait_for_selector").and_then(|v| v.as_str()) {
                let _ = session.wait_for_selector(sel, 30000).await;
            }
            if let Some(ms) = arguments.get("wait_for_timeout").and_then(|v| v.as_u64()) {
                tokio::time::sleep(tokio::time::Duration::from_millis(cap_wait_ms(ms))).await;
            }

            let result = session.get_page_info().await;
            keep_session_alive(context, &mut session).await;
            session.disconnect().await;

            return match result {
                Ok(mut info) => {
                    info["session"] = json!("cdp");
                    ToolExecutionResult::Success(info)
                }
                Err(e) => ToolExecutionResult::tool_error(format!("CDP get_page_info failed: {e}")),
            };
        }

        // Fallback: REST API
        let api_token = match get_api_token(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };

        let wait_for_selector = arguments.get("wait_for_selector").and_then(|v| v.as_str());
        let wait_for_timeout = arguments.get("wait_for_timeout").and_then(|v| v.as_u64());

        let mut code = String::new();
        code.push_str("export default async ({ page }) => {\n");
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
mod tests {
    use super::*;
    use crate::validation::validate_interaction_steps;
    use everruns_core::capabilities::Capability;

    #[test]
    fn test_validate_url_accepts_public_https_url() {
        assert!(validate_url("https://example.com").is_ok());
    }

    #[test]
    fn test_validate_url_rejects_localhost() {
        let err = validate_url("http://localhost:3000").unwrap_err();
        match err {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("blocked"));
            }
            other => panic!("Expected ToolError, got: {other:?}"),
        }
    }

    #[test]
    fn test_validate_url_rejects_private_ip() {
        let err = validate_url("http://10.0.0.5/admin").unwrap_err();
        match err {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("blocked"));
            }
            other => panic!("Expected ToolError, got: {other:?}"),
        }
    }

    #[test]
    fn test_validate_url_rejects_cloud_metadata_ip() {
        let err = validate_url("http://169.254.169.254/latest/meta-data/").unwrap_err();
        match err {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("blocked"));
            }
            other => panic!("Expected ToolError, got: {other:?}"),
        }
    }

    #[test]
    fn test_validate_interaction_steps_rejects_blocked_navigate_url() {
        let steps = vec![json!({
            "action": "navigate",
            "value": "http://127.0.0.1:8080"
        })];
        let err = validate_interaction_steps(&steps).unwrap_err();
        match err {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("blocked"));
            }
            other => panic!("Expected ToolError, got: {other:?}"),
        }
    }

    #[test]
    fn test_screenshot_tool_metadata() {
        let tool = BrowserlessScreenshotTool;
        assert_eq!(tool.name(), "browserless_screenshot");
        assert!(tool.requires_context());
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("url")));
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn test_content_tool_metadata() {
        let tool = BrowserlessContentTool;
        assert_eq!(tool.name(), "browserless_content");
        assert!(tool.requires_context());
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("url")));
    }

    #[test]
    fn test_scrape_tool_metadata() {
        let tool = BrowserlessScrapeTool;
        assert_eq!(tool.name(), "browserless_scrape");
        assert!(tool.requires_context());
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("url")));
        assert!(required.contains(&json!("elements")));
    }

    #[test]
    fn test_interact_tool_metadata() {
        let tool = BrowserlessInteractTool;
        assert_eq!(tool.name(), "browserless_interact");
        assert!(tool.requires_context());
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("url")));
        assert!(required.contains(&json!("steps")));
    }

    #[test]
    fn test_navigate_tool_metadata() {
        let tool = BrowserlessNavigateTool;
        assert_eq!(tool.name(), "browserless_navigate");
        assert!(tool.requires_context());
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("url")));
    }

    #[test]
    fn test_all_schemas_have_no_additional_properties() {
        let cap = crate::BrowserlessCapability;
        for tool in cap.tools() {
            let schema = tool.parameters_schema();
            assert_eq!(
                schema["additionalProperties"],
                false,
                "Tool {} should disallow additional properties",
                tool.name()
            );
        }
    }

    #[test]
    fn test_build_interaction_code_click() {
        let steps = vec![json!({
            "action": "click",
            "selector": "#submit-btn"
        })];
        let code = build_interaction_code("https://example.com", &steps, false);
        assert!(code.contains("page.goto"));
        assert!(code.contains("page.click"));
        assert!(code.contains("#submit-btn"));
        assert!(code.contains("page.content()"));
    }

    #[test]
    fn test_build_interaction_code_type() {
        let steps = vec![json!({
            "action": "type",
            "selector": "#username",
            "value": "admin"
        })];
        let code = build_interaction_code("https://example.com", &steps, false);
        assert!(code.contains("page.type"));
        assert!(code.contains("#username"));
        assert!(code.contains("admin"));
    }

    #[test]
    fn test_build_interaction_code_keyboard() {
        let steps = vec![json!({
            "action": "keyboard",
            "key": "Enter"
        })];
        let code = build_interaction_code("https://example.com", &steps, false);
        assert!(code.contains("page.keyboard.press"));
        assert!(code.contains("Enter"));
    }

    #[test]
    fn test_build_interaction_code_mouse_move() {
        let steps = vec![json!({
            "action": "mouse_move",
            "x": 100.0,
            "y": 200.0
        })];
        let code = build_interaction_code("https://example.com", &steps, false);
        assert!(code.contains("page.mouse.move(100, 200)"));
    }

    #[test]
    fn test_build_interaction_code_touch() {
        let steps = vec![json!({
            "action": "touch",
            "selector": ".menu-item"
        })];
        let code = build_interaction_code("https://example.com", &steps, false);
        assert!(code.contains("page.tap"));
    }

    #[test]
    fn test_build_interaction_code_scroll() {
        let steps = vec![json!({
            "action": "scroll",
            "value": 1000
        })];
        let code = build_interaction_code("https://example.com", &steps, false);
        assert!(code.contains("window.scrollBy(0, 1000)"));
    }

    #[test]
    fn test_build_interaction_code_with_screenshot() {
        let steps = vec![json!({
            "action": "click",
            "selector": "button"
        })];
        let code = build_interaction_code("https://example.com", &steps, true);
        assert!(code.contains("page.screenshot"));
        assert!(!code.contains("page.content()"));
    }

    #[test]
    fn test_build_interaction_code_navigate_step() {
        let steps = vec![json!({
            "action": "navigate",
            "value": "https://other.com/page"
        })];
        let code = build_interaction_code("https://example.com", &steps, false);
        // Should have two page.goto calls: initial + navigate step
        assert!(code.contains("https://other.com/page"));
    }

    #[test]
    fn test_build_interaction_code_wait() {
        let steps = vec![json!({
            "action": "wait",
            "wait_ms": 2000
        })];
        let code = build_interaction_code("https://example.com", &steps, false);
        assert!(code.contains("setTimeout(r, 2000)"));
    }

    #[test]
    fn test_build_interaction_code_wait_for_selector() {
        let steps = vec![json!({
            "action": "wait_for_selector",
            "selector": ".loaded",
            "wait_ms": 5000
        })];
        let code = build_interaction_code("https://example.com", &steps, false);
        assert!(code.contains("waitForSelector"));
        assert!(code.contains(".loaded"));
        assert!(code.contains("5000"));
    }

    #[test]
    fn test_build_interaction_code_click_by_coordinates() {
        let steps = vec![json!({
            "action": "click",
            "x": 50.0,
            "y": 75.0
        })];
        let code = build_interaction_code("https://example.com", &steps, false);
        assert!(code.contains("page.mouse.click(50, 75)"));
    }

    #[test]
    fn test_build_interaction_code_multi_step() {
        let steps = vec![
            json!({"action": "click", "selector": "#login-link"}),
            json!({"action": "wait_for_selector", "selector": "#username"}),
            json!({"action": "type", "selector": "#username", "value": "admin"}),
            json!({"action": "type", "selector": "#password", "value": "secret"}),
            json!({"action": "click", "selector": "#submit"}),
            json!({"action": "wait", "wait_ms": 2000}),
        ];
        let code = build_interaction_code("https://example.com/home", &steps, true);
        assert!(code.contains("#login-link"));
        assert!(code.contains("#username"));
        assert!(code.contains("#password"));
        assert!(code.contains("#submit"));
        assert!(code.contains("page.screenshot"));
    }

    #[tokio::test]
    async fn test_screenshot_tool_no_context_error() {
        let tool = BrowserlessScreenshotTool;
        let result = tool.execute(json!({"url": "https://example.com"})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            other => panic!("Expected ToolError, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_content_tool_no_context_error() {
        let tool = BrowserlessContentTool;
        let result = tool.execute(json!({"url": "https://example.com"})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            other => panic!("Expected ToolError, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_scrape_tool_no_context_error() {
        let tool = BrowserlessScrapeTool;
        let result = tool
            .execute(json!({"url": "https://example.com", "elements": []}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            other => panic!("Expected ToolError, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_interact_tool_no_context_error() {
        let tool = BrowserlessInteractTool;
        let result = tool
            .execute(json!({"url": "https://example.com", "steps": []}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            other => panic!("Expected ToolError, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_navigate_tool_no_context_error() {
        let tool = BrowserlessNavigateTool;
        let result = tool.execute(json!({"url": "https://example.com"})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            other => panic!("Expected ToolError, got: {other:?}"),
        }
    }
}
