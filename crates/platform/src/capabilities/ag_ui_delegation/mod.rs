//! Outbound AG-UI agent delegation: the `ag_ui_delegation` capability.
//!
//! An agent with this capability can hand work to external agents that speak
//! [AG-UI 1.0](https://docs.ag-ui.com). The capability config lists those
//! agents; the model starts a delegation with `spawn_agent` and
//! `target = { "type": "external_ag_ui", "id": "<configured id>" }`, and the
//! generic session-task tools (`wait_task`, `message_task`, `cancel_task`)
//! drive it from there. See `knowledge/integrations/ag-ui-capability.md`.

// Decisions:
// - Mirrors `a2a_delegation`: external agents are listed in the capability
//   config (id, name, endpoint URL, optional bearer-token secret name), the
//   model only picks a configured id, and every run is a session task so the
//   generic `wait_task` / `message_task` / `cancel_task` tools work unchanged.
// - Own task kind (`external_ag_ui`): `find_task_executor` picks the first
//   executor for a kind, so sharing A2A's `external_agent` would route AG-UI
//   tasks to the A2A executor.
// - AG-UI is one streamed HTTP request per run with no "get run" call, so the
//   stream is the only handle. A stream lost with its worker cannot be
//   re-attached; the reaper fails it as orphaned (`can_reattach = false`).
// - A remote interrupt (`RUN_FINISHED` with outcome `interrupt`) ends the
//   stream and parks the task in `awaiting_input`; `message_task` answers it
//   by starting the resuming run on the same thread.
// - Run state lives in the reserved `agent_run:{run_id}` session KV key, the
//   same prefix A2A uses, so the user-facing `kv_store` tool cannot forge it.

mod run;
#[cfg(test)]
mod tests;

use super::delegation_result::normalize_result_schema;
use super::{
    Capability, CapabilityLocalization, CapabilityStatus, RiskLevel, SESSION_TASKS_CAPABILITY_ID,
    SpawnMode, SystemPromptContext,
};
use async_trait::async_trait;
use everruns_ag_ui::Message;
use everruns_core::session_task::{
    CreateSessionTask, SessionTaskState, TASK_KIND_EXTERNAL_AG_UI, TaskLinks, TaskWakePolicy,
};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_provider::tool_types::ToolHints;
use everruns_provider::url_validation::validate_safe_url;
use run::{AgUiRunRecord, DriveOutcome, drive_run, first_input, save_run, spawn_background};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub use super::AG_UI_DELEGATION_CAPABILITY_ID;
pub use run::AgUiAgentTaskExecutor;

/// `spawn_agent` target type served by this capability. Equal to the task
/// kind, [`TASK_KIND_EXTERNAL_AG_UI`].
pub const AG_UI_TARGET_TYPE: &str = "external_ag_ui";
const DEFAULT_WAIT_TIMEOUT_SECS: u64 = 300;
const MAX_WAIT_TIMEOUT_SECS: u64 = 86_400;

/// Headers that carry credentials. A credential belongs in a session secret
/// named by `bearer_token_secret`, never in the stored, non-secret `headers`.
const CREDENTIAL_HEADERS: &[&str] = &["authorization", "proxy-authorization", "cookie"];

/// Outbound AG-UI delegation capability (`ag_ui_delegation`).
///
/// Contributes no tools of its own: it registers the `external_ag_ui`
/// provider of the unified `spawn_agent` tool and depends on `session_tasks`
/// for the task tools. High risk, so only admins can assign it.
pub struct AgUiDelegationCapability;

#[async_trait]
impl Capability for AgUiDelegationCapability {
    fn id(&self) -> &str {
        AG_UI_DELEGATION_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "AG-UI Agent Delegation"
    }

    fn description(&self) -> &str {
        "Delegate work to configured external agents over the AG-UI protocol."
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
                    "description": "External AG-UI agents available for delegation.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "title": "Agent ID",
                                "description": "Stable ID used in spawn_agent target.id."
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
                            "url": {
                                "type": "string",
                                "title": "Endpoint URL",
                                "description": "The agent's AG-UI endpoint. Each run is a POST of RunAgentInput answered with an SSE event stream."
                            },
                            "bearer_token_secret": {
                                "type": "string",
                                "title": "Bearer token secret",
                                "description": "Name of the session secret holding the bearer token sent as Authorization. The value is read when a run starts and never stored with the run."
                            },
                            "headers": {
                                "type": "object",
                                "title": "Headers",
                                "additionalProperties": { "type": "string" },
                                "description": "Non-secret static headers. Credential headers are refused; use bearer_token_secret."
                            },
                            "allow_local_urls": {
                                "type": "boolean",
                                "title": "Allow local URLs",
                                "description": "Testing/dev escape hatch for localhost AG-UI agents. Keep false in production.",
                                "default": false
                            }
                        },
                        "required": ["id", "name", "url"],
                        "additionalProperties": false
                    },
                    "default": []
                }
            },
            "additionalProperties": false
        }))
    }

    fn validate_config(&self, config: &Value) -> std::result::Result<(), String> {
        let parsed = AgUiDelegationConfig::from_value(config)
            .map_err(|e| format!("invalid {AG_UI_DELEGATION_CAPABILITY_ID} config: {e}"))?;
        let mut seen = std::collections::BTreeSet::new();
        for agent in &parsed.agents {
            agent.validate()?;
            if !seen.insert(agent.id.as_str()) {
                return Err(format!("AG-UI agent id {} is listed twice", agent.id));
            }
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
                    "Defines the external AG-UI agents this agent may delegate work to and \
                     how to reach them.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Делегування агентам AG-UI"),
                description: Some(
                    "Делегує роботу налаштованим зовнішнім агентам за протоколом AG-UI.",
                ),
                config_description: Some(
                    "Визначає зовнішніх агентів AG-UI, яким цей агент може делегувати роботу, та параметри підключення до них.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        "agents": {
                            "title": "Зовнішні агенти",
                            "description": "Зовнішні агенти AG-UI, доступні для делегування.",
                            "items": {
                                "properties": {
                                    "id": {
                                        "title": "Ідентифікатор агента",
                                        "description": "Стабільний ідентифікатор, що використовується у spawn_agent (target.id)."
                                    },
                                    "name": {
                                        "title": "Назва",
                                        "description": "Зрозуміла людині назва зовнішнього агента."
                                    },
                                    "description": {
                                        "title": "Опис",
                                        "description": "Необов'язковий опис того, що робить зовнішній агент."
                                    },
                                    "url": {
                                        "title": "URL кінцевої точки",
                                        "description": "Кінцева точка AG-UI агента. Кожен запуск надсилає RunAgentInput методом POST і отримує потік подій SSE."
                                    },
                                    "bearer_token_secret": {
                                        "title": "Секрет із токеном",
                                        "description": "Назва секрету сесії з bearer-токеном для заголовка Authorization. Значення читається на початку запуску й ніколи не зберігається разом із запуском."
                                    },
                                    "headers": {
                                        "title": "Заголовки",
                                        "description": "Несекретні статичні заголовки. Заголовки з обліковими даними відхиляються; використовуйте bearer_token_secret."
                                    },
                                    "allow_local_urls": {
                                        "title": "Дозволити локальні URL",
                                        "description": "Обхідний шлях для тестування та розробки з локальними агентами AG-UI. У продакшені тримайте вимкненим."
                                    }
                                }
                            }
                        }
                    }
                })),
            },
        ]
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![]
    }

    fn delegation_target_with_config(
        &self,
        config: &Value,
    ) -> Option<super::DelegationTargetProvider> {
        Some(super::DelegationTargetProvider {
            target_type: AG_UI_TARGET_TYPE,
            tool: Box::new(SpawnAgUiAgentTool::new(
                AgUiDelegationConfig::from_value(config).unwrap_or_default(),
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
        let config = AgUiDelegationConfig::from_value(config).unwrap_or_default();
        let agents = config
            .agents
            .iter()
            .map(|agent| {
                format!(
                    "- {} ({}) — {}",
                    agent.name,
                    agent.id,
                    agent
                        .description
                        .as_deref()
                        .unwrap_or("External AG-UI agent")
                )
            })
            .collect::<Vec<_>>();

        Some(format!(
            "<capability id=\"{}\">\n\
Delegate work to configured external AG-UI agents with spawn_agent (target.type=\"{AG_UI_TARGET_TYPE}\").\n\
Use mode=\"background\" for long-running work and wait_task (from session_tasks) later for results; use mode=\"foreground\" when blocked on the result.\n\
When a run stops with status input_required, the remote agent is asking something: answer with message_task. To answer several open interrupts differently, send a JSON object keyed by interrupt id. Use cancel_task to stop a run.\n\
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
pub(crate) struct AgUiDelegationConfig {
    #[serde(default)]
    agents: Vec<AgUiAgentConfig>,
}

impl AgUiDelegationConfig {
    fn from_value(value: &Value) -> serde_json::Result<Self> {
        if value.is_null() {
            Ok(Self::default())
        } else {
            serde_json::from_value(value.clone())
        }
    }

    fn agent(&self, id: &str) -> Option<&AgUiAgentConfig> {
        self.agents.iter().find(|agent| agent.id == id)
    }
}

/// One configured AG-UI agent. A snapshot is stored on every run so the
/// task executor can resume it without the capability config; nothing in it
/// is secret (the bearer token is referenced by secret name only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgUiAgentConfig {
    id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    url: String,
    #[serde(default)]
    bearer_token_secret: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    allow_local_urls: bool,
}

impl AgUiAgentConfig {
    fn validate(&self) -> std::result::Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("AG-UI agent id cannot be empty".to_string());
        }
        if self.name.trim().is_empty() {
            return Err(format!("AG-UI agent {} name cannot be empty", self.id));
        }
        self.validate_url()?;
        if let Some(secret) = &self.bearer_token_secret
            && (secret.is_empty()
                || secret.len() > 128
                || !secret
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
        {
            return Err(format!(
                "AG-UI agent {} bearer_token_secret must be a secret name (1-128 of A-Z, a-z, 0-9, '_', '-', '.')",
                self.id
            ));
        }
        for (name, value) in &self.headers {
            // THREAT[TM-AGENT-032]: credentials never sit in the stored config.
            if CREDENTIAL_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                return Err(format!(
                    "AG-UI agent {} header {name} carries credentials; use bearer_token_secret",
                    self.id
                ));
            }
            reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| format!("AG-UI agent {} has invalid header {name}: {e}", self.id))?;
            reqwest::header::HeaderValue::from_str(value).map_err(|e| {
                format!("AG-UI agent {} has invalid value for {name}: {e}", self.id)
            })?;
        }
        Ok(())
    }

    /// Shape check of the endpoint. Private and metadata addresses are
    /// refused unless `allow_local_urls`; the request itself re-checks with
    /// DNS pinning (see `run::build_client`).
    fn validate_url(&self) -> std::result::Result<(), String> {
        if self.allow_local_urls {
            validate_http_url(&self.url)
                .map_err(|e| format!("AG-UI agent {} has invalid url: {e}", self.id))
        } else {
            validate_safe_url(&self.url)
                .map(|_| ())
                .map_err(|e| format!("AG-UI agent {} has unsafe url: {e}", self.id))
        }
    }
}

fn validate_http_url(raw_url: &str) -> std::result::Result<(), String> {
    let url = url::Url::parse(raw_url).map_err(|e| e.to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("disallowed scheme {other}; expected http or https")),
    }
    if url.host_str().is_none() {
        return Err("URL must have a hostname".to_string());
    }
    Ok(())
}

/// The `spawn_agent` provider for `target.type = "external_ag_ui"`.
///
/// Resolves `target.id` against the configured agents, creates the
/// `external_ag_ui` session task, and streams the first run: inline in
/// foreground mode (bounded by `wait_timeout_secs`), on a spawned task in
/// background mode. The returned JSON carries `agent_run_id`, `task_id` and
/// `status`, plus `result` once the run has ended.
#[derive(Clone)]
pub struct SpawnAgUiAgentTool {
    config: AgUiDelegationConfig,
}

impl SpawnAgUiAgentTool {
    fn new(config: AgUiDelegationConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for SpawnAgUiAgentTool {
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
        "Delegate a task to a configured external AG-UI agent in foreground or background mode."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "instructions": {"type": "string", "description": "Instructions to send to the external agent."},
                "target": {
                    "type": "object",
                    "properties": {
                        "type": {"type": "string", "enum": [AG_UI_TARGET_TYPE]},
                        "id": {"type": "string", "description": "Configured external AG-UI agent id."}
                    },
                    "required": ["type", "id"],
                    "additionalProperties": false
                },
                "mode": {"type": "string", "enum": ["background", "foreground"], "default": "foreground"},
                "wait_timeout_secs": {"type": "integer", "minimum": 1, "maximum": MAX_WAIT_TIMEOUT_SECS},
                "wake_on_completion": {"type": "boolean", "default": true},
                "result_schema": {"type": "object", "description": "JSON Schema the agent's RUN_FINISHED result must match."}
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
        if context.storage_store.is_none() {
            return ToolExecutionResult::tool_error(
                "Agent delegation tools require storage_store context",
            );
        }
        let instructions = match super::util::require_str_trimmed(&arguments, "instructions") {
            Ok(instructions) => instructions.to_string(),
            Err(e) => return e,
        };
        let target = arguments.get("target").unwrap_or(&Value::Null);
        if target.get("type").and_then(Value::as_str) != Some(AG_UI_TARGET_TYPE) {
            return ToolExecutionResult::tool_error(format!(
                "spawn_agent here supports target.type = {AG_UI_TARGET_TYPE}"
            ));
        }
        if arguments
            .get("message_schema")
            .is_some_and(|schema| !schema.is_null())
        {
            return ToolExecutionResult::tool_error(format!(
                "message_schema is not supported for {AG_UI_TARGET_TYPE} targets because remote agents cannot receive report_task_progress."
            ));
        }
        // THREAT[TM-AGENT-030]: the model names a configured agent; the URL
        // always comes from the admin-set config, never from the arguments.
        let Some(agent_id) = target
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolExecutionResult::tool_error("Missing required parameter: target.id");
        };
        let Some(agent) = self.config.agent(agent_id).cloned() else {
            return ToolExecutionResult::tool_error(format!(
                "Unknown external AG-UI agent: {agent_id}"
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
            .unwrap_or(DEFAULT_WAIT_TIMEOUT_SECS)
            .clamp(1, MAX_WAIT_TIMEOUT_SECS);
        let wake_on_completion = arguments
            .get("wake_on_completion")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let result_schema = match normalize_result_schema(&arguments) {
            Ok(schema) => schema,
            Err(error) => return error,
        };
        if result_schema.is_some()
            && (context.session_task_registry.is_none() || context.file_store.is_none())
        {
            return ToolExecutionResult::tool_error(format!(
                "result_schema for {AG_UI_TARGET_TYPE} requires session_task_registry and file_store context."
            ));
        }
        if mode == SpawnMode::Background && context.session_task_registry.is_none() {
            return ToolExecutionResult::tool_error(
                "Background spawn_agent requires session_task_registry context so the run can be controlled with wait_task/message_task/cancel_task",
            );
        }

        let mut record = AgUiRunRecord::new(
            &agent,
            instructions.clone(),
            mode,
            wake_on_completion,
            result_schema.clone(),
        );
        record.network_access = context.network_access.clone();
        record.timeout_secs = timeout_secs;
        if let Some(registry) = &context.session_task_registry {
            let created = registry
                .create(CreateSessionTask {
                    session_id: context.session_id,
                    id: None,
                    kind: TASK_KIND_EXTERNAL_AG_UI.to_string(),
                    display_name: agent.name.clone(),
                    spec: json!({
                        "run_id": &record.run_id,
                        "external_agent_id": agent.id,
                        "instructions": &instructions,
                        "mode": &mode,
                        "result_schema": result_schema,
                    }),
                    state: SessionTaskState::Queued,
                    links: TaskLinks::default(),
                    // A background run also wakes the parent when the remote
                    // agent stops to ask something, not only when it ends.
                    wake_policy: match (mode, wake_on_completion) {
                        (SpawnMode::Background, true) => TaskWakePolicy::OnActivity,
                        _ => TaskWakePolicy::Silent,
                    },
                })
                .await;
            match created {
                Ok(task) => record.task_id = Some(task.id),
                Err(e) if mode == SpawnMode::Background || result_schema.is_some() => {
                    return ToolExecutionResult::tool_error(format!(
                        "spawn_agent could not create its session task, so the run was not started: {e}"
                    ));
                }
                Err(_) => {}
            }
        }
        if let Err(e) = save_run(context, &record).await {
            return ToolExecutionResult::internal_error(e);
        }

        let input = first_input(&mut record, Message::user(run::message_id(), instructions));
        match mode {
            SpawnMode::Background => {
                spawn_background(context.clone(), agent, record.clone(), input, timeout_secs);
                ToolExecutionResult::success(record.public_json())
            }
            SpawnMode::Foreground => {
                match drive_run(context, &agent, record, input, timeout_secs).await {
                    DriveOutcome::Finished(record) => {
                        ToolExecutionResult::success(record.public_json())
                    }
                    DriveOutcome::TimedOut(record) => ToolExecutionResult::success(json!({
                        "agent_run_id": record.run_id,
                        "task_id": record.task_id,
                        "status": record.status,
                        "timed_out": true,
                        "message": format!(
                            "Timed out waiting for external AG-UI agent run {} after {timeout_secs}s",
                            record.run_id
                        ),
                    })),
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}
