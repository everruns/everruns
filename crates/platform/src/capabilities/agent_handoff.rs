// Agent handoff capability.
//
// Decision: handoff starts a real configured Agent in a child session instead
// of using a code-defined blueprint. The source agent gets only handoff tools;
// the target agent owns its own tools, capabilities, data, and runtime prompt.
// Credentials are never accepted as tool arguments or config. Required
// provider connections are resolved server-side before the child session starts.

use super::delegation_result::{
    MESSAGE_SCHEMA_SPEC_KEY, RESULT_SCHEMA_SPEC_KEY, normalize_message_schema,
    normalize_result_schema, required_result_is_missing, result_value_for_task,
};
use super::util::{get_subagent_delegate, require_str_nonblank as require_str};
use super::{
    Capability, CapabilityLocalization, CapabilityStatus, RiskLevel, SpawnMode, SystemPromptContext,
};
use async_trait::async_trait;
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
use everruns_provider::tool_types::ToolHints;
use everruns_provider::typed_id::{AgentId, HarnessId};
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

fn parse_seed(arguments: &Value) -> Result<SessionSeedMode, String> {
    match arguments.get("seed").and_then(Value::as_str) {
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
        "Agent Handoff"
    }

    fn description(&self) -> &str {
        "Delegate work to configured first-party agents through an authenticated handoff gate."
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
                    "title": "Handoff targets",
                    "description": "Configured agents this agent may hand work off to.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "title": "Target ID",
                                "description": "Stable target key used as spawn_agent target.id."
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
                                "description": "Public id of the configured target harness."
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
                            }
                        },
                        "required": ["id", "name", "agent_id", "harness_id"],
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
                name: Some("Передання роботи агентам"),
                description: Some(
                    "Делегує роботу налаштованим власним агентам через автентифікований \
                     шлюз передання.",
                ),
                config_description: Some(
                    "Визначає налаштованих агентів, яким цей агент може передавати роботу, та підключення, потрібні для кожного передання.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        "targets": {
                            "title": "Цілі передання",
                            "description": "Налаштовані агенти, яким цей агент може передавати роботу.",
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
Use spawn_agent with target.type=\"agent\" to delegate work to configured first-party agents.\n\
Never ask the user to paste provider tokens into chat or pass credentials in tool arguments.\n\
If a required provider connection is missing, spawn_agent will return a connection_required result and the client should collect credentials through the Connections flow.\n\
Available handoff targets:\n{}\n\
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
    #[serde(default)]
    targets: Vec<AgentHandoffTargetConfig>,
}

impl AgentHandoffConfig {
    fn from_value(value: &Value) -> serde_json::Result<Self> {
        if value.is_null() {
            Ok(Self::default())
        } else {
            serde_json::from_value(value.clone())
        }
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
    id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    agent_id: AgentId,
    harness_id: HarnessId,
    #[serde(default)]
    required_connections: Vec<String>,
    #[serde(default)]
    required_scopes: Vec<String>,
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
            harness_chain_overlay(store, target.harness_id).await?,
            AgentConfigOverlay::from(&target_agent),
        ]),
    ))
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

fn last_agent_message(
    messages: &[everruns_core::subagent_delegation::PlatformMessage],
) -> Option<String> {
    messages
        .iter()
        .rfind(|message| message.role == "agent" || message.role == "assistant")
        .map(|message| message.content.clone())
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
    child_session_id: everruns_provider::typed_id::SessionId,
    status: &str,
) -> Result<String, ToolExecutionResult> {
    let messages = store
        .get_messages(child_session_id, Some(5))
        .await
        .map_err(ToolExecutionResult::internal_error)?;
    Ok(last_agent_message(&messages)
        .unwrap_or_else(|| format!("Handoff completed with status: {status}")))
}

fn spawn_handoff_background_watcher(
    context: &ToolContext,
    child_session_id: everruns_provider::typed_id::SessionId,
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
        tool_call: &everruns_provider::tool_types::ToolCall,
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
        "Delegate work to a configured first-party target agent. Set target.type to \"agent\" and target.id to a configured handoff target id. Runs in the background by default when task tracking is available; set mode to \"foreground\" to block for the result or \"invite\" to add the target as a member of the current session."
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
        let seed = match parse_seed(&arguments) {
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

            let participant_id = match store
                .add_agent_session_participant(context.session_id, target.agent_id)
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

        let child_session = match store
            .create_session_with_options(PlatformCreateSessionRequest {
                harness_id: target.harness_id,
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
    ) -> everruns_provider::error::Result<()> {
        let Some(store) = context.subagent_delegate.as_ref() else {
            return Err(everruns_provider::error::AgentLoopError::tool(
                "agent handoff task delivery requires platform_store context",
            ));
        };
        let Some(child_id) = task.links.child_session_id else {
            return Err(everruns_provider::error::AgentLoopError::tool(format!(
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
    ) -> everruns_provider::error::Result<()> {
        let Some(store) = context.subagent_delegate.as_ref() else {
            return Err(everruns_provider::error::AgentLoopError::tool(
                "agent handoff task cancellation requires platform_store context",
            ));
        };
        let Some(child_id) = task.links.child_session_id else {
            return Err(everruns_provider::error::AgentLoopError::tool(format!(
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
    ) -> everruns_provider::error::Result<()> {
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
mod tests {
    use super::*;
    use crate::PlatformStoreSubagentDelegate;
    use crate::capabilities::session_tasks::tests::InMemorySessionTaskRegistry;
    use crate::platform_store::tests::MockPlatformStore;
    use everruns_core::connection_services::UserConnectionResolver;
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskRegistry, TaskLinks, TaskMessagePart,
    };
    use everruns_core::tools::{Tool, ToolExecutionResult};
    use everruns_provider::error::Result;
    use everruns_provider::typed_id::SessionId;
    use std::collections::HashSet;
    use std::sync::Arc;
    use uuid::Uuid;

    fn delegate(
        store: Arc<MockPlatformStore>,
    ) -> Arc<dyn everruns_core::subagent_delegation::SubagentSessionDelegate> {
        Arc::new(PlatformStoreSubagentDelegate(store))
    }

    fn target_config(
        agent_id: AgentId,
        harness_id: HarnessId,
        required_connections: Vec<&str>,
    ) -> Value {
        json!({
            "targets": [
                {
                    "id": "aws_operator",
                    "name": "AWS Operator",
                    "description": "Manage fake AWS infrastructure",
                    "agent_id": agent_id,
                    "harness_id": harness_id,
                    "required_connections": required_connections,
                    "required_scopes": ["fake_aws:rds:create"]
                }
            ]
        })
    }

    fn spawn_agent_tool(config: &Value) -> Box<dyn Tool> {
        Box::new(SpawnAgentHandoffTool::new(config))
    }

    struct TestConnectionResolver {
        providers: HashSet<String>,
    }

    #[async_trait]
    impl UserConnectionResolver for TestConnectionResolver {
        async fn get_connection_token(
            &self,
            _session_id: SessionId,
            provider: &str,
        ) -> Result<Option<String>> {
            Ok(self
                .providers
                .contains(provider)
                .then(|| "server-side-secret-token".to_string()))
        }

        async fn get_connection_user(
            &self,
            _session_id: SessionId,
            _provider: &str,
        ) -> Result<Option<Uuid>> {
            Ok(None)
        }

        async fn get_connection_token_for_user(
            &self,
            _user_id: Uuid,
            _provider: &str,
        ) -> Result<Option<String>> {
            Ok(None)
        }
    }

    fn context(
        store: Arc<MockPlatformStore>,
        resolver: Option<Arc<dyn UserConnectionResolver>>,
    ) -> ToolContext {
        let mut context = ToolContext::new(store.session.id);
        context.subagent_delegate = Some(delegate(store));
        context.connection_resolver = resolver;
        context
    }

    // Metadata constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn config_schema_exposes_targets_array() {
        let cap = AgentHandoffCapability;
        let schema = cap.config_schema().expect("config schema");
        assert_eq!(schema["properties"]["targets"]["type"], "array");
    }

    #[test]
    fn capability_no_longer_contributes_legacy_handoff_tools() {
        let cap = AgentHandoffCapability;
        assert!(cap.tools_with_config(&json!({})).is_empty());
    }

    #[test]
    fn uk_localization_resolves() {
        let cap = AgentHandoffCapability;
        assert_eq!(
            cap.localized_name(Some("uk-UA")),
            "Передання роботи агентам"
        );
        assert!(
            cap.localized_description(Some("uk-UA"))
                .contains("Делегує роботу")
        );
        assert!(cap.describe_schema(Some("uk")).is_some());
        assert!(cap.describe_schema(None).is_some());
    }

    #[test]
    fn terminal_handoff_status_maps_only_terminal_wait_states() {
        assert_eq!(
            terminal_handoff_status("idle"),
            Some(SubagentStatus::Completed)
        );
        assert_eq!(
            terminal_handoff_status("error"),
            Some(SubagentStatus::Failed)
        );
        assert_eq!(
            terminal_handoff_status("max_iterations_reached"),
            Some(SubagentStatus::MaxIterationsReached)
        );
        assert_eq!(terminal_handoff_status("waiting_for_tool_results"), None);
        assert_eq!(terminal_handoff_status("paused"), None);
    }

    #[test]
    fn validate_config_rejects_duplicate_targets() {
        let agent_id = AgentId::new();
        let harness_id = HarnessId::new();
        let config = json!({
            "targets": [
                { "id": "dup", "name": "One", "agent_id": agent_id, "harness_id": harness_id },
                { "id": "dup", "name": "Two", "agent_id": AgentId::new(), "harness_id": HarnessId::new() }
            ]
        });

        let error = AgentHandoffCapability
            .validate_config(&config)
            .expect_err("duplicate targets should fail");
        assert!(error.contains("Duplicate handoff target id"));
    }

    #[test]
    fn spawn_agent_schema_advertises_only_agent_target() {
        let tool = SpawnAgentHandoffTool::new(&json!({}));
        let schema = tool.parameters_schema();
        assert_eq!(
            schema["properties"]["target"]["properties"]["type"]["enum"],
            json!(["agent"])
        );
        assert_eq!(
            schema["properties"]["target"]["required"],
            json!(["type", "id"])
        );
        assert_eq!(
            schema["properties"]["mode"]["enum"],
            json!(["background", "foreground", "invite"])
        );
        assert_eq!(
            schema["required"],
            json!(["name", "instructions", "target"])
        );
    }

    #[tokio::test]
    async fn spawn_agent_handoff_requires_configured_connection() {
        let store = Arc::new(MockPlatformStore::new());
        let config = target_config(
            store.agent.public_id,
            store.session.harness_id,
            vec!["fake_aws"],
        );
        let tool = spawn_agent_tool(&config);
        let resolver = Arc::new(TestConnectionResolver {
            providers: HashSet::new(),
        });
        let context = context(store, Some(resolver));

        let result = tool
            .execute_with_context(
                json!({
                    "name": "AWS Operator",
                    "instructions": "Create an RDS database named app-db",
                    "target": { "type": "agent", "id": "aws_operator" },
                    "mode": "foreground"
                }),
                &context,
            )
            .await;

        assert!(matches!(
            result,
            ToolExecutionResult::ConnectionRequired { provider } if provider == "fake_aws"
        ));
    }

    #[tokio::test]
    async fn spawn_agent_handoff_rejects_other_target_types() {
        let store = Arc::new(MockPlatformStore::new());
        let config = target_config(store.agent.public_id, store.session.harness_id, vec![]);
        let tool = spawn_agent_tool(&config);
        let context = context(store, None);

        let result = tool
            .execute_with_context(
                json!({
                    "name": "Wrong Target",
                    "instructions": "Do work",
                    "target": { "type": "subagent" }
                }),
                &context,
            )
            .await;

        assert!(
            matches!(result, ToolExecutionResult::ToolError(message) if message.contains("target.type must be \"agent\""))
        );
    }

    #[tokio::test]
    async fn spawn_agent_handoff_creates_agent_handoff_task() {
        let store = Arc::new(MockPlatformStore::new());
        let resolver = Arc::new(TestConnectionResolver {
            providers: HashSet::from(["fake_aws".to_string()]),
        });
        let config = target_config(
            store.agent.public_id,
            store.session.harness_id,
            vec!["fake_aws"],
        );
        let tool = spawn_agent_tool(&config);
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let mut context = context(store.clone(), Some(resolver));
        context.session_task_registry = Some(registry.clone());

        let result = tool
            .execute_with_context(
                json!({
                    "name": "AWS Operator Run",
                    "instructions": "Create an RDS database named app-db",
                    "target": { "type": "agent", "id": "aws_operator" },
                    "mode": "foreground",
                    "public_context": { "region": "us-east-1" }
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        let task_id = value["task_id"].as_str().expect("task_id");
        let task = registry
            .get(store.session.id, task_id)
            .await
            .expect("task lookup")
            .expect("task");
        assert_eq!(value["mode"], "foreground");
        assert_eq!(value["target"], "aws_operator");
        assert_eq!(task.kind, TASK_KIND_AGENT_HANDOFF);
        assert_eq!(task.display_name, "AWS Operator Run");
        assert_eq!(task.state, SessionTaskState::Succeeded);
        assert_eq!(task.spec["target_id"], "aws_operator");
        assert_eq!(task.spec["mode"], "foreground");
        assert!(task.links.child_session_id.is_some());
    }

    #[tokio::test]
    async fn schema_bound_handoff_requires_report_result() {
        let store = Arc::new(MockPlatformStore::new());
        *store.wait_for_idle_status.lock().unwrap() = "completed".to_string();
        let config = target_config(store.agent.public_id, store.session.harness_id, vec![]);
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let mut context = context(store.clone(), None);
        context.session_task_registry = Some(registry.clone());

        let result = spawn_agent_tool(&config)
            .execute_with_context(
                json!({
                    "name": "Structured handoff",
                    "instructions": "Return structured data",
                    "target": {"type": "agent", "id": "aws_operator"},
                    "mode": "foreground",
                    "result_schema": {
                        "type": "object",
                        "properties": {"answer": {"type": "string"}},
                        "required": ["answer"]
                    }
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected terminal handoff result, got {result:?}");
        };
        let task_id = value["task_id"].as_str().expect("task_id");
        let task = registry
            .get(store.session.id, task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            task.spec[RESULT_SCHEMA_SPEC_KEY]["required"],
            json!(["answer"])
        );
        assert_eq!(task.state, SessionTaskState::Failed);
        assert_eq!(
            task.error.as_ref().map(|error| error.kind.as_str()),
            Some("no_result")
        );
    }

    #[tokio::test]
    async fn handoff_message_schema_is_task_backed_and_wakes_on_activity() {
        let store = Arc::new(MockPlatformStore::new());
        let config = target_config(store.agent.public_id, store.session.harness_id, vec![]);
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let mut context = context(store.clone(), None);
        context.session_task_registry = Some(registry.clone());

        let result = spawn_agent_tool(&config)
            .execute_with_context(
                json!({
                    "name": "Progress handoff",
                    "instructions": "Report progress",
                    "target": {"type": "agent", "id": "aws_operator"},
                    "mode": "background",
                    "message_schema": {
                        "type": "object",
                        "properties": {"step": {"type": "string"}},
                        "required": ["step"]
                    }
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected background handoff, got {result:?}");
        };
        let task_id = value["task_id"].as_str().expect("task_id");
        let task = registry
            .get(store.session.id, task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            task.spec[MESSAGE_SCHEMA_SPEC_KEY]["required"],
            json!(["step"])
        );
        assert_eq!(task.wake_policy, TaskWakePolicy::OnActivity);
    }

    #[tokio::test]
    async fn spawn_agent_handoff_background_returns_task_handle() {
        let store = Arc::new(MockPlatformStore::new());
        let config = target_config(store.agent.public_id, store.session.harness_id, vec![]);
        let tool = spawn_agent_tool(&config);
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let mut context = context(store.clone(), None);
        context.session_task_registry = Some(registry.clone());

        let result = tool
            .execute_with_context(
                json!({
                    "name": "AWS Operator Background",
                    "instructions": "List RDS databases",
                    "target": { "type": "agent", "id": "aws_operator" },
                    "mode": "background"
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        let task_id = value["task_id"].as_str().expect("task_id");
        assert_eq!(value["status"], "running");
        assert_eq!(value["mode"], "background");

        let mut task = registry
            .get(store.session.id, task_id)
            .await
            .expect("task lookup")
            .expect("task");
        assert_eq!(task.kind, TASK_KIND_AGENT_HANDOFF);
        assert_eq!(task.wake_policy, TaskWakePolicy::OnTerminal);
        assert_eq!(task.spec["mode"], "background");

        for _ in 0..20 {
            if task.state.is_terminal() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            task = registry
                .get(store.session.id, task_id)
                .await
                .expect("task lookup")
                .expect("task");
        }

        assert_eq!(task.state, SessionTaskState::Succeeded);
        assert_eq!(task.summary.as_deref(), Some("Hi!"));
    }

    #[tokio::test]
    async fn spawn_agent_handoff_invite_adds_member_participant() {
        let store = Arc::new(MockPlatformStore::new());
        let config = target_config(store.agent.public_id, store.session.harness_id, vec![]);
        let tool = spawn_agent_tool(&config);
        let context = context(store.clone(), None);

        let result = tool
            .execute_with_context(
                json!({
                    "name": "AWS Operator Invite",
                    "instructions": "Join this incident session",
                    "target": { "type": "agent", "id": "aws_operator" },
                    "mode": "invite"
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        assert_eq!(value["mode"], "invite");
        assert_eq!(value["status"], "joined");
        assert_eq!(value["target"], "aws_operator");
        assert!(value["participant_id"].as_str().is_some());

        assert!(
            store
                .created_session_harness_ids
                .lock()
                .expect("recorder lock")
                .is_empty(),
            "invite mode must not create a child session"
        );
        let participants = store
            .joined_participants
            .lock()
            .expect("participants lock")
            .clone();
        assert_eq!(participants.len(), 1);
        assert_eq!(participants[0].agent_id, Some(store.agent.public_id));
        assert_eq!(
            participants[0].role,
            crate::session::SessionParticipantRole::Member
        );
    }

    #[tokio::test]
    async fn spawn_agent_handoff_invite_rejects_inherited_harness_capability_conflict() {
        let mut store_value = MockPlatformStore::new();
        let child_harness_id = HarnessId::new();
        // EVE-881: the delegate returns the effective (inheritance-resolved)
        // definition, so a capability inherited from a parent harness arrives
        // already folded into the child's configuration.
        let mut child_harness = store_value.harness.clone();
        child_harness.id = child_harness_id;
        child_harness.capabilities = vec![everruns_capability::CapabilityRef::with_config(
            "web_fetch",
            json!({"max_bytes": 1024}),
        )];
        store_value.session.harness_id = child_harness_id;
        store_value.agent.capabilities = vec![everruns_capability::CapabilityRef::with_config(
            "web_fetch",
            json!({"max_bytes": 2048}),
        )];
        {
            let mut harnesses = store_value.extra_harnesses.lock().unwrap();
            harnesses.insert(child_harness_id, child_harness);
        }
        let store = Arc::new(store_value);
        let config = target_config(store.agent.public_id, child_harness_id, vec![]);
        let tool = spawn_agent_tool(&config);
        let context = context(store.clone(), None);

        let result = tool
            .execute_with_context(
                json!({
                    "name": "AWS Operator Invite",
                    "instructions": "Join this incident session",
                    "target": { "type": "agent", "id": "aws_operator" },
                    "mode": "invite"
                }),
                &context,
            )
            .await;

        assert!(matches!(result, ToolExecutionResult::ToolError(message)
                if message.contains("Invite-mode handoff cannot join target")
                    && message.contains("capability `web_fetch`")
                    && message.contains("Use background or foreground mode")));
        assert!(
            store
                .joined_participants
                .lock()
                .expect("participants lock")
                .is_empty(),
            "conflicting inherited harness invite must not join the participant"
        );
    }

    #[tokio::test]
    async fn spawn_agent_handoff_invite_rejects_capability_conflict() {
        let mut store_value = MockPlatformStore::new();
        store_value.session.capabilities = vec![everruns_capability::CapabilityRef::with_config(
            "web_fetch",
            json!({"max_bytes": 1024}),
        )];
        store_value.agent.capabilities = vec![everruns_capability::CapabilityRef::with_config(
            "web_fetch",
            json!({"max_bytes": 2048}),
        )];
        let store = Arc::new(store_value);
        let config = target_config(store.agent.public_id, store.session.harness_id, vec![]);
        let tool = spawn_agent_tool(&config);
        let context = context(store.clone(), None);

        let result = tool
            .execute_with_context(
                json!({
                    "name": "AWS Operator Invite",
                    "instructions": "Join this incident session",
                    "target": { "type": "agent", "id": "aws_operator" },
                    "mode": "invite"
                }),
                &context,
            )
            .await;

        assert!(matches!(result, ToolExecutionResult::ToolError(message)
                if message.contains("Invite-mode handoff cannot join target")
                    && message.contains("capability `web_fetch`")
                    && message.contains("Use background or foreground mode")));
        assert!(
            store
                .joined_participants
                .lock()
                .expect("participants lock")
                .is_empty(),
            "conflicting invite must not join the participant"
        );
    }

    /// Regression for the confused-deputy issue this PR fixes: the child
    /// session must be created with the *target's* harness, not the
    /// parent session's harness. If a future refactor reintroduces
    /// `parent_session.harness_id` here, the child would inherit the
    /// parent's mounts/capabilities while gaining the target's tools.
    #[tokio::test]
    async fn spawn_agent_handoff_uses_target_harness_not_parent() {
        let store = Arc::new(MockPlatformStore::new());
        let resolver = Arc::new(TestConnectionResolver {
            providers: HashSet::from(["fake_aws".to_string()]),
        });
        let target_harness_id = HarnessId::new();
        // Sanity: the target harness must differ from the parent's,
        // otherwise the assertion below cannot distinguish them.
        assert_ne!(store.session.harness_id, target_harness_id);

        let config = target_config(store.agent.public_id, target_harness_id, vec!["fake_aws"]);
        let tool = spawn_agent_tool(&config);
        let context = context(store.clone(), Some(resolver));

        let result = tool
            .execute_with_context(
                json!({
                    "name": "AWS Operator Run",
                    "instructions": "Create an RDS database named app-db",
                    "target": { "type": "agent", "id": "aws_operator" },
                    "mode": "foreground"
                }),
                &context,
            )
            .await;
        assert!(result.is_success(), "expected success, got {result:?}");

        let recorded = store
            .created_session_harness_ids
            .lock()
            .expect("recorder lock")
            .clone();
        assert_eq!(
            recorded.len(),
            1,
            "expected exactly one child create_session call, got {recorded:?}"
        );
        assert_eq!(
            recorded[0], target_harness_id,
            "child session must inherit the target harness, not the parent's",
        );
        assert_ne!(
            recorded[0], store.session.harness_id,
            "child session must NOT inherit the parent harness (confused-deputy regression)",
        );
    }

    #[tokio::test]
    async fn agent_handoff_task_executor_delivers_followup() {
        let parent_id = SessionId::new();
        let child_id = SessionId::new();
        let mut store_value = MockPlatformStore::new();
        store_value.session.id = child_id;
        store_value.session.parent_session_id = Some(parent_id);
        let store = Arc::new(store_value);

        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let task = registry
            .create(CreateSessionTask {
                session_id: parent_id,
                id: None,
                kind: TASK_KIND_AGENT_HANDOFF.to_string(),
                display_name: "AWS Operator".to_string(),
                // Handoff tasks use the dedicated `agent_handoff` kind and carry
                // target_id/external_agent_id in spec.
                spec: json!({ "target_id": "aws", "external_agent_id": "agent_aws" }),
                state: SessionTaskState::Running,
                links: TaskLinks {
                    child_session_id: Some(child_id),
                    ..Default::default()
                },
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .expect("create task");
        let message = TaskMessage {
            id: "msg_1".to_string(),
            task_id: task.id.clone(),
            direction: everruns_core::session_task::TaskMessageDirection::Inbound,
            content: vec![TaskMessagePart::text("List RDS databases")],
            in_reply_to: None,
            created_at: chrono::Utc::now(),
        };

        let mut ctx = ToolContext::new(parent_id);
        ctx.subagent_delegate = Some(delegate(store.clone()));
        ctx.session_task_registry = Some(registry);

        AgentHandoffTaskExecutor
            .deliver(&task, &message, &ctx)
            .await
            .expect("follow-up delivered");
        assert_eq!(
            *store.sent_messages.lock().unwrap(),
            vec![(child_id, "List RDS databases".to_string())]
        );
    }

    #[tokio::test]
    async fn agent_handoff_task_executor_reconciles_terminal_child() {
        let parent_id = SessionId::new();
        let child_id = SessionId::new();
        let store = Arc::new(MockPlatformStore::new());
        *store.wait_for_idle_status.lock().unwrap() = "completed".to_string();

        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let task = registry
            .create(CreateSessionTask {
                session_id: parent_id,
                id: None,
                kind: TASK_KIND_AGENT_HANDOFF.to_string(),
                display_name: "AWS Operator".to_string(),
                spec: json!({ "target_id": "aws", "external_agent_id": "agent_aws" }),
                state: SessionTaskState::Running,
                links: TaskLinks {
                    child_session_id: Some(child_id),
                    ..Default::default()
                },
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .expect("create task");

        let mut ctx = ToolContext::new(parent_id);
        ctx.subagent_delegate = Some(delegate(store));
        ctx.session_task_registry = Some(registry.clone());

        AgentHandoffTaskExecutor
            .reconcile(&task, &ctx)
            .await
            .expect("reconcile succeeds");
        let task = registry
            .get(parent_id, &task.id)
            .await
            .expect("task lookup")
            .expect("task");
        assert_eq!(task.state, SessionTaskState::Succeeded);
        assert_eq!(task.summary.as_deref(), Some("Hi!"));
    }
}
