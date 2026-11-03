// Agent handoff capability.
//
// Decision: handoff starts a real configured Agent in a child session instead
// of using a code-defined blueprint. The source agent gets only handoff tools;
// the target agent owns its own tools, capabilities, data, and runtime prompt.
// Credentials are never accepted as tool arguments or config. Required
// provider connections are resolved server-side before the child session starts.
//
// Decision (agents that use other agents, 2026-11-02): this capability is the
// one switch for an agent that hands work to other agents, shown as "Agents".
// A member needs only `agent_id`; the platform fills its name and description
// from the agent on save, and the member runs on its own current harness,
// looked up when the hand-off starts. `members` is accepted for `targets`.

use super::delegation_result::{
    MESSAGE_SCHEMA_SPEC_KEY, RESULT_SCHEMA_SPEC_KEY, normalize_message_schema,
    normalize_result_schema, required_result_is_missing, result_value_for_task,
};
use super::util::{get_subagent_delegate, require_str_nonblank as require_str};
use super::{
    Capability, CapabilityLocalization, CapabilityStatus, RiskLevel, SpawnMode, SystemPromptContext,
};
use async_trait::async_trait;
use everruns_contracts::tool_types::ToolHints;
use everruns_contracts::typed_id::{AgentId, HarnessId};
use everruns_core::config_layer::{AgentConfigOverlay, normalize_initial_file_path};
use everruns_core::session::{SessionSeedMode, SubagentStatus};
use everruns_core::session_task::{
    CreateSessionTask, SessionTask, SessionTaskState, SessionTaskUpdate, TASK_KIND_AGENT_HANDOFF,
    TASK_KIND_SESSION, TaskError, TaskExecutor, TaskExecutorPlugin, TaskLinks, TaskMessage,
    TaskWakePolicy, task_message_text,
};
use everruns_core::subagent_delegation::PlatformCreateSessionRequest;
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{Tool, ToolExecutionResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

pub const AGENT_HANDOFF_CAPABILITY_ID: &str = "agent_handoff";
const DEFAULT_WAIT_TIMEOUT_SECS: u64 = 300;
const BACKGROUND_WAIT_SLICE_SECS: u64 = 300;
const BACKGROUND_MAX_WAIT_SECS: u64 = 6 * 60 * 60;
const BACKGROUND_HEARTBEAT_INTERVAL_SECS: u64 = 15;
const BACKGROUND_POLL_BACKOFF_SECS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandoffLifetime {
    Linked,
    Detached,
}

impl HandoffLifetime {
    fn parse(arguments: &Value) -> Result<Self, String> {
        match arguments.get("lifetime").and_then(Value::as_str) {
            None | Some("linked") => Ok(Self::Linked),
            Some("detached") => Ok(Self::Detached),
            Some(other) => Err(format!(
                "Invalid lifetime: {other}. Expected 'linked' or 'detached'."
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Linked => "linked",
            Self::Detached => "detached",
        }
    }
}

/// How a hand-off's session starts. An explicit `seed` wins; otherwise the
/// target's `files` setting decides: `copy` starts the agent with a copy of
/// this session's files (`knowledge/runtime-resources/agents-using-agents.md`).
fn parse_seed(arguments: &Value, files: TargetFiles) -> Result<SessionSeedMode, String> {
    match arguments.get("seed").and_then(Value::as_str) {
        None if files == TargetFiles::Copy => Ok(SessionSeedMode::Workspace),
        None | Some("fresh") => Ok(SessionSeedMode::Fresh),
        Some("fork") => Ok(SessionSeedMode::Fork),
        Some("workspace") => Ok(SessionSeedMode::Workspace),
        Some(other) => Err(format!(
            "Invalid seed: {other}. Expected 'fresh', 'fork', or 'workspace'."
        )),
    }
}

fn terminal_handoff_status(wait_status: &str) -> Option<SubagentStatus> {
    match wait_status {
        "idle" | "completed" => Some(SubagentStatus::Completed),
        "error" | "failed" => Some(SubagentStatus::Failed),
        "cancelled" => Some(SubagentStatus::Cancelled),
        "max_iterations_reached" => Some(SubagentStatus::MaxIterationsReached),
        "sealed" => Some(SubagentStatus::Sealed),
        _ => None,
    }
}

pub struct AgentHandoffCapability;

#[async_trait]
impl Capability for AgentHandoffCapability {
    fn id(&self) -> &str {
        AGENT_HANDOFF_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Agents"
    }

    fn description(&self) -> &str {
        "Hand work to other agents. Each one runs as itself, with its own tools and files."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("user-round-check")
    }

    fn category(&self) -> Option<&str> {
        Some("Orchestration")
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["agent_handoffs"]
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "targets": {
                    "type": "array",
                    "title": "Agents",
                    "description": "Agents this agent can hand work to.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "title": "Target ID",
                                "description": "Name the agent is addressed by in spawn_agent target.id. Defaults to the agent's name."
                            },
                            "name": {
                                "type": "string",
                                "title": "Name",
                                "description": "Human-readable name of the handoff target."
                            },
                            "description": {
                                "type": "string",
                                "title": "Description",
                                "description": "Optional description of what the target agent does."
                            },
                            "agent_id": {
                                "type": "string",
                                "title": "Agent ID",
                                "description": "Public id of the configured target agent."
                            },
                            "harness_id": {
                                "type": "string",
                                "title": "Harness ID",
                                "description": "Optional harness override. Omitted, the agent runs on its own current harness."
                            },
                            "required_connections": {
                                "type": "array",
                                "title": "Required connections",
                                "items": { "type": "string" },
                                "description": "Provider connections required before handoff starts."
                            },
                            "required_scopes": {
                                "type": "array",
                                "title": "Required scopes",
                                "items": { "type": "string" },
                                "description": "Non-secret scope labels recorded for audit and resource metadata."
                            },
                            "files": {
                                "type": "string",
                                "enum": ["own", "copy"],
                                "default": "own",
                                "title": "Files",
                                "description": "\"own\": the agent starts with its own files. \"copy\": it starts with a copy of this session's files, which it changes without touching the originals."
                            }
                        },
                        "required": ["agent_id"],
                        "additionalProperties": false
                    },
                    "default": []
                }
            },
            "additionalProperties": false
        }))
    }

    fn validate_config(&self, config: &Value) -> std::result::Result<(), String> {
        let parsed = AgentHandoffConfig::from_value(config)
            .map_err(|e| format!("invalid agent_handoff config: {e}"))?;
        parsed.validate()
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Defines the configured agents this agent may hand work off to and \
                     the connections each handoff requires.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Агенти"),
                description: Some(
                    "Передає роботу іншим агентам. Кожен працює як сам, зі своїми \
                     інструментами та файлами.",
                ),
                config_description: Some(
                    "Визначає налаштованих агентів, яким цей агент може передавати роботу, та підключення, потрібні для кожного передання.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        "targets": {
                            "title": "Агенти",
                            "description": "Агенти, яким цей агент може передавати роботу.",
                            "items": {
                                "properties": {
                                    "id": {
                                        "title": "Ідентифікатор цілі",
                                        "description": "Стабільний ключ цілі, що використовується як spawn_agent target.id."
                                    },
                                    "name": {
                                        "title": "Назва",
                                        "description": "Зрозуміла людині назва цілі передання."
                                    },
                                    "description": {
                                        "title": "Опис",
                                        "description": "Необов'язковий опис того, що робить цільовий агент."
                                    },
                                    "agent_id": {
                                        "title": "Ідентифікатор агента",
                                        "description": "Публічний ідентифікатор налаштованого цільового агента."
                                    },
                                    "harness_id": {
                                        "title": "Ідентифікатор harness",
                                        "description": "Публічний ідентифікатор налаштованого цільового harness."
                                    },
                                    "required_connections": {
                                        "title": "Обов'язкові підключення",
                                        "description": "Підключення до провайдерів, потрібні перед початком передання."
                                    },
                                    "required_scopes": {
                                        "title": "Обов'язкові scope",
                                        "description": "Несекретні мітки scope, що записуються для аудиту та метаданих ресурсів."
                                    }
                                }
                            }
                        }
                    }
                })),
            },
        ]
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        let _ = AgentHandoffConfig::from_value(config).unwrap_or_default();
        vec![]
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&Value::Null)
    }

    fn delegation_target_with_config(
        &self,
        config: &Value,
    ) -> Option<super::DelegationTargetProvider> {
        Some(super::DelegationTargetProvider {
            target_type: "agent",
            tool: Box::new(SpawnAgentHandoffTool::new(config)),
        })
    }

    async fn system_prompt_contribution_with_config(
        &self,
        _ctx: &SystemPromptContext,
        config: &Value,
    ) -> Option<String> {
        let config = AgentHandoffConfig::from_value(config).unwrap_or_default();
        let targets = config
            .targets
            .iter()
            .map(|target| {
                format!(
                    "- {} ({}) — {}",
                    target.name,
                    target.id,
                    target
                        .description
                        .as_deref()
                        .unwrap_or("Configured handoff target")
                )
            })
            .collect::<Vec<_>>();

        Some(format!(
            "<capability id=\"{}\">\n\
Use spawn_agent with target.type=\"agent\" and target.id set to an agent's name to hand it work. Each agent runs as itself, with its own tools and files; give it everything it needs in the instructions.\n\
Every agent in this run shares the team/ folder in its files. Ask each agent to write its results under team/<its name>/, pass file paths in the instructions instead of pasting long content, and read their results there.\n\
Never ask the user to paste provider tokens into chat or pass credentials in tool arguments.\n\
If a required provider connection is missing, spawn_agent will return a connection_required result and the client should collect credentials through the Connections flow.\n\
Agents you can hand work to:\n{}\n\
</capability>",
            self.id(),
            if targets.is_empty() {
                "- none configured".to_string()
            } else {
                targets.join("\n")
            }
        ))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AgentHandoffConfig {
    #[serde(default, alias = "members")]
    targets: Vec<AgentHandoffTargetConfig>,
}

impl AgentHandoffConfig {
    fn from_value(value: &Value) -> serde_json::Result<Self> {
        if value.is_null() {
            return Ok(Self::default());
        }
        let mut config: Self = serde_json::from_value(value.clone())?;
        for target in &mut config.targets {
            if target.id.trim().is_empty() {
                target.id = target.agent_id.to_string();
            }
            if target.name.trim().is_empty() {
                target.name = target.id.clone();
            }
        }
        Ok(config)
    }

    fn validate(&self) -> std::result::Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for target in &self.targets {
            target.validate()?;
            if !seen.insert(target.id.as_str()) {
                return Err(format!("Duplicate handoff target id: {}", target.id));
            }
        }
        Ok(())
    }

    fn target(&self, id: &str) -> Option<&AgentHandoffTargetConfig> {
        self.targets.iter().find(|target| target.id == id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentHandoffTargetConfig {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: Option<String>,
    agent_id: AgentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    harness_id: Option<HarnessId>,
    #[serde(default)]
    required_connections: Vec<String>,
    #[serde(default)]
    required_scopes: Vec<String>,
    /// The files the agent starts with.
    #[serde(default)]
    files: TargetFiles,
}

/// The files a target agent starts with.
// Decision: a copy, never the same files. Two agents writing one set of files
// overwrite each other; a copy is the agent's own to change, and results come
// back through the team folder or a git branch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum TargetFiles {
    /// Its own files only.
    #[default]
    Own,
    /// A copy of the handing-off session's files.
    Copy,
}

impl AgentHandoffTargetConfig {
    fn validate(&self) -> std::result::Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("Agent handoff target id cannot be empty".to_string());
        }
        if self.name.trim().is_empty() {
            return Err(format!(
                "Agent handoff target {} name cannot be empty",
                self.id
            ));
        }
        for provider in &self.required_connections {
            if provider.trim().is_empty() {
                return Err(format!(
                    "Agent handoff target {} has an empty required connection",
                    self.id
                ));
            }
        }
        for scope in &self.required_scopes {
            if scope.trim().is_empty() {
                return Err(format!(
                    "Agent handoff target {} has an empty required scope",
                    self.id
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpawnAgentHandoffMode {
    Spawn(SpawnMode),
    Invite,
}

impl SpawnAgentHandoffMode {
    fn parse(value: Option<&str>, context: &ToolContext) -> std::result::Result<Self, String> {
        let explicit = match value.map(str::trim).filter(|s| !s.is_empty()) {
            None => None,
            Some(value) => match SpawnMode::parse(value) {
                Some(mode) => Some(Self::Spawn(mode)),
                None => match value {
                    "invite" => Some(Self::Invite),
                    other => {
                        return Err(format!(
                            "Invalid mode: \"{other}\". Valid modes: background, foreground, invite."
                        ));
                    }
                },
            },
        };
        let has_registry = context.session_task_registry.is_some();
        match explicit {
            Some(Self::Spawn(SpawnMode::Background)) if !has_registry => Err(
                "Background mode requires a session task registry, which is not available in this environment. Use mode: \"foreground\" instead."
                    .to_string(),
            ),
            Some(mode) => Ok(mode),
            None if has_registry => Ok(Self::Spawn(SpawnMode::Background)),
            None => Ok(Self::Spawn(SpawnMode::Foreground)),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Spawn(mode) => mode.as_str(),
            Self::Invite => "invite",
        }
    }

    fn is_invite(self) -> bool {
        self == Self::Invite
    }
}

fn capability_conflict_message(
    host: &AgentConfigOverlay,
    guest: &AgentConfigOverlay,
) -> Option<String> {
    guest.capabilities.iter().find_map(|guest_cap| {
        host.capabilities
            .iter()
            .find(|host_cap| host_cap.capability_id() == guest_cap.capability_id())
            .and_then(|host_cap| {
                (host_cap.config_value().clone() != guest_cap.config_value().clone()).then(|| {
                    format!(
                        "capability `{}` has different host and guest configuration",
                        guest_cap.capability_id()
                    )
                })
            })
    })
}

fn initial_file_conflict_message(
    host: &AgentConfigOverlay,
    guest: &AgentConfigOverlay,
) -> Option<String> {
    guest.initial_files.iter().find_map(|guest_file| {
        let guest_path = normalize_initial_file_path(&guest_file.path);
        host.initial_files
            .iter()
            .find(|host_file| normalize_initial_file_path(&host_file.path) == guest_path)
            .and_then(|host_file| {
                (host_file != guest_file)
                    .then(|| format!("mount `{guest_path}` has different host and guest contents"))
            })
    })
}

fn mcp_conflict_message(host: &AgentConfigOverlay, guest: &AgentConfigOverlay) -> Option<String> {
    guest.mcp_servers.iter().find_map(|(name, guest_server)| {
        host.mcp_servers.get(name).and_then(|host_server| {
            (host_server != guest_server)
                .then(|| format!("MCP server `{name}` has different host and guest configuration"))
        })
    })
}

fn invite_conflict_message(
    host: &AgentConfigOverlay,
    guest: &AgentConfigOverlay,
) -> Option<String> {
    capability_conflict_message(host, guest)
        .or_else(|| initial_file_conflict_message(host, guest))
        .or_else(|| mcp_conflict_message(host, guest))
}

async fn harness_chain_overlay(
    store: &dyn everruns_core::subagent_delegation::SubagentSessionDelegate,
    harness_id: HarnessId,
) -> Result<AgentConfigOverlay, ToolExecutionResult> {
    // EVE-881: the delegate returns the effective (inheritance-resolved)
    // definition; parent-chain walking lives behind the platform adapter.
    let harness = store
        .get_harness(harness_id)
        .await
        .map_err(ToolExecutionResult::internal_error)?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!("Harness not found: {harness_id}"))
        })?;
    Ok(AgentConfigOverlay::from(&harness))
}

async fn invite_mode_overlays(
    store: &dyn everruns_core::subagent_delegation::SubagentSessionDelegate,
    parent_session: &everruns_core::session::ExecutionSession,
    target: &AgentHandoffTargetConfig,
) -> Result<(AgentConfigOverlay, AgentConfigOverlay), ToolExecutionResult> {
    let mut host_layers = vec![harness_chain_overlay(store, parent_session.harness_id).await?];
    if let Some(agent_id) = parent_session.agent_id {
        let host_agent = store
            .get_agent_by_id(agent_id)
            .await
            .map_err(ToolExecutionResult::internal_error)?
            .ok_or_else(|| {
                ToolExecutionResult::tool_error(format!("Host agent not found: {agent_id}"))
            })?;
        host_layers.push(AgentConfigOverlay::from(&host_agent));
    }
    host_layers.push(AgentConfigOverlay::from(parent_session));

    let target_agent = store
        .get_agent_by_id(target.agent_id)
        .await
        .map_err(ToolExecutionResult::internal_error)?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!("Target agent not found: {}", target.agent_id))
        })?;

    Ok((
        AgentConfigOverlay::fold(host_layers),
        AgentConfigOverlay::fold([
            harness_chain_overlay(store, target_harness_id(store, target).await?).await?,
            AgentConfigOverlay::from(&target_agent),
        ]),
    ))
}

/// The harness a member runs on: an explicit override, else its own current one.
async fn target_harness_id(
    store: &dyn everruns_core::subagent_delegation::SubagentSessionDelegate,
    target: &AgentHandoffTargetConfig,
) -> Result<HarnessId, ToolExecutionResult> {
    if let Some(harness_id) = target.harness_id {
        return Ok(harness_id);
    }
    store
        .get_agent_harness_id(target.agent_id)
        .await
        .map_err(ToolExecutionResult::internal_error)?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!(
                "Agent \"{}\" is no longer available. Ask the person to check this agent's Agents tab.",
                target.id
            ))
        })
}

fn child_task(task: &str, public_context: Option<&Value>) -> String {
    let Some(public_context) = public_context else {
        return task.to_string();
    };
    format!(
        "{task}\n\n<public_handoff_context>\n{}\n</public_handoff_context>",
        serde_json::to_string_pretty(public_context).unwrap_or_else(|_| "{}".to_string())
    )
}

async fn finish_handoff_task(
    context: &ToolContext,
    task_id: Option<&str>,
    state: SessionTaskState,
    summary: Option<String>,
    error: Option<TaskError>,
    expected_attempt: Option<i32>,
) {
    let (Some(registry), Some(task_id)) = (context.session_task_registry.as_ref(), task_id) else {
        return;
    };
    let _ = registry
        .update(
            context.session_id,
            task_id,
            SessionTaskUpdate {
                state: Some(state),
                summary,
                error,
                expected_attempt,
                ..Default::default()
            },
        )
        .await;
}

async fn finalize_handoff_task(
    context: &ToolContext,
    task_id: Option<&str>,
    mut state: SessionTaskState,
    mut summary: Option<String>,
    mut error: Option<TaskError>,
    expected_attempt: Option<i32>,
) {
    if state == SessionTaskState::Succeeded && required_result_is_missing(context, task_id).await {
        state = SessionTaskState::Failed;
        summary =
            Some("Agent handoff completed without reporting a structured result.".to_string());
        error = Some(TaskError {
            kind: "no_result".to_string(),
            message:
                "Agent handoff completed without calling report_result for its result_schema task."
                    .to_string(),
        });
    }
    finish_handoff_task(context, task_id, state, summary, error, expected_attempt).await;
}

fn handoff_task_state(status: &SubagentStatus) -> SessionTaskState {
    match status {
        SubagentStatus::Completed => SessionTaskState::Succeeded,
        SubagentStatus::Cancelled => SessionTaskState::Canceled,
        SubagentStatus::Failed | SubagentStatus::MaxIterationsReached | SubagentStatus::Sealed => {
            SessionTaskState::Failed
        }
        SubagentStatus::Running | SubagentStatus::Spawning => SessionTaskState::Running,
    }
}

fn handoff_error(status: &str, state: SessionTaskState) -> Option<TaskError> {
    (state == SessionTaskState::Failed).then(|| TaskError {
        kind: "handoff_failed".to_string(),
        message: format!("Handoff ended with status: {status}"),
    })
}

async fn handoff_result(
    store: &dyn everruns_core::subagent_delegation::SubagentSessionDelegate,
    child_session_id: everruns_contracts::typed_id::SessionId,
    status: &str,
) -> Result<String, ToolExecutionResult> {
    let messages = store
        .get_messages(child_session_id, Some(5))
        .await
        .map_err(ToolExecutionResult::internal_error)?;
    Ok(crate::platform_store::last_agent_message(&messages)
        .unwrap_or_else(|| format!("Handoff completed with status: {status}")))
}

fn spawn_handoff_background_watcher(
    context: &ToolContext,
    child_session_id: everruns_contracts::typed_id::SessionId,
    first_message: String,
    task_id: String,
    task_attempt: i32,
) {
    let context = context.clone();
    tokio::spawn(async move {
        let Some(store) = context.subagent_delegate.clone() else {
            return;
        };

        if let Err(error) = store.send_message(child_session_id, &first_message).await {
            finish_handoff_task(
                &context,
                Some(&task_id),
                SessionTaskState::Failed,
                None,
                Some(TaskError {
                    kind: "handoff_failed".to_string(),
                    message: error.to_string(),
                }),
                Some(task_attempt),
            )
            .await;
            return;
        }

        let heartbeat = async {
            let Some(registry) = context.session_task_registry.clone() else {
                return std::future::pending::<()>().await;
            };
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(
                    BACKGROUND_HEARTBEAT_INTERVAL_SECS,
                ))
                .await;
                let _ = registry
                    .update(
                        context.session_id,
                        &task_id,
                        SessionTaskUpdate {
                            heartbeat_at: Some(chrono::Utc::now()),
                            expected_attempt: Some(task_attempt),
                            ..Default::default()
                        },
                    )
                    .await;
            }
        };

        let wait_and_settle = async {
            let started = tokio::time::Instant::now();
            loop {
                let status = match store
                    .wait_for_idle(child_session_id, Some(BACKGROUND_WAIT_SLICE_SECS))
                    .await
                {
                    Ok(status) => status,
                    Err(error) => {
                        finish_handoff_task(
                            &context,
                            Some(&task_id),
                            SessionTaskState::Failed,
                            None,
                            Some(TaskError {
                                kind: "handoff_failed".to_string(),
                                message: error.to_string(),
                            }),
                            Some(task_attempt),
                        )
                        .await;
                        return;
                    }
                };

                if let Some(terminal) = terminal_handoff_status(&status) {
                    let state = handoff_task_state(&terminal);
                    let result = handoff_result(store.as_ref(), child_session_id, &status)
                        .await
                        .ok();
                    let error = handoff_error(&status, state);
                    finalize_handoff_task(
                        &context,
                        Some(&task_id),
                        state,
                        result,
                        error,
                        Some(task_attempt),
                    )
                    .await;
                    return;
                }

                if started.elapsed().as_secs() >= BACKGROUND_MAX_WAIT_SECS {
                    finish_handoff_task(
                        &context,
                        Some(&task_id),
                        SessionTaskState::Failed,
                        None,
                        Some(TaskError {
                            kind: "timeout".to_string(),
                            message: format!(
                                "Background agent handoff did not finish within {BACKGROUND_MAX_WAIT_SECS}s (last status: {status})"
                            ),
                        }),
                        Some(task_attempt),
                    )
                    .await;
                    return;
                }

                if let Some(registry) = context.session_task_registry.as_ref() {
                    let _ = registry
                        .update(
                            context.session_id,
                            &task_id,
                            SessionTaskUpdate {
                                state_detail: Some(format!(
                                    "waiting for agent handoff ({}s elapsed, last status: {status})",
                                    started.elapsed().as_secs()
                                )),
                                expected_attempt: Some(task_attempt),
                                ..Default::default()
                            },
                        )
                        .await;
                }
                if !status.starts_with("timeout") {
                    tokio::time::sleep(std::time::Duration::from_secs(
                        BACKGROUND_POLL_BACKOFF_SECS,
                    ))
                    .await;
                }
            }
        };

        tokio::select! {
            () = wait_and_settle => {}
            () = heartbeat => {}
        }
    });
}

async fn require_connections(
    context: &ToolContext,
    target: &AgentHandoffTargetConfig,
) -> Result<(), ToolExecutionResult> {
    if target.required_connections.is_empty() {
        return Ok(());
    }

    let Some(resolver) = &context.connection_resolver else {
        return Err(ToolExecutionResult::internal_error_msg(
            "Agent handoff connection resolution is not available in this execution context.",
        ));
    };

    for provider in &target.required_connections {
        match resolver
            .get_connection_token(context.session_id, provider)
            .await
        {
            Ok(Some(_token)) => {}
            Ok(None) => return Err(ToolExecutionResult::connection_required(provider.clone())),
            Err(error) => return Err(ToolExecutionResult::internal_error(error)),
        }
    }
    Ok(())
}

pub struct SpawnAgentHandoffTool {
    config: AgentHandoffConfig,
}

impl SpawnAgentHandoffTool {
    pub fn new(config: &Value) -> Self {
        Self {
            config: AgentHandoffConfig::from_value(config).unwrap_or_default(),
        }
    }
}

#[async_trait]
impl Tool for SpawnAgentHandoffTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_subagent_spawn(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Spawn Agent")
    }

    fn description(&self) -> &str {
        "Hand work to another agent. Set target.type to \"agent\" and target.id to the agent's name from your Agents list. Runs in the background by default when task tracking is available; set mode to \"foreground\" to block for the result or \"invite\" to add the target as a member of the current session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Human-readable name for this delegated run."
                },
                "instructions": {
                    "type": "string",
                    "description": "Instructions for the target agent. Do not include credentials or bearer tokens."
                },
                "target": {
                    "type": "object",
                    "properties": {
                        "type": {
                            "type": "string",
                            "enum": ["agent"],
                            "description": "Delegation target type. Use \"agent\" for a configured first-party Agent handoff."
                        },
                        "id": {
                            "type": "string",
                            "description": "Configured handoff target id."
                        }
                    },
                    "required": ["type", "id"],
                    "additionalProperties": false
                },
                "mode": {
                    "type": "string",
                    "enum": ["background", "foreground", "invite"],
                    "description": "Execution mode. \"background\" (default when task tracking is available) returns immediately with a task_id; \"foreground\" blocks until the handoff completes; \"invite\" adds the target agent as a member participant in this session."
                },
                "public_context": {
                    "type": "object",
                    "description": "Non-secret structured context to include with the instructions."
                },
                "result_schema": {
                    "type": "object",
                    "description": "JSON Schema for the child agent's final structured result. The child must call report_result before the task can succeed."
                },
                "message_schema": {
                    "type": "object",
                    "description": "JSON Schema for structured progress messages. The child receives report_task_progress."
                }
            },
            "required": ["name", "instructions", "target"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "spawn_agent requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = match get_subagent_delegate(context) {
            Ok(store) => store,
            Err(error) => return error,
        };

        let target = arguments.get("target").unwrap_or(&Value::Null);
        if target.get("type").and_then(Value::as_str) != Some("agent") {
            return ToolExecutionResult::tool_error(
                "spawn_agent target.type must be \"agent\" for the agent_handoff capability",
            );
        }
        let target_id = match target
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(id) => id,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: target.id");
            }
        };
        let name = match require_str(&arguments, "name") {
            Ok(value) => value.trim().to_string(),
            Err(error) => return error,
        };
        let instructions = match require_str(&arguments, "instructions") {
            Ok(value) => value,
            Err(error) => return error,
        };
        let goal = arguments
            .get("goal")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let lifetime = match HandoffLifetime::parse(&arguments) {
            Ok(value) => value,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };
        let mode = match SpawnAgentHandoffMode::parse(
            arguments.get("mode").and_then(Value::as_str),
            context,
        ) {
            Ok(mode) => mode,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };
        let result_schema = match normalize_result_schema(&arguments) {
            Ok(schema) => schema,
            Err(error) => return error,
        };
        let message_schema = match normalize_message_schema(&arguments) {
            Ok(schema) => schema,
            Err(error) => return error,
        };
        if mode.is_invite() && (result_schema.is_some() || message_schema.is_some()) {
            return ToolExecutionResult::tool_error(
                "result_schema and message_schema require a child task and are not valid for invite-mode agent handoffs.",
            );
        }
        if (result_schema.is_some() || message_schema.is_some())
            && context.session_task_registry.is_none()
        {
            return ToolExecutionResult::tool_error(
                "result_schema and message_schema require session_task_registry context for agent handoffs.",
            );
        }
        if lifetime == HandoffLifetime::Detached && mode.is_invite() {
            return ToolExecutionResult::tool_error(
                "lifetime=\"detached\" is only valid for agent handoffs that create a new session; invite mode joins the current session.",
            );
        }

        let Some(target) = self.config.target(target_id) else {
            return ToolExecutionResult::tool_error(format!(
                "Unknown handoff target: \"{target_id}\". Check configured targets."
            ));
        };
        let seed = match parse_seed(&arguments, target.files) {
            Ok(value) => value,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };

        if let Err(error) = require_connections(context, target).await {
            return error;
        }

        let parent_session = match store.get_session_by_id(context.session_id).await {
            Ok(Some(session)) => session,
            Ok(None) => return ToolExecutionResult::tool_error("Current session not found"),
            Err(error) => return ToolExecutionResult::internal_error(error),
        };

        if lifetime == HandoffLifetime::Linked && parent_session.parent_session_id.is_some() {
            return ToolExecutionResult::tool_error(
                "Agent handoffs cannot be started from child sessions.",
            );
        }

        if mode.is_invite() {
            let (host_overlay, guest_overlay) =
                match invite_mode_overlays(store, &parent_session, target).await {
                    Ok(overlays) => overlays,
                    Err(error) => return error,
                };
            if let Some(conflict) = invite_conflict_message(&host_overlay, &guest_overlay) {
                return ToolExecutionResult::tool_error(format!(
                    "Invite-mode handoff cannot join target \"{}\": {conflict}. Use background or foreground mode for targets that need their own environment.",
                    target.id
                ));
            }

            let reason = crate::platform_store::invite_reason(&target.id);
            let participant_id = match store
                .add_agent_session_participant(context.session_id, target.agent_id, &reason)
                .await
            {
                Ok(participant_id) => participant_id,
                Err(error) => return ToolExecutionResult::internal_error(error),
            };

            return ToolExecutionResult::success(json!({
                "participant_id": participant_id,
                "target": target.id,
                "target_agent_id": target.agent_id,
                "name": name,
                "status": "joined",
                "mode": "invite",
                "message": "Target agent joined this session and can respond when addressed.",
            }));
        }

        // Detached handoffs create the same lifecycle-independent peer shape
        // as detached subagents, so they share the host authorization and
        // origin-root budget linkage invariant.
        let budget_root_session_id = if lifetime == HandoffLifetime::Detached {
            let Some(authority) = context.session_creation_authority.as_ref() else {
                return ToolExecutionResult::tool_error(
                    "Detached handoff requires session-creation authority.",
                );
            };
            match authority
                .authorize_session_creation(context.session_id)
                .await
            {
                Ok(root_session_id) => Some(root_session_id),
                Err(error) => {
                    return ToolExecutionResult::tool_error(format!(
                        "Detached handoff is not authorized to create a session: {error}"
                    ));
                }
            }
        } else {
            None
        };

        let harness_id = match target_harness_id(store, target).await {
            Ok(harness_id) => harness_id,
            Err(error) => return error,
        };
        let child_session = match store
            .create_session_with_options(PlatformCreateSessionRequest {
                harness_id,
                agent_id: Some(target.agent_id),
                title: Some(name.clone()),
                goal,
                locale: parent_session.locale.clone(),
                blueprint_id: None,
                blueprint_config: None,
                parent_session_id: (lifetime == HandoffLifetime::Linked)
                    .then_some(context.session_id),
                forked_from_session_id: (lifetime == HandoffLifetime::Detached)
                    .then_some(context.session_id),
                budget_root_session_id,
                seed,
                reason: crate::platform_store::handoff_reason(&name, &target.id),
            })
            .await
        {
            Ok(session) => session,
            Err(error) => return ToolExecutionResult::internal_error(error),
        };

        let handoff_task = child_task(instructions, arguments.get("public_context"));
        let mut task_id = None;
        let mut task_attempt = 1;
        if let Some(task_registry) = &context.session_task_registry {
            let mut task_spec = json!({
                "target_id": &target.id,
                "external_agent_id": target.agent_id,
                "instructions": instructions,
                "mode": mode.as_str(),
                "lifetime": lifetime.as_str(),
                "seed": seed.as_str(),
            });
            if let Some(spec) = task_spec.as_object_mut() {
                if let Some(schema) = &result_schema {
                    spec.insert(RESULT_SCHEMA_SPEC_KEY.to_string(), schema.clone());
                }
                if let Some(schema) = &message_schema {
                    spec.insert(MESSAGE_SCHEMA_SPEC_KEY.to_string(), schema.clone());
                }
            }
            match task_registry
                .create(CreateSessionTask {
                    session_id: context.session_id,
                    id: None,
                    kind: match lifetime {
                        HandoffLifetime::Linked => TASK_KIND_AGENT_HANDOFF,
                        HandoffLifetime::Detached => TASK_KIND_SESSION,
                    }
                    .to_string(),
                    display_name: name.clone(),
                    spec: task_spec,
                    state: SessionTaskState::Running,
                    links: TaskLinks {
                        child_session_id: Some(child_session.id),
                        ..Default::default()
                    },
                    wake_policy: match (lifetime, mode, message_schema.is_some()) {
                        (HandoffLifetime::Detached, _, _) => TaskWakePolicy::Silent,
                        (
                            HandoffLifetime::Linked,
                            SpawnAgentHandoffMode::Spawn(SpawnMode::Background),
                            true,
                        ) => TaskWakePolicy::OnActivity,
                        (
                            HandoffLifetime::Linked,
                            SpawnAgentHandoffMode::Spawn(SpawnMode::Background),
                            false,
                        ) => TaskWakePolicy::OnTerminal,
                        (
                            HandoffLifetime::Linked,
                            SpawnAgentHandoffMode::Spawn(SpawnMode::Foreground),
                            _,
                        ) => TaskWakePolicy::Silent,
                        (HandoffLifetime::Linked, SpawnAgentHandoffMode::Invite, _) => {
                            unreachable!("invite mode returns before child-session task creation")
                        }
                    },
                })
                .await
            {
                Ok(task) => {
                    task_attempt = task.attempt;
                    task_id = Some(task.id);
                }
                Err(error)
                    if mode == SpawnAgentHandoffMode::Spawn(SpawnMode::Background)
                        || result_schema.is_some()
                        || message_schema.is_some() =>
                {
                    return ToolExecutionResult::tool_error(format!(
                        "Background spawn_agent could not create its session task, so the handoff was not started: {error}"
                    ));
                }
                Err(_) => {}
            }
        }

        match mode {
            SpawnAgentHandoffMode::Spawn(SpawnMode::Background) => {
                let Some(task_id) = task_id else {
                    return ToolExecutionResult::tool_error(
                        "Background spawn_agent requires session_task_registry context so the handoff can be controlled with wait_task/message_task/cancel_task",
                    );
                };
                spawn_handoff_background_watcher(
                    context,
                    child_session.id,
                    handoff_task,
                    task_id.clone(),
                    task_attempt,
                );
                ToolExecutionResult::success(json!({
                    "task_id": task_id,
                    "handoff_id": child_session.id.to_string(),
                    "target": target.id,
                    "target_agent_id": target.agent_id,
                    "name": name,
                    "status": "running",
                    "mode": "background",
                }))
            }
            SpawnAgentHandoffMode::Spawn(SpawnMode::Foreground) => {
                if let Err(error) = store.send_message(child_session.id, &handoff_task).await {
                    finish_handoff_task(
                        context,
                        task_id.as_deref(),
                        SessionTaskState::Failed,
                        None,
                        Some(TaskError {
                            kind: "handoff_failed".to_string(),
                            message: error.to_string(),
                        }),
                        None,
                    )
                    .await;
                    return ToolExecutionResult::internal_error(error);
                }

                let status = match store
                    .wait_for_idle(child_session.id, Some(DEFAULT_WAIT_TIMEOUT_SECS))
                    .await
                {
                    Ok(status) => status,
                    Err(error) => {
                        finish_handoff_task(
                            context,
                            task_id.as_deref(),
                            SessionTaskState::Failed,
                            None,
                            Some(TaskError {
                                kind: "handoff_failed".to_string(),
                                message: error.to_string(),
                            }),
                            None,
                        )
                        .await;
                        return ToolExecutionResult::success(json!({
                            "task_id": task_id,
                            "handoff_id": child_session.id.to_string(),
                            "target": target.id,
                            "target_agent_id": target.agent_id,
                            "name": name,
                            "status": "failed",
                            "error": error.to_string(),
                            "mode": "foreground",
                        }));
                    }
                };

                let result = match handoff_result(store, child_session.id, &status).await {
                    Ok(result) => result,
                    Err(error) => return error,
                };
                if let Some(terminal) = terminal_handoff_status(&status) {
                    let state = handoff_task_state(&terminal);
                    let error = handoff_error(&status, state);
                    finalize_handoff_task(
                        context,
                        task_id.as_deref(),
                        state,
                        Some(result.clone()),
                        error,
                        None,
                    )
                    .await;
                }

                let result_value = result_value_for_task(context, task_id.as_deref())
                    .await
                    .unwrap_or_else(|| json!(result));

                ToolExecutionResult::success(json!({
                    "task_id": task_id,
                    "handoff_id": child_session.id.to_string(),
                    "target": target.id,
                    "target_agent_id": target.agent_id,
                    "name": name,
                    "status": status,
                    "result": result_value,
                    "mode": "foreground",
                }))
            }
            SpawnAgentHandoffMode::Invite => {
                unreachable!("invite mode returns before child-session execution")
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct AgentHandoffTaskExecutor;

#[async_trait]
impl TaskExecutor for AgentHandoffTaskExecutor {
    fn kind(&self) -> &str {
        TASK_KIND_AGENT_HANDOFF
    }

    async fn deliver(
        &self,
        task: &SessionTask,
        message: &TaskMessage,
        context: &ToolContext,
    ) -> everruns_contracts::error::Result<()> {
        let Some(store) = context.subagent_delegate.as_ref() else {
            return Err(everruns_contracts::error::AgentLoopError::tool(
                "agent handoff task delivery requires platform_store context",
            ));
        };
        let Some(child_id) = task.links.child_session_id else {
            return Err(everruns_contracts::error::AgentLoopError::tool(format!(
                "agent handoff task {} has no child session link",
                task.id
            )));
        };
        let text = task_message_text(&message.content);
        store.send_message(child_id, &text).await
    }

    async fn cancel(
        &self,
        task: &SessionTask,
        context: &ToolContext,
    ) -> everruns_contracts::error::Result<()> {
        let Some(store) = context.subagent_delegate.as_ref() else {
            return Err(everruns_contracts::error::AgentLoopError::tool(
                "agent handoff task cancellation requires platform_store context",
            ));
        };
        let Some(child_id) = task.links.child_session_id else {
            return Err(everruns_contracts::error::AgentLoopError::tool(format!(
                "agent handoff task {} has no child session link",
                task.id
            )));
        };
        store
            .send_message(
                child_id,
                "Cancellation requested by the parent session. Stop work, wind down, and reply with a brief summary of progress so far.",
            )
            .await
    }

    async fn reconcile(
        &self,
        task: &SessionTask,
        context: &ToolContext,
    ) -> everruns_contracts::error::Result<()> {
        if task.state.is_terminal() {
            return Ok(());
        }
        let (Some(store), Some(child_id)) = (
            context.subagent_delegate.as_ref(),
            task.links.child_session_id,
        ) else {
            return Ok(());
        };
        let status = store.wait_for_idle(child_id, Some(0)).await?;
        let Some(terminal) = terminal_handoff_status(&status) else {
            return Ok(());
        };
        let state = handoff_task_state(&terminal);
        let result = handoff_result(store.as_ref(), child_id, &status).await.ok();
        let error = handoff_error(&status, state);
        finalize_handoff_task(context, Some(&task.id), state, result, error, None).await;
        Ok(())
    }
}

inventory::submit! {
    TaskExecutorPlugin {
        executor: || Arc::new(AgentHandoffTaskExecutor),
    }
}

#[cfg(test)]
#[path = "agent_handoff_tests.rs"]
mod tests;
