//! Computer use on a Daytona sandbox desktop (experimental).
//!
//! The provider-neutral `computer` tool (`everruns_contracts::runtime::computer_use`)
//! drives a display through [`ComputerBackend`]. This backend's display is the
//! desktop Daytona's default sandbox image ships (Xvfb, xfce, x11vnc), driven
//! through the toolbox's Computer Use API: `/computeruse/mouse/*`,
//! `/computeruse/keyboard/*` and `/computeruse/screenshot`. The display size is
//! set once, at create time, with the image's `VNC_RESOLUTION` variable.
//!
//! Decision: the native Computer Use API, not xdotool through `exec`. Every
//! action is a JSON request to the sandbox daemon, so no shell ever sees text
//! the model typed. Keys the daemon has no name for are refused by the daemon
//! with a readable error rather than emulated.
//!
//! Security: the sandbox is the session's own. Its id lives under a session
//! storage key reserved from `kv_store` (`COMPUTER_USE_DISPLAY_KV_PREFIX`), and
//! is re-checked through `get_sandbox_state`, which runs
//! `verify_owned_external_resource_if_available`. It is leased like any
//! `daytona_create_sandbox` sandbox, so session cleanup deletes it.
//!
//! Security: egress is the Daytona sandbox's own network policy, the same as
//! `daytona_create_sandbox`. The session network access list does not apply
//! inside the desktop.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::Method;
use serde_json::{Value, json};
use tracing::warn;

use everruns_contracts::runtime::LEASED_RESOURCES_FEATURE;
use everruns_contracts::runtime::capabilities::{Capability, CapabilityStatus, RiskLevel};
use everruns_contracts::runtime::computer_use::{
    COMPUTER_TOOL_NAME, COMPUTER_USE_DISPLAY_KV_PREFIX, COMPUTER_USE_SYSTEM_PROMPT, ComputerAction,
    ComputerBackend, ComputerSession, ComputerTool, ComputerUseConfig, DisplaySize, Modifier,
    MouseButton, Screenshot, parse_key_combo, parse_modifiers, png_base64_image,
    png_base64_screenshot,
};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};

use crate::daytona::client::DaytonaClient;
use crate::daytona::client_test_override::daytona_client_from_context;
use crate::daytona::naming::create_sandbox_with_unique_name;
use crate::daytona::state::{
    SandboxState, delete_sandbox_state, get_api_key, get_sandbox_state, release_sandbox_lease,
    save_sandbox_state, touch_sandbox_lease,
};
use crate::daytona::{
    AUTO_ARCHIVE_INTERVAL_MINUTES, AUTO_DELETE_INTERVAL_MINUTES, AUTO_STOP_INTERVAL_MINUTES,
    DAYTONA_WORKSPACE_PATH,
};

/// Capability id of desktop computer use on Daytona.
pub const DAYTONA_COMPUTER_USE_CAPABILITY_ID: &str = "computer_use_daytona";

/// Sandbox environment variable the default image reads for the display size.
const RESOLUTION_ENV: &str = "VNC_RESOLUTION";

/// How long the desktop may take to report itself active.
const DESKTOP_START_TIMEOUT: Duration = Duration::from_secs(60);
const DESKTOP_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// A screenshot waits this long after the last input, so the frame shows
/// what the input did rather than the screen just before it.
const SETTLE_DELAY: Duration = Duration::from_millis(300);

/// Session storage key holding the desktop sandbox id.
fn desktop_sandbox_key() -> String {
    format!("{COMPUTER_USE_DISPLAY_KV_PREFIX}daytona")
}

// ============================================================================
// Action -> Computer Use API requests
// ============================================================================

/// One request to the sandbox's Computer Use API.
#[derive(Debug, Clone, PartialEq)]
pub struct DesktopInput {
    /// Path under `/computeruse`, e.g. `/mouse/click`.
    pub path: &'static str,
    /// JSON body.
    pub body: Value,
}

fn input(path: &'static str, body: Value) -> DesktopInput {
    DesktopInput { path, body }
}

/// Whether `action` acts at the pointer and so needs its position first.
pub fn needs_pointer(action: &ComputerAction) -> bool {
    if let Some(click) = action.as_click() {
        return click.coordinate.is_none();
    }
    matches!(
        action,
        ComputerAction::Scroll {
            coordinate: None,
            ..
        }
    )
}

/// Requests that perform `action`, run in order. `pointer` is where the
/// pointer is, for clicks and scrolls without a coordinate. `screenshot` and
/// `wait` need none; `navigate` is refused because the display is a desktop.
pub fn desktop_inputs(
    action: &ComputerAction,
    pointer: Option<[u32; 2]>,
) -> Result<Vec<DesktopInput>, String> {
    let at = |coordinate: Option<[u32; 2]>| {
        coordinate
            .or(pointer)
            .ok_or_else(|| "the pointer position is unknown".to_string())
    };
    if let Some(click) = action.as_click() {
        let [x, y] = at(click.coordinate)?;
        let modifiers = daytona_modifiers(&parse_modifiers(click.modifiers.unwrap_or(""))?);
        return Ok(vec![input(
            "/mouse/click",
            json!({
                "x": x,
                "y": y,
                "button": click.button.as_str(),
                "clicks": click.count,
                "double": click.count == 2,
                "modifiers": modifiers,
            }),
        )]);
    }
    match action {
        // Run by the session itself: they read the screen or need a pause.
        ComputerAction::Screenshot
        | ComputerAction::Wait { .. }
        | ComputerAction::Zoom { .. }
        | ComputerAction::CursorPosition
        | ComputerAction::HoldKey { .. } => Ok(Vec::new()),
        ComputerAction::LeftClickDrag {
            start_coordinate,
            coordinate,
            text,
        } => {
            // The drag endpoint takes no modifiers: hold them around it.
            let held = daytona_modifiers(&parse_modifiers(text.as_deref().unwrap_or(""))?);
            let mut inputs: Vec<DesktopInput> = held
                .iter()
                .map(|name| input("/keyboard/down", json!({ "key": name })))
                .collect();
            inputs.push(input(
                "/mouse/drag",
                json!({
                    "startX": start_coordinate[0],
                    "startY": start_coordinate[1],
                    "endX": coordinate[0],
                    "endY": coordinate[1],
                    "button": MouseButton::Left.as_str(),
                }),
            ));
            inputs.extend(
                held.iter()
                    .rev()
                    .map(|name| input("/keyboard/up", json!({ "key": name }))),
            );
            Ok(inputs)
        }
        // No coordinate: the daemon presses where the pointer is.
        ComputerAction::LeftMouseDown => Ok(vec![input(
            "/mouse/down",
            json!({ "button": MouseButton::Left.as_str() }),
        )]),
        ComputerAction::LeftMouseUp => Ok(vec![input(
            "/mouse/up",
            json!({ "button": MouseButton::Left.as_str() }),
        )]),
        ComputerAction::MouseMove { coordinate } => Ok(vec![input(
            "/mouse/move",
            json!({ "x": coordinate[0], "y": coordinate[1] }),
        )]),
        ComputerAction::Scroll {
            coordinate,
            scroll_direction,
            scroll_amount,
        } => {
            let [x, y] = at(*coordinate)?;
            Ok(vec![input(
                "/mouse/scroll",
                json!({
                    "x": x,
                    "y": y,
                    "direction": scroll_direction_name(*scroll_direction),
                    "amount": scroll_amount,
                }),
            )])
        }
        // The text travels as one JSON string to the daemon; no shell sees it.
        ComputerAction::Type { text } => Ok(vec![input("/keyboard/type", json!({ "text": text }))]),
        ComputerAction::Key { text, repeat } => {
            let press = key_inputs(text)?;
            Ok((0..repeat.unwrap_or(1))
                .flat_map(|_| press.iter().cloned())
                .collect())
        }
        ComputerAction::Navigate { .. } => Err(
            "navigate is not available on a desktop display; open a browser on the desktop and \
             type the address instead"
                .to_string(),
        ),
        // Clicks returned above.
        _ => Ok(Vec::new()),
    }
}

fn scroll_direction_name(
    direction: everruns_contracts::runtime::computer_use::ScrollDirection,
) -> &'static str {
    use everruns_contracts::runtime::computer_use::ScrollDirection;
    match direction {
        ScrollDirection::Up => "up",
        ScrollDirection::Down => "down",
        ScrollDirection::Left => "left",
        ScrollDirection::Right => "right",
    }
}

/// Modifier names the daemon takes.
fn modifier_name(modifier: Modifier) -> &'static str {
    match modifier {
        Modifier::Shift => "shift",
        Modifier::Ctrl => "ctrl",
        Modifier::Alt => "alt",
        Modifier::Super => "cmd",
    }
}

fn daytona_modifiers(modifiers: &[Modifier]) -> Vec<&'static str> {
    modifiers
        .iter()
        .map(|modifier| modifier_name(*modifier))
        .collect()
}

/// The requests for one key press or combo such as `Return` or `ctrl+a`.
fn key_inputs(text: &str) -> Result<Vec<DesktopInput>, String> {
    let combo = parse_key_combo(text)?;
    let mut modifiers = combo.modifiers;
    // A bare modifier (`shift`, `super`) is pressed and released on its own:
    // the key endpoint wants a non-modifier key.
    if let Ok(held) = parse_modifiers(&combo.key)
        && let [modifier] = held.as_slice()
    {
        modifiers.push(*modifier);
        let names = daytona_modifiers(&modifiers);
        let downs = names
            .iter()
            .map(|name| input("/keyboard/down", json!({ "key": name })));
        let ups = names
            .iter()
            .rev()
            .map(|name| input("/keyboard/up", json!({ "key": name })));
        return Ok(downs.chain(ups).collect());
    }
    let (key, shifted) = daytona_key(&combo.key)?;
    if shifted && !modifiers.contains(&Modifier::Shift) {
        modifiers.push(Modifier::Shift);
    }
    Ok(vec![input(
        "/keyboard/key",
        json!({ "key": key, "modifiers": daytona_modifiers(&modifiers) }),
    )])
}

/// The daemon key names `hold_key` presses for `text`, in press order.
pub fn held_keys(text: &str) -> Result<Vec<String>, String> {
    let combo = parse_key_combo(text)?;
    let mut names: Vec<String> = daytona_modifiers(&combo.modifiers)
        .into_iter()
        .map(str::to_string)
        .collect();
    if let Ok(held) = parse_modifiers(&combo.key)
        && let [modifier] = held.as_slice()
    {
        names.push(modifier_name(*modifier).to_string());
        return Ok(names);
    }
    let (key, shifted) = daytona_key(&combo.key)?;
    if shifted && !names.iter().any(|name| name == "shift") {
        names.push("shift".to_string());
    }
    names.push(key);
    Ok(names)
}

/// US-layout symbols typed with shift, and the key each is on.
const SHIFTED: [(char, char); 21] = [
    ('!', '1'),
    ('@', '2'),
    ('#', '3'),
    ('$', '4'),
    ('%', '5'),
    ('^', '6'),
    ('&', '7'),
    ('*', '8'),
    ('(', '9'),
    (')', '0'),
    ('_', '-'),
    ('+', '='),
    ('{', '['),
    ('}', ']'),
    ('|', '\\'),
    (':', ';'),
    ('"', '\''),
    ('<', ','),
    ('>', '.'),
    ('?', '/'),
    ('~', '`'),
];

/// Map a key name (xdotool or DOM spelling) to the daemon's key name, and
/// whether shift must be held to produce it. Names outside the table go
/// through lower-cased; the daemon refuses one it does not know with a
/// message naming the key, which the model reads.
pub fn daytona_key(name: &str) -> Result<(String, bool), String> {
    let mut chars = name.chars();
    if let (Some(ch), None) = (chars.next(), chars.next()) {
        if ch == ' ' {
            return Ok(("space".to_string(), false));
        }
        if let Some((_, base)) = SHIFTED.iter().find(|(symbol, _)| *symbol == ch) {
            return Ok((base.to_string(), true));
        }
        if ch.is_ascii_uppercase() {
            return Ok((ch.to_ascii_lowercase().to_string(), true));
        }
        return Ok((ch.to_string(), false));
    }
    let lower = name.to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "return" | "enter" => "enter",
        "escape" | "esc" => "escape",
        "tab" => "tab",
        "backspace" => "backspace",
        "delete" | "del" => "delete",
        "space" => "space",
        "home" => "home",
        "end" => "end",
        "page_up" | "pageup" | "prior" | "pgup" => "pageup",
        "page_down" | "pagedown" | "next" | "pgdn" => "pagedown",
        "insert" | "ins" => "insert",
        "up" | "arrowup" => "up",
        "down" | "arrowdown" => "down",
        "left" | "arrowleft" => "left",
        "right" | "arrowright" => "right",
        "caps_lock" | "capslock" => "capslock",
        "menu" => "menu",
        // X keysym names of punctuation keys.
        "minus" => "-",
        "equal" => "=",
        "bracketleft" => "[",
        "bracketright" => "]",
        "backslash" => "\\",
        "semicolon" => ";",
        "apostrophe" => "'",
        "comma" => ",",
        "period" => ".",
        "slash" => "/",
        "grave" => "`",
        "plus" => return Ok(("=".to_string(), true)),
        _ => "",
    };
    if !mapped.is_empty() {
        return Ok((mapped.to_string(), false));
    }
    if lower
        .strip_prefix('f')
        .and_then(|n| n.parse::<u32>().ok())
        .is_some_and(|n| (1..=24).contains(&n))
    {
        return Ok((lower, false));
    }
    if name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Ok((lower, false));
    }
    Err(format!("unknown key `{name}`"))
}

/// The daemon's `message` from an error body, so the model reads the reason
/// rather than a JSON envelope.
fn readable_error(error: &str) -> String {
    let message = error
        .find('{')
        .and_then(|start| serde_json::from_str::<Value>(&error[start..]).ok())
        .and_then(|body| {
            body.get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    match message {
        Some(message) => message
            .trim_start_matches("internal server error: ")
            .trim_start_matches("bad request: ")
            .to_string(),
        None => error.to_string(),
    }
}

// ============================================================================
// Backend
// ============================================================================

/// The `computer` backend on a session-owned Daytona sandbox desktop.
pub struct DaytonaDesktopComputerBackend;

#[async_trait]
impl ComputerBackend for DaytonaDesktopComputerBackend {
    fn id(&self) -> &str {
        "daytona_desktop"
    }

    fn supports_navigation(&self) -> bool {
        false
    }

    async fn acquire(
        &self,
        context: &ToolContext,
        display: DisplaySize,
    ) -> Result<Box<dyn ComputerSession>, ToolExecutionResult> {
        let api_key = get_api_key(context).await?;
        let client = daytona_client_from_context(api_key, context);
        let mut state = desktop_sandbox(&client, context, display).await?;
        let reported = start_desktop(&client, &state.sandbox_id)
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        if reported != display {
            // The size is fixed at create time: a sandbox made for another
            // configured size is replaced rather than driven off target.
            let note = format!("desktop is {reported}, configured {display}");
            warn!("computer_use: {note}; opening a new sandbox");
            discard_sandbox(&client, context, &state.sandbox_id).await;
            state = create_desktop_sandbox(&client, context, display).await?;
            let reported = start_desktop(&client, &state.sandbox_id)
                .await
                .map_err(ToolExecutionResult::tool_error)?;
            if reported != display {
                return Err(ToolExecutionResult::tool_error(format!(
                    "the sandbox desktop is {reported}, not the configured {display}"
                )));
            }
        }
        Ok(Box::new(DaytonaDesktopSession {
            client,
            sandbox_id: state.sandbox_id,
            display,
            last_input: None,
        }))
    }
}

/// The session's desktop sandbox: the recorded one if it is still this
/// session's and usable (started when stopped), otherwise a new one.
async fn desktop_sandbox(
    client: &DaytonaClient,
    context: &ToolContext,
    display: DisplaySize,
) -> Result<SandboxState, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;
    let recorded = storage
        .get_value(context.session_id, &desktop_sandbox_key())
        .await
        .ok()
        .flatten();
    if let Some(sandbox_id) = recorded {
        // The key is reserved from kv_store, and ownership is re-checked here
        // (TM-DAYTONA-005), so a stale or foreign id never reaches Daytona.
        match get_sandbox_state(context, &sandbox_id).await {
            Ok(state) => match wake_sandbox(client, &state.sandbox_id).await {
                Ok(()) => {
                    touch_sandbox_lease(context, &state, None).await?;
                    return Ok(state);
                }
                Err(e) => {
                    warn!("computer_use: desktop sandbox is unusable, opening a new one: {e}");
                    discard_sandbox(client, context, &state.sandbox_id).await;
                }
            },
            Err(_) => warn!("computer_use: recorded desktop sandbox is unavailable"),
        }
    }
    create_desktop_sandbox(client, context, display).await
}

/// Bring a recorded sandbox to `started`. Auto-stop ends the desktop's
/// processes, which [`start_desktop`] starts again.
async fn wake_sandbox(client: &DaytonaClient, sandbox_id: &str) -> Result<(), String> {
    let info = client.get_sandbox(sandbox_id).await?;
    match info.state.as_str() {
        "started" => Ok(()),
        "stopped" | "archived" => {
            client.start_sandbox(sandbox_id).await?;
            client.wait_for_ready(sandbox_id).await
        }
        "starting" | "restoring" | "creating" | "pending_build" => {
            client.wait_for_ready(sandbox_id).await
        }
        other => Err(format!("sandbox is {other}")),
    }
}

async fn create_desktop_sandbox(
    client: &DaytonaClient,
    context: &ToolContext,
    display: DisplaySize,
) -> Result<SandboxState, ToolExecutionResult> {
    let mut labels = serde_json::Map::new();
    labels.insert("everruns".to_string(), json!("true"));
    labels.insert(
        "everruns.session_id".to_string(),
        json!(context.session_id.to_string()),
    );
    labels.insert("everruns.purpose".to_string(), json!("computer_use"));
    // No snapshot: Daytona's default image is the one that ships the desktop.
    let body = json!({
        "autoStopInterval": AUTO_STOP_INTERVAL_MINUTES,
        "autoArchiveInterval": AUTO_ARCHIVE_INTERVAL_MINUTES,
        "autoDeleteInterval": AUTO_DELETE_INTERVAL_MINUTES,
        "labels": labels,
        "env": { RESOLUTION_ENV: display.to_string() },
    });
    let (info, name) = create_sandbox_with_unique_name(client, "everruns-computer-use", body)
        .await
        .map_err(ToolExecutionResult::tool_error)?;
    let state = SandboxState {
        sandbox_id: info.id,
        workspace_path: DAYTONA_WORKSPACE_PATH.to_string(),
        started_at: chrono::Utc::now().to_rfc3339(),
    };
    // Recorded and leased before waiting, so a failed start is still cleaned up.
    save_sandbox_state(context, &state).await?;
    touch_sandbox_lease(context, &state, Some(name.canonical_name)).await?;
    if let Some(storage) = context.storage_store.as_ref()
        && let Err(e) = storage
            .set_value(
                context.session_id,
                &desktop_sandbox_key(),
                &state.sandbox_id,
            )
            .await
    {
        warn!("computer_use: failed to record the desktop sandbox: {e}");
    }
    client
        .wait_for_ready(&state.sandbox_id)
        .await
        .map_err(ToolExecutionResult::tool_error)?;
    Ok(state)
}

/// Delete a desktop sandbox this session no longer uses. Best effort: the
/// lease and Daytona's auto-delete catch whatever fails here.
async fn discard_sandbox(client: &DaytonaClient, context: &ToolContext, sandbox_id: &str) {
    if let Err(e) = client.delete_sandbox(sandbox_id).await {
        warn!("computer_use: failed to delete the old desktop sandbox: {e}");
        return;
    }
    let _ = release_sandbox_lease(context, sandbox_id).await;
    let _ = delete_sandbox_state(context, sandbox_id).await;
}

async fn computer_use(
    client: &DaytonaClient,
    sandbox_id: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    client
        .api_call(
            method,
            &format!("/toolbox/{sandbox_id}/computeruse{path}"),
            body,
        )
        .await
        .map_err(|e| readable_error(&e))
}

/// Start the desktop processes unless they already run, wait until the
/// desktop reports a display, and return its size.
async fn start_desktop(client: &DaytonaClient, sandbox_id: &str) -> Result<DisplaySize, String> {
    let status = computer_use(client, sandbox_id, Method::GET, "/status", None).await?;
    if status.get("status").and_then(Value::as_str) != Some("active") {
        computer_use(client, sandbox_id, Method::POST, "/start", None)
            .await
            .map_err(|e| format!("failed to start the sandbox desktop: {e}"))?;
    }
    let deadline = Instant::now() + DESKTOP_START_TIMEOUT;
    loop {
        let active = computer_use(client, sandbox_id, Method::GET, "/status", None)
            .await
            .ok()
            .and_then(|status| {
                status
                    .get("status")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .is_some_and(|status| status == "active");
        if active
            && let Ok(info) =
                computer_use(client, sandbox_id, Method::GET, "/display/info", None).await
            && let Some(size) = primary_display(&info)
        {
            return Ok(size);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "the sandbox desktop did not start within {} seconds",
                DESKTOP_START_TIMEOUT.as_secs()
            ));
        }
        tokio::time::sleep(DESKTOP_POLL_INTERVAL).await;
    }
}

/// The active display's size from `/display/info`.
pub fn primary_display(info: &Value) -> Option<DisplaySize> {
    let displays = info.get("displays")?.as_array()?;
    let primary = displays
        .iter()
        .find(|display| display.get("isActive").and_then(Value::as_bool) == Some(true))
        .or_else(|| displays.first())?;
    let width = u32::try_from(primary.get("width")?.as_u64()?).ok()?;
    let height = u32::try_from(primary.get("height")?.as_u64()?).ok()?;
    (width > 0 && height > 0).then_some(DisplaySize { width, height })
}

/// One call's handle on a Daytona sandbox's desktop.
pub struct DaytonaDesktopSession {
    client: DaytonaClient,
    sandbox_id: String,
    display: DisplaySize,
    last_input: Option<Instant>,
}

impl DaytonaDesktopSession {
    /// Drive the desktop of `sandbox_id`, starting it when needed. The display
    /// must already be `display`. Ownership of the sandbox is the caller's job.
    pub async fn start(
        client: DaytonaClient,
        sandbox_id: String,
        display: DisplaySize,
    ) -> Result<Self, String> {
        let reported = start_desktop(&client, &sandbox_id).await?;
        if reported != display {
            return Err(format!(
                "the sandbox desktop is {reported}, not the expected {display}"
            ));
        }
        Ok(Self {
            client,
            sandbox_id,
            display,
            last_input: None,
        })
    }

    /// Wait out what is left of [`SETTLE_DELAY`] since the last input.
    async fn settle(&self) {
        if let Some(last) = self.last_input {
            let elapsed = last.elapsed();
            if elapsed < SETTLE_DELAY {
                tokio::time::sleep(SETTLE_DELAY - elapsed).await;
            }
        }
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        computer_use(
            &self.client,
            &self.sandbox_id,
            Method::POST,
            path,
            Some(body),
        )
        .await
    }

    /// Press every key of `text`, hold for `duration`, release in reverse.
    /// Releases run even when a press or the hold failed, so no key stays down.
    async fn hold_key(&mut self, text: &str, duration: f64) -> Result<(), String> {
        let keys = held_keys(text)?;
        self.last_input = Some(Instant::now());
        let mut pressed = Vec::new();
        let mut failure = None;
        for key in &keys {
            match self.post("/keyboard/down", json!({ "key": key })).await {
                Ok(_) => pressed.push(key),
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        if failure.is_none() {
            tokio::time::sleep(Duration::from_secs_f64(duration)).await;
        }
        for key in pressed.iter().rev() {
            if let Err(e) = self.post("/keyboard/up", json!({ "key": key })).await {
                failure.get_or_insert(e);
            }
        }
        self.last_input = Some(Instant::now());
        failure.map_or(Ok(()), Err)
    }

    async fn pointer(&self) -> Result<[u32; 2], String> {
        let position = computer_use(
            &self.client,
            &self.sandbox_id,
            Method::GET,
            "/mouse/position",
            None,
        )
        .await?;
        let coordinate = |key: &str| {
            position
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
        };
        match (coordinate("x"), coordinate("y")) {
            (Some(x), Some(y)) => Ok([x, y]),
            _ => Err("the sandbox did not report the pointer position".to_string()),
        }
    }
}

#[async_trait]
impl ComputerSession for DaytonaDesktopSession {
    fn display(&self) -> DisplaySize {
        self.display
    }

    async fn perform(&mut self, action: &ComputerAction) -> Result<(), String> {
        match action {
            ComputerAction::Wait { duration } => {
                tokio::time::sleep(Duration::from_secs_f64(*duration)).await;
                return Ok(());
            }
            ComputerAction::HoldKey { text, duration } => {
                return self.hold_key(text, *duration).await;
            }
            _ => {}
        }
        let pointer = if needs_pointer(action) {
            Some(self.pointer().await?)
        } else {
            None
        };
        let inputs = desktop_inputs(action, pointer)?;
        for (index, request) in inputs.iter().enumerate() {
            self.last_input = Some(Instant::now());
            if let Err(e) = computer_use(
                &self.client,
                &self.sandbox_id,
                Method::POST,
                request.path,
                Some(request.body.clone()),
            )
            .await
            {
                // Never leave a modifier held down.
                for release in &inputs[index + 1..] {
                    if release.path == "/keyboard/up" {
                        let _ = computer_use(
                            &self.client,
                            &self.sandbox_id,
                            Method::POST,
                            release.path,
                            Some(release.body.clone()),
                        )
                        .await;
                    }
                }
                return Err(e);
            }
        }
        Ok(())
    }

    async fn screenshot(&mut self) -> Result<Screenshot, String> {
        self.settle().await;
        let response = computer_use(
            &self.client,
            &self.sandbox_id,
            Method::GET,
            "/screenshot",
            None,
        )
        .await
        .map_err(|e| format!("failed to capture the desktop: {e}"))?;
        let frame = response
            .get("screenshot")
            .and_then(Value::as_str)
            .ok_or_else(|| "the sandbox returned no screenshot".to_string())?;
        png_base64_screenshot(frame.to_string(), self.display)
    }

    async fn zoom(&mut self, [x0, y0, x1, y1]: [u32; 4]) -> Result<Screenshot, String> {
        self.settle().await;
        // The daemon only scales frames down, so the region comes back at its
        // native size: still full resolution, just not enlarged.
        let path = format!(
            "/screenshot/region?x={x0}&y={y0}&width={}&height={}",
            x1 - x0,
            y1 - y0
        );
        let response = computer_use(&self.client, &self.sandbox_id, Method::GET, &path, None)
            .await
            .map_err(|e| format!("failed to capture the region: {e}"))?;
        let frame = response
            .get("screenshot")
            .and_then(Value::as_str)
            .ok_or_else(|| "the sandbox returned no screenshot".to_string())?;
        png_base64_image(frame.to_string())
    }

    async fn cursor_position(&mut self) -> Result<[u32; 2], String> {
        self.pointer().await
    }

    async fn release(self: Box<Self>) {
        // The sandbox stays up for the next call; Daytona's auto-stop and the
        // session lease (refreshed on acquire) bound its lifetime.
    }
}

// ============================================================================
// Capability
// ============================================================================

const DESKTOP_PROMPT_NOTE: &str = " The display is a Linux desktop, not a browser page: \
there is no navigate action, so open applications from the desktop and type addresses into them.";

/// Provider-neutral computer use on a Daytona sandbox desktop.
pub struct DaytonaDesktopComputerUseCapability;

impl Capability for DaytonaDesktopComputerUseCapability {
    fn id(&self) -> &str {
        DAYTONA_COMPUTER_USE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Daytona Desktop Computer Use"
    }

    fn description(&self) -> &str {
        "Let the agent see and operate a full Linux desktop in a Daytona sandbox the way a person \
         does: it takes screenshots and clicks, types, scrolls, and presses keys at screen \
         coordinates, in any desktop app. Works with any model that accepts images. The sandbox \
         has Daytona's network access, not the session's network access list. Cannot be combined \
         with another computer use capability: each provides the `computer` tool."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("monitor")
    }

    fn category(&self) -> Option<&str> {
        Some("Sandboxes")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(&DESKTOP_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&Value::Null)
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        vec![Box::new(ComputerTool::new(
            Arc::new(DaytonaDesktopComputerBackend),
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
        ComputerUseConfig::from_value_or_default(config).driver_options()
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }

    fn exclusive_group(&self) -> Option<&'static str> {
        // One display per agent: every computer-use backend provides the
        // `computer` tool.
        Some(COMPUTER_TOOL_NAME)
    }

    fn features(&self) -> Vec<&'static str> {
        vec![LEASED_RESOURCES_FEATURE]
    }
}

static DESKTOP_SYSTEM_PROMPT: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| format!("{COMPUTER_USE_SYSTEM_PROMPT}{DESKTOP_PROMPT_NOTE}"));

#[cfg(test)]
#[path = "computer_tests.rs"]
mod tests;
