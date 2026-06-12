// Outbound A2A agent delegation.
//
// Decision: V1 is outbound-only and stores configured external agents in the
// capability config. Run state is persisted as session resources of kind
// `agent_run`, which gives agents and UI a unified local/remote delegation
// handle without introducing inbound A2A app-channel exposure.

use super::{Capability, CapabilityLocalization, CapabilityStatus, RiskLevel, SystemPromptContext};
use crate::session_resource::{
    RegisterSessionResource, SessionResourceFilter, SessionResourceStatus,
};
use crate::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskState, SessionTaskUpdate,
    TASK_KIND_EXTERNAL_AGENT, TaskError, TaskExecutor, TaskExecutorPlugin, TaskInputRequest,
    TaskLinks, TaskMessage, TaskWakePolicy, task_message_text,
};
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::{SessionResourceRegistry, ToolContext};
use crate::{Result, validate_safe_url};
use a2a::{
    AgentCard, CancelTaskRequest, GetTaskRequest, Message, Part, Role, SendMessageConfiguration,
    SendMessageRequest, SendMessageResponse, Task, TaskState,
};
use a2a_client::A2AClientFactory;
use a2a_client::agent_card::AgentCardResolver;
use a2a_client::middleware::CallInterceptor;
use a2a_client::transport::ServiceParams;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{Instant, sleep};
use url::Url;

pub const A2A_AGENT_DELEGATION_CAPABILITY_ID: &str = "a2a_agent_delegation";
const AGENT_RUN_KIND: &str = "agent_run";
const DEFAULT_WAIT_TIMEOUT_SECS: u64 = 300;
const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;
const MAX_RESULT_CHARS: usize = 8_192;

/// Outbound A2A delegation capability.
pub struct A2aAgentDelegationCapability;

#[async_trait]
impl Capability for A2aAgentDelegationCapability {
    fn id(&self) -> &str {
        A2A_AGENT_DELEGATION_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "A2A Agent Delegation"
    }

    fn description(&self) -> &str {
        "Delegate work to configured external agents over the A2A protocol."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("send")
    }

    fn category(&self) -> Option<&str> {
        Some("Orchestration")
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["agent_runs"]
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "agents": {
                    "type": "array",
                    "title": "External agents",
                    "description": "External A2A agents available for delegation.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "title": "Agent ID",
                                "description": "Stable ID used in spawn_agent target.external_agent_id."
                            },
                            "name": {
                                "type": "string",
                                "title": "Name",
                                "description": "Human-readable name of the external agent."
                            },
                            "description": {
                                "type": "string",
                                "title": "Description",
                                "description": "Optional description of what the external agent does."
                            },
                            "base_url": {
                                "type": "string",
                                "title": "Base URL",
                                "description": "Base URL for AgentCard discovery. The client fetches /.well-known/agent-card.json."
                            },
                            "agent_card": {
                                "type": "object",
                                "title": "Agent card",
                                "description": "Optional cached/inline AgentCard. If omitted, base_url discovery is used."
                            },
                            "headers": {
                                "type": "object",
                                "title": "Headers",
                                "additionalProperties": { "type": "string" },
                                "description": "Non-secret static headers to send to the A2A endpoint."
                            },
                            "preferred_binding": {
                                "type": "string",
                                "title": "Preferred transport",
                                "description": "Optional transport preference.",
                                "oneOf": [
                                    { "const": "JSONRPC", "title": "JSON-RPC" },
                                    { "const": "HTTP+JSON", "title": "HTTP+JSON" }
                                ]
                            },
                            "poll_interval_ms": {
                                "type": "integer",
                                "title": "Poll interval (ms)",
                                "description": "Polling interval for remote task status, in milliseconds.",
                                "minimum": 100,
                                "maximum": 60000
                            },
                            "allow_local_urls": {
                                "type": "boolean",
                                "title": "Allow local URLs",
                                "description": "Testing/dev escape hatch for localhost A2A agents. Keep false in production.",
                                "default": false
                            }
                        },
                        "required": ["id", "name"],
                        "additionalProperties": false
                    },
                    "default": []
                }
            },
            "additionalProperties": false
        }))
    }

    fn validate_config(&self, config: &Value) -> std::result::Result<(), String> {
        let parsed = A2aDelegationConfig::from_value(config)
            .map_err(|e| format!("invalid a2a_agent_delegation config: {e}"))?;
        for agent in parsed.agents {
            agent.validate()?;
        }
        Ok(())
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Defines the external A2A agents this agent may delegate work to and \
                     how to reach them.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Делегування агентам A2A"),
                description: Some(
                    "Делегує роботу налаштованим зовнішнім агентам за протоколом A2A.",
                ),
                config_description: Some(
                    "Визначає зовнішніх агентів A2A, яким цей агент може делегувати роботу, та параметри підключення до них.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        "agents": {
                            "title": "Зовнішні агенти",
                            "description": "Зовнішні агенти A2A, доступні для делегування.",
                            "items": {
                                "properties": {
                                    "id": {
                                        "title": "Ідентифікатор агента",
                                        "description": "Стабільний ідентифікатор, що використовується у spawn_agent (target.external_agent_id)."
                                    },
                                    "name": {
                                        "title": "Назва",
                                        "description": "Зрозуміла людині назва зовнішнього агента."
                                    },
                                    "description": {
                                        "title": "Опис",
                                        "description": "Необов'язковий опис того, що робить зовнішній агент."
                                    },
                                    "base_url": {
                                        "title": "Базовий URL",
                                        "description": "Базовий URL для виявлення AgentCard. Клієнт завантажує /.well-known/agent-card.json."
                                    },
                                    "agent_card": {
                                        "title": "AgentCard",
                                        "description": "Необов'язковий кешований або вбудований AgentCard. Якщо не задано, використовується виявлення через base_url."
                                    },
                                    "headers": {
                                        "title": "Заголовки",
                                        "description": "Несекретні статичні заголовки, що надсилаються на кінцеву точку A2A."
                                    },
                                    "preferred_binding": {
                                        "title": "Бажаний транспорт",
                                        "description": "Необов'язкове налаштування транспорту.",
                                        "enum_labels": {
                                            "JSONRPC": "JSON-RPC",
                                            "HTTP+JSON": "HTTP+JSON"
                                        }
                                    },
                                    "poll_interval_ms": {
                                        "title": "Інтервал опитування (мс)",
                                        "description": "Інтервал опитування стану віддаленої задачі в мілісекундах."
                                    },
                                    "allow_local_urls": {
                                        "title": "Дозволити локальні URL",
                                        "description": "Обхідний шлях для тестування та розробки з локальними агентами A2A. У продакшені тримайте вимкненим."
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
        let config = A2aDelegationConfig::from_value(config).unwrap_or_default();
        vec![
            Box::new(SpawnAgentTool::new(config.clone())),
            Box::new(GetAgentRunsTool),
            Box::new(WaitAgentTool::new(config.clone())),
            Box::new(MessageAgentTool::new(config.clone())),
            Box::new(CancelAgentTool::new(config)),
        ]
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&Value::Null)
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    async fn system_prompt_contribution_with_config(
        &self,
        _ctx: &SystemPromptContext,
        config: &Value,
    ) -> Option<String> {
        let config = A2aDelegationConfig::from_value(config).unwrap_or_default();
        let agents = config
            .agents
            .iter()
            .map(|agent| {
                format!(
                    "- {} ({}) — {}",
                    agent.name,
                    agent.id,
                    agent.description.as_deref().unwrap_or("External A2A agent")
                )
            })
            .collect::<Vec<_>>();

        Some(format!(
            "<capability id=\"{}\">\n\
Delegate work to configured external A2A agents with spawn_agent.\n\
Use mode=\"background\" for long-running work; use wait_agent later for results.\n\
Use message_agent for follow-up input or input_required tasks; use cancel_agent to stop a remote task.\n\
Available external agents:\n{}\n\
</capability>",
            self.id(),
            if agents.is_empty() {
                "- none configured".to_string()
            } else {
                agents.join("\n")
            }
        ))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct A2aDelegationConfig {
    #[serde(default)]
    agents: Vec<ExternalA2aAgentConfig>,
}

impl A2aDelegationConfig {
    fn from_value(value: &Value) -> serde_json::Result<Self> {
        if value.is_null() {
            Ok(Self::default())
        } else {
            serde_json::from_value(value.clone())
        }
    }

    fn agent(&self, id: &str) -> Option<&ExternalA2aAgentConfig> {
        self.agents.iter().find(|agent| agent.id == id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExternalA2aAgentConfig {
    id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    agent_card: Option<AgentCard>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    preferred_binding: Option<String>,
    #[serde(default)]
    poll_interval_ms: Option<u64>,
    #[serde(default)]
    allow_local_urls: bool,
}

impl ExternalA2aAgentConfig {
    fn validate(&self) -> std::result::Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("A2A agent id cannot be empty".to_string());
        }
        if self.name.trim().is_empty() {
            return Err(format!("A2A agent {} name cannot be empty", self.id));
        }
        if self.base_url.is_none() && self.agent_card.is_none() {
            return Err(format!(
                "A2A agent {} requires base_url or agent_card",
                self.id
            ));
        }
        if let Some(binding) = &self.preferred_binding
            && binding != "JSONRPC"
            && binding != "HTTP+JSON"
        {
            return Err(format!(
                "A2A agent {} preferred_binding must be JSONRPC or HTTP+JSON",
                self.id
            ));
        }
        if let Some(interval) = self.poll_interval_ms
            && !(100..=60_000).contains(&interval)
        {
            return Err(format!(
                "A2A agent {} poll_interval_ms must be between 100 and 60000",
                self.id
            ));
        }
        if let Some(base_url) = &self.base_url {
            if self.allow_local_urls {
                validate_http_url(base_url)
                    .map_err(|e| format!("A2A agent {} has invalid base_url: {e}", self.id))?;
            } else {
                validate_safe_url(base_url)
                    .map_err(|e| format!("A2A agent {} has unsafe base_url: {e}", self.id))?;
            }
        }
        if let Some(card) = &self.agent_card {
            self.validate_card(card)?;
        }
        Ok(())
    }

    fn validate_card(&self, card: &AgentCard) -> std::result::Result<(), String> {
        for iface in &card.supported_interfaces {
            if self.allow_local_urls {
                validate_http_url(&iface.url)
                    .map_err(|e| format!("A2A agent {} has invalid interface URL: {e}", self.id))?;
            } else {
                validate_safe_url(&iface.url)
                    .map_err(|e| format!("A2A agent {} has unsafe interface URL: {e}", self.id))?;
            }
        }
        Ok(())
    }

    async fn resolve_card(&self) -> std::result::Result<AgentCard, String> {
        self.validate()?;
        if let Some(card) = &self.agent_card {
            return Ok(card.clone());
        }
        let base_url = self
            .base_url
            .as_deref()
            .ok_or_else(|| format!("A2A agent {} has no base_url", self.id))?;
        let card = AgentCardResolver::new(None)
            .resolve(base_url)
            .await
            .map_err(|e| format!("Failed to resolve A2A AgentCard: {e}"))?;
        self.validate_card(&card)?;
        Ok(card)
    }
}

fn validate_http_url(raw_url: &str) -> std::result::Result<(), String> {
    let url = Url::parse(raw_url).map_err(|e| e.to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("disallowed scheme {other}; expected http or https")),
    }
    if url.host_str().is_none() {
        return Err("URL must have a hostname".to_string());
    }
    Ok(())
}

#[derive(Clone)]
struct StaticHeaderInterceptor {
    headers: Vec<(String, String)>,
}

#[async_trait]
impl CallInterceptor for StaticHeaderInterceptor {
    async fn before(
        &self,
        _method: &str,
        params: &mut ServiceParams,
    ) -> std::result::Result<(), a2a::A2AError> {
        for (name, value) in &self.headers {
            params.entry(name.clone()).or_default().push(value.clone());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AgentRunStatus {
    Submitted,
    Working,
    InputRequired,
    AuthRequired,
    Completed,
    Failed,
    Canceled,
    Rejected,
}

impl AgentRunStatus {
    fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Canceled | Self::Rejected
        )
    }
}

impl std::fmt::Display for AgentRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Submitted => "submitted",
            Self::Working => "working",
            Self::InputRequired => "input_required",
            Self::AuthRequired => "auth_required",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Rejected => "rejected",
        };
        write!(f, "{value}")
    }
}

impl From<&TaskState> for AgentRunStatus {
    fn from(state: &TaskState) -> Self {
        match state {
            TaskState::Submitted | TaskState::Unspecified => Self::Submitted,
            TaskState::Working => Self::Working,
            TaskState::InputRequired => Self::InputRequired,
            TaskState::AuthRequired => Self::AuthRequired,
            TaskState::Completed => Self::Completed,
            TaskState::Failed => Self::Failed,
            TaskState::Canceled => Self::Canceled,
            TaskState::Rejected => Self::Rejected,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AgentRunMode {
    Wait,
    Background,
}

impl AgentRunMode {
    fn parse(value: Option<&str>) -> std::result::Result<Self, String> {
        match value.unwrap_or("wait") {
            "wait" => Ok(Self::Wait),
            "background" => Ok(Self::Background),
            other => Err(format!(
                "Invalid mode: {other}. Expected wait or background"
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentRunRecord {
    run_id: String,
    kind: String,
    external_agent_id: String,
    external_agent_name: String,
    task: String,
    mode: AgentRunMode,
    status: AgentRunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_context_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_remote_task_snapshot: Option<Value>,
    #[serde(default)]
    wake_on_completion: bool,
    /// Session task mirroring this run (specs/session-tasks.md). Absent on
    /// records that predate the task registry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    /// Snapshot of the agent config used to create the run, so the task
    /// executor can rebuild the A2A client without the capability config.
    /// Headers are non-secret by config-schema contract, so persisting them
    /// in resource metadata is safe. Absent on old records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_config: Option<ExternalA2aAgentConfig>,
}

impl AgentRunRecord {
    fn new(
        run_id: String,
        agent: &ExternalA2aAgentConfig,
        task: String,
        mode: AgentRunMode,
        wake_on_completion: bool,
    ) -> Self {
        Self {
            run_id,
            kind: "external_a2a".to_string(),
            external_agent_id: agent.id.clone(),
            external_agent_name: agent.name.clone(),
            task,
            mode,
            status: AgentRunStatus::Submitted,
            remote_task_id: None,
            remote_context_id: None,
            result: None,
            result_path: None,
            error: None,
            last_remote_task_snapshot: None,
            wake_on_completion,
            task_id: None,
            agent_config: Some(agent.clone()),
        }
    }

    fn resource_status(&self) -> SessionResourceStatus {
        match self.status {
            AgentRunStatus::Completed => SessionResourceStatus::Completed,
            AgentRunStatus::Failed | AgentRunStatus::Canceled | AgentRunStatus::Rejected => {
                SessionResourceStatus::Failed
            }
            _ => SessionResourceStatus::Active,
        }
    }

    fn public_json(&self) -> Value {
        json!({
            "agent_run_id": self.run_id,
            "kind": self.kind,
            "external_agent_id": self.external_agent_id,
            "external_agent_name": self.external_agent_name,
            "task": self.task,
            "mode": self.mode,
            "status": self.status,
            "remote_task_id": self.remote_task_id,
            "remote_context_id": self.remote_context_id,
            "result": self.result,
            "result_path": self.result_path,
            "error": self.error,
            "wake_on_completion": self.wake_on_completion,
            "task_id": self.task_id,
        })
    }
}

fn run_id() -> String {
    format!("agrun_{}", uuid::Uuid::now_v7().simple())
}

fn require_registry(
    context: &ToolContext,
) -> std::result::Result<&Arc<dyn SessionResourceRegistry>, ToolExecutionResult> {
    context.session_resource_registry.as_ref().ok_or_else(|| {
        ToolExecutionResult::tool_error(
            "Agent delegation tools require session_resource_registry context",
        )
    })
}

fn require_str<'a>(
    args: &'a Value,
    field: &str,
) -> std::result::Result<&'a str, ToolExecutionResult> {
    args.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!("Missing required parameter: {field}"))
        })
}

async fn save_run(context: &ToolContext, record: &AgentRunRecord) -> Result<()> {
    mirror_run_to_task(context, record).await;
    let Some(registry) = &context.session_resource_registry else {
        return Ok(());
    };
    registry
        .register(RegisterSessionResource {
            session_id: context.session_id,
            resource_id: record.run_id.clone(),
            kind: AGENT_RUN_KIND.to_string(),
            display_name: record.external_agent_name.clone(),
            status: record.resource_status(),
            metadata: serde_json::to_value(record).unwrap_or_else(|_| json!({})),
        })
        .await?;
    Ok(())
}

/// A2A run status → session task state (specs/session-tasks.md). Rejection is
/// an `error.kind` on `failed`, not a state.
fn task_state_for(status: &AgentRunStatus) -> SessionTaskState {
    match status {
        AgentRunStatus::Submitted => SessionTaskState::Queued,
        AgentRunStatus::Working => SessionTaskState::Running,
        AgentRunStatus::InputRequired | AgentRunStatus::AuthRequired => {
            SessionTaskState::AwaitingInput
        }
        AgentRunStatus::Completed => SessionTaskState::Succeeded,
        AgentRunStatus::Failed => SessionTaskState::Failed,
        AgentRunStatus::Canceled => SessionTaskState::Canceled,
        AgentRunStatus::Rejected => SessionTaskState::Failed,
    }
}

/// Mirror a run snapshot into the session task registry (best-effort; no-op
/// when the registry is absent or the record predates task creation).
async fn mirror_run_to_task(context: &ToolContext, record: &AgentRunRecord) {
    let Some(registry) = &context.session_task_registry else {
        return;
    };
    let Some(task_id) = &record.task_id else {
        return;
    };
    let state = task_state_for(&record.status);

    // Generate an input request only on the TRANSITION into awaiting_input so
    // repeated polling does not churn the request id.
    let input_request = if state == SessionTaskState::AwaitingInput {
        let already_awaiting = registry
            .get(context.session_id, task_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|task| task.state == SessionTaskState::AwaitingInput);
        if already_awaiting {
            None
        } else {
            Some(TaskInputRequest {
                id: format!("inreq_{}", uuid::Uuid::now_v7().simple()),
                prompt: record
                    .result
                    .clone()
                    .unwrap_or_else(|| "External agent requires additional input".to_string()),
                expected: None,
            })
        }
    } else {
        None
    };

    let error = match record.status {
        AgentRunStatus::Failed => Some(TaskError {
            kind: "remote_failed".to_string(),
            message: record
                .error
                .clone()
                .unwrap_or_else(|| "External agent run failed".to_string()),
        }),
        AgentRunStatus::Rejected => Some(TaskError {
            kind: "rejected".to_string(),
            message: record
                .error
                .clone()
                .unwrap_or_else(|| "External agent rejected the task".to_string()),
        }),
        _ => None,
    };

    let _ = registry
        .update(
            context.session_id,
            task_id,
            SessionTaskUpdate {
                state: Some(state),
                input_request,
                summary: record.result.clone(),
                result_path: record.result_path.clone(),
                error,
                links: record
                    .remote_task_id
                    .clone()
                    .map(|remote_task_id| TaskLinks {
                        remote_task_id: Some(remote_task_id),
                        ..Default::default()
                    }),
                ..Default::default()
            },
        )
        .await;
}

/// Post the completion summary on the task's outbound message channel
/// (best-effort). The legacy wake-up session message is sent separately.
async fn post_task_completion_message(context: &ToolContext, record: &AgentRunRecord) {
    let (Some(registry), Some(task_id)) = (&context.session_task_registry, &record.task_id) else {
        return;
    };
    let summary = record
        .result
        .as_deref()
        .or(record.error.as_deref())
        .unwrap_or("No result text returned");
    let _ = registry
        .record_message(
            context.session_id,
            task_id,
            NewTaskMessage::outbound_text(format!(
                "External agent run {}: {summary}",
                record.status
            )),
        )
        .await;
}

async fn load_run(
    context: &ToolContext,
    run_id: &str,
) -> std::result::Result<AgentRunRecord, ToolExecutionResult> {
    let registry = require_registry(context)?;
    let Some(entry) = registry
        .get(context.session_id, run_id)
        .await
        .map_err(ToolExecutionResult::internal_error)?
    else {
        return Err(ToolExecutionResult::tool_error(format!(
            "No agent run found with id: {run_id}"
        )));
    };
    if entry.kind != AGENT_RUN_KIND {
        return Err(ToolExecutionResult::tool_error(format!(
            "Resource is not an agent run: {run_id}"
        )));
    }
    serde_json::from_value(entry.metadata).map_err(|e| {
        ToolExecutionResult::internal_error_msg(format!("Invalid agent run metadata: {e}"))
    })
}

fn task_text(task: &Task) -> Option<String> {
    task.artifacts
        .as_ref()
        .into_iter()
        .flatten()
        .flat_map(|artifact| artifact.parts.iter())
        .find_map(|part| part.as_text().map(ToString::to_string))
        .or_else(|| {
            task.status
                .message
                .as_ref()
                .and_then(|message| message.text().map(ToString::to_string))
        })
}

fn message_text(message: &Message) -> Option<String> {
    message.text().map(ToString::to_string)
}

fn truncate_text(value: String) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(MAX_RESULT_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}\n[truncated]")
    } else {
        truncated
    }
}

fn bounded_task_snapshot(task: &Task) -> Value {
    json!({
        "id": task.id,
        "context_id": task.context_id,
        "state": task.status.state,
        "text": task_text(task).map(truncate_text),
    })
}

fn set_error(record: &mut AgentRunRecord, error: String) {
    record.error = Some(truncate_text(error));
}

fn apply_task(record: &mut AgentRunRecord, task: &Task) {
    record.status = AgentRunStatus::from(&task.status.state);
    record.remote_task_id = Some(task.id.clone());
    record.remote_context_id = Some(task.context_id.clone());
    record.result = task_text(task)
        .map(truncate_text)
        .or_else(|| record.result.clone());
    record.last_remote_task_snapshot = Some(bounded_task_snapshot(task));
}

async fn write_result_artifact(context: &ToolContext, record: &mut AgentRunRecord) -> Result<()> {
    let Some(file_store) = &context.file_store else {
        return Ok(());
    };
    let dir = format!("/.agent-runs/{}", record.run_id);
    let path = format!("{dir}/result.json");
    let _ = file_store
        .create_directory(context.session_id, "/.agent-runs")
        .await;
    let _ = file_store.create_directory(context.session_id, &dir).await;
    record.result_path = Some(path.clone());
    let body = serde_json::to_string_pretty(&record.public_json())
        .unwrap_or_else(|_| record.public_json().to_string());
    file_store
        .write_file(context.session_id, &path, &body, "utf-8")
        .await?;
    Ok(())
}

async fn wake_parent(context: &ToolContext, record: &AgentRunRecord) -> Result<()> {
    let Some(platform_store) = &context.platform_store else {
        return Ok(());
    };
    let summary = record
        .result
        .as_deref()
        .or(record.error.as_deref())
        .unwrap_or("No result text returned");
    let message = format!(
        "External agent run completed.\n- run_id: {}\n- agent: {}\n- status: {}\n- result_path: {}\n- summary: {}",
        record.run_id,
        record.external_agent_name,
        record.status,
        record.result_path.as_deref().unwrap_or("(not persisted)"),
        summary
    );
    platform_store
        .send_message(context.session_id, &message)
        .await
}

/// Pre-discovery ACL gate: must run BEFORE `resolve_card` so that AgentCard
/// discovery itself (which performs an outbound HTTP fetch against `base_url`)
/// cannot be used to probe disallowed/internal hosts. Only checks `base_url`
/// when it will actually be used for discovery — when an inline `agent_card`
/// is supplied, `resolve_card` never touches `base_url`, so a stale or unused
/// configured URL must not cause spurious failures.
fn enforce_network_access_pre_resolve(
    agent: &ExternalA2aAgentConfig,
    context: &ToolContext,
) -> std::result::Result<(), String> {
    let Some(acl) = context.network_access.as_ref() else {
        return Ok(());
    };
    if agent.agent_card.is_some() {
        return Ok(());
    }
    if let Some(base_url) = &agent.base_url
        && !acl.is_url_allowed(base_url)
    {
        return Err(format!(
            "A2A base URL blocked by network access policy: {base_url}"
        ));
    }
    Ok(())
}

/// Post-discovery ACL gate: every interface URL surfaced by the resolved
/// AgentCard must also be permitted by the runtime ACL before the A2A client
/// is built. This catches the case where an attacker can influence the card
/// (signed/unsigned) to point interface URLs at internal hosts.
fn enforce_network_access_post_resolve(
    card: &AgentCard,
    context: &ToolContext,
) -> std::result::Result<(), String> {
    let Some(acl) = context.network_access.as_ref() else {
        return Ok(());
    };
    for iface in &card.supported_interfaces {
        if !acl.is_url_allowed(&iface.url) {
            return Err(format!(
                "A2A interface URL blocked by network access policy: {}",
                iface.url
            ));
        }
    }
    Ok(())
}

async fn build_client(
    agent: &ExternalA2aAgentConfig,
    context: &ToolContext,
) -> std::result::Result<a2a_client::A2AClient<Box<dyn a2a_client::Transport>>, String> {
    enforce_network_access_pre_resolve(agent, context)?;
    let card = agent.resolve_card().await?;
    enforce_network_access_post_resolve(&card, context)?;
    let mut builder = A2AClientFactory::builder();
    if let Some(binding) = &agent.preferred_binding {
        builder = builder.preferred_bindings(vec![binding.clone()]);
    }
    let headers = agent
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<_>>();
    if !headers.is_empty() {
        builder = builder.with_interceptor(Arc::new(StaticHeaderInterceptor { headers }));
    }
    builder
        .build()
        .create_from_card(&card)
        .await
        .map_err(|e| format!("Failed to create A2A client: {e}"))
}

fn send_request(
    text: &str,
    remote_task_id: Option<String>,
    remote_context_id: Option<String>,
    return_immediately: bool,
) -> SendMessageRequest {
    let mut message = Message::new(Role::User, vec![Part::text(text)]);
    message.task_id = remote_task_id;
    message.context_id = remote_context_id;
    SendMessageRequest {
        message,
        configuration: Some(SendMessageConfiguration {
            accepted_output_modes: Some(vec![
                "text/plain".to_string(),
                "application/json".to_string(),
            ]),
            task_push_notification_config: None,
            history_length: None,
            return_immediately: Some(return_immediately),
        }),
        metadata: None,
        tenant: None,
    }
}

async fn submit_run(
    context: &ToolContext,
    agent: &ExternalA2aAgentConfig,
    record: &mut AgentRunRecord,
    text: &str,
    remote_task_id: Option<String>,
    remote_context_id: Option<String>,
) -> std::result::Result<(), String> {
    let client = build_client(agent, context).await?;
    let response = client
        .send_message(&send_request(text, remote_task_id, remote_context_id, true))
        .await
        .map_err(|e| format!("A2A send_message failed: {e}"))?;
    match response {
        SendMessageResponse::Task(task) => apply_task(record, &task),
        SendMessageResponse::Message(message) => {
            record.status = AgentRunStatus::Completed;
            record.result = message_text(&message).map(truncate_text);
        }
    }
    save_run(context, record).await.map_err(|e| e.to_string())
}

async fn wait_for_run(
    context: &ToolContext,
    agent: &ExternalA2aAgentConfig,
    mut record: AgentRunRecord,
    timeout_secs: u64,
) -> std::result::Result<AgentRunRecord, String> {
    if record.status.is_terminal() {
        write_result_artifact(context, &mut record)
            .await
            .map_err(|e| e.to_string())?;
        save_run(context, &record)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(record);
    }
    let Some(remote_task_id) = record.remote_task_id.clone() else {
        return Ok(record);
    };
    let client = build_client(agent, context).await?;
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let poll_interval = Duration::from_millis(
        agent
            .poll_interval_ms
            .unwrap_or(DEFAULT_POLL_INTERVAL_MS)
            .max(100),
    );

    while Instant::now() < deadline {
        let task = client
            .get_task(&GetTaskRequest {
                id: remote_task_id.clone(),
                history_length: Some(10),
                tenant: None,
            })
            .await
            .map_err(|e| format!("A2A get_task failed: {e}"))?;
        apply_task(&mut record, &task);
        save_run(context, &record)
            .await
            .map_err(|e| e.to_string())?;
        if record.status.is_terminal() {
            write_result_artifact(context, &mut record)
                .await
                .map_err(|e| e.to_string())?;
            save_run(context, &record)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(record);
        }
        sleep(poll_interval).await;
    }

    Err(format!(
        "Timed out waiting for external agent run {} after {}s",
        record.run_id, timeout_secs
    ))
}

async fn timeout_or_error_result(
    context: &ToolContext,
    run_id: &str,
    error: String,
) -> ToolExecutionResult {
    if error.starts_with("Timed out waiting for external agent run") {
        return match load_run(context, run_id).await {
            Ok(record) => ToolExecutionResult::success(json!({
                "agent_run_id": record.run_id,
                "status": record.status,
                "timed_out": true,
                "message": truncate_text(error),
                "remote_task_id": record.remote_task_id,
                "remote_context_id": record.remote_context_id,
            })),
            Err(e) => e,
        };
    }
    ToolExecutionResult::tool_error(error)
}

async fn background_monitor(
    context: ToolContext,
    agent: ExternalA2aAgentConfig,
    record: AgentRunRecord,
    timeout_secs: u64,
) {
    let run_id = record.run_id.clone();
    let fallback_record = record.clone();
    let record = match wait_for_run(&context, &agent, record, timeout_secs).await {
        Ok(record) => record,
        Err(error) => {
            let mut failed = load_run(&context, &run_id).await.unwrap_or(fallback_record);
            failed.status = AgentRunStatus::Failed;
            set_error(&mut failed, error);
            let _ = write_result_artifact(&context, &mut failed).await;
            let _ = save_run(&context, &failed).await;
            post_task_completion_message(&context, &failed).await;
            // Legacy wake: only when no registry is present (registry-level
            // wake_policy handles it otherwise via post_task_completion_message).
            if failed.wake_on_completion && context.session_task_registry.is_none() {
                let _ = wake_parent(&context, &failed).await;
            }
            return;
        }
    };
    post_task_completion_message(&context, &record).await;
    // Registry-level wake_policy handles the wake when a task registry is
    // present (post_task_completion_message records an outbound message which
    // triggers the waker). Fall back to the legacy synthetic session message
    // only when no registry is wired (older execution contexts).
    if record.wake_on_completion && context.session_task_registry.is_none() {
        let _ = wake_parent(&context, &record).await;
    }
    let _ = save_run(&context, &record).await;
}

#[derive(Clone)]
pub struct SpawnAgentTool {
    config: A2aDelegationConfig,
}

impl SpawnAgentTool {
    fn new(config: A2aDelegationConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for SpawnAgentTool {
    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Spawn Agent")
    }

    fn description(&self) -> &str {
        "Delegate a task to a configured external A2A agent. Supports wait and background modes."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "Task to send to the external agent."},
                "target": {
                    "type": "object",
                    "properties": {
                        "type": {"type": "string", "enum": ["external_a2a"]},
                        "external_agent_id": {"type": "string"}
                    },
                    "required": ["type", "external_agent_id"],
                    "additionalProperties": false
                },
                "mode": {"type": "string", "enum": ["wait", "background"], "default": "wait"},
                "wait_timeout_secs": {"type": "integer", "minimum": 1, "maximum": 86400},
                "wake_on_completion": {"type": "boolean", "default": true}
            },
            "required": ["task", "target"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_long_running(true)
            .with_open_world(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("spawn_agent requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        if let Err(e) = require_registry(context) {
            return e;
        }
        let task = match require_str(&arguments, "task") {
            Ok(task) => task.to_string(),
            Err(e) => return e,
        };
        let target = arguments.get("target").unwrap_or(&Value::Null);
        if target.get("type").and_then(Value::as_str) != Some("external_a2a") {
            return ToolExecutionResult::tool_error(
                "spawn_agent currently supports target.type = external_a2a",
            );
        }
        let external_agent_id = match target
            .get("external_agent_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(id) => id,
            None => {
                return ToolExecutionResult::tool_error(
                    "Missing required parameter: target.external_agent_id",
                );
            }
        };
        let Some(agent) = self.config.agent(external_agent_id).cloned() else {
            return ToolExecutionResult::tool_error(format!(
                "Unknown external A2A agent: {external_agent_id}"
            ));
        };
        let mode = match AgentRunMode::parse(arguments.get("mode").and_then(Value::as_str)) {
            Ok(mode) => mode,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };
        let timeout_secs = arguments
            .get("wait_timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_WAIT_TIMEOUT_SECS);
        let wake_on_completion = arguments
            .get("wake_on_completion")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let run_id = run_id();
        let mut record = AgentRunRecord::new(
            run_id.clone(),
            &agent,
            task.clone(),
            mode.clone(),
            wake_on_completion,
        );
        // Create the session task tracking this run (specs/session-tasks.md).
        if let Some(task_registry) = &context.session_task_registry
            && let Ok(created) = task_registry
                .create(CreateSessionTask {
                    session_id: context.session_id,
                    id: None,
                    kind: TASK_KIND_EXTERNAL_AGENT.to_string(),
                    display_name: agent.name.clone(),
                    spec: json!({
                        "external_agent_id": agent.id,
                        "instructions": &task,
                        "mode": &mode,
                    }),
                    state: SessionTaskState::Queued,
                    links: TaskLinks::default(),
                    wake_policy: match mode {
                        AgentRunMode::Background => TaskWakePolicy::OnTerminal,
                        AgentRunMode::Wait => TaskWakePolicy::Silent,
                    },
                })
                .await
        {
            record.task_id = Some(created.id);
        }
        if let Err(e) = save_run(context, &record).await {
            return ToolExecutionResult::internal_error(e);
        }
        if let Err(error) = submit_run(context, &agent, &mut record, &task, None, None).await {
            record.status = AgentRunStatus::Failed;
            set_error(&mut record, error);
            let _ = save_run(context, &record).await;
            return ToolExecutionResult::success(record.public_json());
        }
        match mode {
            AgentRunMode::Background => {
                let context = context.clone();
                let background_record = record.clone();
                tokio::spawn(async move {
                    background_monitor(context, agent, background_record, timeout_secs).await;
                });
                ToolExecutionResult::success(record.public_json())
            }
            AgentRunMode::Wait => match wait_for_run(context, &agent, record, timeout_secs).await {
                Ok(record) => ToolExecutionResult::success(record.public_json()),
                Err(error) => timeout_or_error_result(context, &run_id, error).await,
            },
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub struct GetAgentRunsTool;

#[async_trait]
impl Tool for GetAgentRunsTool {
    fn name(&self) -> &str {
        "get_agent_runs"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Get Agent Runs")
    }

    fn description(&self) -> &str {
        "List external A2A agent runs or fetch one run by id."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_run_id": {"type": "string"},
                "status_filter": {
                    "type": "string",
                    "enum": ["all", "submitted", "working", "input_required", "auth_required", "completed", "failed", "canceled", "rejected"]
                }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("get_agent_runs requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        if let Some(run_id) = arguments
            .get("agent_run_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return match load_run(context, run_id).await {
                Ok(record) => ToolExecutionResult::success(record.public_json()),
                Err(e) => e,
            };
        }
        let registry = match require_registry(context) {
            Ok(registry) => registry,
            Err(e) => return e,
        };
        let entries = match registry
            .list(
                context.session_id,
                Some(&SessionResourceFilter {
                    kind: Some(AGENT_RUN_KIND.to_string()),
                    status: None,
                }),
            )
            .await
        {
            Ok(entries) => entries,
            Err(e) => return ToolExecutionResult::internal_error(e),
        };
        let status_filter = arguments
            .get("status_filter")
            .and_then(Value::as_str)
            .unwrap_or("all");
        let runs = entries
            .into_iter()
            .filter_map(|entry| serde_json::from_value::<AgentRunRecord>(entry.metadata).ok())
            .filter(|record| status_filter == "all" || record.status.to_string() == status_filter)
            .map(|record| record.public_json())
            .collect::<Vec<_>>();
        ToolExecutionResult::success(json!({
            "agent_runs": runs,
            "count": runs.len()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub struct WaitAgentTool {
    config: A2aDelegationConfig,
}

impl WaitAgentTool {
    fn new(config: A2aDelegationConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for WaitAgentTool {
    fn name(&self) -> &str {
        "wait_agent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Wait Agent")
    }

    fn description(&self) -> &str {
        "Wait for an external A2A agent run to complete or reach an interrupted state."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_run_id": {"type": "string"},
                "timeout_secs": {"type": "integer", "minimum": 1, "maximum": 86400}
            },
            "required": ["agent_run_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_long_running(true)
            .with_open_world(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("wait_agent requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let run_id = match require_str(&arguments, "agent_run_id") {
            Ok(id) => id,
            Err(e) => return e,
        };
        let record = match load_run(context, run_id).await {
            Ok(record) => record,
            Err(e) => return e,
        };
        let Some(agent) = self.config.agent(&record.external_agent_id).cloned() else {
            return ToolExecutionResult::tool_error(format!(
                "External A2A agent no longer configured: {}",
                record.external_agent_id
            ));
        };
        let timeout_secs = arguments
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_WAIT_TIMEOUT_SECS);
        match wait_for_run(context, &agent, record, timeout_secs).await {
            Ok(record) => ToolExecutionResult::success(record.public_json()),
            Err(error) => timeout_or_error_result(context, run_id, error).await,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub struct MessageAgentTool {
    config: A2aDelegationConfig,
}

impl MessageAgentTool {
    fn new(config: A2aDelegationConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for MessageAgentTool {
    fn name(&self) -> &str {
        "message_agent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Message Agent")
    }

    fn description(&self) -> &str {
        "Send follow-up input to an existing external A2A agent run."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_run_id": {"type": "string"},
                "message": {"type": "string"},
                "wait_timeout_secs": {"type": "integer", "minimum": 1, "maximum": 86400}
            },
            "required": ["agent_run_id", "message"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_long_running(true)
            .with_open_world(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("message_agent requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let run_id = match require_str(&arguments, "agent_run_id") {
            Ok(id) => id,
            Err(e) => return e,
        };
        let message = match require_str(&arguments, "message") {
            Ok(message) => message.to_string(),
            Err(e) => return e,
        };
        let mut record = match load_run(context, run_id).await {
            Ok(record) => record,
            Err(e) => return e,
        };
        let Some(agent) = self.config.agent(&record.external_agent_id).cloned() else {
            return ToolExecutionResult::tool_error(format!(
                "External A2A agent no longer configured: {}",
                record.external_agent_id
            ));
        };
        let remote_task_id = record.remote_task_id.clone();
        let remote_context_id = record.remote_context_id.clone();
        if let Err(error) = submit_run(
            context,
            &agent,
            &mut record,
            &message,
            remote_task_id,
            remote_context_id,
        )
        .await
        {
            record.status = AgentRunStatus::Failed;
            set_error(&mut record, error);
            let _ = save_run(context, &record).await;
            return ToolExecutionResult::success(record.public_json());
        }
        let timeout_secs = arguments
            .get("wait_timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_WAIT_TIMEOUT_SECS);
        match wait_for_run(context, &agent, record, timeout_secs).await {
            Ok(record) => ToolExecutionResult::success(record.public_json()),
            Err(error) => timeout_or_error_result(context, run_id, error).await,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub struct CancelAgentTool {
    config: A2aDelegationConfig,
}

impl CancelAgentTool {
    fn new(config: A2aDelegationConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for CancelAgentTool {
    fn name(&self) -> &str {
        "cancel_agent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Cancel Agent")
    }

    fn description(&self) -> &str {
        "Cancel an existing external A2A agent run."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_run_id": {"type": "string"}
            },
            "required": ["agent_run_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_open_world(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("cancel_agent requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let run_id = match require_str(&arguments, "agent_run_id") {
            Ok(id) => id,
            Err(e) => return e,
        };
        let mut record = match load_run(context, run_id).await {
            Ok(record) => record,
            Err(e) => return e,
        };
        if record.status.is_terminal() {
            return ToolExecutionResult::success(record.public_json());
        }
        let Some(remote_task_id) = record.remote_task_id.clone() else {
            record.status = AgentRunStatus::Canceled;
            let _ = save_run(context, &record).await;
            return ToolExecutionResult::success(record.public_json());
        };
        let Some(agent) = self.config.agent(&record.external_agent_id).cloned() else {
            return ToolExecutionResult::tool_error(format!(
                "External A2A agent no longer configured: {}",
                record.external_agent_id
            ));
        };
        match build_client(&agent, context).await {
            Ok(client) => match client
                .cancel_task(&CancelTaskRequest {
                    id: remote_task_id,
                    metadata: None,
                    tenant: None,
                })
                .await
            {
                Ok(task) => apply_task(&mut record, &task),
                Err(error) => {
                    record.status = AgentRunStatus::Failed;
                    set_error(&mut record, format!("A2A cancel_task failed: {error}"));
                }
            },
            Err(error) => {
                record.status = AgentRunStatus::Failed;
                set_error(&mut record, error);
            }
        }
        let _ = save_run(context, &record).await;
        ToolExecutionResult::success(record.public_json())
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Task executor: external_agent
// ============================================================================

/// Locate the agent run mirrored by a session task. Runs land in the session
/// resource registry as `agent_run` resources with the task id in metadata.
async fn load_run_for_task(
    context: &ToolContext,
    task: &SessionTask,
) -> std::result::Result<AgentRunRecord, String> {
    let Some(registry) = &context.session_resource_registry else {
        return Err("external agent tasks require session_resource_registry context".to_string());
    };
    let entries = registry
        .list(
            context.session_id,
            Some(&SessionResourceFilter {
                kind: Some(AGENT_RUN_KIND.to_string()),
                status: None,
            }),
        )
        .await
        .map_err(|e| e.to_string())?;
    entries
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<AgentRunRecord>(entry.metadata).ok())
        .find(|record| record.task_id.as_deref() == Some(task.id.as_str()))
        .ok_or_else(|| format!("No agent run found for task {}", task.id))
}

/// Agent config snapshot stored on the run, required to rebuild the A2A
/// client outside the capability's configured tool instances.
fn agent_snapshot(record: &AgentRunRecord) -> std::result::Result<ExternalA2aAgentConfig, String> {
    record.agent_config.clone().ok_or_else(|| {
        format!(
            "Agent run {} has no stored agent config snapshot (created before task support); use message_agent/cancel_agent instead",
            record.run_id
        )
    })
}

/// Control plane for `external_agent` tasks. Rebuilds the A2A client from the
/// agent config snapshot persisted on the run record.
pub struct ExternalAgentTaskExecutor;

#[async_trait]
impl TaskExecutor for ExternalAgentTaskExecutor {
    fn kind(&self) -> &str {
        TASK_KIND_EXTERNAL_AGENT
    }

    async fn deliver(
        &self,
        task: &SessionTask,
        message: &TaskMessage,
        context: &ToolContext,
    ) -> crate::error::Result<()> {
        let mut record = load_run_for_task(context, task)
            .await
            .map_err(crate::error::AgentLoopError::tool)?;
        let agent = agent_snapshot(&record).map_err(crate::error::AgentLoopError::tool)?;
        let text = task_message_text(&message.content);
        let remote_task_id = record.remote_task_id.clone();
        let remote_context_id = record.remote_context_id.clone();
        // On send error the run state stays unchanged — return the error and
        // let the caller decide; the registry already holds the message.
        submit_run(
            context,
            &agent,
            &mut record,
            &text,
            remote_task_id,
            remote_context_id,
        )
        .await
        .map_err(crate::error::AgentLoopError::tool)
    }

    async fn cancel(&self, task: &SessionTask, context: &ToolContext) -> crate::error::Result<()> {
        let mut record = load_run_for_task(context, task)
            .await
            .map_err(crate::error::AgentLoopError::tool)?;
        if record.status.is_terminal() {
            return Ok(());
        }
        let Some(remote_task_id) = record.remote_task_id.clone() else {
            // Never reached the remote agent; cancel locally.
            record.status = AgentRunStatus::Canceled;
            save_run(context, &record).await?;
            return Ok(());
        };
        let agent = agent_snapshot(&record).map_err(crate::error::AgentLoopError::tool)?;
        let client = build_client(&agent, context)
            .await
            .map_err(crate::error::AgentLoopError::tool)?;
        let remote = client
            .cancel_task(&CancelTaskRequest {
                id: remote_task_id,
                metadata: None,
                tenant: None,
            })
            .await
            .map_err(|e| {
                crate::error::AgentLoopError::tool(format!("A2A cancel_task failed: {e}"))
            })?;
        apply_task(&mut record, &remote);
        save_run(context, &record).await?;
        Ok(())
    }

    async fn reconcile(
        &self,
        task: &SessionTask,
        context: &ToolContext,
    ) -> crate::error::Result<()> {
        let mut record = load_run_for_task(context, task)
            .await
            .map_err(crate::error::AgentLoopError::tool)?;
        if record.status.is_terminal() {
            return Ok(());
        }
        let Some(remote_task_id) = record.remote_task_id.clone() else {
            return Ok(());
        };
        let agent = agent_snapshot(&record).map_err(crate::error::AgentLoopError::tool)?;
        let client = build_client(&agent, context)
            .await
            .map_err(crate::error::AgentLoopError::tool)?;
        let remote = client
            .get_task(&GetTaskRequest {
                id: remote_task_id,
                history_length: Some(10),
                tenant: None,
            })
            .await
            .map_err(|e| crate::error::AgentLoopError::tool(format!("A2A get_task failed: {e}")))?;
        apply_task(&mut record, &remote);
        if record.status.is_terminal() {
            let _ = write_result_artifact(context, &mut record).await;
        }
        save_run(context, &record).await?;
        Ok(())
    }
}

inventory::submit! {
    TaskExecutorPlugin {
        executor: || Arc::new(ExternalAgentTaskExecutor),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
    use crate::traits::SessionFileSystem;
    use crate::typed_id::SessionId;
    use a2a::StreamResponse;
    use a2a::{AgentCapabilities, AgentInterface, Artifact, TaskStatus, TaskStatusUpdateEvent};
    use a2a_server::agent_card::agent_card_router;
    use a2a_server::{
        DefaultRequestHandler, InMemoryTaskStore, StaticAgentCard, jsonrpc::jsonrpc_router,
    };
    use axum::Router;
    use futures::stream;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::Mutex;
    use tokio::net::TcpListener;
    use tokio::time::timeout;

    #[derive(Default)]
    struct TestSessionResourceRegistry {
        entries: Mutex<HashMap<String, crate::session_resource::SessionResourceEntry>>,
    }

    #[async_trait]
    impl SessionResourceRegistry for TestSessionResourceRegistry {
        async fn register(
            &self,
            entry: RegisterSessionResource,
        ) -> Result<crate::session_resource::SessionResourceEntry> {
            let now = chrono::Utc::now();
            let mut entries = self.entries.lock().unwrap();
            let existing = entries.get(&entry.resource_id).cloned();
            let out = crate::session_resource::SessionResourceEntry {
                resource_id: entry.resource_id.clone(),
                session_id: entry.session_id,
                kind: entry.kind,
                display_name: entry.display_name,
                status: entry.status,
                metadata: entry.metadata,
                created_at: existing.as_ref().map(|e| e.created_at).unwrap_or(now),
                updated_at: now,
            };
            entries.insert(entry.resource_id, out.clone());
            Ok(out)
        }

        async fn update_status(
            &self,
            _session_id: SessionId,
            resource_id: &str,
            status: SessionResourceStatus,
        ) -> Result<Option<crate::session_resource::SessionResourceEntry>> {
            let mut entries = self.entries.lock().unwrap();
            if let Some(entry) = entries.get_mut(resource_id) {
                entry.status = status;
                return Ok(Some(entry.clone()));
            }
            Ok(None)
        }

        async fn get(
            &self,
            _session_id: SessionId,
            resource_id: &str,
        ) -> Result<Option<crate::session_resource::SessionResourceEntry>> {
            Ok(self.entries.lock().unwrap().get(resource_id).cloned())
        }

        async fn list(
            &self,
            _session_id: SessionId,
            filter: Option<&SessionResourceFilter>,
        ) -> Result<Vec<crate::session_resource::SessionResourceEntry>> {
            let entries = self.entries.lock().unwrap();
            Ok(entries
                .values()
                .filter(|entry| {
                    filter
                        .and_then(|f| f.kind.as_deref())
                        .is_none_or(|kind| entry.kind == kind)
                })
                .cloned()
                .collect())
        }

        async fn deregister(&self, _session_id: SessionId, resource_id: &str) -> Result<bool> {
            Ok(self.entries.lock().unwrap().remove(resource_id).is_some())
        }
    }

    #[derive(Default)]
    struct TestFileStore {
        files: Mutex<HashMap<String, String>>,
    }

    #[async_trait]
    impl SessionFileSystem for TestFileStore {
        async fn read_file(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> Result<Option<SessionFile>> {
            Ok(self
                .files
                .lock()
                .unwrap()
                .get(path)
                .map(|content| SessionFile {
                    id: uuid::Uuid::new_v4(),
                    session_id: session_id.uuid(),
                    path: path.to_string(),
                    name: FileInfo::name_from_path(path),
                    content: Some(content.clone()),
                    encoding: "text".to_string(),
                    is_directory: false,
                    is_readonly: false,
                    size_bytes: content.len() as i64,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }))
        }

        async fn write_file(
            &self,
            session_id: SessionId,
            path: &str,
            content: &str,
            _encoding: &str,
        ) -> Result<SessionFile> {
            self.files
                .lock()
                .unwrap()
                .insert(path.to_string(), content.to_string());
            Ok(SessionFile {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: FileInfo::name_from_path(path),
                content: Some(content.to_string()),
                encoding: "text".to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }

        async fn delete_file(
            &self,
            _session_id: SessionId,
            path: &str,
            _recursive: bool,
        ) -> Result<bool> {
            Ok(self.files.lock().unwrap().remove(path).is_some())
        }

        async fn list_directory(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> Result<Vec<FileInfo>> {
            Ok(vec![])
        }

        async fn stat_file(&self, _session_id: SessionId, _path: &str) -> Result<Option<FileStat>> {
            Ok(None)
        }

        async fn grep_files(
            &self,
            _session_id: SessionId,
            _pattern: &str,
            _path_pattern: Option<&str>,
        ) -> Result<Vec<GrepMatch>> {
            Ok(vec![])
        }

        async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
            Ok(FileInfo {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: FileInfo::name_from_path(path),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }
    }

    struct EchoA2aExecutor;

    impl a2a_server::AgentExecutor for EchoA2aExecutor {
        fn execute(
            &self,
            ctx: a2a_server::ExecutorContext,
        ) -> futures::stream::BoxStream<'static, std::result::Result<StreamResponse, a2a::A2AError>>
        {
            let task_id = ctx.task_id.clone();
            let context_id = ctx.context_id.clone();
            let text = ctx
                .message
                .as_ref()
                .and_then(Message::text)
                .unwrap_or_default()
                .to_string();
            let working = StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
                task_id: task_id.clone(),
                context_id: context_id.clone(),
                status: TaskStatus {
                    state: TaskState::Working,
                    message: None,
                    timestamp: None,
                },
                metadata: None,
            });
            let completed = StreamResponse::Task(Task {
                id: task_id,
                context_id,
                status: TaskStatus {
                    state: TaskState::Completed,
                    message: None,
                    timestamp: None,
                },
                artifacts: Some(vec![Artifact {
                    artifact_id: a2a::new_artifact_id(),
                    name: Some("echo".to_string()),
                    description: None,
                    parts: vec![Part::text(format!("echo: {text}"))],
                    metadata: None,
                    extensions: None,
                }]),
                history: ctx.stored_task.and_then(|task| task.history),
                metadata: None,
            });
            Box::pin(stream::iter(vec![Ok(working), Ok(completed)]))
        }

        fn cancel(
            &self,
            ctx: a2a_server::ExecutorContext,
        ) -> futures::stream::BoxStream<'static, std::result::Result<StreamResponse, a2a::A2AError>>
        {
            let canceled = StreamResponse::Task(Task {
                id: ctx.task_id,
                context_id: ctx.context_id,
                status: TaskStatus {
                    state: TaskState::Canceled,
                    message: None,
                    timestamp: None,
                },
                artifacts: None,
                history: None,
                metadata: None,
            });
            Box::pin(stream::once(async move { Ok(canceled) }))
        }
    }

    async fn spawn_real_a2a_agent() -> String {
        crate::telemetry::install_crypto_provider();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");
        let card = AgentCard {
            name: "Echo A2A Agent".to_string(),
            description: "Real A2A test agent".to_string(),
            version: "1.0.0".to_string(),
            supported_interfaces: vec![AgentInterface::new(
                format!("{base_url}/jsonrpc"),
                "JSONRPC",
            )],
            capabilities: AgentCapabilities {
                streaming: Some(true),
                push_notifications: Some(false),
                extensions: None,
                extended_agent_card: None,
            },
            default_input_modes: vec!["text/plain".to_string()],
            default_output_modes: vec!["text/plain".to_string()],
            skills: vec![],
            provider: None,
            documentation_url: None,
            icon_url: None,
            security_schemes: None,
            security_requirements: None,
            signatures: None,
        };
        let handler = Arc::new(DefaultRequestHandler::new(
            EchoA2aExecutor,
            InMemoryTaskStore::default(),
        ));
        let app = Router::new()
            .merge(agent_card_router(Arc::new(StaticAgentCard::new(card))))
            .nest("/jsonrpc", jsonrpc_router(handler));
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        base_url
    }

    fn configured_capability(base_url: String) -> A2aDelegationConfig {
        A2aDelegationConfig {
            agents: vec![ExternalA2aAgentConfig {
                id: "echo".to_string(),
                name: "Echo".to_string(),
                description: Some("Echo test agent".to_string()),
                base_url: Some(base_url),
                agent_card: None,
                headers: BTreeMap::new(),
                preferred_binding: Some("JSONRPC".to_string()),
                poll_interval_ms: Some(100),
                allow_local_urls: true,
            }],
        }
    }

    fn context(
        registry: Arc<TestSessionResourceRegistry>,
        file_store: Arc<TestFileStore>,
    ) -> ToolContext {
        ToolContext::with_file_store(SessionId::new(), file_store)
            .with_session_resource_registry(registry)
    }

    #[tokio::test]
    async fn spawn_agent_wait_calls_real_a2a_agent() {
        let base_url = spawn_real_a2a_agent().await;
        let config = configured_capability(base_url);
        let tool = SpawnAgentTool::new(config);
        let registry = Arc::new(TestSessionResourceRegistry::default());
        let file_store = Arc::new(TestFileStore::default());
        let ctx = context(registry, file_store);

        let result = tool
            .execute_with_context(
                json!({
                    "task": "hello",
                    "target": {"type": "external_a2a", "external_agent_id": "echo"},
                    "mode": "wait",
                    "wait_timeout_secs": 5
                }),
                &ctx,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}");
        };
        assert_eq!(value["status"], "completed");
        assert_eq!(value["result"], "echo: hello");
        assert!(value["result_path"].as_str().is_some());
    }

    #[tokio::test]
    async fn spawn_agent_background_can_be_waited() {
        let base_url = spawn_real_a2a_agent().await;
        let config = configured_capability(base_url);
        let spawn = SpawnAgentTool::new(config.clone());
        let wait = WaitAgentTool::new(config);
        let registry = Arc::new(TestSessionResourceRegistry::default());
        let file_store = Arc::new(TestFileStore::default());
        let ctx = context(registry, file_store);

        let result = spawn
            .execute_with_context(
                json!({
                    "task": "background",
                    "target": {"type": "external_a2a", "external_agent_id": "echo"},
                    "mode": "background",
                    "wait_timeout_secs": 5,
                    "wake_on_completion": false
                }),
                &ctx,
            )
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}");
        };
        let run_id = value["agent_run_id"].as_str().unwrap();

        let waited = timeout(
            Duration::from_secs(5),
            wait.execute_with_context(json!({"agent_run_id": run_id, "timeout_secs": 5}), &ctx),
        )
        .await
        .unwrap();
        let ToolExecutionResult::Success(value) = waited else {
            panic!("expected success: {waited:?}");
        };
        assert_eq!(value["status"], "completed");
        assert_eq!(value["result"], "echo: background");
    }

    #[test]
    fn uk_localization_and_schema_one_of_match_validation() {
        let cap = A2aAgentDelegationCapability;
        assert_eq!(cap.localized_name(Some("uk-UA")), "Делегування агентам A2A");
        assert!(
            cap.localized_description(Some("uk-UA"))
                .contains("Делегує роботу")
        );
        assert!(cap.describe_schema(Some("uk")).is_some());
        assert!(cap.describe_schema(None).is_some());

        // preferred_binding oneOf consts must be exactly the values
        // validate_config accepts.
        let schema = cap.config_schema().expect("config schema");
        let consts: Vec<&str> =
            schema["properties"]["agents"]["items"]["properties"]["preferred_binding"]["oneOf"]
                .as_array()
                .expect("oneOf")
                .iter()
                .map(|v| v["const"].as_str().expect("const"))
                .collect();
        assert_eq!(consts, vec!["JSONRPC", "HTTP+JSON"]);
        for binding in consts {
            let config = json!({
                "agents": [{
                    "id": "echo",
                    "name": "Echo",
                    "base_url": "https://agent.example.com",
                    "preferred_binding": binding
                }]
            });
            cap.validate_config(&config)
                .unwrap_or_else(|e| panic!("{binding} should validate: {e}"));
        }
        assert!(
            cap.validate_config(&json!({
                "agents": [{
                    "id": "echo",
                    "name": "Echo",
                    "base_url": "https://agent.example.com",
                    "preferred_binding": "SMTP"
                }]
            }))
            .is_err()
        );
    }

    #[test]
    fn validates_local_urls_only_with_escape_hatch() {
        let mut config = configured_capability("http://127.0.0.1:1".to_string());
        config.agents[0].allow_local_urls = false;
        assert!(config.agents[0].validate().is_err());
        config.agents[0].allow_local_urls = true;
        assert!(config.agents[0].validate().is_ok());
    }

    #[test]
    fn enforce_network_access_blocks_disallowed_base_url() {
        use crate::SessionId;
        use crate::network_access::NetworkAccessList;

        let agent = ExternalA2aAgentConfig {
            id: "a".to_string(),
            name: "a".to_string(),
            description: None,
            base_url: Some("https://blocked.example.com".to_string()),
            agent_card: None,
            headers: BTreeMap::new(),
            preferred_binding: None,
            poll_interval_ms: None,
            allow_local_urls: false,
        };
        let card = AgentCard {
            name: "a".to_string(),
            description: "a".to_string(),
            version: "1".to_string(),
            supported_interfaces: vec![],
            capabilities: AgentCapabilities {
                streaming: None,
                push_notifications: None,
                extensions: None,
                extended_agent_card: None,
            },
            default_input_modes: vec![],
            default_output_modes: vec![],
            skills: vec![],
            provider: None,
            documentation_url: None,
            icon_url: None,
            security_schemes: None,
            security_requirements: None,
            signatures: None,
        };

        let ctx = ToolContext::new(SessionId::new()).with_network_access(Some(
            NetworkAccessList::allow_only(vec!["allowed.example.com".to_string()]),
        ));
        let err = enforce_network_access_pre_resolve(&agent, &ctx).unwrap_err();
        assert!(
            err.contains("blocked.example.com"),
            "unexpected error: {err}"
        );

        let ctx = ToolContext::new(SessionId::new()).with_network_access(Some(
            NetworkAccessList::allow_only(vec!["blocked.example.com".to_string()]),
        ));
        enforce_network_access_pre_resolve(&agent, &ctx).unwrap();
        enforce_network_access_post_resolve(&card, &ctx).unwrap();
    }

    #[test]
    fn enforce_network_access_blocks_disallowed_interface_url() {
        use crate::SessionId;
        use crate::network_access::NetworkAccessList;

        let card = AgentCard {
            name: "a".to_string(),
            description: "a".to_string(),
            version: "1".to_string(),
            supported_interfaces: vec![AgentInterface::new(
                "https://probe.internal/api".to_string(),
                "JSONRPC",
            )],
            capabilities: AgentCapabilities {
                streaming: None,
                push_notifications: None,
                extensions: None,
                extended_agent_card: None,
            },
            default_input_modes: vec![],
            default_output_modes: vec![],
            skills: vec![],
            provider: None,
            documentation_url: None,
            icon_url: None,
            security_schemes: None,
            security_requirements: None,
            signatures: None,
        };
        let ctx = ToolContext::new(SessionId::new()).with_network_access(Some(
            NetworkAccessList::allow_only(vec!["allowed.example.com".to_string()]),
        ));
        let err = enforce_network_access_post_resolve(&card, &ctx).unwrap_err();
        assert!(err.contains("probe.internal"), "unexpected error: {err}");
    }

    #[test]
    fn enforce_network_access_pre_resolve_skips_when_inline_card_present() {
        use crate::SessionId;
        use crate::network_access::NetworkAccessList;

        // base_url not on the allowlist, but agent_card is supplied inline so
        // resolve_card never performs discovery against base_url. The pre-resolve
        // gate must skip the base_url check to avoid spurious failures.
        let inline_card = AgentCard {
            name: "a".to_string(),
            description: "a".to_string(),
            version: "1".to_string(),
            supported_interfaces: vec![AgentInterface::new(
                "https://allowed.example.com/api".to_string(),
                "JSONRPC",
            )],
            capabilities: AgentCapabilities {
                streaming: None,
                push_notifications: None,
                extensions: None,
                extended_agent_card: None,
            },
            default_input_modes: vec![],
            default_output_modes: vec![],
            skills: vec![],
            provider: None,
            documentation_url: None,
            icon_url: None,
            security_schemes: None,
            security_requirements: None,
            signatures: None,
        };
        let agent = ExternalA2aAgentConfig {
            id: "a".to_string(),
            name: "a".to_string(),
            description: None,
            base_url: Some("https://stale.unused.example.com".to_string()),
            agent_card: Some(inline_card),
            headers: BTreeMap::new(),
            preferred_binding: None,
            poll_interval_ms: None,
            allow_local_urls: false,
        };
        let ctx = ToolContext::new(SessionId::new()).with_network_access(Some(
            NetworkAccessList::allow_only(vec!["allowed.example.com".to_string()]),
        ));
        enforce_network_access_pre_resolve(&agent, &ctx).unwrap();
    }

    #[test]
    fn local_url_escape_hatch_still_rejects_bad_url_shape() {
        let mut config = configured_capability("file:///tmp/agent".to_string());
        config.agents[0].allow_local_urls = true;
        assert!(config.agents[0].validate().is_err());

        let mut config = configured_capability("http://127.0.0.1:1".to_string());
        config.agents[0].allow_local_urls = true;
        config.agents[0].preferred_binding = Some("SMTP".to_string());
        assert!(config.agents[0].validate().is_err());

        config.agents[0].preferred_binding = Some("JSONRPC".to_string());
        config.agents[0].poll_interval_ms = Some(99);
        assert!(config.agents[0].validate().is_err());
    }
}
