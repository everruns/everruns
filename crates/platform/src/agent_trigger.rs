// Agent trigger domain types
//
// Design Decision:
// - AgentTrigger is an org-scoped, agent-owned entity that describes how an
//   agent gets invoked autonomously (e.g. on a schedule). It mirrors the
//   AgentIdentity CRUD shape (see `agent_identity.rs`).
// - The concrete per-type configuration lives in `config` (JSONB). Typed
//   accessors parse it on demand, mirroring `AppChannel::schedule_config()`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// Reuse the app-side invocation/schedule config so schedule triggers and
// schedule channels share one shape. Do not duplicate these.
use crate::app::InvocationSessionMode;
use everruns_provider::typed_id::{AgentId, TriggerId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// The kind of event that fires an agent trigger. Only scheduled triggers exist
/// today; the enum leaves room for webhook/event triggers later.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum AgentTriggerType {
    /// Cron-driven schedule trigger.
    #[default]
    Schedule,
}

impl std::fmt::Display for AgentTriggerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentTriggerType::Schedule => write!(f, "schedule"),
        }
    }
}

impl From<&str> for AgentTriggerType {
    fn from(_value: &str) -> Self {
        // Only one variant exists today; every stored value maps to Schedule.
        Self::Schedule
    }
}

/// Typed configuration for a `Schedule` trigger.
///
/// `message` is also the template body. `{{path.to.value}}` placeholders are
/// expanded at invocation time. Mirrors `app::ScheduleChannelConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ScheduleTriggerConfig {
    /// Cron expression that drives the durable schedule.
    pub cron_expression: String,
    /// IANA timezone identifier for cron evaluation.
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// Whether invocations reuse a stable session or create a new one.
    #[serde(default)]
    pub session_mode: InvocationSessionMode,
    /// Message content or template sent when the schedule fires.
    pub message: String,
}

fn default_timezone() -> String {
    "UTC".to_string()
}

/// AgentTrigger is a durable, agent-owned invocation trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AgentTrigger {
    /// External identifier (trg_<32-hex>). Shown as `id` in API.
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "trg_01933b5a000070008000000000000001"))]
    pub id: TriggerId,
    /// Agent that owns this trigger.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "agent_01933b5a000070008000000000000001"))]
    pub agent_id: AgentId,
    /// The kind of event that fires this trigger.
    pub trigger_type: AgentTriggerType,
    /// Type-specific configuration (parsed via typed accessors).
    pub config: serde_json::Value,
    /// Whether the trigger is currently active.
    pub enabled: bool,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Archive timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Delete timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

impl AgentTrigger {
    /// Parse `config` as [`ScheduleTriggerConfig`]. Errors if this is not a
    /// schedule trigger or the config is malformed. Mirrors
    /// `AppChannel::schedule_config()`.
    pub fn schedule_config(&self) -> anyhow::Result<ScheduleTriggerConfig> {
        if self.trigger_type != AgentTriggerType::Schedule {
            anyhow::bail!("agent trigger {} is not a schedule trigger", self.id);
        }
        Ok(serde_json::from_value(self.config.clone())?)
    }
}
