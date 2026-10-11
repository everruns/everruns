// Dry run: try an agent without its side effects.
//
// A session with this capability runs tools that declare themselves read-only
// for real and answers every other call with a simulated result naming the
// tool and its arguments, so a test run shows what the agent would do without
// sending, writing or deleting anything.
//
// Decisions:
// - **Read-only means declared read-only.** A tool counts as read-only when
//   its hints say `readonly` and nothing marks it destructive or
//   approval-gated; MCP tools get there through their annotations or a
//   person's risk label. Anything undeclared is simulated: a test run errs on
//   the side of not acting.
// - **A policy gate.** It decides on the final arguments, after every
//   transforming hook, so the simulated call is the one that would have run.
// - **A successful result, not an error.** The model should carry on as if
//   the call worked, so the rest of the run is exercised; the result says it
//   was simulated so the model does not report it as done.
// Spec: knowledge/runtime-resources/proactive-agents.md (D5).

use std::sync::Arc;

use async_trait::async_trait;

use crate::builtins::capabilities::{Capability, CapabilityStatus};
use crate::builtins::tool_hooks::{PolicyGate, PreToolUseDecision, PreToolUseHook};
use crate::builtins::tool_types::{ToolCall, ToolDefinition, ToolPolicy, ToolResult};
use crate::tool_context::ToolContext;

pub const DRY_RUN_CAPABILITY_ID: &str = "dry_run";

/// Key in a simulated result that marks it, for the trace and the chat.
pub const DRY_RUN_RESULT_KEY: &str = "dry_run";

/// "Dry run": simulate every tool that is not declared read-only.
pub struct DryRunCapability;

#[async_trait]
impl Capability for DryRunCapability {
    fn id(&self) -> &str {
        DRY_RUN_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Dry Run"
    }

    fn description(&self) -> &str {
        "Runs read-only tools for real and simulates every other call, so a test run shows what the agent would do without doing it."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Safety")
    }

    fn is_guardrail(&self) -> bool {
        true
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "This is a dry run. Tools that only read run for real; every other tool call is \
             simulated and changes nothing. Carry on as if simulated calls succeeded, and in \
             your final answer say which actions you would have taken.",
        )
    }

    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn PreToolUseHook>> {
        vec![Arc::new(PolicyGate(DryRunHook))]
    }
}

/// Whether a dry run lets this tool run for real.
pub fn runs_in_dry_run(tool_def: &ToolDefinition) -> bool {
    let hints = tool_def.hints();
    hints.readonly == Some(true)
        && hints.destructive != Some(true)
        && !matches!(tool_def.policy(), ToolPolicy::RequiresApproval)
}

struct DryRunHook;

#[async_trait]
impl PreToolUseHook for DryRunHook {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> PreToolUseDecision {
        if runs_in_dry_run(tool_def) {
            return PreToolUseDecision::Continue(tool_call);
        }
        let result = ToolResult {
            tool_call_id: tool_call.id.clone(),
            result: Some(serde_json::json!({
                DRY_RUN_RESULT_KEY: {
                    "simulated": true,
                    "tool": tool_call.name,
                    "arguments": tool_call.arguments,
                    "note": format!(
                        "Dry run: `{}` was not executed. Nothing was changed.",
                        tool_call.name
                    ),
                }
            })),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };
        PreToolUseDecision::Defer { tool_call, result }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::tool_types::{BuiltinTool, ToolHints};
    use crate::builtins::typed_id::SessionId;
    use serde_json::json;

    fn tool_with(hints: ToolHints) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: "t".to_string(),
            display_name: None,
            description: String::new(),
            parameters: json!({}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints,
            full_parameters: None,
        })
    }

    fn call() -> ToolCall {
        ToolCall {
            id: "call_1".to_string(),
            name: "visti_send".to_string(),
            arguments: json!({"title": "CI red"}),
        }
    }

    async fn decide(tool: &ToolDefinition) -> PreToolUseDecision {
        DryRunCapability.pre_tool_use_hooks()[0]
            .before_exec(call(), tool, &ToolContext::new(SessionId::new_random()))
            .await
    }

    #[tokio::test]
    async fn read_only_tools_run_for_real() {
        let tool = tool_with(ToolHints {
            readonly: Some(true),
            open_world: Some(true),
            ..Default::default()
        });
        assert!(matches!(
            decide(&tool).await,
            PreToolUseDecision::Continue(_)
        ));
    }

    #[tokio::test]
    async fn everything_else_is_simulated_as_a_success() {
        for hints in [
            ToolHints::default(),
            ToolHints {
                readonly: Some(true),
                destructive: Some(true),
                ..Default::default()
            },
            ToolHints {
                readonly: Some(false),
                ..Default::default()
            },
        ] {
            let PreToolUseDecision::Defer { result, .. } = decide(&tool_with(hints)).await else {
                panic!("expected a simulated result");
            };
            assert!(result.error.is_none());
            let simulated = &result.result.unwrap()[DRY_RUN_RESULT_KEY];
            assert_eq!(simulated["tool"], json!("visti_send"));
            assert_eq!(simulated["arguments"], json!({"title": "CI red"}));
            assert_eq!(simulated["simulated"], json!(true));
        }
    }

    #[test]
    fn the_gate_sees_final_arguments() {
        assert!(DryRunCapability.pre_tool_use_hooks()[0].is_policy_gate());
    }
}
