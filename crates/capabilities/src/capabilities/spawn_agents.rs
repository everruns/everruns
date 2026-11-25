// Spawn agents capability (knowledge/runtime-resources/threads.md).
//
// One-off calls: brief in, result out, then done. The agent gets one
// `spawn_agent` tool (plus the task tools from `session_tasks`); `agent`
// picks who does the work: itself (`self`, a copy with a fresh context), one
// of its configured `agents` by name, or, with `any_agent`, any agent the
// person may run by agent id.
//
// Decision (2026-11-25): this replaces `subagents` and `agent_handoff`. The
// retired ids are rewritten into it on read and by migration
// (`capability::delegation`), and their code stays the engine underneath:
// the self spawn is the subagent tool, a named agent is the hand-off tool.
// Decision: never both Spawn agents and Threads on one agent. Validation
// refuses that on save; Threads supersedes a harness default of Spawn agents.
// Decision: no detached spawns and no invite mode on `spawn_agent`. Joining
// an agent to the session is its own tool, `invite_agent`, offered when the
// agent has named agents.

use super::agent_handoff::{AgentHandoffConfig, SpawnAgentHandoffTool};
use super::subagents::SpawnSubagentAsAgentTool;
use super::util::require_str_nonblank;
use super::{Capability, CapabilityLocalization, CapabilityStatus, RiskLevel, SystemPromptContext};
use async_trait::async_trait;
use everruns_contracts::capability::delegation::SPAWN_AGENTS_CAPABILITY_ID;
use everruns_contracts::tool_types::{ToolCall, ToolHints};
use everruns_core::capabilities::{DelegationAgent, DelegationTargetProvider};
use everruns_core::helper_models::HelperModelSettings;
use everruns_core::tool_context::ToolContext;
use everruns_core::tool_narration::{ToolNarrationContext, ToolNarrationPhase};
use everruns_core::tools::{Tool, ToolExecutionResult};
use serde_json::{Value, json};

/// Parsed Spawn agents config. Limits are read by the host's nesting policy
/// (`max_depth`, `max_open`, `max_total`).
#[derive(Debug, Clone)]
pub(crate) struct SpawnAgentsConfig {
    pub(crate) runs_self: bool,
    pub(crate) handoff: AgentHandoffConfig,
    pub(crate) raw: Value,
}

impl SpawnAgentsConfig {
    pub(crate) fn parse(config: &Value) -> Self {
        let runs_self = config.get("self").and_then(Value::as_bool).unwrap_or(true);
        let handoff = AgentHandoffConfig::from_value(&json!({
            "agents": config.get("agents").cloned().unwrap_or_else(|| json!([])),
            "any_agent": config.get("any_agent").and_then(Value::as_bool).unwrap_or(false),
        }))
        .unwrap_or_default();
        Self {
            runs_self,
            handoff,
            raw: config.clone(),
        }
    }

    fn named(&self) -> bool {
        !self.handoff.targets.is_empty() || self.handoff.any_agent
    }

    pub(crate) fn agents(&self, target_type: &'static str) -> Vec<DelegationAgent> {
        self.handoff
            .targets
            .iter()
            .map(|target| DelegationAgent {
                id: target.id.clone(),
                name: target.name.clone(),
                description: target.description.clone(),
                target_type,
            })
            .collect()
    }
}

pub struct SpawnAgentsCapability;

#[async_trait]
impl Capability for SpawnAgentsCapability {
    fn id(&self) -> &str {
        SPAWN_AGENTS_CAPABILITY_ID
    }

    /// Retired id, rewritten on read (`capability::delegation`).
    fn aliases(&self) -> Vec<&'static str> {
        vec!["subagents", "agent_handoff"]
    }

    fn name(&self) -> &str {
        "Spawn agents"
    }

    fn description(&self) -> &str {
        "Hand one piece of work to an agent and get the result back: a copy of this agent with a fresh context, or another agent that runs as itself."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Запуск агентів",
            "Передайте агенту одне завдання й отримайте результат: копії цього агента з чистим контекстом або іншому агенту, який працює як він сам.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("git-branch")
    }

    fn category(&self) -> Option<&str> {
        Some("Orchestration")
    }

    fn risk_level(&self) -> RiskLevel {
        // Spawned agents are sessions that spend; depth and fan-out are capped.
        RiskLevel::High
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["spawn_agents"]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec![super::SESSION_TASKS_CAPABILITY_ID]
    }

    fn config_schema(&self) -> Option<Value> {
        let (helper_models_schema, helper_model_pick_schema) =
            everruns_core::helper_models::helper_model_config_schema();
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "self": {
                    "type": "boolean",
                    "default": true,
                    "title": "This agent",
                    "description": "The agent may spawn a copy of itself with a fresh context."
                },
                "agents": {
                    "type": "array",
                    "title": "Agents",
                    "description": "Other agents it may spawn by name. Each runs as itself, with its own tools and files.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "agent_id": { "type": "string" },
                            "files": {
                                "type": "string",
                                "enum": ["own", "copy"],
                                "default": "own",
                                "description": "own starts it with its own files; copy with a copy of this session's files."
                            },
                            "required_connections": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Provider connections the person must have before it starts."
                            }
                        },
                        "required": ["agent_id"]
                    }
                },
                "any_agent": {
                    "type": "boolean",
                    "default": false,
                    "title": "Any agent",
                    "description": "Any agent the person may run can be spawned by agent id."
                },
                "max_open": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 1024,
                    "default": everruns_core::delegation_services::DEFAULT_MAX_ACTIVE_DESCENDANT_SUBAGENT_TASKS,
                    "description": "Spawned agents that may be working at once in one run."
                },
                "max_total": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 10000,
                    "default": everruns_core::delegation_services::DEFAULT_MAX_TOTAL_DESCENDANT_SUBAGENT_TASKS,
                    "description": "Spawned agents one run may start in total."
                },
                "max_depth": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 16,
                    "default": everruns_core::delegation_services::DEFAULT_MAX_SUBAGENT_DEPTH,
                    "description": "How deep spawned agents may spawn their own. 0 blocks spawning."
                },
                "helper_models": helper_models_schema,
                "helper_model_pick": helper_model_pick_schema
            }
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if !config.is_null() && !config.is_object() {
            return Err("config must be an object".to_string());
        }
        for (key, max) in [("max_depth", 16), ("max_open", 1024), ("max_total", 10_000)] {
            if let Some(value) = config.get(key) {
                let Some(number) = value.as_u64() else {
                    return Err(format!("{key} must be a non-negative integer"));
                };
                if number > max {
                    return Err(format!("{key} must be <= {max}"));
                }
            }
        }
        if let Some(agents) = config.get("agents") {
            let Some(agents) = agents.as_array() else {
                return Err("agents must be a list".to_string());
            };
            for agent in agents {
                let id = agent.get("agent_id").and_then(Value::as_str).unwrap_or("");
                if id.parse::<everruns_contracts::typed_id::AgentId>().is_err() {
                    return Err(format!(
                        "agents[].agent_id must be an agent id, got \"{id}\""
                    ));
                }
            }
        }
        let parsed = SpawnAgentsConfig::parse(config);
        if !parsed.runs_self && !parsed.named() {
            return Err(
                "Spawn agents needs at least one agent: set self, list agents, or allow any_agent"
                    .to_string(),
            );
        }
        everruns_core::helper_models::validate_helper_model_config(config)
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        let parsed = SpawnAgentsConfig::parse(config);
        if parsed.handoff.targets.is_empty() {
            return Vec::new();
        }
        vec![Box::new(InviteAgentTool {
            config: config.clone(),
        })]
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        Vec::new()
    }

    fn delegation_target_with_config(&self, config: &Value) -> Option<DelegationTargetProvider> {
        let parsed = SpawnAgentsConfig::parse(config);
        Some(
            DelegationTargetProvider::new(
                "local",
                Box::new(LocalSpawnTool {
                    subagent: parsed.runs_self.then(|| SpawnSubagentAsAgentTool {
                        helpers: HelperModelSettings::from_config(&parsed.raw),
                    }),
                    handoff: parsed
                        .named()
                        .then(|| SpawnAgentHandoffTool::from_config(parsed.handoff.clone())),
                }),
            )
            .with_self(parsed.runs_self)
            .with_agents(parsed.agents("agent"))
            .with_any_agent(parsed.handoff.any_agent),
        )
    }

    async fn system_prompt_contribution_with_config(
        &self,
        _ctx: &SystemPromptContext,
        config: &Value,
    ) -> Option<String> {
        let parsed = SpawnAgentsConfig::parse(config);
        let mut text = String::from(SPAWN_AGENTS_PROMPT);
        if parsed.named() {
            text.push_str(SPAWN_NAMED_PROMPT);
        }
        Some(format!(
            "<capability id=\"{SPAWN_AGENTS_CAPABILITY_ID}\">\n{text}\n</capability>"
        ))
    }
}

const SPAWN_AGENTS_PROMPT: &str = "Use spawn_agent for independent work that can run apart from you, or that needs a clean context: one piece of work per call, with complete instructions (the agent sees nothing else). Spawns run in the background: you get a task_id, keep working, and are told when each finishes; follow up with get_task, wait_task, message_task or cancel_task. Set wait to true only when you are blocked on the result. Avoid spawning for the next step of what you are doing yourself. Every agent in this run shares the shared/ folder: pass file paths instead of pasting long content, and ask agents to write results there.";

const SPAWN_NAMED_PROMPT: &str = " Named agents run as themselves, with their own tools and files: pass the agent's name as agent. Never ask the person to paste provider tokens; if a required connection is missing, spawn_agent returns connection_required and the person connects it in Connections.";

/// The local provider behind `spawn_agent`: the self spawn (subagent tool) or
/// a named agent (hand-off tool), picked by `target.type`.
struct LocalSpawnTool {
    subagent: Option<SpawnSubagentAsAgentTool>,
    handoff: Option<SpawnAgentHandoffTool>,
}

impl LocalSpawnTool {
    fn pick(&self, arguments: &Value) -> Option<&dyn Tool> {
        match arguments.pointer("/target/type").and_then(Value::as_str) {
            Some("subagent") => self.subagent.as_ref().map(|tool| tool as &dyn Tool),
            Some("agent") => self.handoff.as_ref().map(|tool| tool as &dyn Tool),
            _ => None,
        }
    }
}

#[async_trait]
impl Tool for LocalSpawnTool {
    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        self.pick(&tool_call.arguments)?
            .narrate(tool_call, phase, locale, ctx)
    }

    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn description(&self) -> &str {
        "Spawn an agent for one piece of work."
    }

    fn parameters_schema(&self) -> Value {
        // The unified tool reads the self spawn's `model` property from here.
        self.subagent
            .as_ref()
            .map(|tool| tool.parameters_schema())
            .unwrap_or_else(|| json!({ "type": "object" }))
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("spawn_agent requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        match self.pick(&arguments) {
            Some(tool) => tool.execute_with_context(arguments, context).await,
            None => ToolExecutionResult::tool_error(
                "This agent cannot spawn that agent. Check the agents listed in spawn_agent.",
            ),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// Join one of the agent's named agents to this session as a participant.
struct InviteAgentTool {
    config: Value,
}

#[async_trait]
impl Tool for InviteAgentTool {
    fn name(&self) -> &str {
        "invite_agent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Invite Agent")
    }

    fn description(&self) -> &str {
        "Add one of your named agents to this session, so it can answer when addressed here. It joins this conversation instead of working on its own; use spawn_agent to hand it a piece of work."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent": { "type": "string", "description": "The agent's name from spawn_agent's list." }
            },
            "required": ["agent"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("invite_agent requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let agent = match require_str_nonblank(&arguments, "agent") {
            Ok(agent) => agent.trim().to_string(),
            Err(error) => return error,
        };
        let parsed = SpawnAgentsConfig::parse(&self.config);
        let Some(target) = parsed
            .handoff
            .targets
            .iter()
            .find(|target| target.id == agent || target.name.eq_ignore_ascii_case(&agent))
        else {
            return ToolExecutionResult::tool_error(format!(
                "Unknown agent \"{agent}\". Invite one of the agents listed in spawn_agent."
            ));
        };
        SpawnAgentHandoffTool::from_config(parsed.handoff.clone())
            .execute_with_context(
                json!({
                    "name": target.name,
                    "instructions": "Join this session.",
                    "target": { "type": "agent", "id": target.id },
                    "mode": "invite",
                }),
                context,
            )
            .await
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// A Spawn agents `any_agent` target: an agent id the model passed, which
/// must resolve for the person running the session.
pub(crate) async fn any_agent_target(
    store: &dyn everruns_core::subagent_delegation::SubagentSessionDelegate,
    target_id: &str,
) -> Result<super::agent_handoff::AgentHandoffTargetConfig, ToolExecutionResult> {
    let Ok(agent_id) = target_id.parse::<everruns_contracts::typed_id::AgentId>() else {
        return Err(ToolExecutionResult::tool_error(format!(
            "Unknown agent: \"{target_id}\". Pass a configured agent's name or an agent id."
        )));
    };
    // THREAT[TM-AUTHZ]: the delegate runs as the person, so an agent they may
    // not see resolves to None and is refused.
    let agent = store
        .get_agent_by_id(agent_id)
        .await
        .map_err(ToolExecutionResult::internal_error)?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!("Agent {target_id} was not found."))
        })?;
    serde_json::from_value(json!({
        "id": target_id,
        "name": agent.name,
        "description": agent.description,
        "agent_id": agent_id,
    }))
    .map_err(ToolExecutionResult::internal_error)
}

/// Named agents a delegation target lists in `spawn_agent`: `(id, name,
/// description)` of each configured agent.
pub(crate) fn delegation_agents(
    target_type: &'static str,
    agents: impl Iterator<Item = (String, String, Option<String>)>,
) -> Vec<everruns_core::capabilities::DelegationAgent> {
    agents
        .map(
            |(id, name, description)| everruns_core::capabilities::DelegationAgent {
                id,
                name,
                description,
                target_type,
            },
        )
        .collect()
}
