// Agent handoff capability.
//
// Decision: handoff starts a real configured Agent in a child session instead
// of using a code-defined blueprint. The source agent gets only handoff tools;
// the target agent owns its own tools, capabilities, data, and runtime prompt.
// Credentials are never accepted as tool arguments or config. Required
// provider connections are resolved server-side before the child session starts.

use super::{Capability, CapabilityLocalization, CapabilityStatus, RiskLevel, SystemPromptContext};
use crate::platform_store::PlatformStore;
use crate::session::SubagentStatus;
use crate::session_task::{
    CreateSessionTask, SessionTaskState, SessionTaskUpdate, TASK_KIND_SUBAGENT, TaskError,
    TaskLinks, TaskWakePolicy,
};
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use crate::typed_id::{AgentId, HarnessId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const AGENT_HANDOFF_CAPABILITY_ID: &str = "agent_handoff";
const DEFAULT_WAIT_TIMEOUT_SECS: u64 = 300;

fn terminal_handoff_status(wait_status: &str) -> Option<SubagentStatus> {
    match wait_status {
        "idle" | "completed" => Some(SubagentStatus::Completed),
        "error" | "failed" => Some(SubagentStatus::Failed),
        "cancelled" => Some(SubagentStatus::Cancelled),
        "max_iterations_reached" => Some(SubagentStatus::MaxIterationsReached),
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
                                "description": "Stable target key used in start_agent_handoff."
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
                                        "description": "Стабільний ключ цілі, що використовується у start_agent_handoff."
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
        let config = AgentHandoffConfig::from_value(config).unwrap_or_default();
        vec![
            Box::new(StartAgentHandoffTool::new(config.clone())),
            Box::new(GetAgentHandoffsTool),
            Box::new(MessageAgentHandoffTool),
        ]
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
Use start_agent_handoff to delegate work to configured first-party agents.\n\
Never ask the user to paste provider tokens into chat or pass credentials in tool arguments.\n\
If a required provider connection is missing, the handoff tool will return a connection_required result and the client should collect credentials through the Connections flow.\n\
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
enum AgentHandoffMode {
    Wait,
}

impl AgentHandoffMode {
    fn parse(value: Option<&str>) -> std::result::Result<Self, String> {
        match value.unwrap_or("wait") {
            "wait" => Ok(Self::Wait),
            other => Err(format!("Invalid mode: {other}. Only wait is supported")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Wait => "wait",
        }
    }
}

fn get_platform_store(context: &ToolContext) -> Result<&dyn PlatformStore, ToolExecutionResult> {
    context
        .platform_store
        .as_ref()
        .map(|store| store.as_ref())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error("Agent handoff tools require platform_store context.")
        })
}

fn require_str<'a>(args: &'a Value, field: &str) -> Result<&'a str, ToolExecutionResult> {
    args.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!("Missing required parameter: {field}"))
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

fn last_agent_message(messages: &[crate::platform_store::PlatformMessage]) -> Option<String> {
    messages
        .iter()
        .rfind(|message| message.role == "agent" || message.role == "assistant")
        .map(|message| message.content.clone())
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

pub struct StartAgentHandoffTool {
    config: AgentHandoffConfig,
}

impl StartAgentHandoffTool {
    fn new(config: AgentHandoffConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for StartAgentHandoffTool {
    fn name(&self) -> &str {
        "start_agent_handoff"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Start Agent Handoff")
    }

    fn description(&self) -> &str {
        "Start an authenticated handoff to a configured first-party target agent. Credentials are resolved through Connections, never passed as arguments."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "Configured handoff target id."
                },
                "task": {
                    "type": "string",
                    "description": "Work request for the target agent. Do not include credentials or bearer tokens."
                },
                "public_context": {
                    "type": "object",
                    "description": "Non-secret structured context to include with the task."
                }
            },
            "required": ["target", "task"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "start_agent_handoff requires context. This tool must be executed with session context.",
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

        let target_id = match require_str(&arguments, "target") {
            Ok(value) => value,
            Err(error) => return error,
        };
        let task = match require_str(&arguments, "task") {
            Ok(value) => value,
            Err(error) => return error,
        };
        let mode = match AgentHandoffMode::parse(arguments.get("mode").and_then(Value::as_str)) {
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
                Some(&target.name),
                parent_session.locale.as_deref(),
                None,
                None,
            )
            .await
        {
            Ok(session) => session,
            Err(error) => return ToolExecutionResult::internal_error(error),
        };

        let handoff_task = child_task(task, arguments.get("public_context"));
        let child_session = match store
            .set_subagent_metadata(
                child_session.id,
                context.session_id,
                &target.name,
                &handoff_task,
                SubagentStatus::Running,
            )
            .await
        {
            Ok(session) => session,
            Err(error) => return ToolExecutionResult::internal_error(error),
        };

        // Register a subagent-kind session task so the handoff appears in the
        // task registry alongside other work-shaped tasks (specs/session-tasks.md).
        let handoff_task_id: Option<String> =
            if let Some(task_registry) = &context.session_task_registry {
                task_registry
                    .create(CreateSessionTask {
                        session_id: context.session_id,
                        id: None,
                        kind: TASK_KIND_SUBAGENT.to_string(),
                        display_name: target.name.clone(),
                        spec: json!({
                            "target_id": &target.id,
                            "external_agent_id": target.agent_id,
                            "mode": mode.as_str(),
                        }),
                        state: SessionTaskState::Running,
                        links: TaskLinks {
                            child_session_id: Some(child_session.id),
                            ..Default::default()
                        },
                        wake_policy: TaskWakePolicy::Silent,
                    })
                    .await
                    .ok()
                    .map(|t| t.id)
            } else {
                None
            };

        if let Err(error) = store.send_message(child_session.id, &handoff_task).await {
            return ToolExecutionResult::internal_error(error);
        }

        let status = match store
            .wait_for_idle(child_session.id, Some(DEFAULT_WAIT_TIMEOUT_SECS))
            .await
        {
            Ok(status) => status,
            Err(error) => {
                return ToolExecutionResult::success(json!({
                    "handoff_id": child_session.id.to_string(),
                    "target": target.id,
                    "target_agent_id": target.agent_id,
                    "name": target.name,
                    "status": "failed",
                    "error": error.to_string(),
                    "mode": "wait",
                }));
            }
        };

        let messages = match store.get_messages(child_session.id, Some(5)).await {
            Ok(messages) => messages,
            Err(error) => return ToolExecutionResult::internal_error(error),
        };
        let result = last_agent_message(&messages)
            .unwrap_or_else(|| format!("Handoff completed with status: {status}"));

        if let Some(subagent_status) = terminal_handoff_status(&status) {
            if let Err(error) = store
                .set_subagent_metadata(
                    child_session.id,
                    context.session_id,
                    &target.name,
                    &handoff_task,
                    subagent_status.clone(),
                )
                .await
            {
                tracing::warn!(
                    session_id = %context.session_id,
                    child_session_id = %child_session.id,
                    error = %error,
                    "failed to persist terminal handoff metadata"
                );
            }

            // Mirror terminal state into the session task (best-effort).
            if let (Some(registry), Some(task_id)) =
                (&context.session_task_registry, &handoff_task_id)
            {
                let task_state = match subagent_status {
                    SubagentStatus::Completed => SessionTaskState::Succeeded,
                    SubagentStatus::Failed | SubagentStatus::MaxIterationsReached => {
                        SessionTaskState::Failed
                    }
                    SubagentStatus::Cancelled => SessionTaskState::Canceled,
                    SubagentStatus::Running | SubagentStatus::Spawning => SessionTaskState::Running,
                };
                let error = if task_state == SessionTaskState::Failed {
                    Some(TaskError {
                        kind: "handoff_failed".to_string(),
                        message: format!("Handoff ended with status: {status}"),
                    })
                } else {
                    None
                };
                let _ = registry
                    .update(
                        context.session_id,
                        task_id,
                        SessionTaskUpdate {
                            state: Some(task_state),
                            summary: Some(result.clone()),
                            error,
                            ..Default::default()
                        },
                    )
                    .await;
            }
        }

        ToolExecutionResult::success(json!({
            "handoff_id": child_session.id.to_string(),
            "target": target.id,
            "target_agent_id": target.agent_id,
            "name": target.name,
            "status": status,
            "result": result,
            "mode": "wait",
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct GetAgentHandoffsTool;

#[async_trait]
impl Tool for GetAgentHandoffsTool {
    fn name(&self) -> &str {
        "get_agent_handoffs"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Get Agent Handoffs")
    }

    fn description(&self) -> &str {
        "List agent handoffs started by the current session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "handoff_id": {
                    "type": "string",
                    "description": "Optional child session id to retrieve."
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "get_agent_handoffs requires context. This tool must be executed with session context.",
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
        let sessions = match store.list_sessions(Some(100), None).await {
            Ok(sessions) => sessions,
            Err(error) => return ToolExecutionResult::internal_error(error),
        };
        let handoff_id = arguments.get("handoff_id").and_then(Value::as_str);
        let handoffs = sessions
            .into_iter()
            .filter(|session| session.parent_session_id == Some(context.session_id))
            .filter(|session| handoff_id.is_none_or(|id| session.id.to_string() == id))
            .map(|session| {
                json!({
                    "handoff_id": session.id.to_string(),
                    "name": session.subagent_name,
                    "status": session.subagent_status,
                    "target_agent_id": session.agent_id,
                    "created_at": session.created_at,
                    "updated_at": session.updated_at,
                })
            })
            .collect::<Vec<_>>();
        ToolExecutionResult::success(json!({ "handoffs": handoffs }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct MessageAgentHandoffTool;

#[async_trait]
impl Tool for MessageAgentHandoffTool {
    fn name(&self) -> &str {
        "message_agent_handoff"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Message Agent Handoff")
    }

    fn description(&self) -> &str {
        "Send follow-up input to an existing agent handoff child session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "handoff_id": {
                    "type": "string",
                    "description": "Child session id returned by start_agent_handoff."
                },
                "message": {
                    "type": "string",
                    "description": "Follow-up message. Do not include credentials or bearer tokens."
                }
            },
            "required": ["handoff_id", "message"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "message_agent_handoff requires context. This tool must be executed with session context.",
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
        let handoff_id = match require_str(&arguments, "handoff_id") {
            Ok(value) => value,
            Err(error) => return error,
        };
        let message = match require_str(&arguments, "message") {
            Ok(value) => value,
            Err(error) => return error,
        };
        let child_id = match handoff_id.parse::<crate::typed_id::SessionId>() {
            Ok(id) => id,
            Err(_) => {
                return ToolExecutionResult::tool_error(format!(
                    "Invalid handoff_id: {handoff_id}"
                ));
            }
        };

        let child = match store.get_session_by_id(child_id).await {
            Ok(Some(session)) => session,
            Ok(None) => return ToolExecutionResult::tool_error("Agent handoff not found"),
            Err(error) => return ToolExecutionResult::internal_error(error),
        };
        if child.parent_session_id != Some(context.session_id) {
            return ToolExecutionResult::tool_error(
                "Agent handoff does not belong to the current session.",
            );
        }

        if let Err(error) = store.send_message(child_id, message).await {
            return ToolExecutionResult::internal_error(error);
        }
        let status = match store
            .wait_for_idle(child_id, Some(DEFAULT_WAIT_TIMEOUT_SECS))
            .await
        {
            Ok(status) => status,
            Err(error) => return ToolExecutionResult::internal_error(error),
        };
        let messages = match store.get_messages(child_id, Some(5)).await {
            Ok(messages) => messages,
            Err(error) => return ToolExecutionResult::internal_error(error),
        };
        let result = last_agent_message(&messages)
            .unwrap_or_else(|| format!("Handoff processed message with status: {status}"));

        ToolExecutionResult::success(json!({
            "handoff_id": child_id.to_string(),
            "status": status,
            "result": result,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;
    use crate::platform_store::tests::MockPlatformStore;
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

    fn start_tool(config: Value) -> Box<dyn Tool> {
        AgentHandoffCapability
            .tools_with_config(&config)
            .into_iter()
            .find(|tool| tool.name() == "start_agent_handoff")
            .expect("start_agent_handoff tool")
    }

    fn get_tool(config: Value) -> Box<dyn Tool> {
        AgentHandoffCapability
            .tools_with_config(&config)
            .into_iter()
            .find(|tool| tool.name() == "get_agent_handoffs")
            .expect("get_agent_handoffs tool")
    }

    fn message_tool(config: Value) -> Box<dyn Tool> {
        AgentHandoffCapability
            .tools_with_config(&config)
            .into_iter()
            .find(|tool| tool.name() == "message_agent_handoff")
            .expect("message_agent_handoff tool")
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

    #[test]
    fn capability_metadata_and_schema() {
        let cap = AgentHandoffCapability;
        assert_eq!(cap.id(), AGENT_HANDOFF_CAPABILITY_ID);
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.risk_level(), RiskLevel::High);
        assert_eq!(cap.features(), vec!["agent_handoffs"]);

        let schema = cap.config_schema().expect("config schema");
        assert_eq!(schema["properties"]["targets"]["type"], "array");
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

    #[tokio::test]
    async fn start_handoff_requires_configured_connection() {
        let store = Arc::new(MockPlatformStore::new());
        let config = target_config(
            store.agent.public_id,
            store.session.harness_id,
            vec!["fake_aws"],
        );
        let tool = start_tool(config);
        let resolver = Arc::new(TestConnectionResolver {
            providers: HashSet::new(),
        });
        let context = context(store, Some(resolver));

        let result = tool
            .execute_with_context(
                json!({
                    "target": "aws_operator",
                    "task": "Create an RDS database named app-db"
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
    async fn start_handoff_without_connection_resolver_is_internal_error() {
        let store = Arc::new(MockPlatformStore::new());
        let config = target_config(
            store.agent.public_id,
            store.session.harness_id,
            vec!["fake_aws"],
        );
        let tool = start_tool(config);
        let context = context(store, None);

        let result = tool
            .execute_with_context(
                json!({
                    "target": "aws_operator",
                    "task": "Create an RDS database named app-db"
                }),
                &context,
            )
            .await;

        assert!(matches!(result, ToolExecutionResult::InternalError(_)));
    }

    #[tokio::test]
    async fn start_handoff_creates_child_session_for_target_agent() {
        let store = Arc::new(MockPlatformStore::new());
        let resolver = Arc::new(TestConnectionResolver {
            providers: HashSet::from(["fake_aws".to_string()]),
        });
        let config = target_config(
            store.agent.public_id,
            store.session.harness_id,
            vec!["fake_aws"],
        );
        let tool = start_tool(config);
        let context = context(store.clone(), Some(resolver));

        let result = tool
            .execute_with_context(
                json!({
                    "target": "aws_operator",
                    "task": "Create an RDS database named app-db",
                    "public_context": { "region": "us-east-1" }
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        assert_eq!(value["target"], "aws_operator");
        assert_eq!(value["target_agent_id"], json!(store.agent.public_id));
        assert_eq!(value["result"], "Hi!");
    }

    /// Regression for the confused-deputy issue this PR fixes: the child
    /// session must be created with the *target's* harness, not the
    /// parent session's harness. If a future refactor reintroduces
    /// `parent_session.harness_id` here, the child would inherit the
    /// parent's mounts/capabilities while gaining the target's tools.
    #[tokio::test]
    async fn start_handoff_uses_target_harness_not_parent() {
        let store = Arc::new(MockPlatformStore::new());
        let resolver = Arc::new(TestConnectionResolver {
            providers: HashSet::from(["fake_aws".to_string()]),
        });
        let target_harness_id = HarnessId::new();
        // Sanity: the target harness must differ from the parent's,
        // otherwise the assertion below cannot distinguish them.
        assert_ne!(store.session.harness_id, target_harness_id);

        let config = target_config(store.agent.public_id, target_harness_id, vec!["fake_aws"]);
        let tool = start_tool(config);
        let context = context(store.clone(), Some(resolver));

        let result = tool
            .execute_with_context(
                json!({
                    "target": "aws_operator",
                    "task": "Create an RDS database named app-db"
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
    async fn get_handoffs_lists_child_sessions_as_handoffs() {
        // The get_agent_handoffs tool uses list_sessions filtered by parent_session_id.
        // Set up a mock store whose session record already represents a child session
        // so list_sessions returns it in the filtered results.
        let parent_id = SessionId::new();
        let mut store_value = MockPlatformStore::new();
        store_value.session.parent_session_id = Some(parent_id);
        store_value.session.subagent_name = Some("AWS Operator".to_string());
        store_value.session.subagent_status = Some(SubagentStatus::Completed);
        let store = Arc::new(store_value);
        let config = target_config(store.agent.public_id, store.session.harness_id, vec![]);
        let get = get_tool(config);

        let mut ctx = ToolContext::new(parent_id);
        ctx.platform_store = Some(store);

        let result = get.execute_with_context(json!({}), &ctx).await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        let handoffs = value["handoffs"].as_array().expect("handoffs");
        assert_eq!(handoffs.len(), 1);
        assert_eq!(handoffs[0]["name"], "AWS Operator");
        assert_eq!(handoffs[0]["status"], "completed");
    }

    #[tokio::test]
    async fn message_handoff_rejects_invalid_handoff_id() {
        let store = Arc::new(MockPlatformStore::new());
        let tool = message_tool(json!({}));
        let context = context(store, None);

        let result = tool
            .execute_with_context(
                json!({
                    "handoff_id": "not-a-session-id",
                    "message": "List RDS databases"
                }),
                &context,
            )
            .await;

        assert!(
            matches!(result, ToolExecutionResult::ToolError(message) if message.contains("Invalid handoff_id"))
        );
    }

    #[tokio::test]
    async fn message_handoff_rejects_non_child_session() {
        let parent_id = SessionId::new();
        let child_id = SessionId::new();
        let mut store_value = MockPlatformStore::new();
        store_value.session.id = child_id;
        store_value.session.parent_session_id = None;
        let store = Arc::new(store_value);
        let tool = message_tool(json!({}));
        let mut context = ToolContext::new(parent_id);
        context.platform_store = Some(store);

        let result = tool
            .execute_with_context(
                json!({
                    "handoff_id": child_id.to_string(),
                    "message": "List RDS databases"
                }),
                &context,
            )
            .await;

        assert!(
            matches!(result, ToolExecutionResult::ToolError(message) if message.contains("does not belong"))
        );
    }

    #[tokio::test]
    async fn message_handoff_sends_followup_to_owned_child_session() {
        let parent_id = SessionId::new();
        let child_id = SessionId::new();
        let mut store_value = MockPlatformStore::new();
        store_value.session.id = child_id;
        store_value.session.parent_session_id = Some(parent_id);
        let store = Arc::new(store_value);
        let tool = message_tool(json!({}));
        let mut context = ToolContext::new(parent_id);
        context.platform_store = Some(store);

        let result = tool
            .execute_with_context(
                json!({
                    "handoff_id": child_id.to_string(),
                    "message": "List RDS databases"
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        assert_eq!(value["handoff_id"], child_id.to_string());
        assert_eq!(value["status"], "idle");
        assert_eq!(value["result"], "Hi!");
    }
}
