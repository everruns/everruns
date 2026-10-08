//! Anthropic's native browser toolset (EVE-1133, Claude browser toolset
//! parity).
//!
//! When the call asks for native browser use
//! ([`everruns_contracts::native_computer::NativeBrowserUse`]) and the model
//! takes `browser_toolset_20260801`, the `browser` function tool is replaced
//! by the toolset entry. Claude then calls members (`navigate`, `read_page`,
//! `left_click`, ...) as `tool_use` blocks named after the member and marked
//! `"toolset_name": "browser"`. Each becomes a call of the provider-neutral
//! `browser` tool with the member as its `action`, so the agent loop executes,
//! gates and budgets it like any other `browser` call.
//!
//! Replay goes the other way. Every `tool_result` for a member call echoes
//! `toolset_name`, and a successful one is rebuilt from the `browser` tool's
//! JSON into the content the toolset requires: a `text` block (none for tab
//! members, whose text the API renders itself), the screenshot `image` blocks,
//! and one `browser_state` block from the result's `browser_state`. Error
//! results keep their text and carry no state block.
//!
//! Wire shapes come from the browser use tool reference
//! (platform.claude.com/docs/en/agents-and-tools/tool-use/browser-use-tool,
//! checked 2026-10-08): the entry takes no `name`; 27 of the 31 members are on
//! by default, and the four that are off (`javascript_exec`, `file_upload`,
//! `read_console`, `read_network`) are the four the `browser` tool refuses,
//! so the entry sends no `configs`.
//!
//! Batches work as for the computer toolset: every call converted from one
//! response carries `native_batch: {"id": ...}`, the id of the response's
//! first browser member call, and the tool answers the rest of a batch after a
//! failure with the browser toolset's own skip text. A failed computer call
//! does not skip browser calls, so the two toolsets name their batches apart.

use everruns_contracts::driver_registry::LlmCallConfig;
use everruns_contracts::native_computer::{
    BROWSER_TOOL_NAME, NATIVE_BATCH_KEY, NativeBrowserUse, anthropic_has_browser_toolset,
};
use everruns_contracts::tool_types::ToolCall;
use serde_json::{Map, Value, json};

/// The toolset's `type`.
pub(crate) const TOOLSET_TYPE: &str = "browser_toolset_20260801";

/// `toolset_name` on member `tool_use` and `tool_result` blocks.
const TOOLSET_NAME: &str = BROWSER_TOOL_NAME;

/// Members on by default; each is the `browser` action of the same name.
const MEMBERS: [&str; 27] = [
    "navigate",
    "screenshot",
    "zoom",
    "left_click",
    "right_click",
    "middle_click",
    "double_click",
    "triple_click",
    "hover",
    "left_click_drag",
    "left_mouse_down",
    "left_mouse_up",
    "mouse_move",
    "scroll",
    "scroll_to",
    "type",
    "key",
    "hold_key",
    "wait",
    "read_page",
    "find",
    "get_page_text",
    "form_input",
    "new_tab",
    "list_tabs",
    "switch_tab",
    "close_tab",
];

/// Members whose result is the `browser_state` block alone.
const TAB_MEMBERS: [&str; 4] = ["new_tab", "list_tabs", "switch_tab", "close_tab"];

/// Whether this call uses the toolset.
pub(crate) fn active(config: &LlmCallConfig, wire_model: &str) -> bool {
    anthropic_has_browser_toolset(wire_model) && NativeBrowserUse::requested(config).is_some()
}

/// The `tools` entry, with the API's default members.
pub(crate) fn toolset_entry(cache: bool) -> Value {
    let mut entry = json!({ "type": TOOLSET_TYPE });
    if cache {
        entry["cache_control"] = json!({ "type": "ephemeral" });
    }
    entry
}

/// Whether a raw `content_block_start` block is a toolset member call.
pub(crate) fn is_member_block(block: &Value) -> bool {
    block.get("type").and_then(Value::as_str) == Some("tool_use")
        && block.get("toolset_name").and_then(Value::as_str) == Some(TOOLSET_NAME)
}

/// Turn a finished member call into a call of the `browser` tool. `earlier`
/// holds the response's calls finished before this one; the batch is named
/// after its first browser member call.
pub(crate) fn into_browser_call(call: &mut ToolCall, earlier: &[ToolCall]) {
    let batch = earlier
        .iter()
        .filter(|earlier| earlier.name == BROWSER_TOOL_NAME)
        .find_map(|earlier| earlier.arguments.get(NATIVE_BATCH_KEY).cloned())
        .unwrap_or_else(|| json!({ "id": call.id }));
    let mut arguments = match std::mem::take(&mut call.arguments) {
        Value::Object(arguments) => arguments,
        _ => Map::new(),
    };
    arguments.insert("action".to_string(), Value::from(call.name.clone()));
    arguments.insert(NATIVE_BATCH_KEY.to_string(), batch);
    call.name = BROWSER_TOOL_NAME.to_string();
    call.arguments = Value::Object(arguments);
}

/// Put request messages in the toolset's shape: `browser` calls become member
/// calls, and their results become toolset results.
pub(crate) fn rewrite_messages(messages: &mut [Value]) {
    // Member call id -> member name.
    let mut members: Vec<(String, String)> = Vec::new();
    for message in messages.iter_mut() {
        let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for block in blocks.iter_mut() {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    if is_member_block(block) || as_member_call(block) {
                        let id = block.get("id").and_then(Value::as_str);
                        let name = block.get("name").and_then(Value::as_str);
                        if let (Some(id), Some(name)) = (id, name) {
                            members.push((id.to_string(), name.to_string()));
                        }
                    }
                }
                Some("tool_result") => {
                    let member = block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .and_then(|id| members.iter().find(|(member_id, _)| member_id == id))
                        .map(|(_, name)| name.clone());
                    if let Some(member) = member {
                        as_member_result(block, &member);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Rewrite a `browser` function-tool `tool_use` block into a member call.
fn as_member_call(block: &mut Value) -> bool {
    if block.get("name").and_then(Value::as_str) != Some(BROWSER_TOOL_NAME) {
        return false;
    }
    let Some(input) = block.get_mut("input").and_then(Value::as_object_mut) else {
        return false;
    };
    input.remove(NATIVE_BATCH_KEY);
    let Some(member) = input
        .get("action")
        .and_then(Value::as_str)
        .filter(|action| MEMBERS.contains(action))
        .map(str::to_string)
    else {
        return false;
    };
    input.remove("action");
    block["name"] = Value::from(member);
    block["toolset_name"] = Value::from(TOOLSET_NAME);
    true
}

/// Rebuild a member call's `tool_result` in the toolset's shape.
fn as_member_result(block: &mut Value, member: &str) {
    block["toolset_name"] = Value::from(TOOLSET_NAME);
    if block.get("is_error").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let (text, images) = match block.get("content") {
        Some(Value::String(text)) => (text.clone(), Vec::new()),
        Some(Value::Array(parts)) => {
            let text = parts
                .iter()
                .find(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .and_then(|part| part.get("text").and_then(Value::as_str))
                .unwrap_or_default()
                .to_string();
            let images: Vec<Value> = parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("image"))
                .cloned()
                .collect();
            (text, images)
        }
        _ => return,
    };
    // Only a `browser` success carries a state; anything else (a skipped or
    // gated call answered with plain text) stays as it is.
    let Ok(result) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let Some(state) = result.get("browser_state").and_then(Value::as_object) else {
        return;
    };
    let mut content = Vec::new();
    if !TAB_MEMBERS.contains(&member) {
        let text = result
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .unwrap_or("Done.");
        content.push(json!({ "type": "text", "text": text }));
    }
    content.extend(images);
    let mut state_block = Map::new();
    state_block.insert("type".to_string(), Value::from("browser_state"));
    state_block.insert(
        "tabs".to_string(),
        state.get("tabs").cloned().unwrap_or_else(|| json!([])),
    );
    if let Some(changes) = state
        .get("state_changes")
        .and_then(Value::as_array)
        .filter(|changes| !changes.is_empty())
    {
        state_block.insert("state_changes".to_string(), Value::Array(changes.clone()));
    }
    content.push(Value::Object(state_block));
    block["content"] = Value::Array(content);
}

#[cfg(test)]
#[path = "browser_toolset_tests.rs"]
mod tests;
