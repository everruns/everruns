//! Minimal Chrome DevTools Protocol (CDP) client over WebSocket.
//!
//! Decision: Minimal CDP client — just JSON-RPC over WebSocket. No external CDP crate.
//! Decision: Short-lived connections. Connect, do work, call Browserless.reconnect, disconnect.
//!   The browser stays alive on Browserless servers between tool calls.
//! Decision: 60s default reconnect timeout. Covers LLM thinking time between tool calls.

use futures_util::stream::SplitSink;
use futures_util::stream::SplitStream;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing::debug;

/// Default reconnect timeout in milliseconds (60 seconds).
pub const DEFAULT_RECONNECT_TIMEOUT_MS: u64 = 60_000;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsSink = SplitSink<WsStream, Message>;
type WsSource = SplitStream<WsStream>;

// ============================================================================
// CdpSession — a single CDP connection to a Browserless browser
// ============================================================================

pub struct CdpSession {
    sink: WsSink,
    source: WsSource,
    next_id: u32,
}

impl CdpSession {
    /// Connect to a Browserless WebSocket endpoint.
    ///
    /// `ws_url` should be a full WebSocket URL, e.g.:
    ///   `wss://production-sfo.browserless.io?token=TOKEN`
    ///   or a reconnect endpoint returned by `Browserless.reconnect`.
    pub async fn connect(ws_url: &str) -> Result<Self, String> {
        // Redact token from log output to avoid leaking credentials
        let redacted = if let Some(idx) = ws_url.find("token=") {
            format!("{}token=<REDACTED>", &ws_url[..idx])
        } else {
            ws_url.to_string()
        };
        debug!("CDP connecting to {redacted}");
        let (ws_stream, _response) = connect_async(ws_url)
            .await
            .map_err(|e| format!("CDP WebSocket connection failed: {e}"))?;

        let (sink, source) = ws_stream.split();
        Ok(Self {
            sink,
            source,
            next_id: 1,
        })
    }

    /// Send a CDP command and wait for the response with the matching ID.
    /// Events (messages without an `id` field) are silently skipped.
    pub async fn send_command(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;

        let msg = json!({
            "id": id,
            "method": method,
            "params": params
        });

        debug!("CDP send: {method} (id={id})");
        self.sink
            .send(Message::Text(msg.to_string().into()))
            .await
            .map_err(|e| format!("CDP send failed: {e}"))?;

        // Read messages until we find the response with matching id
        loop {
            match self.source.next().await {
                Some(Ok(Message::Text(text))) => {
                    let parsed: Value = serde_json::from_str(&text)
                        .map_err(|e| format!("CDP invalid JSON response: {e}"))?;

                    // Check if this is our response (has matching id)
                    if parsed.get("id").and_then(|v| v.as_u64()) == Some(id as u64) {
                        // Check for CDP error
                        if let Some(error) = parsed.get("error") {
                            let msg = error
                                .get("message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Unknown CDP error");
                            return Err(format!("CDP error in {method}: {msg}"));
                        }
                        return Ok(parsed.get("result").cloned().unwrap_or(json!({})));
                    }

                    // Otherwise it's an event — skip it
                    if let Some(event_method) = parsed.get("method").and_then(|v| v.as_str()) {
                        debug!("CDP event (skipped): {event_method}");
                    }
                }
                Some(Ok(Message::Binary(_))) => {
                    // Binary frames — skip
                    continue;
                }
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {
                    continue;
                }
                Some(Ok(Message::Close(_))) => {
                    return Err("CDP WebSocket closed unexpectedly".to_string());
                }
                Some(Ok(Message::Frame(_))) => {
                    continue;
                }
                Some(Err(e)) => {
                    return Err(format!("CDP WebSocket error: {e}"));
                }
                None => {
                    return Err("CDP WebSocket stream ended".to_string());
                }
            }
        }
    }

    /// Disconnect the WebSocket gracefully.
    /// Important: Call `reconnect()` before this if you want the browser to stay alive.
    pub async fn disconnect(mut self) {
        let _ = self.sink.close().await;
    }

    // ========================================================================
    // High-level CDP helpers
    // ========================================================================

    /// Navigate to a URL and wait for the page to load.
    pub async fn navigate(&mut self, url: &str) -> Result<Value, String> {
        self.send_command("Page.enable", json!({})).await?;
        let result = self
            .send_command("Page.navigate", json!({ "url": url }))
            .await?;

        // Wait for page to finish loading
        let _ = self
            .send_command("Page.setLifecycleEventsEnabled", json!({ "enabled": true }))
            .await;
        // Give the page time to load. We use Runtime.evaluate with a load-wait.
        let _ = self
            .send_command(
                "Runtime.evaluate",
                json!({
                    "expression": "new Promise(r => { if (document.readyState === 'complete') r(); else window.addEventListener('load', r); })",
                    "awaitPromise": true,
                    "timeout": 30000
                }),
            )
            .await;

        Ok(result)
    }

    /// Take a screenshot. Returns base64-encoded PNG data.
    pub async fn screenshot(&mut self, full_page: bool) -> Result<String, String> {
        // If full_page, get the full page dimensions first
        let clip = if full_page {
            let metrics = self
                .send_command(
                    "Runtime.evaluate",
                    json!({
                        "expression": "JSON.stringify({ width: document.documentElement.scrollWidth, height: document.documentElement.scrollHeight })",
                        "returnByValue": true
                    }),
                )
                .await?;

            if let Some(value_str) = metrics
                .get("result")
                .and_then(|r| r.get("value"))
                .and_then(|v| v.as_str())
            {
                if let Ok(dims) = serde_json::from_str::<Value>(value_str) {
                    let w = dims.get("width").and_then(|v| v.as_f64()).unwrap_or(1280.0);
                    let h = dims.get("height").and_then(|v| v.as_f64()).unwrap_or(800.0);
                    Some(json!({ "x": 0, "y": 0, "width": w, "height": h, "scale": 1 }))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let mut params = json!({ "format": "png" });
        if let Some(clip) = clip {
            params["clip"] = clip;
            params["captureBeyondViewport"] = json!(true);
        }

        let result = self.send_command("Page.captureScreenshot", params).await?;
        result
            .get("data")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "CDP screenshot returned no data".to_string())
    }

    /// Evaluate a JS expression and return the string result.
    async fn eval_string(&mut self, expression: &str) -> Result<String, String> {
        let result = self
            .send_command(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true
                }),
            )
            .await?;

        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }

    /// Get the page's rendered HTML content (outerHTML of document element).
    pub async fn get_content(&mut self) -> Result<String, String> {
        let html = self
            .eval_string("document.documentElement.outerHTML")
            .await?;
        if html.is_empty() {
            return Err("CDP getContent returned no data".to_string());
        }
        Ok(html)
    }

    /// Get the page title.
    pub async fn get_title(&mut self) -> Result<String, String> {
        self.eval_string("document.title").await
    }

    /// Get the current page URL.
    pub async fn get_url(&mut self) -> Result<String, String> {
        self.eval_string("window.location.href").await
    }

    /// Evaluate a JavaScript expression and return the result.
    pub async fn evaluate(&mut self, expression: &str) -> Result<Value, String> {
        self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": true,
                "timeout": 30000
            }),
        )
        .await
    }

    /// Click at specific coordinates.
    pub async fn click_at(&mut self, x: f64, y: f64) -> Result<(), String> {
        self.send_command(
            "Input.dispatchMouseEvent",
            json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1 }),
        )
        .await?;
        self.send_command(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1 }),
        )
        .await?;
        Ok(())
    }

    /// Find the center coordinates of an element by CSS selector.
    async fn get_element_center(&mut self, selector: &str) -> Result<(f64, f64), String> {
        let selector_js = serde_json::to_string(selector).unwrap();
        let coord_str = self
            .eval_string(&format!(
                "(() => {{ const el = document.querySelector({selector_js}); if (!el) throw new Error('Element not found: ' + {selector_js}); const r = el.getBoundingClientRect(); return JSON.stringify({{ x: r.x + r.width/2, y: r.y + r.height/2 }}); }})()"
            ))
            .await?;

        if coord_str.is_empty() {
            return Err(format!("Element not found: {selector}"));
        }

        let coord_val: Value = serde_json::from_str(&coord_str)
            .map_err(|e| format!("Failed to parse coordinates: {e}"))?;

        let x = coord_val
            .get("x")
            .and_then(|v| v.as_f64())
            .ok_or("No x coordinate")?;
        let y = coord_val
            .get("y")
            .and_then(|v| v.as_f64())
            .ok_or("No y coordinate")?;

        Ok((x, y))
    }

    /// Click an element by CSS selector. Finds center coordinates via JS, then dispatches mouse events.
    pub async fn click_selector(&mut self, selector: &str) -> Result<(), String> {
        let (x, y) = self.get_element_center(selector).await?;
        self.click_at(x, y).await
    }

    /// Type text by dispatching key events for each character.
    pub async fn type_text(&mut self, text: &str) -> Result<(), String> {
        for ch in text.chars() {
            self.send_command(
                "Input.dispatchKeyEvent",
                json!({
                    "type": "keyDown",
                    "text": ch.to_string(),
                    "key": ch.to_string(),
                }),
            )
            .await?;
            self.send_command(
                "Input.dispatchKeyEvent",
                json!({
                    "type": "keyUp",
                    "key": ch.to_string(),
                }),
            )
            .await?;
        }
        Ok(())
    }

    /// Type text into a specific element (focus + type).
    pub async fn type_into_selector(&mut self, selector: &str, text: &str) -> Result<(), String> {
        // Focus the element
        let selector_js = serde_json::to_string(selector).unwrap();
        self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": format!(
                    "(() => {{ const el = document.querySelector({selector_js}); if (!el) throw new Error('Element not found: ' + {selector_js}); el.focus(); }})()"
                ),
                "returnByValue": true
            }),
        )
        .await?;

        self.type_text(text).await
    }

    /// Press a special key (Enter, Tab, Escape, etc.).
    pub async fn press_key(&mut self, key: &str) -> Result<(), String> {
        self.send_command(
            "Input.dispatchKeyEvent",
            json!({
                "type": "keyDown",
                "key": key,
                "code": key_to_code(key),
            }),
        )
        .await?;
        self.send_command(
            "Input.dispatchKeyEvent",
            json!({
                "type": "keyUp",
                "key": key,
                "code": key_to_code(key),
            }),
        )
        .await?;
        Ok(())
    }

    /// Move mouse to coordinates.
    pub async fn mouse_move(&mut self, x: f64, y: f64) -> Result<(), String> {
        self.send_command(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseMoved", "x": x, "y": y }),
        )
        .await?;
        Ok(())
    }

    /// Tap an element (touch simulation).
    pub async fn tap_selector(&mut self, selector: &str) -> Result<(), String> {
        let (x, y) = self.get_element_center(selector).await?;

        self.send_command(
            "Input.dispatchTouchEvent",
            json!({
                "type": "touchStart",
                "touchPoints": [{ "x": x, "y": y }]
            }),
        )
        .await?;
        self.send_command(
            "Input.dispatchTouchEvent",
            json!({
                "type": "touchEnd",
                "touchPoints": []
            }),
        )
        .await?;
        Ok(())
    }

    /// Scroll the page by a pixel amount.
    pub async fn scroll(&mut self, delta_y: i64) -> Result<(), String> {
        self.evaluate(&format!("window.scrollBy(0, {delta_y})"))
            .await?;
        Ok(())
    }

    /// Wait for a CSS selector to appear on the page.
    pub async fn wait_for_selector(
        &mut self,
        selector: &str,
        timeout_ms: u64,
    ) -> Result<(), String> {
        let selector_js = serde_json::to_string(selector).unwrap();
        let result = self
            .send_command(
                "Runtime.evaluate",
                json!({
                    "expression": format!(
                        "new Promise((resolve, reject) => {{ \
                            const el = document.querySelector({selector_js}); \
                            if (el) return resolve(true); \
                            const observer = new MutationObserver(() => {{ \
                                if (document.querySelector({selector_js})) {{ observer.disconnect(); resolve(true); }} \
                            }}); \
                            observer.observe(document.body, {{ childList: true, subtree: true }}); \
                            setTimeout(() => {{ observer.disconnect(); reject(new Error('Timeout waiting for ' + {selector_js})); }}, {timeout_ms}); \
                        }})"
                    ),
                    "awaitPromise": true,
                    "timeout": timeout_ms + 1000
                }),
            )
            .await?;

        // Check for exception
        if let Some(exception) = result.get("exceptionDetails") {
            let msg = exception
                .get("exception")
                .and_then(|e| e.get("description"))
                .and_then(|v| v.as_str())
                .unwrap_or("Timeout");
            return Err(format!("wait_for_selector failed: {msg}"));
        }

        Ok(())
    }

    /// Call Browserless.reconnect to keep the browser alive after disconnect.
    /// Returns the new WebSocket endpoint to use for reconnection.
    pub async fn reconnect(&mut self, timeout_ms: u64) -> Result<String, String> {
        let result = self
            .send_command("Browserless.reconnect", json!({ "timeout": timeout_ms }))
            .await?;

        if let Some(error) = result.get("error").and_then(|v| v.as_str()) {
            return Err(format!("Browserless.reconnect failed: {error}"));
        }

        result
            .get("browserWSEndpoint")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "Browserless.reconnect returned no endpoint".to_string())
    }

    /// Get page metadata (title, URL, headings, links).
    pub async fn get_page_info(&mut self) -> Result<Value, String> {
        let result = self
            .send_command(
                "Runtime.evaluate",
                json!({
                    "expression": r#"JSON.stringify({
                        title: document.title,
                        url: window.location.href,
                        headings: Array.from(document.querySelectorAll('h1, h2, h3')).slice(0, 20).map(h => ({ tag: h.tagName, text: h.textContent?.trim()?.substring(0, 200) })),
                        links: Array.from(document.querySelectorAll('a[href]')).slice(0, 50).map(a => ({ text: a.textContent?.trim()?.substring(0, 100), href: a.href })),
                        meta: Array.from(document.querySelectorAll('meta[name], meta[property]')).slice(0, 20).map(m => ({ name: m.getAttribute('name') || m.getAttribute('property'), content: m.getAttribute('content')?.substring(0, 200) }))
                    })"#,
                    "returnByValue": true
                }),
            )
            .await?;

        let json_str = result
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .ok_or("Failed to get page info")?;

        serde_json::from_str(json_str).map_err(|e| format!("Invalid page info JSON: {e}"))
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Map key names to CDP key codes. Most keys map to themselves; only " " needs special handling.
fn key_to_code(key: &str) -> &str {
    if key == " " { "Space" } else { key }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_to_code() {
        assert_eq!(key_to_code("Enter"), "Enter");
        assert_eq!(key_to_code("Tab"), "Tab");
        assert_eq!(key_to_code("Escape"), "Escape");
        assert_eq!(key_to_code("Space"), "Space");
        assert_eq!(key_to_code(" "), "Space");
        assert_eq!(key_to_code("ArrowUp"), "ArrowUp");
        assert_eq!(key_to_code("SomeOtherKey"), "SomeOtherKey");
    }
}
