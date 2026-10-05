//! Computer use on a Browserless page (EVE-1119).
//!
//! The provider-neutral `computer` tool (`everruns_contracts::runtime::computer_use`) drives
//! a display through [`ComputerBackend`]. This module is the browser-only
//! backend: the display is one Chromium page, reached over CDP.
//!
//! Decision: the CDP action layer ([`CdpDisplay`]) knows nothing about
//! Browserless. It takes any attached [`CdpSession`], so the same code drives a
//! local Chromium in tests and any future CDP-speaking desktop.
//!
//! Decision: the page renders at the configured display size with a device
//! scale factor of 1 (`Emulation.setDeviceMetricsOverride`, reapplied on every
//! acquire), so screenshot pixels and input coordinates are the same space.
//!
//! Decision: text is entered with `Input.insertText`, which handles any
//! Unicode and fires `input` events; keys go through `Input.dispatchKeyEvent`
//! with the key, code, and virtual key code Chromium needs to run default
//! actions (Enter submits, Tab moves focus).
//!
//! Decision: the browser session is the same persistent session the
//! `browserless_*` tools use (one per agent session). The cursor position is
//! kept in session storage because each tool call reconnects.
//!
//! THREAT[TM-TOOL-015]: `navigate` goes through the shared SSRF validator and
//! the session's network access list. Pages can still navigate on their own
//! (a clicked link, a redirect), so after every action the main frame URL is
//! checked against the same policy and the page is sent to `about:blank` if it
//! left it.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use everruns_contracts::runtime::computer_use::{
    COMPUTER_TOOL_NAME, COMPUTER_USE_CAPABILITY_ID, COMPUTER_USE_SYSTEM_PROMPT, ComputerAction,
    ComputerBackend, ComputerSession, ComputerTool, ComputerUseConfig, DisplaySize, Modifier,
    MouseButton, Screenshot, ScrollDirection, parse_key_combo, parse_modifiers,
};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tool_hooks::PreToolUseHook;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use tracing::warn;

use crate::browser_egress::BrowserEgress;
use crate::cdp::{CdpSession, DEFAULT_RECONNECT_TIMEOUT_MS};
use crate::session_tools::{register_session_endpoint, try_get_cdp_session};
use crate::state::get_api_token;
use crate::validation::{is_local_browser_url, validate_browserless_navigation};

/// Session storage key for the last cursor position (`"x,y"`).
const CURSOR_KEY: &str = "computer_use.cursor";

/// Pixels per scroll wheel click.
const SCROLL_STEP_PX: i64 = 100;

// ============================================================================
// CdpDisplay — computer actions over any attached CDP page
// ============================================================================

/// A browser page driven as a computer display.
pub struct CdpDisplay {
    session: CdpSession,
    display: DisplaySize,
    cursor: [u32; 2],
}

impl CdpDisplay {
    /// Wrap an attached page and size its viewport to `display`.
    pub async fn attach(
        mut session: CdpSession,
        display: DisplaySize,
        cursor: [u32; 2],
    ) -> Result<Self, (CdpSession, String)> {
        let sized = session
            .send_command(
                "Emulation.setDeviceMetricsOverride",
                json!({
                    "width": display.width,
                    "height": display.height,
                    "deviceScaleFactor": 1,
                    "mobile": false
                }),
            )
            .await;
        if let Err(e) = sized {
            return Err((session, format!("failed to size the display: {e}")));
        }
        let cursor = if display.contains(cursor) {
            cursor
        } else {
            [0, 0]
        };
        Ok(Self {
            session,
            display,
            cursor,
        })
    }

    /// The last known cursor position.
    pub fn cursor(&self) -> [u32; 2] {
        self.cursor
    }

    /// The underlying CDP session.
    pub fn session_mut(&mut self) -> &mut CdpSession {
        &mut self.session
    }

    /// Give the CDP session back.
    pub fn into_session(self) -> CdpSession {
        self.session
    }

    /// Current main-frame URL.
    pub async fn current_url(&mut self) -> Result<String, String> {
        self.session.get_url().await
    }

    /// Perform one action. `navigate` loads the URL as given; callers enforce
    /// URL policy before calling.
    pub async fn perform(&mut self, action: &ComputerAction) -> Result<(), String> {
        if let Some(click) = action.as_click() {
            let point = click.coordinate.unwrap_or(self.cursor);
            let mask = modifier_mask(&parse_modifiers(click.modifiers.unwrap_or(""))?);
            return self.click(point, click.button, click.count, mask).await;
        }
        match action {
            ComputerAction::Screenshot => Ok(()),
            ComputerAction::LeftClickDrag {
                start_coordinate,
                coordinate,
            } => self.drag(*start_coordinate, *coordinate).await,
            ComputerAction::MouseMove { coordinate } => self.move_to(*coordinate, 0).await,
            ComputerAction::Scroll {
                coordinate,
                scroll_direction,
                scroll_amount,
            } => {
                let point = coordinate.unwrap_or(self.cursor);
                self.scroll(point, *scroll_direction, *scroll_amount).await
            }
            ComputerAction::Type { text } => self
                .session
                .send_command("Input.insertText", json!({ "text": text }))
                .await
                .map(|_| ()),
            ComputerAction::Key { text, repeat } => {
                for _ in 0..repeat.unwrap_or(1) {
                    self.press(text).await?;
                }
                Ok(())
            }
            ComputerAction::Wait { duration } => {
                tokio::time::sleep(Duration::from_secs_f64(*duration)).await;
                Ok(())
            }
            ComputerAction::Navigate { url } => self.session.navigate(url).await.map(|_| ()),
            // Clicks returned above.
            _ => Ok(()),
        }
    }

    /// Capture the viewport as PNG.
    pub async fn screenshot(&mut self) -> Result<Screenshot, String> {
        let base64 = self.session.screenshot(false).await?;
        Ok(Screenshot {
            base64,
            media_type: "image/png".to_string(),
        })
    }

    async fn mouse(&mut self, params: Value) -> Result<(), String> {
        self.session
            .send_command("Input.dispatchMouseEvent", params)
            .await
            .map(|_| ())
    }

    async fn move_to(&mut self, [x, y]: [u32; 2], buttons: u8) -> Result<(), String> {
        self.mouse(json!({ "type": "mouseMoved", "x": x, "y": y, "buttons": buttons }))
            .await?;
        self.cursor = [x, y];
        Ok(())
    }

    async fn click(
        &mut self,
        point: [u32; 2],
        button: MouseButton,
        count: u8,
        modifiers: u8,
    ) -> Result<(), String> {
        self.move_to(point, 0).await?;
        let [x, y] = point;
        // Chromium recognises a double click as a second press carrying
        // clickCount 2, so send each press with its running count.
        for n in 1..=count {
            for kind in ["mousePressed", "mouseReleased"] {
                self.mouse(json!({
                    "type": kind,
                    "x": x,
                    "y": y,
                    "button": button.as_str(),
                    "clickCount": n,
                    "modifiers": modifiers
                }))
                .await?;
            }
        }
        Ok(())
    }

    async fn drag(&mut self, from: [u32; 2], to: [u32; 2]) -> Result<(), String> {
        self.move_to(from, 0).await?;
        self.mouse(json!({
            "type": "mousePressed", "x": from[0], "y": from[1],
            "button": "left", "buttons": 1, "clickCount": 1
        }))
        .await?;
        // A midpoint gives drag handlers a move event between press and release.
        let mid = [(from[0] + to[0]) / 2, (from[1] + to[1]) / 2];
        self.move_to(mid, 1).await?;
        self.move_to(to, 1).await?;
        self.mouse(json!({
            "type": "mouseReleased", "x": to[0], "y": to[1],
            "button": "left", "buttons": 0, "clickCount": 1
        }))
        .await
    }

    async fn scroll(
        &mut self,
        point: [u32; 2],
        direction: ScrollDirection,
        amount: u32,
    ) -> Result<(), String> {
        self.move_to(point, 0).await?;
        let distance = SCROLL_STEP_PX * i64::from(amount);
        let (dx, dy) = match direction {
            ScrollDirection::Up => (0, -distance),
            ScrollDirection::Down => (0, distance),
            ScrollDirection::Left => (-distance, 0),
            ScrollDirection::Right => (distance, 0),
        };
        self.mouse(json!({
            "type": "mouseWheel", "x": point[0], "y": point[1], "deltaX": dx, "deltaY": dy
        }))
        .await
    }

    async fn press(&mut self, combo: &str) -> Result<(), String> {
        let combo = parse_key_combo(combo)?;
        let key = cdp_key(&combo.key);
        let mask = modifier_mask(&combo.modifiers);
        // Held modifiers go down first and up last, like a person's hand.
        for modifier in &combo.modifiers {
            self.key_event("keyDown", &modifier_key(*modifier), mask, None)
                .await?;
        }
        // Printable text is only inserted when no command modifier is held;
        // ctrl+a must select, not type "a".
        let typing = combo
            .modifiers
            .iter()
            .all(|modifier| *modifier == Modifier::Shift);
        let text = key.text.as_deref().filter(|_| typing);
        self.key_event("keyDown", &key, mask, text).await?;
        self.key_event("keyUp", &key, mask, None).await?;
        for modifier in combo.modifiers.iter().rev() {
            self.key_event("keyUp", &modifier_key(*modifier), 0, None)
                .await?;
        }
        Ok(())
    }

    async fn key_event(
        &mut self,
        kind: &str,
        key: &CdpKey,
        modifiers: u8,
        text: Option<&str>,
    ) -> Result<(), String> {
        let mut params = json!({
            "type": kind,
            "key": key.key,
            "code": key.code,
            "windowsVirtualKeyCode": key.virtual_key,
            "nativeVirtualKeyCode": key.virtual_key,
            "modifiers": modifiers
        });
        if let Some(text) = text {
            params["text"] = json!(text);
            params["unmodifiedText"] = json!(text);
        }
        self.session
            .send_command("Input.dispatchKeyEvent", params)
            .await
            .map(|_| ())
    }
}

/// CDP modifier bitmask: Alt=1, Ctrl=2, Meta=4, Shift=8.
fn modifier_mask(modifiers: &[Modifier]) -> u8 {
    modifiers.iter().fold(0, |mask, modifier| {
        mask | match modifier {
            Modifier::Alt => 1,
            Modifier::Ctrl => 2,
            Modifier::Super => 4,
            Modifier::Shift => 8,
        }
    })
}

/// A key as `Input.dispatchKeyEvent` wants it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CdpKey {
    key: String,
    code: String,
    virtual_key: u32,
    /// Text the key inserts, if printable.
    text: Option<String>,
}

fn named(key: &str, code: &str, virtual_key: u32, text: Option<&str>) -> CdpKey {
    CdpKey {
        key: key.to_string(),
        code: code.to_string(),
        virtual_key,
        text: text.map(str::to_string),
    }
}

fn modifier_key(modifier: Modifier) -> CdpKey {
    match modifier {
        Modifier::Shift => named("Shift", "ShiftLeft", 16, None),
        Modifier::Ctrl => named("Control", "ControlLeft", 17, None),
        Modifier::Alt => named("Alt", "AltLeft", 18, None),
        Modifier::Super => named("Meta", "MetaLeft", 91, None),
    }
}

/// Map an xdotool-style key name (`Return`, `Page_Down`, `a`) to CDP.
fn cdp_key(name: &str) -> CdpKey {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "return" | "enter" | "kp_enter" => return named("Enter", "Enter", 13, Some("\r")),
        "tab" => return named("Tab", "Tab", 9, None),
        "escape" | "esc" => return named("Escape", "Escape", 27, None),
        "backspace" => return named("Backspace", "Backspace", 8, None),
        "delete" | "del" => return named("Delete", "Delete", 46, None),
        "space" => return named(" ", "Space", 32, Some(" ")),
        "up" | "arrowup" => return named("ArrowUp", "ArrowUp", 38, None),
        "down" | "arrowdown" => return named("ArrowDown", "ArrowDown", 40, None),
        "left" | "arrowleft" => return named("ArrowLeft", "ArrowLeft", 37, None),
        "right" | "arrowright" => return named("ArrowRight", "ArrowRight", 39, None),
        "home" => return named("Home", "Home", 36, None),
        "end" => return named("End", "End", 35, None),
        "page_up" | "pageup" | "prior" => return named("PageUp", "PageUp", 33, None),
        "page_down" | "pagedown" | "next" => return named("PageDown", "PageDown", 34, None),
        "insert" => return named("Insert", "Insert", 45, None),
        "shift" => return modifier_key(Modifier::Shift),
        "ctrl" | "control" => return modifier_key(Modifier::Ctrl),
        "alt" => return modifier_key(Modifier::Alt),
        "super" | "meta" | "cmd" => return modifier_key(Modifier::Super),
        _ => {}
    }
    if let Some(n) = lower
        .strip_prefix('f')
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|n| (1..=12).contains(n))
    {
        return named(&format!("F{n}"), &format!("F{n}"), 111 + n, None);
    }
    let mut chars = name.chars();
    if let (Some(ch), None) = (chars.next(), chars.next()) {
        let upper = ch.to_ascii_uppercase();
        let (code, virtual_key) = if ch.is_ascii_alphabetic() {
            (format!("Key{upper}"), upper as u32)
        } else if ch.is_ascii_digit() {
            (format!("Digit{ch}"), ch as u32)
        } else {
            (String::new(), 0)
        };
        return CdpKey {
            key: ch.to_string(),
            code,
            virtual_key,
            text: Some(ch.to_string()),
        };
    }
    // Unknown multi-character names pass through as the DOM key value.
    named(name, name, 0, None)
}

// ============================================================================
// Browserless backend
// ============================================================================

/// URL policy shared by `navigate` and the post-action guard.
fn url_blocked(context: &ToolContext, url: &str) -> Option<String> {
    if is_local_browser_url(url) {
        return None;
    }
    match validate_browserless_navigation(context.network_access.as_ref(), url) {
        Ok(()) => None,
        Err(ToolExecutionResult::ToolError(message)) => Some(message),
        Err(other) => Some(format!("{other:?}")),
    }
}

/// Pages the guard leaves alone: blank pages and in-browser documents that
/// never reach the network.
fn is_local_page(url: &str) -> bool {
    is_local_browser_url(url)
}

/// The `computer` backend on a persistent Browserless browser.
pub struct BrowserlessComputerBackend;

#[async_trait]
impl ComputerBackend for BrowserlessComputerBackend {
    fn id(&self) -> &str {
        "browserless"
    }

    fn supports_navigation(&self) -> bool {
        true
    }

    async fn acquire(
        &self,
        context: &ToolContext,
        display: DisplaySize,
    ) -> Result<Box<dyn ComputerSession>, ToolExecutionResult> {
        let api_token = get_api_token(context).await?;
        let session = match try_get_cdp_session(context).await {
            Some(session) => session,
            None => {
                let ws_url = crate::browser_session_url(&crate::browserless_ws_base(), &api_token);
                CdpSession::connect(&ws_url, BrowserEgress::for_context(context), None)
                    .await
                    .map_err(ToolExecutionResult::tool_error)?
            }
        };
        let cursor = load_cursor(context).await;
        match CdpDisplay::attach(session, display, cursor).await {
            Ok(display) => Ok(Box::new(BrowserlessComputerSession {
                display,
                context: context.clone(),
            })),
            Err((session, e)) => {
                session.disconnect().await;
                Err(ToolExecutionResult::tool_error(e))
            }
        }
    }
}

struct BrowserlessComputerSession {
    display: CdpDisplay,
    context: ToolContext,
}

#[async_trait]
impl ComputerSession for BrowserlessComputerSession {
    fn display(&self) -> DisplaySize {
        self.display.display
    }

    async fn perform(&mut self, action: &ComputerAction) -> Result<(), String> {
        if let ComputerAction::Navigate { url } = action
            && let Some(reason) = url_blocked(&self.context, url)
        {
            return Err(reason);
        }
        let outcome = self.display.perform(action).await;
        // THREAT[TM-TOOL-015]: the page may have navigated itself.
        if let Ok(url) = self.display.current_url().await
            && !is_local_page(&url)
            && let Some(reason) = url_blocked(&self.context, &url)
        {
            let _ = self.display.session_mut().navigate("about:blank").await;
            return Err(format!(
                "the page navigated to a blocked address and was reset to a blank page ({reason})"
            ));
        }
        outcome
    }

    async fn screenshot(&mut self) -> Result<Screenshot, String> {
        self.display.screenshot().await
    }

    async fn release(self: Box<Self>) {
        let Self { display, context } = *self;
        save_cursor(&context, display.cursor()).await;
        let mut session = display.into_session();
        match session.reconnect(DEFAULT_RECONNECT_TIMEOUT_MS).await {
            Ok(endpoint) => {
                let browser_context_id = session.browser_context_id().to_string();
                if let Err(e) =
                    register_session_endpoint(&context, endpoint, &browser_context_id).await
                {
                    warn!("computer_use: failed to persist the browser session: {e:?}");
                }
            }
            Err(e) => warn!("computer_use: failed to keep the browser alive: {e}"),
        }
        session.disconnect().await;
    }
}

async fn load_cursor(context: &ToolContext) -> [u32; 2] {
    let Some(storage) = context.storage_store.as_ref() else {
        return [0, 0];
    };
    storage
        .get_value(context.session_id, CURSOR_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|value| {
            let (x, y) = value.split_once(',')?;
            Some([x.parse().ok()?, y.parse().ok()?])
        })
        .unwrap_or([0, 0])
}

async fn save_cursor(context: &ToolContext, [x, y]: [u32; 2]) {
    if let Some(storage) = context.storage_store.as_ref() {
        let _ = storage
            .set_value(context.session_id, CURSOR_KEY, &format!("{x},{y}"))
            .await;
    }
}

// ============================================================================
// Capability
// ============================================================================

/// Provider-neutral computer use on a Browserless browser.
pub struct BrowserlessComputerUseCapability;

impl everruns_contracts::runtime::capabilities::Capability for BrowserlessComputerUseCapability {
    fn id(&self) -> &str {
        COMPUTER_USE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Computer Use"
    }

    fn description(&self) -> &str {
        "Let the agent see and operate a browser the way a person does: it takes screenshots and \
         clicks, types, scrolls, and presses keys at screen coordinates. Works with any model that \
         accepts images. The model sees everything on the screen, so on-screen text can try to \
         steer it; keep credentials out of reach. Runs on a Browserless browser."
    }

    fn status(&self) -> everruns_contracts::runtime::capabilities::CapabilityStatus {
        everruns_contracts::runtime::capabilities::CapabilityStatus::Available
    }

    fn risk_level(&self) -> everruns_contracts::runtime::capabilities::RiskLevel {
        everruns_contracts::runtime::capabilities::RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("monitor")
    }

    fn category(&self) -> Option<&str> {
        Some("Browser")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(COMPUTER_USE_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&Value::Null)
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        vec![Box::new(ComputerTool::new(
            Arc::new(BrowserlessComputerBackend),
            ComputerUseConfig::from_value_or_default(config),
        ))]
    }

    fn config_schema(&self) -> Option<Value> {
        Some(ComputerUseConfig::json_schema())
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        ComputerUseConfig::from_value(config).map(|_| ())
    }

    fn driver_options(&self, config: &Value) -> Vec<(String, Value)> {
        // Native OpenAI / Anthropic computer tools where the model has one;
        // every other driver ignores the option and keeps the function tool.
        ComputerUseConfig::from_value_or_default(config).driver_options()
    }

    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn PreToolUseHook>> {
        self.pre_tool_use_hooks_with_config(&Value::Null)
    }

    fn pre_tool_use_hooks_with_config(&self, _config: &Value) -> Vec<Arc<dyn PreToolUseHook>> {
        // Computer use relies on soft approval (the stop-and-ask prompt
        // rule): no per-call hard gate (EVE-1133 decision; EVE-1140 found
        // hosted sessions have no hard gate at all).
        vec![]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }

    fn exclusive_group(&self) -> Option<&'static str> {
        // One display per agent: every computer-use backend provides the
        // `computer` tool, so enabling two is rejected rather than letting
        // the last one silently win.
        Some(COMPUTER_TOOL_NAME)
    }

    fn features(&self) -> Vec<&'static str> {
        vec![everruns_contracts::runtime::LEASED_RESOURCES_FEATURE]
    }
}

#[cfg(test)]
#[path = "computer_tests.rs"]
mod tests;
