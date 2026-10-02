//! OpenAI Responses `computer` tool wire mapping (EVE-1133).
//!
//! Pure JSON translation between OpenAI's computer items and the
//! provider-neutral `computer` tool's arguments
//! (`everruns_core::computer_use::ComputerAction`). The Responses driver uses
//! it to surface a `computer_call` as a call of the `computer` tool and to
//! replay the transcript in OpenAI's shape. See [`crate::native_computer`].
//!
//! Wire shapes (developers.openai.com/api/docs/guides/tools-computer-use,
//! checked 2026-10-01):
//!
//! - request tool: `{"type": "computer"}`, no display size;
//! - output item: `{"type": "computer_call", "call_id", "actions": [..],
//!   "status"}`; the GA tool batches several actions in one call;
//! - answer: `{"type": "computer_call_output", "call_id", "output":
//!   {"type": "computer_screenshot", "image_url", "detail": "original"}}`.
//!
//! UNCONFIRMED wire details, kept in this module on purpose so a correction is
//! a one-file change:
//! - action fields other than `click` (`button`, `x`, `y`) and `type`
//!   (`text`) are taken from the `computer-use-preview` reference: `double_click`
//!   / `move` (`x`, `y`), `drag` (`path: [{x, y}]`), `scroll` (`x`, `y`,
//!   `scroll_x`, `scroll_y`), `keypress` (`keys`), `wait`, `screenshot`;
//! - a single `action` object (the preview shape) is accepted as a batch of one;
//! - `pending_safety_checks` (preview) are carried into the call's arguments so
//!   the approval gate asks a person, and acknowledged on replay only for a call
//!   that ran;
//! - `computer_call_output` has no error field, so a call that did not run is
//!   answered with the last screenshot of the session (or a blank frame) and the
//!   error goes in a user message right after it.

use serde_json::{Value, json};

/// Pixels per neutral scroll click; matches the browser backend's wheel step.
const SCROLL_STEP_PX: i64 = 100;

/// Most scroll clicks one neutral `scroll` may carry (`MAX_SCROLL_AMOUNT`).
const MAX_SCROLL_CLICKS: i64 = 50;

/// Seconds a bare OpenAI `wait` pauses; the action carries no duration.
const DEFAULT_WAIT_SECONDS: f64 = 2.0;

/// A 1x1 transparent PNG, the frame a `computer_call_output` carries when the
/// session has no screenshot yet.
pub const BLANK_SCREENSHOT: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";

/// Argument key carrying OpenAI's pending safety checks into the call.
pub const PENDING_SAFETY_CHECKS_KEY: &str = "pending_safety_checks";

/// The `request tools` entry.
pub fn wire_tool() -> Value {
    json!({ "type": "computer" })
}

/// `(call_id, arguments)` of the `computer` tool call a finished
/// `computer_call` output item stands for; `None` for any other item, or a
/// call still in progress.
pub fn computer_call_arguments(item: &Value) -> Option<(String, Value)> {
    if item.get("type").and_then(Value::as_str) != Some("computer_call") {
        return None;
    }
    if item
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status != "completed")
    {
        return None;
    }
    let call_id = ["call_id", "id"]
        .iter()
        .find_map(|key| item.get(*key).and_then(Value::as_str))
        .filter(|id| !id.is_empty())?;
    let actions: Vec<Value> = match (item.get("actions"), item.get("action")) {
        (Some(Value::Array(actions)), _) => actions.iter().map(neutral_action).collect(),
        (_, Some(action)) => vec![neutral_action(action)],
        _ => Vec::new(),
    };
    let mut arguments = json!({ "actions": actions });
    if let Some(checks) = item
        .get(PENDING_SAFETY_CHECKS_KEY)
        .and_then(Value::as_array)
        .filter(|checks| !checks.is_empty())
    {
        arguments[PENDING_SAFETY_CHECKS_KEY] = Value::Array(checks.clone());
    }
    Some((call_id.to_string(), arguments))
}

fn point(action: &Value) -> Option<Value> {
    let coordinate = |key: &str| {
        action
            .get(key)
            .and_then(Value::as_f64)
            .map(|v| v.max(0.0).round() as u64)
    };
    Some(json!([coordinate("x")?, coordinate("y")?]))
}

fn unsupported(action: &Value) -> Value {
    // Not a neutral action: the tool rejects it with a readable error, and the
    // original survives for replay.
    json!({ "action": "unsupported", "openai_action": action })
}

/// One OpenAI action as a neutral `computer` action.
pub fn neutral_action(action: &Value) -> Value {
    let kind = action.get("type").and_then(Value::as_str).unwrap_or("");
    let at = point(action);
    let modifiers = action
        .get("keys")
        .and_then(Value::as_array)
        .map(|keys| {
            keys.iter()
                .filter_map(Value::as_str)
                .map(neutral_key)
                .collect::<Vec<_>>()
                .join("+")
        })
        .filter(|keys| !keys.is_empty());
    let with_point = |name: &str| {
        let mut neutral = json!({ "action": name });
        if let Some(at) = &at {
            neutral["coordinate"] = at.clone();
        }
        if let Some(modifiers) = &modifiers {
            neutral["text"] = Value::from(modifiers.clone());
        }
        neutral
    };
    match kind {
        "screenshot" => json!({ "action": "screenshot" }),
        "click" => match action
            .get("button")
            .and_then(Value::as_str)
            .unwrap_or("left")
        {
            "left" => with_point("left_click"),
            "right" => with_point("right_click"),
            "wheel" | "middle" => with_point("middle_click"),
            "back" => json!({ "action": "key", "text": "alt+Left" }),
            "forward" => json!({ "action": "key", "text": "alt+Right" }),
            _ => unsupported(action),
        },
        "double_click" => with_point("double_click"),
        "triple_click" => with_point("triple_click"),
        "move" => match at {
            Some(at) => json!({ "action": "mouse_move", "coordinate": at }),
            None => unsupported(action),
        },
        "drag" => {
            let path: Vec<Value> = action
                .get("path")
                .and_then(Value::as_array)
                .map(|path| path.iter().filter_map(point).collect())
                .unwrap_or_default();
            match (path.first(), path.last()) {
                (Some(start), Some(end)) if path.len() >= 2 => json!({
                    "action": "left_click_drag", "start_coordinate": start, "coordinate": end
                }),
                _ => unsupported(action),
            }
        }
        "scroll" => {
            let delta = |key: &str| action.get(key).and_then(Value::as_f64).unwrap_or(0.0);
            let (dx, dy) = (delta("scroll_x"), delta("scroll_y"));
            let (direction, pixels) = if dy.abs() >= dx.abs() {
                (if dy < 0.0 { "up" } else { "down" }, dy.abs())
            } else {
                (if dx < 0.0 { "left" } else { "right" }, dx.abs())
            };
            let clicks =
                ((pixels / SCROLL_STEP_PX as f64).ceil() as i64).clamp(1, MAX_SCROLL_CLICKS);
            let mut neutral = json!({
                "action": "scroll", "scroll_direction": direction, "scroll_amount": clicks
            });
            if let Some(at) = at {
                neutral["coordinate"] = at;
            }
            neutral
        }
        "keypress" => match modifiers {
            Some(combo) => json!({ "action": "key", "text": combo }),
            None => unsupported(action),
        },
        "type" => json!({
            "action": "type",
            "text": action.get("text").and_then(Value::as_str).unwrap_or_default()
        }),
        "wait" => {
            let seconds = action
                .get("ms")
                .and_then(Value::as_f64)
                .map(|ms| ms / 1000.0)
                .unwrap_or(DEFAULT_WAIT_SECONDS);
            json!({ "action": "wait", "duration": seconds })
        }
        _ => unsupported(action),
    }
}

/// OpenAI key names (`CTRL`, `ENTER`, `ARROWLEFT`, `A`) in the neutral
/// xdotool-style spelling (`ctrl`, `Return`, `Left`, `a`).
fn neutral_key(key: &str) -> String {
    match key.to_ascii_uppercase().as_str() {
        "CTRL" | "CONTROL" => "ctrl".into(),
        "SHIFT" => "shift".into(),
        "ALT" | "OPTION" => "alt".into(),
        "META" | "CMD" | "COMMAND" | "SUPER" | "WIN" => "super".into(),
        "ENTER" | "RETURN" => "Return".into(),
        "ESC" | "ESCAPE" => "Escape".into(),
        "TAB" => "Tab".into(),
        "BACKSPACE" => "BackSpace".into(),
        "DELETE" | "DEL" => "Delete".into(),
        "SPACE" => "space".into(),
        "ARROWUP" | "UP" => "Up".into(),
        "ARROWDOWN" | "DOWN" => "Down".into(),
        "ARROWLEFT" | "LEFT" => "Left".into(),
        "ARROWRIGHT" | "RIGHT" => "Right".into(),
        "PAGEUP" => "Page_Up".into(),
        "PAGEDOWN" => "Page_Down".into(),
        "HOME" => "Home".into(),
        "END" => "End".into(),
        _ if key.chars().count() == 1 => key.to_lowercase(),
        _ => key.to_string(),
    }
}

/// The neutral spelling back in OpenAI's (`ctrl` -> `CTRL`, `Return` -> `ENTER`).
fn openai_key(key: &str) -> String {
    match key.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => "CTRL".into(),
        "super" | "meta" | "cmd" | "command" | "win" => "META".into(),
        "return" | "enter" | "kp_enter" => "ENTER".into(),
        "escape" | "esc" => "ESC".into(),
        "backspace" => "BACKSPACE".into(),
        "page_up" | "pageup" => "PAGEUP".into(),
        "page_down" | "pagedown" => "PAGEDOWN".into(),
        "up" => "ARROWUP".into(),
        "down" => "ARROWDOWN".into(),
        "left" => "ARROWLEFT".into(),
        "right" => "ARROWRIGHT".into(),
        other => other.to_ascii_uppercase(),
    }
}

fn openai_keys(combo: &str) -> Value {
    Value::Array(
        combo
            .split('+')
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(|key| Value::from(openai_key(key)))
            .collect(),
    )
}

/// One neutral action back in OpenAI's shape, for replaying a `computer_call`.
pub fn openai_action(neutral: &Value) -> Value {
    if let Some(original) = neutral.get("openai_action") {
        return original.clone();
    }
    let field = |key: &str| neutral.get(key);
    let xy = |key: &str| {
        field(key)
            .and_then(Value::as_array)
            .map(|p| (p.first().cloned(), p.get(1).cloned()))
            .map(|(x, y)| json!({ "x": x.unwrap_or(json!(0)), "y": y.unwrap_or(json!(0)) }))
            .unwrap_or_else(|| json!({ "x": 0, "y": 0 }))
    };
    let with = |mut base: Value, extra: Value| {
        if let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
            base.extend(extra.clone());
        }
        base
    };
    let click = |button: &str| {
        let mut click = with(
            json!({ "type": "click", "button": button }),
            xy("coordinate"),
        );
        if let Some(keys) = field("text").and_then(Value::as_str) {
            click["keys"] = openai_keys(keys);
        }
        click
    };
    match field("action").and_then(Value::as_str).unwrap_or("") {
        "left_click" => click("left"),
        "right_click" => click("right"),
        "middle_click" => click("wheel"),
        "double_click" => with(json!({ "type": "double_click" }), xy("coordinate")),
        "triple_click" => with(json!({ "type": "triple_click" }), xy("coordinate")),
        "mouse_move" => with(json!({ "type": "move" }), xy("coordinate")),
        "left_click_drag" => {
            json!({ "type": "drag", "path": [xy("start_coordinate"), xy("coordinate")] })
        }
        "scroll" => {
            let pixels =
                field("scroll_amount").and_then(Value::as_i64).unwrap_or(1) * SCROLL_STEP_PX;
            let (dx, dy) = match field("scroll_direction").and_then(Value::as_str) {
                Some("up") => (0, -pixels),
                Some("left") => (-pixels, 0),
                Some("right") => (pixels, 0),
                _ => (0, pixels),
            };
            with(
                json!({ "type": "scroll", "scroll_x": dx, "scroll_y": dy }),
                xy("coordinate"),
            )
        }
        "key" => json!({
            "type": "keypress",
            "keys": openai_keys(field("text").and_then(Value::as_str).unwrap_or_default())
        }),
        "type" => json!({ "type": "type", "text": field("text").cloned().unwrap_or(json!("")) }),
        "wait" => json!({ "type": "wait" }),
        _ => json!({ "type": "screenshot" }),
    }
}

/// The `computer_call` input item replaying a call of the `computer` tool.
pub fn replay_call(call_id: &str, arguments: &Value) -> Value {
    let actions: Vec<Value> = match arguments.get("actions").and_then(Value::as_array) {
        Some(actions) => actions.iter().map(openai_action).collect(),
        // A single-action call (made through the function tool before the
        // native tool was on) replays as a batch of one.
        None => vec![openai_action(arguments)],
    };
    let mut item = json!({
        "type": "computer_call",
        "call_id": call_id,
        "actions": actions,
        "status": "completed",
    });
    if let Some(checks) = arguments.get(PENDING_SAFETY_CHECKS_KEY) {
        item[PENDING_SAFETY_CHECKS_KEY] = checks.clone();
    }
    item
}

/// The `computer_call_output` input item answering `call_id` with `image_url`.
/// `acknowledged` are the safety checks the person approved with the call.
pub fn replay_output(call_id: &str, image_url: &str, acknowledged: Option<&Value>) -> Value {
    let mut item = json!({
        "type": "computer_call_output",
        "call_id": call_id,
        "output": {
            "type": "computer_screenshot",
            "image_url": image_url,
            "detail": "original",
        },
    });
    if let Some(checks) =
        acknowledged.filter(|checks| checks.as_array().is_some_and(|c| !c.is_empty()))
    {
        item["acknowledged_safety_checks"] = checks.clone();
    }
    item
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batched_call_maps_every_action() {
        let (call_id, arguments) = computer_call_arguments(&json!({
            "type": "computer_call", "id": "cu_1", "call_id": "call_002", "status": "completed",
            "actions": [
                { "type": "click", "button": "left", "x": 405, "y": 157 },
                { "type": "type", "text": "penguin" },
                { "type": "keypress", "keys": ["ENTER"] },
            ]
        }))
        .unwrap();
        assert_eq!(call_id, "call_002");
        assert_eq!(
            arguments,
            json!({ "actions": [
                { "action": "left_click", "coordinate": [405, 157] },
                { "action": "type", "text": "penguin" },
                { "action": "key", "text": "Return" },
            ]})
        );
    }

    #[test]
    fn preview_single_action_is_a_batch_of_one() {
        let (_, arguments) = computer_call_arguments(&json!({
            "type": "computer_call", "call_id": "c", "action": { "type": "screenshot" }
        }))
        .unwrap();
        assert_eq!(
            arguments,
            json!({ "actions": [{ "action": "screenshot" }] })
        );
    }

    #[test]
    fn other_items_and_unfinished_calls_are_not_calls() {
        assert!(
            computer_call_arguments(&json!({ "type": "function_call", "call_id": "c" })).is_none()
        );
        assert!(
            computer_call_arguments(&json!({
                "type": "computer_call", "call_id": "c", "status": "in_progress", "actions": []
            }))
            .is_none()
        );
        assert!(
            computer_call_arguments(&json!({ "type": "computer_call", "actions": [] })).is_none()
        );
    }

    #[test]
    fn safety_checks_travel_with_the_call() {
        let checks = json!([{ "id": "sc_1", "code": "malicious_instructions", "message": "?" }]);
        let (_, arguments) = computer_call_arguments(&json!({
            "type": "computer_call", "call_id": "c", "actions": [{ "type": "screenshot" }],
            "pending_safety_checks": checks
        }))
        .unwrap();
        assert_eq!(arguments[PENDING_SAFETY_CHECKS_KEY], checks);
        let replayed = replay_call("c", &arguments);
        assert_eq!(replayed[PENDING_SAFETY_CHECKS_KEY], checks);
        let output = replay_output(
            "c",
            BLANK_SCREENSHOT,
            arguments.get(PENDING_SAFETY_CHECKS_KEY),
        );
        assert_eq!(output["acknowledged_safety_checks"], checks);
        assert!(
            replay_output("c", BLANK_SCREENSHOT, None)
                .get("acknowledged_safety_checks")
                .is_none()
        );
    }

    #[test]
    fn pointer_scroll_and_key_actions_map_both_ways() {
        let cases = [
            (
                json!({ "type": "click", "button": "right", "x": 1, "y": 2 }),
                json!({ "action": "right_click", "coordinate": [1, 2] }),
            ),
            (
                json!({ "type": "click", "button": "wheel", "x": 1, "y": 2 }),
                json!({ "action": "middle_click", "coordinate": [1, 2] }),
            ),
            (
                json!({ "type": "double_click", "x": 3, "y": 4 }),
                json!({ "action": "double_click", "coordinate": [3, 4] }),
            ),
            (
                json!({ "type": "move", "x": 5, "y": 6 }),
                json!({ "action": "mouse_move", "coordinate": [5, 6] }),
            ),
            (
                json!({ "type": "drag", "path": [{ "x": 1, "y": 1 }, { "x": 9, "y": 9 }] }),
                json!({ "action": "left_click_drag", "start_coordinate": [1, 1], "coordinate": [9, 9] }),
            ),
            (
                json!({ "type": "scroll", "x": 10, "y": 20, "scroll_x": 0, "scroll_y": 300 }),
                json!({ "action": "scroll", "scroll_direction": "down", "scroll_amount": 3, "coordinate": [10, 20] }),
            ),
            (
                json!({ "type": "keypress", "keys": ["CTRL", "A"] }),
                json!({ "action": "key", "text": "ctrl+a" }),
            ),
        ];
        for (openai, neutral) in cases {
            assert_eq!(neutral_action(&openai), neutral, "{openai}");
            assert_eq!(openai_action(&neutral)["type"], openai["type"], "{neutral}");
        }
        assert_eq!(
            openai_action(&json!({ "action": "key", "text": "ctrl+a" }))["keys"],
            json!(["CTRL", "A"])
        );
        assert_eq!(
            openai_action(
                &json!({ "action": "scroll", "scroll_direction": "up", "scroll_amount": 2, "coordinate": [1, 1] })
            ),
            json!({ "type": "scroll", "scroll_x": 0, "scroll_y": -200, "x": 1, "y": 1 })
        );
    }

    #[test]
    fn unknown_actions_are_rejected_by_the_tool_and_replay_verbatim() {
        let original = json!({ "type": "teleport", "x": 1 });
        let neutral = neutral_action(&original);
        assert_eq!(neutral["action"], "unsupported");
        assert_eq!(openai_action(&neutral), original);
        // A move without coordinates is not a move.
        assert_eq!(
            neutral_action(&json!({ "type": "move" }))["action"],
            "unsupported"
        );
    }

    #[test]
    fn replay_renders_call_and_screenshot_output() {
        let call = replay_call(
            "call_1",
            &json!({ "actions": [{ "action": "type", "text": "hi" }] }),
        );
        assert_eq!(
            call,
            json!({ "type": "computer_call", "call_id": "call_1", "status": "completed",
                    "actions": [{ "type": "type", "text": "hi" }] })
        );
        // A single-action function-tool call replays as a batch of one.
        let single = replay_call("call_2", &json!({ "action": "screenshot" }));
        assert_eq!(single["actions"], json!([{ "type": "screenshot" }]));
        assert_eq!(
            replay_output("call_1", "data:image/png;base64,AAA", None),
            json!({ "type": "computer_call_output", "call_id": "call_1",
                    "output": { "type": "computer_screenshot",
                                "image_url": "data:image/png;base64,AAA", "detail": "original" } })
        );
    }
}
