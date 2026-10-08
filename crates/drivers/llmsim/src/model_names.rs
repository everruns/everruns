//! Behaviour selected by markers in the model name: `-ttft-{ms}` delays the
//! first token, and `-realistic` turns on an agent-shaped workload
//! (`llmsim-realistic`).
//!
//! The zero-latency default isolates platform overhead, but it makes every
//! turn one instant LLM call with a one-line answer: no tool phase, a handful
//! of stream deltas, and no time for turns to overlap. Real turns stream for
//! seconds, call tools, and hold work open while they do. Load tests that
//! should resemble production use a model id containing [`MARKER`].

use everruns_contracts::driver_registry::{LlmCallConfig, Message, MessageRole};
use everruns_contracts::tool_types::{ToolCall, ToolDefinition};
use llmsim::generator::{LoremGenerator, ResponseGenerator};
use llmsim::latency::LatencyProfile;

use super::{GeneratedTurn, LlmSimDriver};

/// Model-name marker that turns the workload on.
const MARKER: &str = "-realistic";

/// Tool rounds per user message, cycled. Averages 1.2 rounds a message.
const TOOL_ROUNDS: [usize; 5] = [1, 2, 0, 1, 2];

/// Final answer length in tokens, cycled. Averages about 220 tokens.
const ANSWER_TOKENS: [usize; 4] = [180, 320, 120, 260];

/// One step of the workload, or `None` when the model is not `-realistic`.
///
/// Each user message gets a planned number of tool rounds (cycled from
/// [`TOOL_ROUNDS`] by user-message count, so runs are reproducible). Until the
/// plan is met, a step is a short preamble plus one call to a cheap tool the
/// agent actually has; then the final answer is lorem text sized from
/// [`ANSWER_TOKENS`]. An agent with neither tool answers straight away.
pub(crate) fn realistic_turn(
    driver: &LlmSimDriver,
    messages: &[Message],
    config: &LlmCallConfig,
) -> Option<GeneratedTurn> {
    if !config.model.contains(MARKER) {
        return None;
    }
    Some(step(driver, messages, &config.tools))
}

fn step(driver: &LlmSimDriver, messages: &[Message], tools: &[ToolDefinition]) -> GeneratedTurn {
    let user_turn = messages
        .iter()
        .filter(|m| m.role == MessageRole::User)
        .count()
        .saturating_sub(1);
    let rounds_done = messages
        .iter()
        .rev()
        .take_while(|m| m.role != MessageRole::User)
        .filter(|m| {
            m.role == MessageRole::Assistant && m.tool_calls.as_ref().is_some_and(|c| !c.is_empty())
        })
        .count();
    let planned = TOOL_ROUNDS[user_turn % TOOL_ROUNDS.len()];
    if rounds_done < planned
        && let Some(call) = tool_call(tools, user_turn, rounds_done)
    {
        return GeneratedTurn {
            text: format!("Step {}: checking the workspace first.", rounds_done + 1),
            tool_calls: Some(vec![call]),
            stream_stall: false,
        };
    }
    let tokens = ANSWER_TOKENS[user_turn % ANSWER_TOKENS.len()];
    GeneratedTurn {
        text: LoremGenerator::new(tokens).generate(&driver.to_chat_request(messages)),
        tool_calls: None,
        stream_stall: false,
    }
}

/// Streaming timing for `-realistic` models, `None` for any other model.
///
/// A model family in the name picks llmsim's profile for it
/// (`llmsim-realistic-haiku`, `llmsim-realistic-gpt-5`). Otherwise the
/// default matches what production sees: time to first token around 0.9 s
/// (0.8 to 1.3 s measured on app.everruns.com) and about 65 tokens a second.
pub(crate) fn realistic_latency(model_name: &str) -> Option<LatencyProfile> {
    let (_, rest) = model_name.split_once(MARKER)?;
    let family = rest.trim_start_matches('-');
    Some(if family.is_empty() {
        LatencyProfile::new(900, 250, 15, 5)
    } else {
        LatencyProfile::from_model(family)
    })
}

/// A cheap, side-effect-free call to a tool the agent has: `bash` (an
/// `echo`), else `write_file` into a scratch path. `None` when it has neither.
fn tool_call(tools: &[ToolDefinition], user_turn: usize, round: usize) -> Option<ToolCall> {
    let find = |name: &str| tools.iter().find(|t| t.name() == name);
    let (name, arguments) = if let Some(bash) = find("bash") {
        let command = format!("echo llmsim turn {user_turn} step {round}");
        ("bash", bash_arguments(bash.parameters(), command))
    } else if find("write_file").is_some() {
        (
            "write_file",
            serde_json::json!({
                "path": format!("/workspace/llmsim/turn-{user_turn}-step-{round}.md"),
                "content": format!("llmsim turn {user_turn} step {round}\n"),
            }),
        )
    } else {
        return None;
    };
    // History can be compacted, so the turn index alone may repeat within a
    // session; a wall-clock component keeps ids unique across restarts too.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    Some(ToolCall {
        id: format!("call_llmsim_{nonce:x}_{user_turn}_{round}"),
        name: name.to_string(),
        arguments,
    })
}

/// Arguments for a `bash` tool. Shells name their script argument differently
/// (`commands` for Bashkit, `command` for environment shells), so the name
/// comes from the tool's own schema: its first required property.
fn bash_arguments(schema: &serde_json::Value, command: String) -> serde_json::Value {
    let field = schema["required"]
        .get(0)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("command");
    let value = if schema["properties"][field]["type"] == "array" {
        serde_json::json!([command])
    } else {
        serde_json::json!(command)
    };
    serde_json::json!({ field: value })
}

/// Parse TTFT (time to first token) delay from model name if it contains "-ttft-{ms}" pattern.
/// For example: "llmsim-ttft-2000" returns Some(Duration::from_millis(2000))
///
/// This allows tests to opt-in to response delays by using specific model names,
/// which is useful for testing cancellation of active turns.
pub(crate) fn ttft(model_name: &str) -> Option<std::time::Duration> {
    if let Some(idx) = model_name.find("-ttft-") {
        let after_ttft = &model_name[idx + 6..]; // skip "-ttft-"
        let ms_str: String = after_ttft
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(ms) = ms_str.parse::<u64>()
            && ms > 0
        {
            return Some(std::time::Duration::from_millis(ms));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_message(content: &str) -> Message {
        Message::text(MessageRole::User, content)
    }

    fn latency_profile_of(model_name: &str) -> LatencyProfile {
        realistic_latency(model_name).expect("realistic model")
    }

    fn assistant_tool_round(name: &str) -> Vec<Message> {
        let mut call = Message::text(MessageRole::Assistant, "step");
        call.tool_calls = Some(vec![ToolCall {
            id: "call_1".to_string(),
            name: name.to_string(),
            arguments: serde_json::json!({}),
        }]);
        vec![call, Message::text(MessageRole::Tool, "ok")]
    }

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition::function(name, "test tool", serde_json::json!({"type": "object"}))
    }

    #[test]
    fn realistic_turn_calls_tools_as_planned_then_answers() {
        let driver = LlmSimDriver::default_driver();
        let bash = ToolDefinition::function(
            "bash",
            "Bashkit shell",
            serde_json::json!({
                "type": "object",
                "properties": {"commands": {"type": "string"}, "timeout_ms": {"type": "integer"}},
                "required": ["commands"],
            }),
        );
        let tools = vec![tool("web_fetch"), bash];

        // Second user message: plan is two rounds.
        let mut messages = vec![user_message("first"), user_message("second")];
        for round in 0..2 {
            let tool_step = step(&driver, &messages, &tools);
            let calls = tool_step.tool_calls.expect("tool round planned");
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].name, "bash");
            assert_eq!(
                calls[0].arguments["commands"],
                format!("echo llmsim turn 1 step {round}")
            );
            messages.extend(assistant_tool_round("bash"));
        }

        let answer = step(&driver, &messages, &tools);
        assert!(answer.tool_calls.is_none());
        // ~4 chars a token; the plan asks for 320 tokens here.
        assert!(
            answer.text.len() > 600,
            "answer too short: {}",
            answer.text.len()
        );
    }

    #[test]
    fn realistic_turn_falls_back_to_write_file_and_skips_without_tools() {
        let driver = LlmSimDriver::default_driver();
        let messages = vec![user_message("first")];

        let first = step(&driver, &messages, &[tool("write_file")]);
        let calls = first.tool_calls.expect("tool round planned");
        assert_eq!(calls[0].name, "write_file");
        assert_eq!(
            calls[0].arguments["path"],
            "/workspace/llmsim/turn-0-step-0.md"
        );

        let answer = step(&driver, &messages, &[tool("web_fetch")]);
        assert!(answer.tool_calls.is_none());
        assert!(!answer.text.is_empty());
    }

    #[test]
    fn bash_arguments_follow_the_tool_schema() {
        let command = "echo hi".to_string();
        assert_eq!(
            bash_arguments(&serde_json::json!({}), command.clone()),
            serde_json::json!({"command": "echo hi"})
        );
        assert_eq!(
            bash_arguments(
                &serde_json::json!({"required": ["commands"], "properties": {"commands": {"type": "array"}}}),
                command,
            ),
            serde_json::json!({"commands": ["echo hi"]})
        );
    }

    #[test]
    fn realistic_latency_matches_production_unless_a_family_is_named() {
        let default = latency_profile_of("llmsim-realistic");
        assert_eq!((default.ttft_mean_ms, default.tbt_mean_ms), (900, 15));

        let haiku = latency_profile_of("llmsim-realistic-haiku");
        assert_eq!(
            haiku.ttft_mean_ms,
            LatencyProfile::claude_haiku().ttft_mean_ms
        );

        let driver = LlmSimDriver::default_driver();
        assert_eq!(
            driver
                .resolve_latency_profile("llmsim-realistic")
                .ttft_mean_ms,
            900
        );
        assert_eq!(
            driver
                .resolve_latency_profile("llmsim-default")
                .ttft_mean_ms,
            0
        );
    }

    #[test]
    fn test_ttft() {
        // Valid patterns
        assert_eq!(
            ttft("llmsim-ttft-2000"),
            Some(std::time::Duration::from_millis(2000))
        );
        assert_eq!(
            ttft("test-ttft-500-extra"),
            Some(std::time::Duration::from_millis(500))
        );

        // No TTFT patterns
        assert_eq!(ttft("llmsim-model"), None);
        assert_eq!(ttft("llmsim-ttft-0"), None);
        assert_eq!(ttft("llmsim-ttft-abc"), None);
    }
}
