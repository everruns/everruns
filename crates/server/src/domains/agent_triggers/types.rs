// Agent-triggers domain types — request shapes for the HTTP/MCP surface.
//
// Storage row types are re-exported from `storage::models`. The stored `config`
// column is a JSONB blob parsed via `ScheduleTriggerConfig`; the request DTOs
// below are the flat shape callers send, which the commands normalize into that
// config.

use chrono::{DateTime, Utc};
use everruns_platform::InvocationSessionMode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub use crate::storage::models::{AgentTriggerRow, CreateAgentTriggerRow, UpdateAgentTrigger};

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

/// Request to create a schedule trigger on an agent.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateAgentTriggerRequest {
    /// Cron expression that drives the durable schedule. Accepts 5-field
    /// (min hour day month weekday) or 7-field (sec … year) form.
    #[schema(example = "0 9 * * *")]
    pub cron_expression: String,
    /// IANA timezone identifier for cron evaluation (default `UTC`).
    #[serde(default = "default_timezone")]
    #[schema(example = "UTC")]
    pub timezone: String,
    /// Whether invocations reuse a stable session or create a new one.
    #[serde(default)]
    pub session_mode: InvocationSessionMode,
    /// Message content or `{{template}}` sent when the schedule fires.
    #[schema(example = "Run the daily digest")]
    pub message: String,
    /// Whether the trigger is active on creation (default `true`).
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

/// Request to update a schedule trigger. Only provided fields change; the rest
/// are preserved from the stored config.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct UpdateAgentTriggerRequest {
    /// Replacement cron expression.
    #[serde(default)]
    pub cron_expression: Option<String>,
    /// Replacement IANA timezone identifier.
    #[serde(default)]
    pub timezone: Option<String>,
    /// Replacement session reuse strategy.
    #[serde(default)]
    pub session_mode: Option<InvocationSessionMode>,
    /// Replacement message sent when the trigger fires.
    #[serde(default)]
    pub message: Option<String>,
    /// Replacement enabled state.
    #[serde(default)]
    pub enabled: Option<bool>,
}

fn default_timezone() -> String {
    "UTC".to_string()
}

fn default_enabled() -> bool {
    true
}
