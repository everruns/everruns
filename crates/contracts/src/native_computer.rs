//! Native provider computer tools (EVE-1133, computer use phase 2).
//!
//! The provider-neutral `computer` function tool
//! works with any model that reads tool-result images. OpenAI and Anthropic
//! also train their models on a native computer tool: OpenAI's Responses
//! `computer` tool (`computer_call` / `computer_call_output`) and Anthropic's
//! `computer_toolset_20260801`. A model drives the native tool better than a
//! function tool that only describes the same actions.
//!
//! The computer-use capability asks for the native tool by contributing
//! [`NATIVE_COMPUTER_USE_OPTION`] in `LlmCallConfig::driver_options`. A driver
//! that knows a native tool for the model swaps the `computer` function tool's
//! definition for the native one, turns each native call back into a call of
//! the `computer` tool, and replays results in the native shape. Execution never
//! changes: the agent loop still runs every action through the same tool, the
//! same soft-approval policy and the same budget.
//!
//! Decision: unlike [`crate::openai_hosted_tools`], the option is a request, not
//! a requirement. Every other driver ignores it and the function tool keeps
//! working, so an agent does not fail when its model has no native tool.
//!
//! Decision: the native tool is client-executed. It is not a hosted tool and
//! never takes the provider-executed path (`HostedToolCall` events, hosted call
//! pricing, the no-reissue rule), because Everruns runs every action.
//!
//! Decision: which models have a native tool is a model-id rule in this module
//! rather than a `ModelProfile` flag. It is one fact per vendor, it changes
//! with vendor launches, and a profile field would touch every profile literal.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::driver_registry::LlmCallConfig;

/// `driver_options` key carrying [`NativeComputerUse`].
pub const NATIVE_COMPUTER_USE_OPTION: &str = "everruns/computer_use";

/// Name of the provider-neutral computer tool every native call maps back to.
pub const COMPUTER_TOOL_NAME: &str = "computer";

/// The display a native computer tool drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeComputerUse {
    /// Display width in pixels.
    pub display_width: u32,
    /// Display height in pixels.
    pub display_height: u32,
}

impl NativeComputerUse {
    /// The `(key, value)` pair a capability contributes.
    pub fn to_driver_option(&self) -> (String, Value) {
        (
            NATIVE_COMPUTER_USE_OPTION.to_string(),
            serde_json::to_value(self).unwrap_or(Value::Null),
        )
    }

    /// Read the option. A malformed payload means "not requested": the
    /// function tool still works, so this never fails a call.
    pub fn from_driver_options(options: &HashMap<String, Value>) -> Option<Self> {
        options
            .get(NATIVE_COMPUTER_USE_OPTION)
            .and_then(|raw| serde_json::from_value(raw.clone()).ok())
    }

    /// The native tool request for this call: the option is set and the call
    /// offers the `computer` tool it stands in for.
    pub fn requested(config: &LlmCallConfig) -> Option<Self> {
        let native = Self::from_driver_options(&config.driver_options)?;
        config
            .tools
            .iter()
            .any(|tool| tool.name() == COMPUTER_TOOL_NAME)
            .then_some(native)
    }
}

/// Whether an OpenAI model takes the GA Responses `computer` tool.
///
/// It shipped with GPT-5.4 and every later model has it. Older models only had
/// `computer-use-preview`, which Everruns does not target.
pub fn openai_has_native_computer(model: &str) -> bool {
    let model = model.rsplit('/').next().unwrap_or(model);
    if model.starts_with("gpt-6") {
        return true;
    }
    model
        .strip_prefix("gpt-5.")
        .and_then(|rest| {
            let minor: String = rest.chars().take_while(char::is_ascii_digit).collect();
            minor.parse::<u32>().ok()
        })
        .is_some_and(|minor| minor >= 4)
}

/// Models that take Anthropic's `computer_toolset_20260801`, from the computer
/// use tool reference (2026-10). Claude Opus 5.5, Sonnet 5.5, and Haiku 5.5
/// accept only the toolset on the Claude API, so for them the function tool is
/// the only other option.
const ANTHROPIC_COMPUTER_TOOLSET_MODELS: [&str; 10] = [
    "claude-fable-5-1",
    "claude-mythos-5-1",
    "claude-fable-5",
    "claude-mythos-5",
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-sonnet-5-5",
    "claude-sonnet-5",
    "claude-haiku-5-5",
    "claude-opus-4-8",
];

/// Whether an Anthropic model takes `computer_toolset_20260801`.
///
/// Matches the bare id, a dated snapshot (`claude-opus-5-20260601`) and the
/// `[1m]` large-context alias, but not a longer sibling id: `claude-opus-5`
/// must not match `claude-opus-5-5`'s prefix rules the other way round.
pub fn anthropic_has_computer_toolset(model: &str) -> bool {
    let model = model.split('[').next().unwrap_or(model);
    ANTHROPIC_COMPUTER_TOOLSET_MODELS.iter().any(|known| {
        model == *known
            || model
                .strip_prefix(known)
                .and_then(|rest| rest.strip_prefix('-'))
                .is_some_and(|rest| rest.len() >= 8 && rest.chars().all(|c| c.is_ascii_digit()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_types::{BuiltinTool, ToolDefinition};
    use serde_json::json;

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.to_string(),
            display_name: None,
            description: String::new(),
            parameters: json!({}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: Default::default(),
            full_parameters: None,
        })
    }

    #[test]
    fn requested_needs_the_option_and_the_computer_tool() {
        let native = NativeComputerUse {
            display_width: 1280,
            display_height: 800,
        };
        let mut config = LlmCallConfig::new("gpt-6.1-sol");
        config.tools = vec![tool("computer")];
        assert_eq!(NativeComputerUse::requested(&config), None);

        let (key, value) = native.to_driver_option();
        config.driver_options.insert(key, value);
        assert_eq!(NativeComputerUse::requested(&config), Some(native));

        config.tools = vec![tool("web_fetch")];
        assert_eq!(NativeComputerUse::requested(&config), None);
    }

    #[test]
    fn malformed_option_is_not_a_request() {
        let options = HashMap::from([(
            NATIVE_COMPUTER_USE_OPTION.to_string(),
            json!({ "display_width": "wide" }),
        )]);
        assert_eq!(NativeComputerUse::from_driver_options(&options), None);
    }

    #[test]
    fn openai_native_computer_starts_at_gpt_5_4() {
        for model in [
            "gpt-6.1-sol",
            "gpt-6-astra",
            "gpt-5.4",
            "gpt-5.6-sol",
            "gpt-5.10",
        ] {
            assert!(openai_has_native_computer(model), "{model}");
        }
        for model in ["gpt-5.2", "gpt-5", "gpt-4.1", "o3", "computer-use-preview"] {
            assert!(!openai_has_native_computer(model), "{model}");
        }
    }

    #[test]
    fn anthropic_toolset_models_match_ids_snapshots_and_aliases() {
        for model in [
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "claude-haiku-5-5",
            "claude-haiku-5-5[1m]",
            "claude-opus-4-8",
            "claude-opus-4-8[1m]",
            "claude-opus-5-20260601",
        ] {
            assert!(anthropic_has_computer_toolset(model), "{model}");
        }
        for model in [
            "claude-haiku-4-5",
            "claude-opus-4-7",
            "claude-sonnet-4-6",
            "claude-opus-5-x",
        ] {
            assert!(!anthropic_has_computer_toolset(model), "{model}");
        }
    }
}
