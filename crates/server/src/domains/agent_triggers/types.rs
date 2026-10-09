// Agent-triggers domain types — request shapes for the HTTP/MCP surface.
//
// Storage row types are re-exported from `crate::storage`. The stored `config`
// column is a JSONB blob parsed through its trigger-specific config type; the
// request DTOs below are the flat shape callers send, which commands normalize.

use crate::records::{AgentTriggerType, TriggerEventFilter};
use chrono::{DateTime, Utc};
use everruns_contracts::runtime::saved_scripts::ScriptRun;
use everruns_core::channel::SessionBinding;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

pub use crate::storage::{AgentTriggerRow, CreateAgentTriggerRow, UpdateAgentTrigger};

/// One recent durable execution of an agent schedule trigger.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentTriggerRun {
    /// Durable execution identifier.
    pub id: String,
    /// Current durable execution status.
    pub status: String,
    /// Time the execution was scheduled.
    pub scheduled_at: DateTime<Utc>,
    /// Time the execution completed, when terminal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Failure message for an unsuccessful execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Request to create a trigger on an agent.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateAgentTriggerRequest {
    /// Trigger kind. Omitted values retain the schedule API default.
    #[serde(default)]
    pub trigger_type: AgentTriggerType,
    /// Cron expression that drives the durable schedule. Accepts 5-field
    /// (min hour day month weekday) or 7-field (sec … year) form.
    #[schema(example = "0 9 * * *")]
    #[serde(default)]
    pub cron_expression: Option<String>,
    /// IANA timezone identifier for cron evaluation (default `UTC`).
    #[serde(default = "default_timezone")]
    #[schema(example = "UTC")]
    pub timezone: String,
    /// Whether invocations reuse a stable session or create a new one.
    #[serde(default)]
    pub session_mode: SessionBinding,
    /// Message content or `{{template}}` sent when the trigger fires.
    #[schema(example = "Run the daily digest")]
    pub message: String,
    /// Schedule and webhook only: a saved script the trigger runs instead of
    /// asking the model. The run is recorded under `message`.
    #[serde(default)]
    pub script: Option<ScriptRun>,
    /// Shared secret for webhook triggers.
    #[serde(default)]
    pub token: Option<String>,
    /// Optional per-ingress, per-IP webhook request limit.
    #[serde(default)]
    #[schema(example = 60)]
    pub rate_limit_per_minute: Option<u32>,
    /// Webhook only: template for the delivery idempotency key.
    #[serde(default)]
    #[schema(example = "{{webhook.headers.x-github-delivery}}")]
    pub event_id_template: Option<String>,
    /// Webhook and MCP event: template for the event subject. Required for
    /// `session_mode: per_thread`, which keeps one session per subject.
    #[serde(default)]
    #[schema(example = "{{webhook.json.action}}")]
    pub subject_template: Option<String>,
    /// Webhook, GitHub and MCP event: conditions an event must meet to start a run.
    #[serde(default)]
    pub filter: Option<TriggerEventFilter>,
    /// GitHub only: subscribed events (`pull_request` or
    /// `pull_request.opened`). Defaults to pull request open/update events.
    #[serde(default)]
    #[schema(example = json!(["pull_request.opened"]))]
    pub github_events: Option<Vec<String>>,
    /// GitHub only: repositories (`owner/name`) to accept; empty accepts all.
    #[serde(default)]
    #[schema(example = json!(["everruns/everruns"]))]
    pub repositories: Option<Vec<String>>,
    /// MCP event only: name of the agent's MCP server attachment to subscribe to.
    #[serde(default)]
    #[schema(example = "tracker")]
    pub mcp_server: Option<String>,
    /// MCP event only: event name from the server's `events/list`.
    #[serde(default)]
    #[schema(example = "issue.created")]
    pub mcp_event: Option<String>,
    /// MCP event only: subscription arguments object (the event's `inputSchema`).
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub mcp_event_arguments: Option<Value>,
    /// Shared endpoint auth is not supported by webhook triggers.
    #[serde(default)]
    pub auth: Option<Value>,
    /// Whether the trigger is active on creation (default `true`).
    #[serde(default = "default_enabled")]
    #[schema(example = true)]
    pub enabled: bool,
}

/// Request to update a trigger. Only provided fields change; the rest are
/// preserved from the stored config.
#[derive(Debug, Clone, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateAgentTriggerRequest {
    /// Replacement cron expression.
    #[serde(default)]
    #[schema(example = "0 9 * * 1-5")]
    pub cron_expression: Option<String>,
    /// Replacement IANA timezone identifier.
    #[serde(default)]
    #[schema(example = "UTC")]
    pub timezone: Option<String>,
    /// Replacement session reuse strategy.
    #[serde(default)]
    pub session_mode: Option<SessionBinding>,
    /// Replacement message sent when the trigger fires.
    #[serde(default)]
    #[schema(example = "Run the daily digest")]
    pub message: Option<String>,
    /// Replacement saved script. An empty script name removes it, so the
    /// trigger asks the model again.
    #[serde(default)]
    pub script: Option<ScriptRun>,
    /// Replacement webhook token.
    #[serde(default)]
    pub token: Option<String>,
    /// Replacement per-ingress, per-IP webhook request limit.
    #[serde(default)]
    #[schema(example = 60)]
    pub rate_limit_per_minute: Option<u32>,
    /// Replacement idempotency-key template. An empty string removes it.
    #[serde(default)]
    #[schema(example = "{{webhook.headers.x-github-delivery}}")]
    pub event_id_template: Option<String>,
    /// Replacement subject template. An empty string removes it.
    #[serde(default)]
    #[schema(example = "{{webhook.json.action}}")]
    pub subject_template: Option<String>,
    /// Replacement filter. A filter with no conditions removes it.
    #[serde(default)]
    pub filter: Option<TriggerEventFilter>,
    /// Replacement GitHub event subscriptions.
    #[serde(default)]
    #[schema(example = json!(["pull_request.opened"]))]
    pub github_events: Option<Vec<String>>,
    /// Replacement GitHub repository scope. An empty list accepts all.
    #[serde(default)]
    #[schema(example = json!(["everruns/everruns"]))]
    pub repositories: Option<Vec<String>>,
    /// Replacement MCP server attachment name.
    #[serde(default)]
    #[schema(example = "tracker")]
    pub mcp_server: Option<String>,
    /// Replacement MCP event name.
    #[serde(default)]
    #[schema(example = "issue.created")]
    pub mcp_event: Option<String>,
    /// Replacement MCP event subscription arguments.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub mcp_event_arguments: Option<Value>,
    /// Shared endpoint auth is not supported by webhook triggers.
    #[serde(default)]
    pub auth: Option<Value>,
    /// Replacement enabled state.
    #[serde(default)]
    #[schema(example = true)]
    pub enabled: Option<bool>,
}

fn default_timezone() -> String {
    "UTC".to_string()
}

fn default_enabled() -> bool {
    true
}
