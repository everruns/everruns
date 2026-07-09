// Agent handoff capability.
//
// Decision: handoff starts a real configured Agent in a child session instead
// of using a code-defined blueprint. The source agent gets only handoff tools;
// the target agent owns its own tools, capabilities, data, and runtime prompt.
// Credentials are never accepted as tool arguments or config. Required
// provider connections are resolved server-side before the child session starts.

use super::util::{get_platform_store, require_str_nonblank as require_str};
use super::{Capability, CapabilityLocalization, CapabilityStatus, RiskLevel, SystemPromptContext};
use crate::session::SubagentStatus;
use crate::session_task::{
    CreateSessionTask, SessionTask, SessionTaskState, SessionTaskUpdate, TASK_KIND_AGENT_HANDOFF,
    TaskError, TaskExecutor, TaskExecutorPlugin, TaskLinks, TaskMessage, TaskWakePolicy,
    task_message_text,
};
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use crate::typed_id::{AgentId, HarnessId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

pub const AGENT_HANDOFF_CAPABILITY_ID: &str = "agent_handoff";
const DEFAULT_WAIT_TIMEOUT_SECS: u64 = 300;
const BACKGROUND_WAIT_SLICE_SECS: u64 = 300;
const BACKGROUND_MAX_WAIT_SECS: u64 = 6 * 60 * 60;
const BACKGROUND_HEARTBEAT_INTERVAL_SECS: u64 = 15;
const BACKGROUND_POLL_BACKOFF_SECS: u64 = 5;

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SpawnAgentHandoffMode {
    Background,
    Foreground,
}

impl SpawnAgentHandoffMode {
    fn parse(value: Option<&str>, context: &ToolContext) -> std::result::Result<Self, String> {
        let explicit = match value.map(str::trim).filter(|s| !s.is_empty()) {
            None => None,
            Some("background") => Some(Self::Background),
            Some("foreground") => Some(Self::Foreground),
            Some(other) => {
                return Err(format!(
                    "Invalid mode: \"{other}\". Valid modes: background, foreground."
                ));
            }
        };
        let has_registry = context.session_task_registry.is_some();
        match explicit {
            Some(Self::Background) if !has_registry => Err(
                "Background mode requires a session task registry, which is not available in this environment. Use mode: \"foreground\" instead."
                    .to_string(),
            ),
            Some(mode) => Ok(mode),
            None if has_registry => Ok(Self::Background),
            None => Ok(Self::Foreground),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Foreground => "foreground",
        }
    }
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

fn last_agent_message(messages: &[crate::platform_store::PlatformMessage]) -> Option<String> {
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
    store: &dyn crate::platform_store::PlatformStore,
    child_session_id: crate::typed_id::SessionId,
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
    child_session_id: crate::typed_id::SessionId,
    first_message: String,
    task_id: String,
    task_attempt: i32,
) {
    let context = context.clone();
    tokio::spawn(async move {
        let Some(store) = context.platform_store.clone() else {
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
                    finish_handoff_task(
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
    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Spawn Agent")
    }

    fn description(&self) -> &str {
        "Delegate work to a configured first-party target agent. Set target.type to \"agent\" and target.id to a configured handoff target id. Runs in the background by default when task tracking is available; set mode to \"foreground\" to block for the result."
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
                    "enum": ["background", "foreground"],
                    "description": "Execution mode. \"background\" (default when task tracking is available) returns immediately with a task_id; \"foreground\" blocks until the handoff completes."
                },
                "public_context": {
                    "type": "object",
                    "description": "Non-secret structured context to include with the instructions."
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
        let store = match get_platform_store(context) {
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
        let mode = match SpawnAgentHandoffMode::parse(
            arguments.get("mode").and_then(Value::as_str),
            context,
        ) {
            Ok(mode) => mode,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };

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

        if parent_session.parent_session_id.is_some() {
            return ToolExecutionResult::tool_error(
                "Agent handoffs cannot be started from child sessions.",
            );
        }

        let child_session = match store
            .create_session(
                target.harness_id,
                Some(target.agent_id),
                Some(&name),
                parent_session.locale.as_deref(),
                None,
                None,
                Some(context.session_id),
            )
            .await
        {
            Ok(session) => session,
            Err(error) => return ToolExecutionResult::internal_error(error),
        };

        let handoff_task = child_task(instructions, arguments.get("public_context"));
        let mut task_id = None;
        let mut task_attempt = 1;
        if let Some(task_registry) = &context.session_task_registry {
            match task_registry
                .create(CreateSessionTask {
                    session_id: context.session_id,
                    id: None,
                    kind: TASK_KIND_AGENT_HANDOFF.to_string(),
                    display_name: name.clone(),
                    spec: json!({
                        "target_id": &target.id,
                        "external_agent_id": target.agent_id,
                        "instructions": instructions,
                        "mode": mode.as_str(),
                    }),
                    state: SessionTaskState::Running,
                    links: TaskLinks {
                        child_session_id: Some(child_session.id),
                        ..Default::default()
                    },
                    wake_policy: match mode {
                        SpawnAgentHandoffMode::Background => TaskWakePolicy::OnTerminal,
                        SpawnAgentHandoffMode::Foreground => TaskWakePolicy::Silent,
                    },
                })
                .await
            {
                Ok(task) => {
                    task_attempt = task.attempt;
                    task_id = Some(task.id);
                }
                Err(error) if mode == SpawnAgentHandoffMode::Background => {
                    return ToolExecutionResult::tool_error(format!(
                        "Background spawn_agent could not create its session task, so the handoff was not started: {error}"
                    ));
                }
                Err(_) => {}
            }
        }

        match mode {
            SpawnAgentHandoffMode::Background => {
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
            SpawnAgentHandoffMode::Foreground => {
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
                    finish_handoff_task(
                        context,
                        task_id.as_deref(),
                        state,
                        Some(result.clone()),
                        error,
                        None,
                    )
                    .await;
                }

                ToolExecutionResult::success(json!({
                    "task_id": task_id,
                    "handoff_id": child_session.id.to_string(),
                    "target": target.id,
                    "target_agent_id": target.agent_id,
                    "name": name,
                    "status": status,
                    "result": result,
                    "mode": "foreground",
                }))
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
    ) -> crate::error::Result<()> {
        let Some(store) = context.platform_store.as_ref() else {
            return Err(crate::error::AgentLoopError::tool(
                "agent handoff task delivery requires platform_store context",
            ));
        };
        let Some(child_id) = task.links.child_session_id else {
            return Err(crate::error::AgentLoopError::tool(format!(
                "agent handoff task {} has no child session link",
                task.id
            )));
        };
        let text = task_message_text(&message.content);
        store.send_message(child_id, &text).await
    }

    async fn cancel(&self, task: &SessionTask, context: &ToolContext) -> crate::error::Result<()> {
        let Some(store) = context.platform_store.as_ref() else {
            return Err(crate::error::AgentLoopError::tool(
                "agent handoff task cancellation requires platform_store context",
            ));
        };
        let Some(child_id) = task.links.child_session_id else {
            return Err(crate::error::AgentLoopError::tool(format!(
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
    ) -> crate::error::Result<()> {
        if task.state.is_terminal() {
            return Ok(());
        }
        let (Some(store), Some(child_id)) =
            (context.platform_store.as_ref(), task.links.child_session_id)
        else {
            return Ok(());
        };
        let status = store.wait_for_idle(child_id, Some(0)).await?;
        let Some(terminal) = terminal_handoff_status(&status) else {
            return Ok(());
        };
        let state = handoff_task_state(&terminal);
        let result = handoff_result(store.as_ref(), child_id, &status).await.ok();
        let error = handoff_error(&status, state);
        finish_handoff_task(context, Some(&task.id), state, result, error, None).await;
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
    use crate::Result;
    use crate::capabilities::session_tasks::tests::InMemorySessionTaskRegistry;
    use crate::platform_store::tests::MockPlatformStore;
    use crate::session_task::{CreateSessionTask, SessionTaskRegistry, TaskLinks, TaskMessagePart};
    use crate::tools::{Tool, ToolExecutionResult};
    use crate::traits::UserConnectionResolver;
    use crate::typed_id::SessionId;
    use std::collections::HashSet;
    use std::sync::Arc;
    use uuid::Uuid;

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
        context.platform_store = Some(store);
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
            direction: crate::session_task::TaskMessageDirection::Inbound,
            content: vec![TaskMessagePart::text("List RDS databases")],
            in_reply_to: None,
            created_at: chrono::Utc::now(),
        };

        let mut ctx = ToolContext::new(parent_id);
        ctx.platform_store = Some(store);
        ctx.session_task_registry = Some(registry);

        AgentHandoffTaskExecutor
            .deliver(&task, &message, &ctx)
            .await
            .expect("follow-up delivered");
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
        ctx.platform_store = Some(store);
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
