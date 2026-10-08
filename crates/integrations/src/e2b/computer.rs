//! Computer use on an E2B desktop sandbox (EVE-1133, experimental).
//!
//! The provider-neutral `computer` tool (`everruns_contracts::runtime::computer_use`)
//! drives a display through [`ComputerBackend`]. This backend's display is a
//! full X desktop inside a session-owned E2B sandbox: Xvfb at exactly the
//! configured size, an xfce session when the template ships one, actions
//! through `xdotool`, frames through ImageMagick `import` (or the first other
//! screenshot tool the template has) read back as PNG bytes.
//!
//! Decision: E2B over Daytona. The `desktop` template already carries Xvfb,
//! xdotool and a desktop session, and envd's process API takes an argv, so
//! no shell sits between the model's text and xdotool.
//!
//! Security: typed text and key names never pass through a shell. Every
//! xdotool call is `exec_argv("xdotool", [...])`; the two fixed shell scripts
//! (display start, screenshot) take only numbers and constant paths as
//! positional arguments. `--` ends option parsing before model-supplied text.
//!
//! Security: the sandbox is the session's own. Its id lives under a session
//! storage key reserved from `kv_store` (`COMPUTER_USE_DISPLAY_KV_PREFIX`), and
//! is re-checked through `get_sandbox_state`, which runs
//! `verify_owned_external_resource_if_available`. It is leased like any
//! `e2b_create_sandbox` sandbox, so session cleanup deletes it.
//!
//! Security: egress is the E2B sandbox's own network policy, the same as
//! `e2b_create_sandbox` (TM-AGENT-019). This backend adds no bypass and no
//! extra filter: unlike the Browserless backend, the session network access
//! list does not apply inside the desktop (TM-E2B-005).
//!
//! Decision: the pointer lives where xdotool keeps it, in the X server, so
//! clicks without a coordinate need no stored cursor.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::warn;

use everruns_contracts::runtime::LEASED_RESOURCES_FEATURE;
use everruns_contracts::runtime::capabilities::{Capability, CapabilityStatus, RiskLevel};
use everruns_contracts::runtime::computer_use::{
    COMPUTER_TOOL_NAME, COMPUTER_USE_DISPLAY_KV_PREFIX, COMPUTER_USE_SYSTEM_PROMPT, ComputerAction,
    ComputerBackend, ComputerSession, ComputerTool, ComputerUseConfig, DisplaySize, Modifier,
    MouseButton, Screenshot, ScrollDirection, parse_key_combo, parse_modifiers, png_image,
    zoom_factor,
};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tool_hooks::PreToolUseHook;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};

use crate::e2b::E2B_DEFAULT_TIMEOUT_SECS;
use crate::e2b::client::E2BClient;
use crate::e2b::state::{
    SandboxState, build_state, get_api_key, get_sandbox_state, save_sandbox_state,
    touch_sandbox_lease,
};

/// Capability id of desktop computer use on E2B.
pub const DESKTOP_COMPUTER_USE_CAPABILITY_ID: &str = "computer_use_desktop";

/// E2B template with Xvfb, xdotool and a desktop session.
pub const E2B_DESKTOP_TEMPLATE: &str = "desktop";

/// X display the backend starts. Not `:0`, so it never collides with a
/// display the template or a user started on their own.
pub const DESKTOP_DISPLAY: &str = ":99";

/// Session storage key holding the desktop sandbox id.
fn desktop_sandbox_key() -> String {
    format!("{COMPUTER_USE_DISPLAY_KV_PREFIX}e2b")
}

/// Working directory for display state, logs and frames inside the sandbox.
const WORK_DIR: &str = "/tmp/everruns-computer";
const SCREENSHOT_PATH: &str = "/tmp/everruns-computer/screen.png";

/// Per-command timeout for xdotool and the screenshot script.
const COMMAND_TIMEOUT_MS: u64 = 30_000;
/// The display start waits up to 10 seconds for Xvfb.
const START_TIMEOUT_MS: u64 = 60_000;

/// Delay between typed characters, in milliseconds (xdotool's default).
const TYPE_DELAY_MS: &str = "12";

/// Starts Xvfb at `$2` (`WIDTHxHEIGHT`) on display `$1`, unless it already
/// runs at that size, then an xfce session when the template has one. State,
/// logs and the marker live in `$3`. Positional arguments only: nothing is
/// interpolated into the script text.
const START_DISPLAY_SCRIPT: &str = r#"set -u
display="$1"; size="$2"; dir="$3"
for tool in Xvfb xdotool; do
  command -v "$tool" >/dev/null 2>&1 || { echo "$tool is not installed in this sandbox; computer use needs the E2B desktop template" >&2; exit 127; }
done
mkdir -p "$dir"
if [ "$(cat "$dir/display" 2>/dev/null)" = "$display $size" ] && DISPLAY="$display" xdotool getdisplaygeometry >/dev/null 2>&1; then
  exit 0
fi
pkill -f "Xvfb $display" >/dev/null 2>&1 || true
sleep 0.2
setsid Xvfb "$display" -ac -screen 0 "${size}x24" -dpi 96 -nolisten tcp >"$dir/xvfb.log" 2>&1 </dev/null &
i=0
until DISPLAY="$display" xdotool getdisplaygeometry >/dev/null 2>&1; do
  i=$((i + 1))
  if [ "$i" -ge 100 ]; then
    echo "the virtual display did not start" >&2
    tail -n 20 "$dir/xvfb.log" >&2
    exit 1
  fi
  sleep 0.1
done
if command -v startxfce4 >/dev/null 2>&1; then
  DISPLAY="$display" setsid startxfce4 >"$dir/session.log" 2>&1 </dev/null &
fi
echo "$display $size" >"$dir/display"
"#;

/// Captures display `$1` as a PNG at `$2` with the first tool available.
const SCREENSHOT_SCRIPT: &str = r#"set -u
display="$1"; out="$2"
rm -f "$out"
if command -v import >/dev/null 2>&1; then
  import -display "$display" -window root "png:$out"
elif command -v xwd >/dev/null 2>&1 && command -v convert >/dev/null 2>&1; then
  xwd -display "$display" -root -silent | convert xwd:- "png:$out"
elif command -v scrot >/dev/null 2>&1; then
  DISPLAY="$display" scrot "$out"
elif command -v xfce4-screenshooter >/dev/null 2>&1; then
  DISPLAY="$display" xfce4-screenshooter -f -s "$out"
else
  echo "no screenshot tool (import, xwd and convert, scrot, xfce4-screenshooter) in this sandbox" >&2
  exit 127
fi
[ -s "$out" ] || { echo "the screenshot produced no image" >&2; exit 1; }
"#;

/// Captures the `$5`x`$6` region at (`$3`, `$4`) of display `$1`, scaled to
/// `$7`x`$8`, as a PNG at `$2`. ImageMagick's `import` when the template has
/// it, else a full frame cropped and scaled by `ffmpeg` (E2B's `desktop`
/// template ships ffmpeg and scrot but no ImageMagick).
const ZOOM_SCRIPT: &str = r#"set -u
display="$1"; out="$2"; x="$3"; y="$4"; w="$5"; h="$6"; sw="$7"; sh="$8"
rm -f "$out"
if command -v import >/dev/null 2>&1; then
  import -display "$display" -window root -crop "${w}x${h}+${x}+${y}" +repage -resize "${sw}x${sh}!" "png:$out"
elif command -v ffmpeg >/dev/null 2>&1; then
  full="$out.full.png"; rm -f "$full"
  if command -v scrot >/dev/null 2>&1; then
    DISPLAY="$display" scrot "$full"
  elif command -v xwd >/dev/null 2>&1; then
    full="$out.full.xwd"; xwd -display "$display" -root -silent >"$full"
  fi
  [ -s "$full" ] || { echo "zoom could not capture the display" >&2; exit 1; }
  ffmpeg -loglevel error -y -i "$full" -vf "crop=$w:$h:$x:$y,scale=$sw:$sh:flags=lanczos" -frames:v 1 "$out"
  rm -f "$full"
else
  echo "zoom needs ImageMagick import or ffmpeg in this sandbox" >&2
  exit 127
fi
[ -s "$out" ] || { echo "the zoom produced no image" >&2; exit 1; }
"#;

/// Path of the zoomed region inside the sandbox.
const ZOOM_PATH: &str = "/tmp/everruns-computer/zoom.png";

/// `bash -c` arguments of [`ZOOM_SCRIPT`]: the crop geometry and the output
/// size that fits the region to the display with its aspect ratio kept.
pub fn zoom_args(region: [u32; 4], display: DisplaySize) -> Vec<String> {
    let [x0, y0, x1, y1] = region;
    let factor = zoom_factor(region, display);
    let scaled = |length: u32| ((f64::from(length) * factor).round() as u32).max(1);
    vec![
        "-c".to_string(),
        ZOOM_SCRIPT.to_string(),
        "everruns-zoom".to_string(),
        DESKTOP_DISPLAY.to_string(),
        ZOOM_PATH.to_string(),
        x0.to_string(),
        y0.to_string(),
        (x1 - x0).to_string(),
        (y1 - y0).to_string(),
        scaled(x1 - x0).to_string(),
        scaled(y1 - y0).to_string(),
    ]
}

/// `X=12` / `Y=34` lines of `xdotool getmouselocation --shell`.
pub fn parse_mouse_location(output: &str) -> Result<[u32; 2], String> {
    let value = |name: &str| {
        output
            .lines()
            .find_map(|line| line.trim().strip_prefix(name))
            .and_then(|rest| rest.strip_prefix('='))
            .and_then(|number| number.trim().parse::<u32>().ok())
    };
    match (value("X"), value("Y")) {
        (Some(x), Some(y)) => Ok([x, y]),
        _ => Err("xdotool did not report the pointer position".to_string()),
    }
}

// ============================================================================
// Action -> xdotool argv
// ============================================================================

/// xdotool argv lists (without the leading `xdotool`) that perform `action`,
/// run in order. `screenshot` and `wait` need none; `navigate` is refused
/// because the display is a desktop, not a browser page.
pub fn xdotool_commands(action: &ComputerAction) -> Result<Vec<Vec<String>>, String> {
    if let Some(click) = action.as_click() {
        let modifiers = parse_modifiers(click.modifiers.unwrap_or(""))?;
        let mut commands = Vec::new();
        if let Some(point) = click.coordinate {
            commands.push(move_to(point));
        }
        for modifier in &modifiers {
            commands.push(argv(["keydown", modifier_name(*modifier)]));
        }
        let mut press = argv(["click"]);
        if click.count > 1 {
            press.extend(argv(["--repeat", &click.count.to_string()]));
        }
        press.push(button_number(click.button).to_string());
        commands.push(press);
        // Released in reverse, like a hand lifting off the keys.
        for modifier in modifiers.iter().rev() {
            commands.push(argv(["keyup", modifier_name(*modifier)]));
        }
        return Ok(commands);
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
            let modifiers = parse_modifiers(text.as_deref().unwrap_or(""))?;
            // A midpoint gives drag handlers a motion event between press and
            // release.
            let mid = [
                (start_coordinate[0] + coordinate[0]) / 2,
                (start_coordinate[1] + coordinate[1]) / 2,
            ];
            let mut commands = vec![move_to(*start_coordinate)];
            for modifier in &modifiers {
                commands.push(argv(["keydown", modifier_name(*modifier)]));
            }
            commands.extend([
                argv(["mousedown", "1"]),
                move_to(mid),
                move_to(*coordinate),
                argv(["mouseup", "1"]),
            ]);
            for modifier in modifiers.iter().rev() {
                commands.push(argv(["keyup", modifier_name(*modifier)]));
            }
            Ok(commands)
        }
        ComputerAction::LeftMouseDown => Ok(vec![argv(["mousedown", "1"])]),
        ComputerAction::LeftMouseUp => Ok(vec![argv(["mouseup", "1"])]),
        ComputerAction::MouseMove { coordinate } => Ok(vec![move_to(*coordinate)]),
        ComputerAction::Scroll {
            coordinate,
            scroll_direction,
            scroll_amount,
        } => {
            let mut commands = Vec::new();
            if let Some(point) = coordinate {
                commands.push(move_to(*point));
            }
            commands.push(argv([
                "click",
                "--repeat",
                &scroll_amount.to_string(),
                scroll_button(*scroll_direction),
            ]));
            Ok(commands)
        }
        // `--` ends option parsing: text that starts with `-` is typed, not
        // read as a flag. The text is one argv entry, never shell input.
        ComputerAction::Type { text } => Ok(vec![argv([
            "type",
            "--delay",
            TYPE_DELAY_MS,
            "--",
            text.as_str(),
        ])]),
        ComputerAction::Key { text, repeat } => {
            let combo = xdotool_key_combo(text)?;
            // Repeats as separate keystroke arguments rather than `--repeat`,
            // which older xdotool builds lack.
            let mut command = argv(["key", "--"]);
            for _ in 0..repeat.unwrap_or(1) {
                command.push(combo.clone());
            }
            Ok(vec![command])
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

fn argv<const N: usize>(parts: [&str; N]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_string()).collect()
}

fn move_to([x, y]: [u32; 2]) -> Vec<String> {
    vec![
        "mousemove".to_string(),
        "--sync".to_string(),
        x.to_string(),
        y.to_string(),
    ]
}

fn button_number(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
    }
}

/// X wheel buttons: 4 up, 5 down, 6 left, 7 right.
fn scroll_button(direction: ScrollDirection) -> &'static str {
    match direction {
        ScrollDirection::Up => "4",
        ScrollDirection::Down => "5",
        ScrollDirection::Left => "6",
        ScrollDirection::Right => "7",
    }
}

fn modifier_name(modifier: Modifier) -> &'static str {
    match modifier {
        Modifier::Shift => "shift",
        Modifier::Ctrl => "ctrl",
        Modifier::Alt => "alt",
        Modifier::Super => "super",
    }
}

/// `ctrl+shift+t` -> `ctrl+shift+t`, `Enter` -> `Return`, `+` -> `plus`.
pub fn xdotool_key_combo(text: &str) -> Result<String, String> {
    let combo = parse_key_combo(text)?;
    let mut parts: Vec<String> = combo
        .modifiers
        .iter()
        .map(|modifier| modifier_name(*modifier).to_string())
        .collect();
    parts.push(xdotool_keysym(&combo.key)?);
    Ok(parts.join("+"))
}

/// Map a key name (xdotool or DOM spelling) to an X keysym name xdotool takes.
fn xdotool_keysym(name: &str) -> Result<String, String> {
    let lower = name.to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "return" | "enter" => "Return",
        "kp_enter" => "KP_Enter",
        "tab" => "Tab",
        "escape" | "esc" => "Escape",
        "backspace" => "BackSpace",
        "delete" | "del" => "Delete",
        "space" => "space",
        "up" | "arrowup" => "Up",
        "down" | "arrowdown" => "Down",
        "left" | "arrowleft" => "Left",
        "right" | "arrowright" => "Right",
        "home" => "Home",
        "end" => "End",
        "page_up" | "pageup" | "prior" => "Page_Up",
        "page_down" | "pagedown" | "next" => "Page_Down",
        "insert" => "Insert",
        "shift" => "shift",
        "ctrl" | "control" => "ctrl",
        "alt" => "alt",
        "super" | "meta" | "cmd" => "super",
        _ => "",
    };
    if !mapped.is_empty() {
        return Ok(mapped.to_string());
    }
    if let Some(n) = lower
        .strip_prefix('f')
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|n| (1..=24).contains(n))
    {
        return Ok(format!("F{n}"));
    }
    let mut chars = name.chars();
    if let (Some(ch), None) = (chars.next(), chars.next()) {
        // xdotool splits combos on `+` and reads a leading `-` as an option;
        // named keysyms sidestep both.
        return Ok(match ch {
            '+' => "plus".to_string(),
            '-' => "minus".to_string(),
            ' ' => "space".to_string(),
            _ => ch.to_string(),
        });
    }
    // Any other multi-character name must look like a keysym
    // (`XF86AudioMute`, `KP_Add`).
    if name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Ok(name.to_string());
    }
    Err(format!("unknown key `{name}`"))
}

// ============================================================================
// Screenshot decoding
// ============================================================================

pub use everruns_contracts::runtime::computer_use::{
    png_dimensions, png_screenshot as decode_screenshot,
};

// ============================================================================
// Backend
// ============================================================================

/// The `computer` backend on a session-owned E2B desktop sandbox.
pub struct E2BDesktopComputerBackend;

#[async_trait]
impl ComputerBackend for E2BDesktopComputerBackend {
    fn id(&self) -> &str {
        "e2b_desktop"
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
        let client = E2BClient::new(api_key);
        let state = desktop_sandbox(&client, context).await?;
        let session = E2BDesktopSession::start(client, state, display)
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        Ok(Box::new(session))
    }
}

/// The session's desktop sandbox: the recorded one if it is still this
/// session's and reachable (resumed when paused), otherwise a new one.
async fn desktop_sandbox(
    client: &E2BClient,
    context: &ToolContext,
) -> Result<SandboxState, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;
    let key = desktop_sandbox_key();
    let recorded = storage
        .get_value(context.session_id, &key)
        .await
        .ok()
        .flatten();
    if let Some(sandbox_id) = recorded {
        // THREAT[TM-E2B-007]: the key is reserved from kv_store, and ownership
        // is re-checked here (TM-E2B-003), so a stale or foreign id never
        // reaches the E2B API.
        match get_sandbox_state(context, &sandbox_id).await {
            Ok(mut state) => {
                let alive = match client
                    .set_timeout(&state.sandbox_id, state.timeout_seconds)
                    .await
                {
                    Ok(()) => true,
                    // Paused (auto-pause on timeout): resume keeps memory, so
                    // the running X server comes back with it.
                    Err(_) => match client
                        .resume_sandbox(&state.sandbox_id, state.timeout_seconds)
                        .await
                    {
                        Ok(resumed) => {
                            if resumed.envd_access_token.is_some() {
                                state.envd_access_token = resumed.envd_access_token;
                            }
                            if let Some(domain) = resumed.domain {
                                state.sandbox_domain = domain;
                            }
                            save_sandbox_state(context, &state).await?;
                            true
                        }
                        Err(e) => {
                            warn!("computer_use: desktop sandbox is gone, opening a new one: {e}");
                            false
                        }
                    },
                };
                if alive {
                    touch_sandbox_lease(context, &state, None).await?;
                    return Ok(state);
                }
            }
            Err(_) => warn!("computer_use: recorded desktop sandbox is unavailable"),
        }
    }

    let title = "Everruns computer-use desktop";
    let mut metadata = serde_json::Map::new();
    metadata.insert("everruns".to_string(), json!("true"));
    metadata.insert("everruns.title".to_string(), json!(title));
    metadata.insert(
        "everruns.session_id".to_string(),
        json!(context.session_id.to_string()),
    );
    metadata.insert("everruns.purpose".to_string(), json!("computer_use"));
    // Same create call, and so the same network policy, as e2b_create_sandbox.
    let created = client
        .create_sandbox(
            E2B_DESKTOP_TEMPLATE,
            E2B_DEFAULT_TIMEOUT_SECS,
            Value::Object(metadata),
            json!({}),
        )
        .await
        .map_err(ToolExecutionResult::tool_error)?;
    let state = build_state(
        &crate::e2b::tools::detail_from_create(created),
        E2B_DEFAULT_TIMEOUT_SECS,
    );
    save_sandbox_state(context, &state).await?;
    touch_sandbox_lease(context, &state, Some(title.to_string())).await?;
    if let Err(e) = storage
        .set_value(context.session_id, &key, &state.sandbox_id)
        .await
    {
        warn!("computer_use: failed to record the desktop sandbox: {e}");
    }
    Ok(state)
}

/// Run the display start script; a no-op when Xvfb already runs at `display`.
async fn start_display(
    client: &E2BClient,
    state: &SandboxState,
    display: DisplaySize,
) -> Result<(), String> {
    let args = start_display_args(display);
    let result = client
        .exec_argv(state, "bash", &args, &[], None, Some(START_TIMEOUT_MS))
        .await?;
    if result.exit_code != 0 {
        return Err(format!(
            "failed to start the desktop display: {}",
            result.stderr.trim()
        ));
    }
    Ok(())
}

/// `bash -c SCRIPT NAME DISPLAY SIZE DIR`: values arrive as `$1..$3`.
fn start_display_args(display: DisplaySize) -> Vec<String> {
    vec![
        "-c".to_string(),
        START_DISPLAY_SCRIPT.to_string(),
        "everruns-start-display".to_string(),
        DESKTOP_DISPLAY.to_string(),
        display.to_string(),
        WORK_DIR.to_string(),
    ]
}

/// One call's handle on a desktop sandbox's display.
pub struct E2BDesktopSession {
    client: E2BClient,
    state: SandboxState,
    display: DisplaySize,
}

impl E2BDesktopSession {
    /// Bring up the display at `display` in a sandbox built from the desktop
    /// template (a no-op when it already runs at that size). Ownership of
    /// `state` is the caller's job.
    pub async fn start(
        client: E2BClient,
        state: SandboxState,
        display: DisplaySize,
    ) -> Result<Self, String> {
        start_display(&client, &state, display).await?;
        Ok(Self {
            client,
            state,
            display,
        })
    }

    // THREAT[TM-E2B-006]: argv straight to xdotool, no shell in between.
    async fn xdotool(&self, args: &[String]) -> Result<(), String> {
        let result = self
            .client
            .exec_argv(
                &self.state,
                "xdotool",
                args,
                &[("DISPLAY", DESKTOP_DISPLAY)],
                None,
                Some(COMMAND_TIMEOUT_MS),
            )
            .await?;
        if result.exit_code != 0 {
            // Name the subcommand only: the arguments may be typed text.
            let command = args.first().map(String::as_str).unwrap_or("");
            return Err(format!(
                "xdotool {command} failed (exit {}): {}",
                result.exit_code,
                result.stderr.trim()
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl ComputerSession for E2BDesktopSession {
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
                let combo = xdotool_key_combo(text)?;
                self.xdotool(&argv(["keydown", "--", &combo])).await?;
                tokio::time::sleep(Duration::from_secs_f64(*duration)).await;
                return self.xdotool(&argv(["keyup", "--", &combo])).await;
            }
            _ => {}
        }
        let commands = xdotool_commands(action)?;
        for (index, command) in commands.iter().enumerate() {
            if let Err(e) = self.xdotool(command).await {
                // Never leave a modifier or button held down.
                for release in &commands[index + 1..] {
                    if matches!(
                        release.first().map(String::as_str),
                        Some("keyup" | "mouseup")
                    ) {
                        let _ = self.xdotool(release).await;
                    }
                }
                return Err(e);
            }
        }
        Ok(())
    }

    async fn screenshot(&mut self) -> Result<Screenshot, String> {
        let args = vec![
            "-c".to_string(),
            SCREENSHOT_SCRIPT.to_string(),
            "everruns-screenshot".to_string(),
            DESKTOP_DISPLAY.to_string(),
            SCREENSHOT_PATH.to_string(),
        ];
        let result = self
            .client
            .exec_argv(
                &self.state,
                "bash",
                &args,
                &[],
                None,
                Some(COMMAND_TIMEOUT_MS),
            )
            .await?;
        if result.exit_code != 0 {
            return Err(format!(
                "failed to capture the desktop: {}",
                result.stderr.trim()
            ));
        }
        let bytes = self
            .client
            .read_file_bytes(&self.state, SCREENSHOT_PATH)
            .await?;
        decode_screenshot(&bytes, self.display)
    }

    async fn zoom(&mut self, region: [u32; 4]) -> Result<Screenshot, String> {
        let args = zoom_args(region, self.display);
        let result = self
            .client
            .exec_argv(
                &self.state,
                "bash",
                &args,
                &[],
                None,
                Some(COMMAND_TIMEOUT_MS),
            )
            .await?;
        if result.exit_code != 0 {
            return Err(format!(
                "failed to capture the region: {}",
                result.stderr.trim()
            ));
        }
        let bytes = self.client.read_file_bytes(&self.state, ZOOM_PATH).await?;
        png_image(&bytes)
    }

    async fn cursor_position(&mut self) -> Result<[u32; 2], String> {
        let result = self
            .client
            .exec_argv(
                &self.state,
                "xdotool",
                &argv(["getmouselocation", "--shell"]),
                &[("DISPLAY", DESKTOP_DISPLAY)],
                None,
                Some(COMMAND_TIMEOUT_MS),
            )
            .await?;
        if result.exit_code != 0 {
            return Err(format!(
                "xdotool getmouselocation failed: {}",
                result.stderr.trim()
            ));
        }
        parse_mouse_location(&result.stdout)
    }

    async fn release(self: Box<Self>) {
        // The sandbox stays up for the next call; its E2B timeout and the
        // session lease (refreshed on acquire) bound its lifetime.
    }
}

// ============================================================================
// Capability
// ============================================================================

const DESKTOP_PROMPT_NOTE: &str = " The display is a Linux desktop, not a browser page: \
there is no navigate action, so open applications from the desktop and type addresses into them.";

/// Provider-neutral computer use on an E2B desktop sandbox.
pub struct E2BDesktopComputerUseCapability;

impl Capability for E2BDesktopComputerUseCapability {
    fn id(&self) -> &str {
        DESKTOP_COMPUTER_USE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Desktop Computer Use"
    }

    fn description(&self) -> &str {
        "Let the agent see and operate a full Linux desktop in an E2B sandbox the way a person \
         does: it takes screenshots and clicks, types, scrolls, and presses keys at screen \
         coordinates, in any desktop app. Works with any model that accepts images. The sandbox \
         has E2B's network access, not the session's network access list. Cannot be combined with \
         Computer Use: both provide the `computer` tool."
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
            Arc::new(E2BDesktopComputerBackend),
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

    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn PreToolUseHook>> {
        self.pre_tool_use_hooks_with_config(&Value::Null)
    }

    fn pre_tool_use_hooks_with_config(&self, _config: &Value) -> Vec<Arc<dyn PreToolUseHook>> {
        // Soft approval only, like the Browserless backend (EVE-1133).
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
        vec![LEASED_RESOURCES_FEATURE]
    }
}

static DESKTOP_SYSTEM_PROMPT: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| format!("{COMPUTER_USE_SYSTEM_PROMPT}{DESKTOP_PROMPT_NOTE}"));

#[cfg(test)]
#[path = "computer_tests.rs"]
mod tests;
