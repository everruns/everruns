//! Anthropic's native computer toolset (EVE-1133, computer use phase 2).
//!
//! When the call asks for native computer use
//! ([`everruns_contracts::native_computer`]) and the model takes
//! `computer_toolset_20260801`, the `computer` function tool is replaced by the
//! toolset entry. Claude then calls toolset members (`left_click`, `type`, ...)
//! as `tool_use` blocks named after the member and marked
//! `"toolset_name": "computer"`. Each becomes a call of the provider-neutral
//! `computer` tool with the member as its `action`, so the agent loop executes,
//! gates and budgets it like any other `computer` call. Replay goes the other
//! way, and every `tool_result` for a member call echoes `toolset_name`, which
//! the API requires.
//!
//! Wire shapes come from the computer use tool reference
//! (platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool,
//! checked 2026-10-01): the entry takes no `name` and no display size, members
//! take the same argument names as the neutral vocabulary (`coordinate`,
//! `text`, `scroll_direction`, ...), and the optional `configs` map turns
//! members off. All seventeen members are on by default.
//!
//! Every member maps to a neutral action, so the entry turns none off.
//!
//! Claude sends a batch as several member calls in one response and asks
//! clients to stop at the first failure, answering every later call with
//! [`NATIVE_BATCH_SKIPPED`](everruns_contracts::native_computer::NATIVE_BATCH_SKIPPED).
//! The calls run one at a time as separate `computer` calls, so each carries
//! `native_batch: {"id": ...}` (the id of the response's first member call)
//! and the tool skips the rest of a batch after a failure. Replay strips it.

use std::collections::HashSet;

use everruns_contracts::driver_registry::LlmCallConfig;
use everruns_contracts::native_computer::{
    COMPUTER_TOOL_NAME, NATIVE_BATCH_KEY, NativeComputerUse, anthropic_has_computer_toolset,
};
use everruns_contracts::tool_types::ToolCall;
use serde_json::{Map, Value, json};

/// The toolset's `type`.
pub(crate) const TOOLSET_TYPE: &str = "computer_toolset_20260801";

/// `toolset_name` on member `tool_use` and `tool_result` blocks.
const TOOLSET_NAME: &str = COMPUTER_TOOL_NAME;

/// Toolset members; each is the neutral `computer` action of the same name.
const MEMBERS: [&str; 17] = [
    "screenshot",
    "left_click",
    "right_click",
    "middle_click",
    "double_click",
    "triple_click",
    "left_click_drag",
    "left_mouse_down",
    "left_mouse_up",
    "mouse_move",
    "cursor_position",
    "scroll",
    "type",
    "key",
    "hold_key",
    "wait",
    "zoom",
];

/// Whether this call uses the toolset.
pub(crate) fn active(config: &LlmCallConfig, wire_model: &str) -> bool {
    anthropic_has_computer_toolset(wire_model) && NativeComputerUse::requested(config).is_some()
}

/// The `tools` entry, with every member on (the API default).
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

/// Turn a finished member call into a call of the `computer` tool. `earlier`
/// holds the response's calls finished before this one; the batch is named
/// after its first computer member call.
pub(crate) fn into_computer_call(call: &mut ToolCall, earlier: &[ToolCall]) {
    let batch = earlier
        .iter()
        .filter(|earlier| earlier.name == COMPUTER_TOOL_NAME)
        .find_map(|earlier| earlier.arguments.get(NATIVE_BATCH_KEY).cloned())
        .unwrap_or_else(|| json!({ "id": call.id }));
    let mut arguments = match std::mem::take(&mut call.arguments) {
        Value::Object(arguments) => arguments,
        _ => Map::new(),
    };
    arguments.insert("action".to_string(), Value::from(call.name.clone()));
    arguments.insert(NATIVE_BATCH_KEY.to_string(), batch);
    call.name = COMPUTER_TOOL_NAME.to_string();
    call.arguments = Value::Object(arguments);
}

/// Put request messages in the toolset's shape: `computer` calls become member
/// calls, and every result of a member call echoes `toolset_name`. Blocks
/// replayed raw from an earlier response are already member calls.
pub(crate) fn rewrite_messages(messages: &mut [Value]) {
    let mut member_ids: HashSet<String> = HashSet::new();
    for message in messages.iter_mut() {
        let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for block in blocks.iter_mut() {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    let id = block.get("id").and_then(Value::as_str).map(str::to_string);
                    if is_member_block(block) || as_member_call(block) {
                        member_ids.extend(id);
                    }
                }
                Some("tool_result")
                    if block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| member_ids.contains(id)) =>
                {
                    block["toolset_name"] = Value::from(TOOLSET_NAME);
                }
                _ => {}
            }
        }
    }
}

/// Rewrite a `computer` function-tool `tool_use` block into a member call.
/// Actions with no member (`navigate`, a batch) stay as they are.
fn as_member_call(block: &mut Value) -> bool {
    if block.get("name").and_then(Value::as_str) != Some(COMPUTER_TOOL_NAME) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::ToolDefinition;

    #[test]
    fn active_needs_the_option_the_tool_and_a_toolset_model() {
        let mut config = LlmCallConfig::new("claude-opus-5-5");
        config.tools = vec![ToolDefinition::function("computer", "", json!({}))];
        assert!(!active(&config, "claude-opus-5-5"));
        let (key, value) = NativeComputerUse {
            display_width: 1280,
            display_height: 800,
        }
        .to_driver_option();
        config.driver_options.insert(key, value);
        assert!(active(&config, "claude-opus-5-5"));
        assert!(!active(&config, "claude-haiku-4-5"));
    }

    #[test]
    fn entry_keeps_every_member_on() {
        let entry = toolset_entry(false);
        assert_eq!(entry, json!({ "type": TOOLSET_TYPE }));
        assert_eq!(toolset_entry(true)["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn member_call_becomes_a_computer_call() {
        let mut call = ToolCall {
            id: "toolu_1".into(),
            name: "left_click".into(),
            arguments: json!({ "coordinate": [10, 20] }),
        };
        into_computer_call(&mut call, &[]);
        assert_eq!(call.name, "computer");
        assert_eq!(
            call.arguments,
            json!({ "action": "left_click", "coordinate": [10, 20],
                    "native_batch": { "id": "toolu_1" } })
        );

        let mut bare = ToolCall {
            id: "toolu_2".into(),
            name: "screenshot".into(),
            arguments: json!(""),
        };
        let other = ToolCall {
            id: "toolu_0".into(),
            name: "web_fetch".into(),
            arguments: json!({}),
        };
        into_computer_call(&mut bare, &[other, call]);
        // Same batch as the response's first member call.
        assert_eq!(
            bare.arguments,
            json!({ "action": "screenshot", "native_batch": { "id": "toolu_1" } })
        );
    }

    #[test]
    fn replay_rewrites_calls_and_tags_their_results() {
        let mut messages = vec![
            json!({ "role": "assistant", "content": [
                { "type": "tool_use", "id": "t1", "name": "computer",
                  "input": { "action": "type", "text": "Ada", "native_batch": { "id": "t1" } } },
                { "type": "tool_use", "id": "t2", "name": "left_click", "toolset_name": "computer",
                  "input": { "coordinate": [1, 2] } },
                { "type": "tool_use", "id": "t3", "name": "web_fetch", "input": {} },
                { "type": "tool_use", "id": "t4", "name": "computer",
                  "input": { "action": "navigate", "url": "https://example.com" } },
            ]}),
            json!({ "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "t1", "content": "ok" },
                { "type": "tool_result", "tool_use_id": "t2", "content": "ok" },
                { "type": "tool_result", "tool_use_id": "t3", "content": "ok" },
                { "type": "tool_result", "tool_use_id": "t4", "content": "ok" },
            ]}),
        ];
        rewrite_messages(&mut messages);
        let calls = messages[0]["content"].as_array().unwrap();
        assert_eq!(
            calls[0],
            json!({ "type": "tool_use", "id": "t1", "name": "type", "toolset_name": "computer",
                    "input": { "text": "Ada" } })
        );
        assert_eq!(calls[1]["name"], "left_click");
        assert!(calls[2].get("toolset_name").is_none());
        // No member for `navigate`: left as a plain call.
        assert_eq!(calls[3]["name"], "computer");
        let results = messages[1]["content"].as_array().unwrap();
        assert_eq!(results[0]["toolset_name"], "computer");
        assert_eq!(results[1]["toolset_name"], "computer");
        assert!(results[2].get("toolset_name").is_none());
        assert!(results[3].get("toolset_name").is_none());
    }
}
