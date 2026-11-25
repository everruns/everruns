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
//! - apply_capabilities() merges capability contributions into TurnAgent
//! - The agent-loop remains execution-focused; capabilities are applied before execution
//! - System prompt sections use XML tags for clear boundaries between components.
//!   This follows Anthropic's recommendation for multi-component prompts and reduces
//!   misattribution between capability instructions, user-provided AGENTS.md, and the
//!   agent's base system prompt.
//!
//! Each capability is in its own file with collocated tools.

use crate::runtime::tool_context::ToolContext;
use crate::runtime::tool_types::ToolCall;
use crate::runtime::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;

use super::*;

/// One provider behind `spawn_agent`: the agent's own Spawn agents
/// (`target_type` `local`: itself and its configured agents) or an external
/// A2A / AG-UI integration.
pub struct DelegationTargetProvider {
    pub target_type: &'static str,
    pub tool: Box<dyn Tool>,
    /// The agent may spawn itself (`agent: "self"`).
    pub runs_self: bool,
    /// Agents this provider runs by name.
    pub agents: Vec<DelegationAgent>,
    /// Any agent the person may run, by name or id, is allowed too.
    pub any_agent: bool,
}

impl DelegationTargetProvider {
    pub fn new(target_type: &'static str, tool: Box<dyn Tool>) -> Self {
        Self {
            target_type,
            tool,
            runs_self: false,
            agents: Vec::new(),
            any_agent: false,
        }
    }

    pub fn with_self(mut self, runs_self: bool) -> Self {
        self.runs_self = runs_self;
        self
    }

    pub fn with_agents(mut self, agents: Vec<DelegationAgent>) -> Self {
        self.agents = agents;
        self
    }

    pub fn with_any_agent(mut self, any_agent: bool) -> Self {
        self.any_agent = any_agent;
        self
    }
}

/// A named agent a provider runs. `target_type` is what the provider's tool
/// reads in `target.type` (`agent`, `external_a2a`, `external_ag_ui`).
#[derive(Debug, Clone)]
pub struct DelegationAgent {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub target_type: &'static str,
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
    description: String,
}

/// Fields only a spawn of the agent itself accepts.
const SELF_ONLY_FIELDS: [&str; 4] = ["blueprint", "config", "model", "sandbox"];

/// Where one `spawn_agent` call goes: the provider and the `target` its tool
/// reads.
pub(crate) struct ResolvedSpawn<'a> {
    pub(crate) provider: &'a DelegationTargetProvider,
    pub(crate) target_type: &'static str,
    pub(crate) target_id: Option<String>,
}

pub(crate) fn validate_spawn_agent_target_fields(
    arguments: &serde_json::Value,
    target_type: &str,
) -> Result<(), String> {
    for field in SELF_ONLY_FIELDS {
        if target_type != "subagent" && arguments.get(field).is_some_and(|value| !value.is_null()) {
            return Err(format!(
                "{field} is only valid when the agent spawns itself (agent \"self\")."
            ));
        }
    }
    Ok(())
}

impl UnifiedSpawnAgentTool {
    pub(crate) fn new(providers: Vec<DelegationTargetProvider>) -> Self {
        let description = spawn_agent_description(&providers);
        Self {
            providers,
            description,
        }
    }

    fn runs_self(&self) -> bool {
        self.providers.iter().any(|provider| provider.runs_self)
    }

    fn agent_names(&self) -> Vec<&str> {
        self.providers
            .iter()
            .flat_map(|provider| provider.agents.iter().map(|agent| agent.name.as_str()))
            .collect()
    }

    /// Resolve `agent` (absent or `self` for this agent, else a name or id).
    pub(crate) fn resolve(&self, agent: Option<&str>) -> Result<ResolvedSpawn<'_>, String> {
        let agent = agent.map(str::trim).filter(|agent| !agent.is_empty());
        let available = || {
            let mut names = self.agent_names();
            if self.runs_self() {
                names.insert(0, "self");
            }
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        };
        match agent {
            None | Some("self") => self
                .providers
                .iter()
                .find(|provider| provider.runs_self)
                .map(|provider| ResolvedSpawn {
                    provider,
                    target_type: "subagent",
                    target_id: None,
                })
                .ok_or_else(|| {
                    format!(
                        "This agent cannot spawn itself. Pass agent with one of: {}.",
                        available()
                    )
                }),
            Some(wanted) => {
                for provider in &self.providers {
                    if let Some(found) = provider.agents.iter().find(|candidate| {
                        candidate.id == wanted || candidate.name.eq_ignore_ascii_case(wanted)
                    }) {
                        return Ok(ResolvedSpawn {
                            provider,
                            target_type: found.target_type,
                            target_id: Some(found.id.clone()),
                        });
                    }
                }
                self.providers
                    .iter()
                    .find(|provider| provider.any_agent)
                    .map(|provider| ResolvedSpawn {
                        provider,
                        target_type: "agent",
                        target_id: Some(wanted.to_string()),
                    })
                    .ok_or_else(|| {
                        format!(
                            "Unknown agent \"{wanted}\". Agents you can spawn: {}.",
                            available()
                        )
                    })
            }
        }
    }

    /// The arguments the resolved provider's tool reads: `target` and `mode`
    /// in place of `agent` and `wait`.
    fn provider_arguments(
        arguments: &serde_json::Value,
        resolved: &ResolvedSpawn<'_>,
    ) -> serde_json::Value {
        let mut arguments = arguments.clone();
        if let Some(map) = arguments.as_object_mut() {
            let wait = map
                .remove("wait")
                .and_then(|wait| wait.as_bool())
                .unwrap_or(false);
            map.remove("agent");
            let mut target = serde_json::json!({ "type": resolved.target_type });
            if let Some(id) = &resolved.target_id {
                target["id"] = serde_json::Value::String(id.clone());
            }
            map.insert("target".to_string(), target);
            // No `wait` leaves the mode to the provider: background, or
            // foreground for embedders without a task registry.
            // External providers default to foreground themselves, so say
            // background for them explicitly.
            let mode = if wait {
                Some(SpawnMode::Foreground)
            } else if matches!(resolved.target_type, "external_a2a" | "external_ag_ui") {
                Some(SpawnMode::Background)
            } else {
                None
            };
            if let Some(mode) = mode {
                map.insert(
                    "mode".to_string(),
                    serde_json::Value::String(mode.as_str().to_string()),
                );
            }
        }
        arguments
    }
}

/// Decision: calls written in the retired form (`target: {type, id}` and
/// `mode`) still work, so scripted tests, evals and replays recorded before
/// the unified schema keep running: `target` becomes `agent` and
/// `mode: "foreground"` becomes `wait: true`. Invite and detached calls left
/// this tool (`invite_agent`, the session fork action) and are refused.
pub(crate) fn legacy_arguments(
    mut arguments: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let Some(map) = arguments.as_object_mut() else {
        return Ok(arguments);
    };
    if let Some(target) = map.remove("target")
        && !map.contains_key("agent")
    {
        let agent = match target.get("type").and_then(serde_json::Value::as_str) {
            Some("subagent") | None => "self".to_string(),
            Some(_) => target
                .get("id")
                .or_else(|| target.get("external_agent_id"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        };
        map.insert("agent".to_string(), serde_json::Value::String(agent));
    }
    match map
        .remove("mode")
        .as_ref()
        .and_then(serde_json::Value::as_str)
    {
        Some("foreground") if !map.contains_key("wait") => {
            map.insert("wait".to_string(), serde_json::Value::Bool(true));
        }
        Some("invite") => {
            return Err("spawn_agent no longer invites; call invite_agent instead.".to_string());
        }
        _ => {}
    }
    for retired in ["lifetime", "detached", "seed"] {
        map.remove(retired);
    }
    Ok(arguments)
}

fn spawn_agent_description(providers: &[DelegationTargetProvider]) -> String {
    let mut text = String::from(
        "Spawn an agent for one piece of work: it gets your instructions, works on its own, and returns a result. Runs in the background by default: you get a task_id, keep working, and are told when it finishes (get_task, wait_task, message_task, cancel_task). Set wait to true only when you are blocked on the result.",
    );
    let runs_self = providers.iter().any(|provider| provider.runs_self);
    let agents: Vec<&DelegationAgent> = providers
        .iter()
        .flat_map(|provider| provider.agents.iter())
        .collect();
    let any = providers.iter().any(|provider| provider.any_agent);
    if runs_self {
        text.push_str(
            " Omit agent (or pass \"self\") to spawn a copy of yourself with a fresh context.",
        );
    }
    if !agents.is_empty() {
        text.push_str(" Agents you can spawn by name:");
        for agent in agents {
            text.push_str(&format!("\n- {}", agent.name));
            if let Some(description) = agent.description.as_deref().filter(|d| !d.is_empty()) {
                text.push_str(&format!(": {description}"));
            }
        }
    }
    if any {
        text.push_str("\nAny other agent the person may run can be passed by name or agent id.");
    }
    text
}

impl UnifiedSpawnAgentTool {
    fn base_parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "A short label for this piece of work, shown in the task list."
                },
                "instructions": {
                    "type": "string",
                    "description": "Everything the agent needs: the ask, relevant context, and what done looks like. It sees nothing else. Do not include credentials or bearer tokens."
                },
                "agent": {
                    "type": "string",
                    "description": "Which agent does the work: \"self\" (default) or an agent's name from this tool's description."
                },
                "wait": {
                    "type": "boolean",
                    "default": false,
                    "description": "true waits for the result in this call; false (default) returns a task_id at once and you are told when it finishes."
                },
                "goal": {
                    "type": "string",
                    "description": "Optional objective stored on the spawned session and shown to it at system-prompt level."
                },
                "sandbox": {
                    "type": "string",
                    "enum": ["fresh", "fork"],
                    "default": "fresh",
                    "description": "Only when spawning yourself. fresh gives it a new sandbox; fork starts it from a copy of this session's running sandbox (installed packages, build output), falling back to fresh where the provider cannot copy it."
                },
                "blueprint": {
                    "type": "string",
                    "description": "Only when spawning yourself: a blueprint id for a specialist with its own tools and model."
                },
                "config": {
                    "type": "object",
                    "description": "Only with blueprint: the blueprint's configuration."
                },
                "result_schema": {
                    "type": "object",
                    "description": "JSON Schema for a required structured result. The agent must return it with finish (external agents return a matching structured result)."
                },
                "message_schema": {
                    "type": "object",
                    "description": "JSON Schema for structured progress the agent sends with report_progress. Not for external agents."
                },
                "public_context": {
                    "type": "object",
                    "description": "Non-secret structured context passed to a named agent with the instructions."
                },
                "wait_timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 86400,
                    "description": "External agents only: how long wait=true waits."
                },
                "wake_on_completion": {
                    "type": "boolean",
                    "description": "External agents only: whether a background run wakes you when it finishes."
                }
            },
            "required": ["name", "instructions"],
            "additionalProperties": false
        })
    }
}

#[async_trait]
impl Tool for UnifiedSpawnAgentTool {
    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: crate::runtime::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        ctx: crate::runtime::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        // A call still streaming its arguments, or one naming an unknown
        // agent, must not fall back to "Running Spawn Agent": narrate the
        // delegation directly so the line always names the agent being spawned.
        let agent = tool_call
            .arguments
            .get("agent")
            .and_then(serde_json::Value::as_str);
        let from_provider = self.resolve(agent).ok().and_then(|resolved| {
            let mut call = tool_call.clone();
            call.arguments = Self::provider_arguments(&tool_call.arguments, &resolved);
            resolved.provider.tool.narrate(&call, phase, locale, ctx)
        });
        Some(from_provider.unwrap_or_else(|| {
            crate::runtime::tool_narration::narrate_subagent_spawn(
                &tool_call.arguments,
                phase,
                locale,
            )
        }))
    }

    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Spawn Agent")
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let mut schema = self.base_parameters_schema();
        let Some(properties) = schema
            .get_mut("properties")
            .and_then(serde_json::Value::as_object_mut)
        else {
            return schema;
        };
        // The self provider owns the helper-model keys: its `model` property
        // (an enum of the agent's keys and when to use each) is advertised
        // as-is, and absent when the agent cannot switch models.
        let self_provider = self.providers.iter().find(|provider| provider.runs_self);
        if let Some(model) = self_provider
            .map(|provider| provider.tool.parameters_schema())
            .and_then(|schema| schema.pointer("/properties/model").cloned())
        {
            properties.insert("model".to_string(), model);
        }
        if self_provider.is_none() {
            for field in SELF_ONLY_FIELDS {
                properties.remove(field);
            }
        }
        let external = self
            .providers
            .iter()
            .any(|provider| matches!(provider.target_type, "external_a2a" | "external_ag_ui"));
        if !external {
            properties.remove("wait_timeout_secs");
            properties.remove("wake_on_completion");
        }
        if self.agent_names().is_empty() && !self.providers.iter().any(|p| p.any_agent) {
            properties.remove("agent");
            properties.remove("public_context");
        }
        schema
    }

    fn hints(&self) -> crate::runtime::tool_types::ToolHints {
        let mut hints = crate::runtime::tool_types::ToolHints::default()
            .with_long_running(true)
            .with_concurrency_class(SPAWN_AGENT_CONCURRENCY_CLASS);
        if self
            .providers
            .iter()
            .any(|provider| matches!(provider.target_type, "external_a2a" | "external_ag_ui"))
        {
            hints = hints.with_open_world(true);
        }
        hints
    }

    fn upgrade_arguments(&self, arguments: serde_json::Value) -> serde_json::Value {
        // An invite is refused with its own message when the call runs, so
        // let it past the schema check.
        let mut arguments = arguments;
        if arguments.get("mode").and_then(serde_json::Value::as_str) == Some("invite")
            && let Some(map) = arguments.as_object_mut()
        {
            map.remove("mode");
        }
        let mut upgraded = legacy_arguments(arguments.clone()).unwrap_or(arguments);
        // `self` is the default, and the schema has no `agent` when the tool
        // only spawns itself.
        if upgraded.get("agent").and_then(serde_json::Value::as_str) == Some("self")
            && let Some(map) = upgraded.as_object_mut()
        {
            map.remove("agent");
        }
        upgraded
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
        let arguments = match legacy_arguments(arguments) {
            Ok(arguments) => arguments,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };
        let agent = arguments.get("agent").and_then(serde_json::Value::as_str);
        let resolved = match self.resolve(agent) {
            Ok(resolved) => resolved,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };
        if let Err(error) = validate_spawn_agent_target_fields(&arguments, resolved.target_type) {
            return ToolExecutionResult::tool_error(error);
        }
        let external = matches!(resolved.target_type, "external_a2a" | "external_ag_ui");
        if external
            && arguments
                .get("message_schema")
                .is_some_and(|schema| !schema.is_null())
        {
            return ToolExecutionResult::tool_error(
                "message_schema is not supported for external agents: they cannot call report_progress.",
            );
        }
        let arguments = Self::provider_arguments(&arguments, &resolved);
        resolved
            .provider
            .tool
            .execute_with_context(arguments, context)
            .await
    }

    fn requires_context(&self) -> bool {
        true
    }
}
