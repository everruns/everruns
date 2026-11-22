//! Gemini's built-in computer use tool on the Interactions API.
//!
//! When the call asks for native computer use
//! ([`everruns_contracts::native_computer`]) and the model has Gemini's
//! `computer_use` tool, the `computer` function tool is replaced by
//! `{"type": "computer_use", "environment": ...}`. Gemini then answers with
//! ordinary `function_call` steps named after its predefined actions (`click`,
//! `type`, `scroll`, ...). Each becomes a call of the provider-neutral
//! `computer` tool, so the agent loop executes, gates and budgets it like any
//! other `computer` call. Replay goes the other way.
//!
//! Wire shapes come from the Gemini computer use guide
//! (ai.google.dev/gemini-api/docs/computer-use, checked 2026-11-21) and a live
//! probe of `gemini-4-hydrogen`: every action carries `intent`, coordinates are
//! integers on a 0-999 grid whatever the screen size, and a result is a
//! `function_result` holding a JSON text item and the new screenshot.
//!
//! Decision: the environment is `browser` when the `computer` tool offers
//! `navigate` (a Browserless browser), otherwise `desktop`. Gemini's browser
//! set adds `navigate`, `go_back` and `go_forward`; the history steps become
//! `alt+Left` / `alt+Right`, which every browser honors.
//!
//! Decision: `key_down` and `key_up` have no neutral action (a key held across
//! tool calls), so they are turned off with `excluded_predefined_functions`.
//! `hold_key` covers the cases a model needs.
//!
//! Decision: the converted call keeps Gemini's own call (name and arguments)
//! under `native_batch.call`, so replay sends back exactly what the model
//! produced instead of reconstructing it from pixels. `native_batch.id` is the
//! response's first computer call, as for Anthropic's toolset: a parallel
//! batch stops at its first failure.
//!
//! Decision: a `safety_decision` the model attaches travels as
//! `pending_safety_checks`, as OpenAI's do. The call only runs once its
//! approval passed, so a result that ran is replayed with
//! `safety_acknowledgement: true`.

use std::collections::HashSet;

use everruns_contracts::driver_registry::LlmCallConfig;
use everruns_contracts::native_computer::{
    COMPUTER_TOOL_NAME, NATIVE_BATCH_KEY, NativeComputerUse, gemini_has_native_computer,
};
use everruns_contracts::openai_computer::PENDING_SAFETY_CHECKS_KEY;
use everruns_contracts::tool_types::ToolCall;
use serde_json::{Map, Value, json};

/// Predefined actions Gemini may call. `key_down` and `key_up` are excluded
/// from the tool, so they never arrive.
const ACTIONS: [&str; 18] = [
    "click",
    "double_click",
    "triple_click",
    "middle_click",
    "right_click",
    "mouse_down",
    "mouse_up",
    "move",
    "type",
    "drag_and_drop",
    "wait",
    "press_key",
    "hotkey",
    "take_screenshot",
    "scroll",
    "go_back",
    "go_forward",
    "navigate",
];

/// Turned off on the tool: no neutral action holds a key across calls.
const EXCLUDED: [&str; 2] = ["key_down", "key_up"];

/// Gemini's coordinate grid: 0..=999 on both axes.
const GRID: f64 = 1000.0;

/// Native computer use for one Interactions call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GeminiComputer {
    width: u32,
    height: u32,
    browser: bool,
}

impl GeminiComputer {
    /// The native tool for this call, when the model has it and the call asks
    /// for it.
    pub(crate) fn for_call(config: &LlmCallConfig) -> Option<Self> {
        if !gemini_has_native_computer(&config.model) {
            return None;
        }
        let native = NativeComputerUse::requested(config)?;
        let browser = config
            .tools
            .iter()
            .find(|tool| tool.name() == COMPUTER_TOOL_NAME)
            .and_then(|tool| tool.parameters().pointer("/properties/action/enum"))
            .and_then(Value::as_array)
            .is_some_and(|actions| actions.iter().any(|a| a == "navigate"));
        Some(Self {
            width: native.display_width.max(1),
            height: native.display_height.max(1),
            browser,
        })
    }

    /// The `tools` entry that stands in for the `computer` function.
    pub(crate) fn tool_entry(&self) -> Value {
        json!({
            "type": "computer_use",
            "environment": if self.browser { "browser" } else { "desktop" },
            "excluded_predefined_functions": EXCLUDED,
        })
    }

    /// Whether a model call named `name` is one of Gemini's actions.
    pub(crate) fn is_action(name: &str) -> bool {
        ACTIONS.contains(&name)
    }

    /// Turn a finished Gemini action call into a call of the `computer` tool.
    /// `batch` is the id of the response's first computer call, set on the
    /// first conversion.
    pub(crate) fn into_computer_call(self, call: &mut ToolCall, batch: &mut Option<String>) {
        let batch_id = batch.get_or_insert_with(|| call.id.clone()).clone();
        let original = json!({ "name": call.name, "arguments": call.arguments });
        let mut arguments = match self.neutral(&call.name, &call.arguments) {
            Ok(actions) if actions.len() == 1 => actions.into_iter().next().unwrap_or_default(),
            Ok(actions) => json!({ "actions": actions }),
            // An action we cannot express still becomes a computer call, so
            // the tool reports the problem to the model as a tool error.
            Err(problem) => json!({ "action": format!("{}: {problem}", call.name) }),
        };
        arguments[NATIVE_BATCH_KEY] = json!({ "id": batch_id, "call": original });
        if let Some(decision) = call
            .arguments
            .get("safety_decision")
            .filter(|decision| decision.is_object())
        {
            arguments[PENDING_SAFETY_CHECKS_KEY] = json!([decision]);
        }
        call.name = COMPUTER_TOOL_NAME.to_string();
        call.arguments = arguments;
    }

    /// Neutral actions for one Gemini action, in order.
    fn neutral(&self, name: &str, args: &Value) -> Result<Vec<Value>, String> {
        let point = |x: &str, y: &str| -> Result<[u32; 2], String> {
            let x = number(args, x).ok_or_else(|| format!("missing `{x}`"))?;
            let y = number(args, y).ok_or_else(|| format!("missing `{y}`"))?;
            Ok([scale(x, self.width), scale(y, self.height)])
        };
        let at = || point("x", "y");
        let click = |action: &str| -> Result<Vec<Value>, String> {
            Ok(vec![json!({ "action": action, "coordinate": at()? })])
        };
        let text = |key: &str| -> Result<String, String> {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("missing `{key}`"))
        };
        match name {
            "click" => click("left_click"),
            "double_click" | "triple_click" | "middle_click" | "right_click" => click(name),
            "move" => click("mouse_move"),
            "mouse_down" => Ok(vec![
                json!({ "action": "mouse_move", "coordinate": at()? }),
                json!({ "action": "left_mouse_down" }),
            ]),
            "mouse_up" => Ok(vec![
                json!({ "action": "mouse_move", "coordinate": at()? }),
                json!({ "action": "left_mouse_up" }),
            ]),
            "type" => {
                let mut actions = Vec::new();
                // Gemini may aim the typing at a field.
                if let Ok(coordinate) = at() {
                    actions.push(json!({ "action": "left_click", "coordinate": coordinate }));
                }
                actions.push(json!({ "action": "type", "text": text("text")? }));
                if args.get("press_enter").and_then(Value::as_bool) == Some(true) {
                    actions.push(json!({ "action": "key", "text": "Return" }));
                }
                Ok(actions)
            }
            "drag_and_drop" => Ok(vec![json!({
                "action": "left_click_drag",
                "start_coordinate": point("start_x", "start_y")?,
                "coordinate": point("end_x", "end_y")?,
            })]),
            "wait" => {
                let seconds = number(args, "seconds").unwrap_or(5.0).clamp(0.0, 30.0);
                Ok(vec![json!({ "action": "wait", "duration": seconds })])
            }
            "press_key" => Ok(vec![
                json!({ "action": "key", "text": key_name(&text("key")?) }),
            ]),
            "hotkey" => {
                let keys: Vec<String> = args
                    .get("keys")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(key_name)
                    .collect();
                if keys.is_empty() {
                    return Err("missing `keys`".to_string());
                }
                Ok(vec![json!({ "action": "key", "text": keys.join("+") })])
            }
            "take_screenshot" => Ok(vec![json!({ "action": "screenshot" })]),
            "scroll" => {
                let direction = text("direction")?.to_ascii_lowercase();
                // About 100 pixels per wheel click; Gemini leaves the
                // distance out for an ordinary page scroll.
                let amount = number(args, "magnitude_in_pixels")
                    .map_or(5, |pixels| (pixels / 100.0).round().clamp(1.0, 50.0) as u32);
                let mut action = json!({
                    "action": "scroll",
                    "scroll_direction": direction,
                    "scroll_amount": amount,
                });
                if let Ok(coordinate) = at() {
                    action["coordinate"] = json!(coordinate);
                }
                Ok(vec![action])
            }
            "go_back" => Ok(vec![json!({ "action": "key", "text": "alt+Left" })]),
            "go_forward" => Ok(vec![json!({ "action": "key", "text": "alt+Right" })]),
            "navigate" => Ok(vec![json!({ "action": "navigate", "url": text("url")? })]),
            other => Err(format!("unsupported Gemini action `{other}`")),
        }
    }
}

/// The Gemini call a converted `computer` call came from, if any.
pub(crate) fn original_call(arguments: &Value) -> Option<(&str, &Value)> {
    let call = arguments.pointer("/native_batch/call")?;
    let name = call.get("name")?.as_str()?;
    Some((name, call.get("arguments").unwrap_or(&Value::Null)))
}

/// The `function_result` payload for a converted call: Gemini wants a JSON
/// text item first, then the screenshot. A result that carries a screenshot
/// ran, so any safety decision on its call was approved.
pub(crate) fn result_items(mut items: Vec<Value>, call_arguments: &Value) -> Vec<Value> {
    let ran = items.iter().any(|item| item["type"] == "image");
    let mut text: Vec<Value> = Vec::new();
    let mut rest = Vec::new();
    for item in items.drain(..) {
        if item["type"] == "text" {
            text.push(item);
        } else {
            rest.push(item);
        }
    }
    let joined = text
        .iter()
        .filter_map(|item| item["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut body = match serde_json::from_str::<Value>(&joined) {
        Ok(Value::Object(object)) => object,
        _ => {
            let mut object = Map::new();
            if !joined.is_empty() {
                object.insert("output".into(), Value::from(joined));
            }
            object
        }
    };
    if ran
        && call_arguments
            .pointer("/native_batch/call/arguments/safety_decision")
            .is_some()
    {
        body.insert("safety_acknowledgement".into(), Value::Bool(true));
    }
    let mut out = vec![json!({ "type": "text", "text": Value::Object(body).to_string() })];
    out.extend(rest);
    out
}

/// Function names the request declares, so a user tool that happens
/// to share an action name is never rewritten.
pub(crate) fn declared_functions(tools: &[Value]) -> HashSet<String> {
    tools
        .iter()
        .filter(|tool| tool["type"] == "function")
        .filter_map(|tool| tool["name"].as_str().map(str::to_string))
        .collect()
}

fn number(args: &Value, key: &str) -> Option<f64> {
    let value = args.get(key)?;
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

/// A 0-999 grid position to a pixel on a `size`-pixel axis.
fn scale(value: f64, size: u32) -> u32 {
    let pixel = (value.clamp(0.0, GRID - 1.0) / GRID * f64::from(size)).round() as u32;
    pixel.min(size.saturating_sub(1))
}

/// Gemini key names (`Enter`, `Control`, `ArrowUp`) in the xdotool naming the
/// `computer` tool takes. Single characters and unknown names pass through.
fn key_name(key: &str) -> String {
    let named = match key.trim().to_ascii_lowercase().as_str() {
        "enter" | "return" => "Return",
        "escape" | "esc" => "Escape",
        "backspace" => "BackSpace",
        "tab" => "Tab",
        "delete" | "del" => "Delete",
        "space" | " " => "space",
        "arrowup" | "up" => "Up",
        "arrowdown" | "down" => "Down",
        "arrowleft" | "left" => "Left",
        "arrowright" | "right" => "Right",
        "pageup" | "page_up" => "Page_Up",
        "pagedown" | "page_down" => "Page_Down",
        "home" => "Home",
        "end" => "End",
        "insert" => "Insert",
        "control" | "ctrl" => "ctrl",
        "shift" => "shift",
        "alt" | "option" => "alt",
        "meta" | "command" | "cmd" | "super" | "win" | "windows" => "super",
        _ => return key.trim().to_string(),
    };
    named.to_string()
}

#[cfg(test)]
#[path = "computer_use_tests.rs"]
mod tests;
