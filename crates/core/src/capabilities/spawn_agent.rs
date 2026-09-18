//! The unified spawn_agent tool and its delegation targets.

//! Capabilities Module for Agent Loop
//!
//! This module provides the capabilities abstraction that allows composing
//! agent functionality through modular units. Each capability can contribute:
//! - System prompt additions
//! - Tools for the agent
//! - Behavior modifications (future)
//!
//! Design decisions:
//! - Capabilities are defined via the Capability trait for flexibility
//! - CapabilityRegistry holds all available capability implementations
//! - apply_capabilities() merges capability contributions into RuntimeAgent
//! - The agent-loop remains execution-focused; capabilities are applied before execution
//! - System prompt sections use XML tags for clear boundaries between components.
//!   This follows Anthropic's recommendation for multi-component prompts and reduces
//!   misattribution between capability instructions, user-provided AGENTS.md, and the
//!   agent's base system prompt. See knowledge/project/xml-prompt-formatting.md for rationale.
//!
//! Each capability is in its own file with collocated tools.

use crate::tool_context::ToolContext;
use crate::tool_types::ToolCall;
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;

use super::*;

pub struct DelegationTargetProvider {
    pub target_type: &'static str,
    pub tool: Box<dyn Tool>,
}

/// Shared execution mode accepted natively by every `spawn_agent` provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnMode {
    Background,
    Foreground,
}

impl SpawnMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "background" => Some(Self::Background),
            "foreground" => Some(Self::Foreground),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Foreground => "foreground",
        }
    }
}

pub(crate) struct UnifiedSpawnAgentTool {
    pub(crate) providers: Vec<DelegationTargetProvider>,
}

pub(crate) fn validate_spawn_agent_target_fields(
    arguments: &serde_json::Value,
    target_type: &str,
) -> Result<(), String> {
    for field in ["blueprint", "config"] {
        if target_type != "subagent" && arguments.get(field).is_some_and(|value| !value.is_null()) {
            return Err(format!(
                "{field} is only valid for subagent targets, not {target_type}."
            ));
        }
    }
    Ok(())
}

impl UnifiedSpawnAgentTool {
    pub(crate) fn new(providers: Vec<DelegationTargetProvider>) -> Self {
        Self { providers }
    }

    pub(crate) fn provider_for(&self, target_type: &str) -> Option<&dyn Tool> {
        self.providers
            .iter()
            .find(|provider| provider.target_type == target_type)
            .map(|provider| provider.tool.as_ref())
    }

    pub(crate) fn target_types(&self) -> Vec<&'static str> {
        ["subagent", "agent", "external_a2a"]
            .into_iter()
            .filter(|target_type| {
                self.providers
                    .iter()
                    .any(|provider| provider.target_type == *target_type)
            })
            .collect()
    }

    /// Per-`target.type` constraint branches, nested inside the `target`
    /// property. Anthropic rejects `oneOf`/`allOf`/`anyOf` at the top level
    /// of a tool `input_schema`, so provider-specific requirements must live
    /// below the root (nested composition is accepted).
    pub(crate) fn target_constraint_branches(&self) -> Vec<serde_json::Value> {
        self.target_types()
            .into_iter()
            .filter_map(|target_type| match target_type {
                "subagent" => Some(serde_json::json!({
                    "properties": {
                        "type": {"const": "subagent"}
                    }
                })),
                "agent" => Some(serde_json::json!({
                    "properties": {
                        "type": {"const": "agent"}
                    },
                    "required": ["type", "id"]
                })),
                "external_a2a" => Some(serde_json::json!({
                    "properties": {
                        "type": {"const": "external_a2a"}
                    },
                    "anyOf": [
                        {"required": ["id"]},
                        {"required": ["external_agent_id"]}
                    ]
                })),
                _ => None,
            })
            .collect()
    }

    // NOTE: subagent and agent providers require `name` at execution
    // (`require_str`), while external_a2a ignores it. A schema that required
    // `name` only for the local targets would need a top-level
    // `oneOf`/`if`/`allOf`, which Anthropic rejects in a tool `input_schema`.
    // `name` is therefore required at the root unconditionally: requiring a
    // field external_a2a merely ignores is safe (the schema never permits a
    // call execution would reject), whereas omitting it would let a
    // `name`-less subagent call pass validation and then fail at dispatch —
    // exactly the mismatch #2787 set out to close.
}

#[async_trait]
impl Tool for UnifiedSpawnAgentTool {
    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        // A call still streaming its arguments, or one naming an unknown
        // target, must not fall back to "Running Spawn Agent": narrate the
        // delegation directly so the line always names the agent being spawned.
        let from_provider = tool_call
            .arguments
            .get("target")
            .and_then(|target| target.get("type"))
            .and_then(serde_json::Value::as_str)
            .and_then(|target_type| self.provider_for(target_type))
            .and_then(|tool| tool.narrate(tool_call, phase, locale, ctx));
        Some(from_provider.unwrap_or_else(|| {
            crate::tool_narration::narrate_subagent_spawn(&tool_call.arguments, phase, locale)
        }))
    }

    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Spawn Agent")
    }

    fn description(&self) -> &str {
        "Delegate work to another agent target. Set target.type to one of the advertised target types; background returns a task_id for generic task tools, and foreground waits for the result."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Human-readable name for the delegated run (subagent, first-party handoff, or external delegation). Used as the task label."
                },
                "instructions": {
                    "type": "string",
                    "description": "Instructions for the delegated agent. Do not include credentials or bearer tokens."
                },
                "goal": {
                    "type": "string",
                    "description": "Optional objective stored on the spawned session and made visible at system-prompt level."
                },
                "lifetime": {
                    "type": "string",
                    "enum": ["linked", "detached"],
                    "default": "linked",
                    "description": "linked creates a lifecycle child; detached creates an independent top-level peer session. Not valid for external_a2a."
                },
                "seed": {
                    "type": "string",
                    "enum": ["fresh", "fork", "workspace"],
                    "default": "fresh",
                    "description": "Detached-session seed mode: fresh starts blank, fork copies history/workspace/session storage, workspace copies workspace files only."
                },
                "target": {
                    "type": "object",
                    "properties": {
                        "type": {
                            "type": "string",
                            "enum": self.target_types(),
                            "description": "Delegation target type. Use subagent for same-agent child sessions, agent for configured first-party handoffs, or external_a2a for configured remote A2A agents."
                        },
                        "id": {
                            "type": "string",
                            "description": "Configured target id for first-party handoffs or external A2A agents."
                        },
                        "external_agent_id": {
                            "type": "string",
                            "description": "Configured external A2A agent id."
                        }
                    },
                    "required": ["type"],
                    "oneOf": self.target_constraint_branches(),
                    "additionalProperties": false
                },
                "mode": {
                    "type": "string",
                    "enum": ["background", "foreground"],
                    "description": "Execution mode. Use background to return immediately with a task_id, or foreground to block until the delegated work reaches a terminal state or timeout."
                },
                "blueprint": {
                    "type": "string",
                    "description": "Subagent-only blueprint ID to spawn a specialist agent with its own tools and model."
                },
                "config": {
                    "type": "object",
                    "description": "Subagent-only blueprint configuration. Only valid when blueprint is set."
                },
                "result_schema": {
                    "type": "object",
                    "description": "JSON Schema for a required final structured result. Local child agents must call report_result; external A2A agents must return a structured data artifact."
                },
                "message_schema": {
                    "type": "object",
                    "description": "JSON Schema for structured progress messages from local child agents. When set, the child receives report_task_progress. External A2A targets reject this option explicitly."
                },
                "public_context": {
                    "type": "object",
                    "description": "Agent-handoff-only non-secret structured context to include with the instructions."
                },
                "wait_timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 86400,
                    "description": "External-A2A-only foreground timeout."
                },
                "wake_on_completion": {
                    "type": "boolean",
                    "description": "External-A2A-only control for background completion wake-ups."
                }
            },
            "required": ["name", "instructions", "target"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> crate::tool_types::ToolHints {
        let mut hints = crate::tool_types::ToolHints::default()
            .with_long_running(true)
            .with_concurrency_class(SPAWN_AGENT_CONCURRENCY_CLASS);
        if self.provider_for("external_a2a").is_some() {
            hints = hints.with_open_world(true);
        }
        hints
    }

    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "spawn_agent requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: serde_json::Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let target_type = match arguments
            .get("target")
            .and_then(|target| target.get("type"))
            .and_then(serde_json::Value::as_str)
        {
            Some(target_type) => target_type,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: target.type");
            }
        };

        let Some(provider) = self.provider_for(target_type) else {
            let supported = self.target_types().join(", ");
            return ToolExecutionResult::tool_error(format!(
                "Unsupported spawn_agent target.type: \"{target_type}\". Supported target types: {supported}"
            ));
        };
        if let Err(error) = validate_spawn_agent_target_fields(&arguments, target_type) {
            return ToolExecutionResult::tool_error(error);
        }
        if target_type == "external_a2a"
            && arguments
                .get("lifetime")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| value == "detached")
        {
            return ToolExecutionResult::tool_error(
                "lifetime=\"detached\" is only valid for local session targets (subagent or agent), not external_a2a.",
            );
        }
        if target_type == "external_a2a"
            && arguments
                .get("message_schema")
                .is_some_and(|schema| !schema.is_null())
        {
            return ToolExecutionResult::tool_error(
                "message_schema is not supported for external_a2a targets because remote agents cannot receive report_task_progress.",
            );
        }

        provider.execute_with_context(arguments, context).await
    }

    fn requires_context(&self) -> bool {
        true
    }
}
