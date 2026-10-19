//! Tool changes sent inside the conversation, so `tools` stays the epoch's
//! baseline and the cached prefix survives a reveal.
//!
//! Each recorded change becomes an in-place system message before the input it
//! took effect at: `tool_addition` blocks carrying the full definition (a
//! same-name definition replaces the declared one, which is how a deferred stub
//! becomes its schema) and a text line for removals and notes. Removal is soft:
//! the tool stays declared and a call to it is refused by the runtime.
//!
//! By-value additions are a Claude API beta, so this runs only against the
//! first-party endpoint, on models whose profile says a live round trip kept
//! the cache. Anything this module cannot express (hosted tool search, native
//! toolsets, a change with no user turn before it) falls back to sending the
//! tools wanted now, which is the behavior before recorded changes existed.

// Design of record: knowledge/execution/cache-stable-tool-changes.md.
use everruns_contracts::driver_registry::LlmCallConfig;
use everruns_contracts::model::INLINE_TOOL_CHANGES_PARAMETER;
use everruns_contracts::tool_changes::ToolChanges;
use everruns_contracts::tool_types::ToolDefinition;
use serde_json::{Value, json};

use super::{
    AnthropicChatDriver, AnthropicContentBlock, AnthropicMessage, AnthropicTool, AnthropicToolEntry,
};

pub(super) const INLINE_TOOLS_BETA: &str = "inline-tools-2026-09-15";

/// Declared from the first request of an epoch so the scaffolding the API adds
/// for tool changes is in the cached prefix before the first change. Deferred,
/// so the model never sees it.
const PLACEHOLDER: &str = "__everruns_deferred_placeholder__";

/// Whether the request can carry the API's inline tool changes at all:
/// first-party endpoint, a verified model, and none of the tool shapes that
/// rewrite `tools` per request.
pub(super) fn eligible(config: &LlmCallConfig, model: &str, first_party: bool) -> bool {
    let reshapes_tools = config.tool_search.as_ref().is_some_and(|ts| ts.enabled)
        || crate::anthropic::computer_toolset::active(config, model)
        || crate::anthropic::browser_toolset::active(config, model);
    first_party
        && !reshapes_tools
        && config.tool_baseline.as_ref().is_some_and(|b| !b.is_empty())
        && everruns_contracts::get_model_profile(&everruns_contracts::DriverId::Anthropic, model)
            .is_some_and(|profile| profile.supports_parameter(INLINE_TOOL_CHANGES_PARAMETER))
}

/// The in-place system message content for one recorded change.
pub(super) fn blocks(changes: &ToolChanges) -> Vec<AnthropicContentBlock> {
    let mut lines: Vec<String> = changes.note.iter().cloned().collect();
    if !changes.removed.is_empty() {
        lines.push(format!(
            "These tools are no longer available; do not call them: {}.",
            changes.removed.join(", ")
        ));
    }
    let text = (!lines.is_empty()).then(|| AnthropicContentBlock::Text {
        text: lines.join("\n\n"),
        cache_control: None,
    });
    let additions = changes.added.iter().chain(&changes.redefined).map(|tool| {
        AnthropicContentBlock::Raw(json!({
            "type": "tool_addition",
            "tool": {"type": "tool_definition", "definition": definition(tool)},
        }))
    });
    text.into_iter().chain(additions).collect()
}

fn definition(tool: &ToolDefinition) -> Value {
    json!({
        "name": tool.name(),
        "description": tool.description(),
        "input_schema": AnthropicChatDriver::sanitize_input_schema(tool.parameters().clone()),
    })
}

/// The request's `tools` in inline mode: the epoch's baseline, then the
/// placeholder.
pub(super) fn tools(config: &LlmCallConfig, prompt_cache_enabled: bool) -> Vec<AnthropicToolEntry> {
    let baseline = config.tool_baseline.as_deref().unwrap_or_default();
    let mut entries = AnthropicChatDriver::convert_tools(baseline, prompt_cache_enabled);
    entries.push(AnthropicToolEntry::Function(AnthropicTool {
        name: PLACEHOLDER.to_string(),
        description: "Reserved placeholder. Never available. Never call this.".to_string(),
        input_schema: json!({"type": "object", "properties": {}}),
        cache_control: None,
        defer_loading: Some(true),
    }));
    entries
}

/// Whether all `expected` tool definitions reached a system message. A change
/// whose message had no user turn before it was folded into `system`, which
/// cannot hold a definition. A folded removal or note only costs the cache.
pub(super) fn all_placed(messages: &[AnthropicMessage], expected: usize) -> bool {
    let placed = messages
        .iter()
        .filter(|message| message.role == "system")
        .flat_map(|message| &message.content)
        .filter(|block| matches!(block, AnthropicContentBlock::Raw(_)))
        .count();
    placed == expected
}

#[cfg(test)]
mod tests {
    use super::super::layout;
    use super::*;
    use everruns_contracts::driver_registry::{Message, MessageRole, ToolSearchConfig};

    const MODEL: &str = "claude-sonnet-5-5";

    fn tool(name: &str, required: bool) -> ToolDefinition {
        let mut schema = json!({"type": "object", "properties": {"env": {"type": "string"}}});
        if required {
            schema["required"] = json!(["env"]);
        }
        ToolDefinition::function(name, "Deploy status", schema)
    }

    fn with_changes(mut message: Message, changes: ToolChanges) -> Message {
        message.tool_changes = Some(changes);
        message
    }

    fn reveal() -> ToolChanges {
        ToolChanges {
            redefined: vec![tool("deploy_status", true)],
            ..ToolChanges::default()
        }
    }

    /// `(system, messages)` as the request would carry them, and whether every
    /// tool definition found a place.
    fn layout_for(messages: &[Message]) -> (Option<String>, Value, bool) {
        let prepared = layout::keep_later_system_messages_in_place(messages, MODEL, true);
        let (system, converted) = AnthropicChatDriver::convert_messages_with_options(
            &prepared.messages,
            Some(&prepared),
            false,
            0,
            false,
        );
        let placed = all_placed(&converted, prepared.tool_definition_count());
        (system, serde_json::to_value(converted).unwrap(), placed)
    }

    fn config(baseline: Vec<ToolDefinition>) -> LlmCallConfig {
        let mut config = LlmCallConfig::new(MODEL);
        config.tool_baseline = Some(baseline);
        config
    }

    #[test]
    fn a_change_goes_out_before_the_next_answer_and_history_stays_append_only() {
        use MessageRole::*;
        let mut call = Message::text(Assistant, "");
        call.tool_calls = Some(vec![everruns_contracts::tool_types::ToolCall {
            id: "call_1".into(),
            name: "tool_search".into(),
            arguments: json!({"query": "deploy"}),
        }]);
        let mut result = Message::text(Tool, "found deploy_status");
        result.tool_call_id = Some("call_1".into());
        let turn_one = vec![
            Message::text(System, "agent prompt"),
            Message::text(User, "is prod deployed?"),
            call,
            with_changes(result, reveal()),
        ];
        let (system, one, placed) = layout_for(&turn_one);
        assert!(placed);
        assert_eq!(system.as_deref(), Some("agent prompt"));
        assert_eq!(
            one[3],
            json!({"role": "system", "content": [{
                "type": "tool_addition",
                "tool": {"type": "tool_definition", "definition": {
                    "name": "deploy_status",
                    "description": "Deploy status",
                    "input_schema": tool("deploy_status", true).parameters(),
                }},
            }]})
        );

        let mut turn_two = turn_one.clone();
        turn_two.extend([
            Message::text(Assistant, "prod is deployed"),
            Message::text(User, "thanks"),
        ]);
        let (system_two, two, _) = layout_for(&turn_two);
        assert_eq!(system, system_two);
        let one = one.as_array().unwrap();
        assert_eq!(&two.as_array().unwrap()[..one.len()], one.as_slice());
    }

    #[test]
    fn a_change_takes_the_turn_reminders_into_its_message() {
        use MessageRole::*;
        let mut facts = Message::text(System, "<facts>10:00</facts>");
        facts.mark_turn_scoped_system();
        let messages = [
            Message::text(System, "agent prompt"),
            with_changes(Message::text(User, "deploy"), reveal()),
            facts,
        ];
        let (system, converted, placed) = layout_for(&messages);
        assert!(placed);
        assert_eq!(system.as_deref(), Some("agent prompt"));
        let reminder = &converted[1];
        assert_eq!(reminder["role"], "system");
        assert!(reminder.get("clear_at").is_none(), "{reminder}");
        assert_eq!(reminder["content"][0]["type"], "tool_addition");
        assert_eq!(reminder["content"][1]["text"], "<facts>10:00</facts>");
    }

    #[test]
    fn a_change_with_no_user_turn_before_it_is_reported_unplaced() {
        use MessageRole::*;
        let messages = [
            Message::text(System, "agent prompt"),
            Message::text(User, "go on"),
            Message::text(Assistant, "first"),
            with_changes(Message::text(Assistant, "second"), reveal()),
        ];
        let (_, _, placed) = layout_for(&messages);
        assert!(!placed);
    }

    #[test]
    fn removals_and_notes_are_one_text_line_ahead_of_the_additions() {
        let changes = ToolChanges {
            added: vec![tool("deploy_status", false)],
            removed: vec!["old_a".into(), "old_b".into()],
            note: Some("Deploy tools are connected.".into()),
            ..ToolChanges::default()
        };
        let blocks = serde_json::to_value(blocks(&changes)).unwrap();
        assert_eq!(
            blocks[0]["text"],
            "Deploy tools are connected.\n\nThese tools are no longer available; do not call them: old_a, old_b."
        );
        assert_eq!(blocks[1]["type"], "tool_addition");
        assert_eq!(blocks.as_array().unwrap().len(), 2);
    }

    #[test]
    fn tools_are_the_baseline_then_a_deferred_placeholder() {
        let entries = serde_json::to_value(tools(
            &config(vec![tool("a", false), tool("b", false)]),
            true,
        ))
        .unwrap();
        let names: Vec<_> = entries
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["a", "b", PLACEHOLDER]);
        assert!(
            entries[1]["cache_control"].is_object(),
            "breakpoint on the baseline"
        );
        assert_eq!(entries[2]["defer_loading"], true);
    }

    #[test]
    fn only_verified_first_party_requests_with_a_baseline_go_inline() {
        let baseline = || vec![tool("a", false)];
        assert!(eligible(&config(baseline()), MODEL, true));
        assert!(eligible(&config(baseline()), "claude-opus-5-5", true));
        assert!(!eligible(&config(baseline()), MODEL, false), "a gateway");
        assert!(
            !eligible(&config(baseline()), "claude-sonnet-5", true),
            "unverified"
        );
        assert!(
            !eligible(&config(Vec::new()), MODEL, true),
            "empty baseline"
        );
        assert!(
            !eligible(&LlmCallConfig::new(MODEL), MODEL, true),
            "no baseline"
        );
        let mut searching = config(baseline());
        searching.tool_search = Some(ToolSearchConfig {
            enabled: true,
            ..Default::default()
        });
        assert!(!eligible(&searching, MODEL, true), "hosted search");
    }
}
