// Outbound A2A agent delegation.
//
// Decision: V1 is outbound-only and stores configured external agents in the
// capability config. Run state is persisted as session storage KV entries
// (key = "agent_run:{run_id}") so agents and UI have a unified local/remote
// delegation handle. Listing derives from `list_keys` by prefix — no index
// key, so concurrent spawns cannot race. The resource registry is unused for
// runs (retired as part of the session-tasks dual-write cleanup).

use super::delegation_result::{
    normalize_result_schema, schema_validation_errors, write_task_result_value,
};
use super::{
    Capability, CapabilityLocalization, CapabilityStatus, RiskLevel, SESSION_TASKS_CAPABILITY_ID,
    SpawnMode, SystemPromptContext,
};
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::tool_types::ToolHints;
use everruns_contracts::url_validation::validate_safe_url;
use everruns_core::a2a::{
    self, AgentCard, CancelTaskRequest, DnsResolver, GetTaskRequest, Message, Part, PartContent,
    Role, SendMessageConfiguration, SendMessageRequest, SendMessageResponse, Task, TaskState,
};
use everruns_core::deployment::DeploymentGrade;
use everruns_core::network_access::NetworkAccessList;
use everruns_core::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskState, SessionTaskUpdate,
    TASK_KIND_EXTERNAL_AGENT, TaskError, TaskExecutor, TaskExecutorPlugin, TaskInputRequest,
    TaskLinks, TaskMessage, TaskWakePolicy, task_message_text,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{session_services::SessionStorageStore, tool_context::ToolContext};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{Instant, sleep};
use url::Url;

// The capability-ID constant lives ungated in `capabilities::mod` so session
// attachment logic (ard_attachment) can reference it even in builds that gate
// out the A2A delegation implementation. See the `a2a` feature.
pub use super::A2A_AGENT_DELEGATION_CAPABILITY_ID;

#[cfg(test)]
mod network;
const DEFAULT_WAIT_TIMEOUT_SECS: u64 = 300;
const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;
/// Error prefix returned by `wait_for_run` when the attempt fence reveals the
/// executor was superseded (reaper re-attached the task elsewhere). The
/// background monitor exits without writing failure state on this error.
const SUPERSEDED_ERROR_PREFIX: &str = "Superseded external agent poll for";
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
                                "description": "Testing/dev escape hatch for localhost A2A agents. Only honored when DEPLOYMENT_GRADE=dev; rejected in every other grade.",
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
                                        "description": "Обхідний шлях для тестування та розробки з локальними агентами A2A. Діє лише за DEPLOYMENT_GRADE=dev; в інших режимах відхиляється."
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
        let _ = A2aDelegationConfig::from_value(config).unwrap_or_default();
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
            target_type: "external_a2a",
            tool: Box::new(SpawnAgentTool::new(
                A2aDelegationConfig::from_value(config).unwrap_or_default(),
            )),
        })
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec![SESSION_TASKS_CAPABILITY_ID]
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
Use mode=\"background\" for long-running work and wait_task (from session_tasks) later for results; use mode=\"foreground\" when blocked on the result.\n\
Use message_task for follow-up input or input_required tasks; use cancel_task to stop a remote task.\n\
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
    /// `allow_local_urls` is a development escape hatch. It is honored only
    /// when the process deployment grade is `dev` so production/preview/poc
    /// cannot open loopback or private A2A targets via config alone (EVE-1173).
    fn local_urls_permitted(&self) -> bool {
        self.local_urls_permitted_for_grade(DeploymentGrade::from_env())
    }

    fn local_urls_permitted_for_grade(&self, grade: DeploymentGrade) -> bool {
        self.allow_local_urls && grade.is_dev()
    }

    fn validate(&self) -> std::result::Result<(), String> {
        self.validate_for_grade(DeploymentGrade::from_env())
    }

    fn validate_for_grade(&self, grade: DeploymentGrade) -> std::result::Result<(), String> {
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
        // THREAT[TM-AGENT-024]: the hatch is isolated to deployment grade dev.
        if self.allow_local_urls && !grade.is_dev() {
            return Err(format!(
                "A2A agent {} allow_local_urls is only permitted when DEPLOYMENT_GRADE=dev",
                self.id
            ));
        }
        let allow_local = self.local_urls_permitted_for_grade(grade);
        if let Some(base_url) = &self.base_url {
            if allow_local {
                validate_http_url(base_url)
                    .map_err(|e| format!("A2A agent {} has invalid base_url: {e}", self.id))?;
            } else {
                validate_safe_url(base_url)
                    .map_err(|e| format!("A2A agent {} has unsafe base_url: {e}", self.id))?;
            }
        }
        if let Some(card) = &self.agent_card {
            self.validate_card_for_grade(card, grade)?;
        }
        Ok(())
    }

    fn validate_card(&self, card: &AgentCard) -> std::result::Result<(), String> {
        self.validate_card_for_grade(card, DeploymentGrade::from_env())
    }

    fn validate_card_for_grade(
        &self,
        card: &AgentCard,
        grade: DeploymentGrade,
    ) -> std::result::Result<(), String> {
        let allow_local = self.local_urls_permitted_for_grade(grade);
        for iface in &card.supported_interfaces {
            if allow_local {
                validate_http_url(&iface.url)
                    .map_err(|e| format!("A2A agent {} has invalid interface URL: {e}", self.id))?;
            } else {
                validate_safe_url(&iface.url)
                    .map_err(|e| format!("A2A agent {} has unsafe interface URL: {e}", self.id))?;
            }
        }
        Ok(())
    }

    async fn resolve_card_with_resolver(
        &self,
        resolver: Option<&DnsResolver>,
    ) -> std::result::Result<AgentCard, String> {
        self.validate()?;
        if let Some(card) = &self.agent_card {
            return Ok(card.clone());
        }
        let base_url = self
            .base_url
            .as_deref()
            .ok_or_else(|| format!("A2A agent {} has no base_url", self.id))?;
        // The protocol owner preserves pinned DNS and no-redirect discovery.
        let card =
            a2a::discover_agent_card(base_url, None, self.local_urls_permitted(), resolver).await?;
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
struct AgentRunRecord {
    run_id: String,
    kind: String,
    external_agent_id: String,
    external_agent_name: String,
    #[serde(alias = "task")]
    instructions: String,
    #[serde(deserialize_with = "deserialize_agent_run_mode")]
    mode: SpawnMode,
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
    error_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    structured_result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_remote_task_snapshot: Option<Value>,
    #[serde(default)]
    wake_on_completion: bool,
    /// Session task mirroring this run (knowledge/runtime-resources/session-tasks.md). Absent on
    /// records that predate the task registry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    /// Snapshot of the agent config used to create the run, so the task
    /// executor can rebuild the A2A client without the capability config.
    /// Headers are non-secret by config-schema contract, so persisting them
    /// in resource metadata is safe. Absent on old records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_config: Option<ExternalA2aAgentConfig>,
    /// Merged network policy captured at spawn time. Re-attach runs outside
    /// ActAtom, so it must restore this before rebuilding outbound clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    network_access: Option<NetworkAccessList>,
}

fn deserialize_agent_run_mode<'de, D>(deserializer: D) -> std::result::Result<SpawnMode, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mode = String::deserialize(deserializer)?;
    match mode.as_str() {
        // Legacy durable A2A foreground runs were stored as "wait" before the
        // shared spawn-agent vocabulary was unified on "foreground".
        "wait" => Ok(SpawnMode::Foreground),
        _ => SpawnMode::parse(&mode).ok_or_else(|| {
            serde::de::Error::unknown_variant(&mode, &["background", "foreground", "wait"])
        }),
    }
}

impl AgentRunRecord {
    fn new(
        run_id: String,
        agent: &ExternalA2aAgentConfig,
        instructions: String,
        mode: SpawnMode,
        wake_on_completion: bool,
        result_schema: Option<Value>,
    ) -> Self {
        Self {
            run_id,
            kind: "external_a2a".to_string(),
            external_agent_id: agent.id.clone(),
            external_agent_name: agent.name.clone(),
            instructions,
            mode,
            status: AgentRunStatus::Submitted,
            remote_task_id: None,
            remote_context_id: None,
            result: None,
            result_path: None,
            error: None,
            error_kind: None,
            result_schema,
            structured_result: None,
            last_remote_task_snapshot: None,
            wake_on_completion,
            task_id: None,
            agent_config: Some(agent.clone()),
            network_access: None,
        }
    }

    fn public_json(&self) -> Value {
        json!({
            "agent_run_id": self.run_id,
            "kind": self.kind,
            "external_agent_id": self.external_agent_id,
            "external_agent_name": self.external_agent_name,
            "instructions": self.instructions,
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

fn require_storage(
    context: &ToolContext,
) -> std::result::Result<&Arc<dyn SessionStorageStore>, ToolExecutionResult> {
    context.storage_store.as_ref().ok_or_else(|| {
        ToolExecutionResult::tool_error("Agent delegation tools require storage_store context")
    })
}

/// Prefix shared by all agent run KV keys. Single source of truth: `run_key`
/// builds keys from it, and `session_storage` reserves it from the user-facing
/// `kv_store` tool (see `is_internal_session_kv_key`) so session/tool actors
/// cannot forge or read A2A run records.
pub(crate) use super::AGENT_RUN_KEY_PREFIX;

/// KV key for a specific agent run record.
fn run_key(run_id: &str) -> String {
    format!("{AGENT_RUN_KEY_PREFIX}{run_id}")
}

/// List run_ids by prefix-filtering the session's KV keys (test helper).
#[cfg(test)]
async fn list_run_ids(
    storage: &dyn SessionStorageStore,
    session_id: everruns_contracts::typed_id::SessionId,
) -> Vec<String> {
    storage
        .list_keys(session_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|info| {
            info.key
                .strip_prefix(AGENT_RUN_KEY_PREFIX)
                .map(ToString::to_string)
        })
        .collect()
}

use super::util::require_str_trimmed as require_str;

async fn save_run(context: &ToolContext, record: &AgentRunRecord) -> Result<()> {
    mirror_run_to_task(context, record).await;
    let Some(storage) = &context.storage_store else {
        return Ok(());
    };
    let serialized = serde_json::to_string(record).map_err(|e| {
        everruns_contracts::error::AgentLoopError::store(format!(
            "failed to serialize agent run: {e}"
        ))
    })?;
    storage
        .set_value(context.session_id, &run_key(&record.run_id), &serialized)
        .await?;
    Ok(())
}

/// A2A run status → session task state (knowledge/runtime-resources/session-tasks.md). Rejection is
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
            kind: record
                .error_kind
                .clone()
                .unwrap_or_else(|| "remote_failed".to_string()),
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
    let storage = require_storage(context)?;
    let Some(serialized) = storage
        .get_value(context.session_id, &run_key(run_id))
        .await
        .map_err(ToolExecutionResult::internal_error)?
    else {
        return Err(ToolExecutionResult::tool_error(format!(
            "No agent run found with id: {run_id}"
        )));
    };
    serde_json::from_str(&serialized).map_err(|e| {
        ToolExecutionResult::internal_error_msg(format!("Invalid agent run record: {e}"))
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

fn first_data_artifact(task: &Task) -> Option<&Value> {
    task.artifacts
        .as_ref()?
        .iter()
        .flat_map(|artifact| artifact.parts.iter())
        .find_map(|part| match &part.content {
            PartContent::Data(value) => Some(value),
            _ => None,
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
    record.structured_result = first_data_artifact(task).cloned();
    record.last_remote_task_snapshot = Some(bounded_task_snapshot(task));
}

async fn write_result_artifact(context: &ToolContext, record: &mut AgentRunRecord) -> Result<()> {
    if let Some(schema) = record.result_schema.clone() {
        if record.status != AgentRunStatus::Completed {
            return Ok(());
        }
        let Some(value) = record.structured_result.clone() else {
            record.status = AgentRunStatus::Failed;
            record.error_kind = Some("no_result".to_string());
            set_error(
                record,
                "External agent completed without a structured result artifact".to_string(),
            );
            record.result = None;
            record.result_path = None;
            return Ok(());
        };
        let errors = schema_validation_errors(&schema, &value);
        if !errors.is_empty() {
            record.status = AgentRunStatus::Failed;
            record.error_kind = Some("schema_mismatch".to_string());
            set_error(
                record,
                format!(
                    "External agent result did not match result_schema: {}",
                    errors.join("; ")
                ),
            );
            record.result = None;
            record.result_path = None;
            return Ok(());
        }
        let Some(task_id) = record.task_id.as_deref() else {
            record.status = AgentRunStatus::Failed;
            record.error_kind = Some("result_write_failed".to_string());
            set_error(
                record,
                "Structured result has no local session task".to_string(),
            );
            return Ok(());
        };
        let Some(result_path) = write_task_result_value(context, task_id, &value).await? else {
            record.status = AgentRunStatus::Failed;
            record.error_kind = Some("result_write_failed".to_string());
            set_error(
                record,
                "Structured result could not be persisted".to_string(),
            );
            return Ok(());
        };
        record.result_path = Some(result_path);
        record.result = Some(value.to_string());
        return Ok(());
    }

    let Some(file_store) = context.runtime_artifact_file_store() else {
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
    let Some(platform_store) = &context.subagent_delegate else {
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
) -> std::result::Result<a2a::Client, String> {
    build_client_with_resolver(agent, context, None).await
}

async fn build_client_with_resolver(
    agent: &ExternalA2aAgentConfig,
    context: &ToolContext,
    resolver: Option<&DnsResolver>,
) -> std::result::Result<a2a::Client, String> {
    enforce_network_access_pre_resolve(agent, context)?;
    let card = agent.resolve_card_with_resolver(resolver).await?;
    enforce_network_access_post_resolve(&card, context)?;
    a2a::client_for_card(
        &card,
        agent.preferred_binding.as_deref(),
        &agent.headers,
        context.network_access.as_ref(),
        agent.local_urls_permitted(),
        resolver,
    )
    .await
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
    if record.status.is_terminal() {
        write_result_artifact(context, record)
            .await
            .map_err(|e| e.to_string())?;
    }
    save_run(context, record).await.map_err(|e| e.to_string())
}

/// Poll a remote A2A task until it reaches a terminal state or the deadline
/// expires. Sends a registry heartbeat on every poll iteration when
/// `heartbeat_attempt` is provided, so the reaper knows the worker is alive
/// and stale writes from a superseded executor are rejected.
/// Terminal outcome of a poll loop, replacing the previous stringly-typed
/// control flow (timeout/supersede were detected via `error.starts_with(...)`).
/// `Err(String)` is still used for genuine I/O/transport failures; the
/// non-error terminal states (completed, timed out, superseded) are typed.
enum WaitOutcome {
    /// The run reached a terminal status (or had no remote task to poll).
    /// Boxed: `AgentRunRecord` is ~900 bytes and dwarfs the other variants;
    /// boxing keeps the enum small (clippy `large_enum_variant`).
    Completed(Box<AgentRunRecord>),
    /// The poll deadline elapsed before the run finished.
    TimedOut { run_id: String, timeout_secs: u64 },
    /// The attempt fence revealed a newer executor owns this task.
    Superseded {
        run_id: String,
        attempt: i32,
        by_attempt: i32,
    },
}

impl WaitOutcome {
    /// User-facing timeout message. Kept byte-identical to the legacy string so
    /// `timeout_or_error_result` surfaces the same text downstream.
    fn timed_out_message(run_id: &str, timeout_secs: u64) -> String {
        format!("Timed out waiting for external agent run {run_id} after {timeout_secs}s")
    }

    /// Diagnostic supersede message. Logged only; never surfaced to callers.
    fn superseded_message(run_id: &str, attempt: i32, by_attempt: i32) -> String {
        format!(
            "{SUPERSEDED_ERROR_PREFIX} run {run_id} (attempt {attempt} superseded by {by_attempt})"
        )
    }
}

/// Heartbeat once through the registry. A fence miss means a newer executor
/// owns the task — stop before building a client or writing failure state.
async fn heartbeat_or_superseded(
    context: &ToolContext,
    record: &AgentRunRecord,
    attempt: i32,
) -> Option<WaitOutcome> {
    let (Some(registry), Some(task_id)) =
        (&context.session_task_registry, record.task_id.as_deref())
    else {
        return None;
    };
    let heartbeat = registry
        .update(
            context.session_id,
            task_id,
            SessionTaskUpdate {
                heartbeat_at: Some(chrono::Utc::now()),
                expected_attempt: Some(attempt),
                ..Default::default()
            },
        )
        .await;
    if let Ok(Some(task)) = heartbeat
        && task.attempt != attempt
    {
        return Some(WaitOutcome::Superseded {
            run_id: record.run_id.clone(),
            attempt,
            by_attempt: task.attempt,
        });
    }
    None
}

async fn wait_for_run(
    context: &ToolContext,
    agent: &ExternalA2aAgentConfig,
    mut record: AgentRunRecord,
    timeout_secs: u64,
    // When Some, write a heartbeat on every poll with this attempt fence so
    // a superseded executor's stale writes are rejected.
    heartbeat_attempt: Option<i32>,
) -> std::result::Result<WaitOutcome, String> {
    if record.status.is_terminal() {
        write_result_artifact(context, &mut record)
            .await
            .map_err(|e| e.to_string())?;
        save_run(context, &record)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(WaitOutcome::Completed(Box::new(record)));
    }
    let Some(remote_task_id) = record.remote_task_id.clone() else {
        return Ok(WaitOutcome::Completed(Box::new(record)));
    };
    // Check the fence before DNS-pinning / building the A2A client so a
    // superseded executor never opens outbound connections (EVE-1173).
    if let Some(attempt) = heartbeat_attempt
        && let Some(superseded) = heartbeat_or_superseded(context, &record, attempt).await
    {
        return Ok(superseded);
    }
    let client = build_client(agent, context).await?;
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let poll_interval = Duration::from_millis(
        agent
            .poll_interval_ms
            .unwrap_or(DEFAULT_POLL_INTERVAL_MS)
            .max(100),
    );

    while Instant::now() < deadline {
        // Heartbeat through the registry so the reaper sees a live worker.
        // A fence miss (returned attempt differs from ours) means the reaper
        // superseded this executor — stop polling immediately so we never
        // write failure state over the new attempt's work.
        if let Some(attempt) = heartbeat_attempt
            && let Some(superseded) = heartbeat_or_superseded(context, &record, attempt).await
        {
            return Ok(superseded);
        }

        let task = client
            .get_task(&GetTaskRequest {
                id: remote_task_id.clone(),
                history_length: Some(10),
                tenant: None,
            })
            .await
            .map_err(|e| format!("A2A get_task failed: {e}"))?;
        apply_task(&mut record, &task);
        if record.status.is_terminal() {
            write_result_artifact(context, &mut record)
                .await
                .map_err(|e| e.to_string())?;
            save_run(context, &record)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(WaitOutcome::Completed(Box::new(record)));
        }
        save_run(context, &record)
            .await
            .map_err(|e| e.to_string())?;
        sleep(poll_interval).await;
    }

    Ok(WaitOutcome::TimedOut {
        run_id: record.run_id.clone(),
        timeout_secs,
    })
}

/// Render a timeout outcome as a (successful) tool result reporting
/// `timed_out: true`. Separated from the error path now that timeout is a
/// typed `WaitOutcome` variant rather than a string-prefix sniff.
async fn timed_out_result(
    context: &ToolContext,
    run_id: &str,
    message: String,
) -> ToolExecutionResult {
    match load_run(context, run_id).await {
        Ok(record) => ToolExecutionResult::success(json!({
            "agent_run_id": record.run_id,
            "status": record.status,
            "timed_out": true,
            "message": truncate_text(message),
            "remote_task_id": record.remote_task_id,
            "remote_context_id": record.remote_context_id,
        })),
        Err(e) => e,
    }
}

/// Persist a Failed run record with `message`, notify the parent, and wake it
/// when appropriate. Shared by the background monitor's timeout and error
/// paths, which previously both flowed through a single `Err(String)` arm.
async fn persist_failed_run(
    context: &ToolContext,
    run_id: &str,
    fallback_record: AgentRunRecord,
    message: String,
) {
    let mut failed = load_run(context, run_id).await.unwrap_or(fallback_record);
    failed.status = AgentRunStatus::Failed;
    set_error(&mut failed, message);
    let _ = write_result_artifact(context, &mut failed).await;
    let _ = save_run(context, &failed).await;
    post_task_completion_message(context, &failed).await;
    // Legacy wake: only when no registry is present (registry-level
    // wake_policy handles it otherwise via post_task_completion_message).
    if failed.wake_on_completion && context.session_task_registry.is_none() {
        let _ = wake_parent(context, &failed).await;
    }
}

/// Background poll loop for an A2A run. `heartbeat_attempt` is the task
/// attempt number captured at spawn/re-attach time; heartbeats carry this so
/// the fence rejects writes from a previously superseded attempt.
async fn background_monitor(
    context: ToolContext,
    agent: ExternalA2aAgentConfig,
    record: AgentRunRecord,
    timeout_secs: u64,
    heartbeat_attempt: Option<i32>,
) {
    let run_id = record.run_id.clone();
    let fallback_record = record.clone();
    let record = match wait_for_run(&context, &agent, record, timeout_secs, heartbeat_attempt).await
    {
        Ok(WaitOutcome::Completed(record)) => *record,
        // Superseded by a newer attempt (reaper re-attached the task to
        // another executor): exit silently — the new owner reports state;
        // writing failure here would overwrite its work.
        Ok(WaitOutcome::Superseded { .. }) => {
            tracing::info!(run_id = %run_id, "A2A background monitor superseded; exiting");
            return;
        }
        // Timeout and genuine errors both persist a Failed run record. Build
        // the user-facing message from the typed outcome so the persisted text
        // stays identical to the legacy string.
        Ok(WaitOutcome::TimedOut {
            run_id: timed_out_run_id,
            timeout_secs,
        }) => {
            let message = WaitOutcome::timed_out_message(&timed_out_run_id, timeout_secs);
            persist_failed_run(&context, &run_id, fallback_record, message).await;
            return;
        }
        Err(error) => {
            persist_failed_run(&context, &run_id, fallback_record, error).await;
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
        "Delegate a task to a configured external A2A agent in foreground or background mode."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "instructions": {"type": "string", "description": "Instructions to send to the external agent."},
                "target": {
                    "type": "object",
                    "properties": {
                        "type": {"type": "string", "enum": ["external_a2a"]},
                        "id": {"type": "string", "description": "Configured external A2A agent id."},
                        "external_agent_id": {"type": "string", "description": "Deprecated provider-specific spelling; prefer target.id."}
                    },
                    "required": ["type"],
                    "additionalProperties": false
                },
                "mode": {"type": "string", "enum": ["background", "foreground"], "default": "foreground"},
                "wait_timeout_secs": {"type": "integer", "minimum": 1, "maximum": 86400},
                "wake_on_completion": {"type": "boolean", "default": true},
                "result_schema": {"type": "object", "description": "JSON Schema for a required structured result artifact from the external agent."},
                "message_schema": {"type": "object", "description": "Not supported for external A2A targets; supplied values fail explicitly."}
            },
            "required": ["instructions", "target"],
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
        if let Err(e) = require_storage(context) {
            return e;
        }
        let instructions = match require_str(&arguments, "instructions") {
            Ok(instructions) => instructions.to_string(),
            Err(e) => return e,
        };
        let target = arguments.get("target").unwrap_or(&Value::Null);
        if target.get("type").and_then(Value::as_str) != Some("external_a2a") {
            return ToolExecutionResult::tool_error(
                "spawn_agent currently supports target.type = external_a2a",
            );
        }
        if arguments
            .get("lifetime")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "detached")
        {
            return ToolExecutionResult::tool_error(
                "lifetime=\"detached\" is only valid for local session targets (subagent or agent), not external_a2a.",
            );
        }
        let external_agent_id = match target
            .get("id")
            .or_else(|| target.get("external_agent_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(id) => id,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: target.id");
            }
        };
        let Some(agent) = self.config.agent(external_agent_id).cloned() else {
            return ToolExecutionResult::tool_error(format!(
                "Unknown external A2A agent: {external_agent_id}"
            ));
        };
        let mode = match arguments.get("mode").and_then(Value::as_str) {
            None => SpawnMode::Foreground,
            Some(value) => match SpawnMode::parse(value) {
                Some(mode) => mode,
                None => {
                    return ToolExecutionResult::tool_error(format!(
                        "Invalid mode: {value}. Expected background, foreground"
                    ));
                }
            },
        };
        let timeout_secs = arguments
            .get("wait_timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_WAIT_TIMEOUT_SECS);
        let wake_on_completion = arguments
            .get("wake_on_completion")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let result_schema = match normalize_result_schema(&arguments) {
            Ok(schema) => schema,
            Err(error) => return error,
        };
        if arguments
            .get("message_schema")
            .is_some_and(|schema| !schema.is_null())
        {
            return ToolExecutionResult::tool_error(
                "message_schema is not supported for external_a2a targets because remote agents cannot receive report_task_progress.",
            );
        }
        if result_schema.is_some()
            && (context.session_task_registry.is_none() || context.file_store.is_none())
        {
            return ToolExecutionResult::tool_error(
                "result_schema for external_a2a requires session_task_registry and file_store context.",
            );
        }
        let run_id = run_id();
        let mut record = AgentRunRecord::new(
            run_id.clone(),
            &agent,
            instructions.clone(),
            mode,
            wake_on_completion,
            result_schema.clone(),
        );
        record.network_access = context.network_access.clone();
        // Create the session task tracking this run (knowledge/runtime-resources/session-tasks.md).
        // Background runs must be task-backed before any remote work starts so
        // wait_task/message_task/cancel_task have a usable control handle.
        // run_id is stored in spec so load_run_for_task can do a direct key lookup.
        match &context.session_task_registry {
            Some(task_registry) => {
                match task_registry
                    .create(CreateSessionTask {
                        session_id: context.session_id,
                        id: None,
                        kind: TASK_KIND_EXTERNAL_AGENT.to_string(),
                        display_name: agent.name.clone(),
                        spec: json!({
                            "run_id": &run_id,
                            "external_agent_id": agent.id,
                            "instructions": &instructions,
                            "mode": &mode,
                            "result_schema": result_schema,
                        }),
                        state: SessionTaskState::Queued,
                        links: TaskLinks::default(),
                        wake_policy: match mode {
                            SpawnMode::Background => TaskWakePolicy::OnTerminal,
                            SpawnMode::Foreground => TaskWakePolicy::Silent,
                        },
                    })
                    .await
                {
                    Ok(created) => record.task_id = Some(created.id),
                    Err(e) if mode == SpawnMode::Background || result_schema.is_some() => {
                        // Background runs must be task-backed before remote work
                        // starts; surface this as a user-facing tool error (per
                        // the capability contract) rather than an internal error,
                        // and do not launch the run.
                        return ToolExecutionResult::tool_error(format!(
                            "Background spawn_agent could not create its session task, so the run \
                             was not started: {e}"
                        ));
                    }
                    Err(_) => {}
                }
            }
            None if mode == SpawnMode::Background => {
                return ToolExecutionResult::tool_error(
                    "Background spawn_agent requires session_task_registry context so the run can be controlled with wait_task/message_task/cancel_task",
                );
            }
            None => {}
        }
        if let Err(e) = save_run(context, &record).await {
            return ToolExecutionResult::internal_error(e);
        }
        if let Err(error) =
            submit_run(context, &agent, &mut record, &instructions, None, None).await
        {
            record.status = AgentRunStatus::Failed;
            set_error(&mut record, error);
            let _ = save_run(context, &record).await;
            return ToolExecutionResult::success(record.public_json());
        }
        match mode {
            SpawnMode::Background => {
                let context = context.clone();
                // Capture the attempt at spawn time (1 for a fresh spawn).
                // The heartbeat loop uses this to fence stale writes from any
                // future superseded attempt.
                let heartbeat_attempt = record.task_id.is_some().then_some(1i32);
                let background_record = record.clone();
                tokio::spawn(async move {
                    background_monitor(
                        context,
                        agent,
                        background_record,
                        timeout_secs,
                        heartbeat_attempt,
                    )
                    .await;
                });
                ToolExecutionResult::success(record.public_json())
            }
            SpawnMode::Foreground => {
                // Foreground wait: the tool executor owns the call stack so no
                // separate heartbeat thread is needed; pass None.
                match wait_for_run(context, &agent, record, timeout_secs, None).await {
                    Ok(WaitOutcome::Completed(record)) => {
                        ToolExecutionResult::success(record.public_json())
                    }
                    Ok(WaitOutcome::TimedOut {
                        run_id: timed_out_run_id,
                        timeout_secs,
                    }) => {
                        let message =
                            WaitOutcome::timed_out_message(&timed_out_run_id, timeout_secs);
                        timed_out_result(context, &run_id, message).await
                    }
                    // Foreground waits pass `heartbeat_attempt: None`, so the
                    // fence never fires and Superseded is unreachable here.
                    // Surface it as a tool error rather than panicking if that
                    // invariant ever changes.
                    Ok(WaitOutcome::Superseded {
                        run_id: superseded_run_id,
                        attempt,
                        by_attempt,
                    }) => ToolExecutionResult::tool_error(WaitOutcome::superseded_message(
                        &superseded_run_id,
                        attempt,
                        by_attempt,
                    )),
                    Err(error) => ToolExecutionResult::tool_error(error),
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Task executor: external_agent
// ============================================================================

/// Locate the agent run mirrored by a session task.
/// The run_id is stored in the task's spec so we can do a direct KV lookup.
fn reattach_network_access(
    record: &AgentRunRecord,
    context: &ToolContext,
) -> Option<NetworkAccessList> {
    record
        .network_access
        .clone()
        .or_else(|| context.network_access.clone())
}

async fn load_run_for_task(
    context: &ToolContext,
    task: &SessionTask,
) -> std::result::Result<AgentRunRecord, String> {
    let Some(storage) = &context.storage_store else {
        return Err("external agent tasks require storage_store context".to_string());
    };
    // Direct lookup via run_id stored in task spec (set at spawn time).
    if let Some(run_id) = task.spec.get("run_id").and_then(Value::as_str)
        && let Ok(Some(serialized)) = storage
            .get_value(context.session_id, &run_key(run_id))
            .await
    {
        return serde_json::from_str::<AgentRunRecord>(&serialized)
            .map_err(|e| format!("invalid agent run record for task {}: {e}", task.id));
    }
    Err(format!("No agent run found for task {}", task.id))
}

/// Agent config snapshot stored on the run, required to rebuild the A2A
/// client outside the capability's configured tool instances.
fn agent_snapshot(record: &AgentRunRecord) -> std::result::Result<ExternalA2aAgentConfig, String> {
    record.agent_config.clone().ok_or_else(|| {
        format!(
            "Agent run {} has no stored agent config snapshot (created before task support); use message_task/cancel_task instead",
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

    fn can_reattach(&self) -> bool {
        true
    }

    /// Re-attach to a running external A2A task after worker loss.
    ///
    /// Loads the persisted `AgentRunRecord` from session storage, then:
    /// - If already terminal: mirrors the terminal state to the registry and
    ///   returns (idempotent reconcile).
    /// - If `remote_task_id` or `agent_config` is absent: returns an error so
    ///   the reaper falls back to failing the task as orphaned.
    /// - Otherwise: rebuilds the A2A client from the stored config snapshot and
    ///   resumes the background poll loop with `heartbeat_attempt = task.attempt`
    ///   (the NEW attempt number after the reaper bumped it) so stale writes from
    ///   the superseded executor are rejected.
    async fn start(
        &self,
        task: &SessionTask,
        context: &ToolContext,
    ) -> everruns_contracts::error::Result<()> {
        let record = load_run_for_task(context, task)
            .await
            .map_err(everruns_contracts::error::AgentLoopError::tool)?;
        let context = context
            .clone()
            .with_network_access(reattach_network_access(&record, context));

        // If the run is already terminal, just mirror and return.
        if record.status.is_terminal() {
            mirror_run_to_task(&context, &record).await;
            return Ok(());
        }

        // Missing remote_task_id means we never sent to the remote agent —
        // there is nothing to poll; caller will fail this as orphaned.
        if record.remote_task_id.is_none() {
            return Err(everruns_contracts::error::AgentLoopError::tool(format!(
                "external_agent task {} has no remote_task_id; cannot re-attach",
                task.id
            )));
        }

        let agent =
            agent_snapshot(&record).map_err(everruns_contracts::error::AgentLoopError::tool)?;

        // Resume the background poll loop. Use the NEW attempt (bumped by the
        // reaper) for heartbeating so the superseded executor's stale writes
        // are rejected by the fence.
        let heartbeat_attempt = Some(task.attempt);
        tokio::spawn(async move {
            background_monitor(
                context,
                agent,
                record,
                DEFAULT_WAIT_TIMEOUT_SECS,
                heartbeat_attempt,
            )
            .await;
        });
        Ok(())
    }

    async fn deliver(
        &self,
        task: &SessionTask,
        message: &TaskMessage,
        context: &ToolContext,
    ) -> everruns_contracts::error::Result<()> {
        let mut record = load_run_for_task(context, task)
            .await
            .map_err(everruns_contracts::error::AgentLoopError::tool)?;
        let agent =
            agent_snapshot(&record).map_err(everruns_contracts::error::AgentLoopError::tool)?;
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
        .map_err(everruns_contracts::error::AgentLoopError::tool)
    }

    async fn cancel(
        &self,
        task: &SessionTask,
        context: &ToolContext,
    ) -> everruns_contracts::error::Result<()> {
        let mut record = load_run_for_task(context, task)
            .await
            .map_err(everruns_contracts::error::AgentLoopError::tool)?;
        if record.status.is_terminal() {
            return Ok(());
        }
        let Some(remote_task_id) = record.remote_task_id.clone() else {
            // Never reached the remote agent; cancel locally.
            record.status = AgentRunStatus::Canceled;
            save_run(context, &record).await?;
            return Ok(());
        };
        let agent =
            agent_snapshot(&record).map_err(everruns_contracts::error::AgentLoopError::tool)?;
        let client = build_client(&agent, context)
            .await
            .map_err(everruns_contracts::error::AgentLoopError::tool)?;
        let remote = client
            .cancel_task(&CancelTaskRequest {
                id: remote_task_id,
                metadata: None,
                tenant: None,
            })
            .await
            .map_err(|e| {
                everruns_contracts::error::AgentLoopError::tool(format!(
                    "A2A cancel_task failed: {e}"
                ))
            })?;
        apply_task(&mut record, &remote);
        save_run(context, &record).await?;
        Ok(())
    }

    async fn reconcile(
        &self,
        task: &SessionTask,
        context: &ToolContext,
    ) -> everruns_contracts::error::Result<()> {
        let mut record = load_run_for_task(context, task)
            .await
            .map_err(everruns_contracts::error::AgentLoopError::tool)?;
        if record.status.is_terminal() {
            return Ok(());
        }
        let Some(remote_task_id) = record.remote_task_id.clone() else {
            return Ok(());
        };
        let agent =
            agent_snapshot(&record).map_err(everruns_contracts::error::AgentLoopError::tool)?;
        let client = build_client(&agent, context)
            .await
            .map_err(everruns_contracts::error::AgentLoopError::tool)?;
        let remote = client
            .get_task(&GetTaskRequest {
                id: remote_task_id,
                history_length: Some(10),
                tenant: None,
            })
            .await
            .map_err(|e| {
                everruns_contracts::error::AgentLoopError::tool(format!("A2A get_task failed: {e}"))
            })?;
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
mod network_tests;
#[cfg(test)]
mod tests;
