// Inputs and type names of the durable activities the worker runs.
//
// Turns are not queued activities: a worker runs a session's whole turn on an
// actor lease (`actor_runtime`). The queue carries only the activities below;
// `task_driver` runs them.

use serde::{Deserialize, Serialize};

/// Durable schedule target input for an agent channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledChannelInput {
    pub org_id: i64,
    pub channel_id: String,
}

/// Durable schedule target input for an agent-owned schedule trigger (EVE-757).
/// Mirrors [`ScheduledChannelInput`], re-homed on the agent + trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledAgentTriggerInput {
    pub org_id: i64,
    pub agent_id: String,
    pub trigger_id: String,
}

// ============================================================================
// Activity Type Constants
// ============================================================================

/// Activity types the worker claims. Every value is persisted in schedule
/// targets and task rows: keep them stable.
pub mod activity_types {
    pub const LEASED_RESOURCE_CLEANUP: &str = "leased_resource_cleanup";
    pub const SESSION_TASK_REAPER: &str = "session_task_reaper";
    /// Persisted schedule targets are converted to this name by the contract migration.
    pub const INVOKE_SCHEDULED_CHANNEL: &str = "invoke_scheduled_channel";
    pub const INVOKE_AGENT_TRIGGER: &str = "invoke_agent_trigger";
}

/// The activity types a worker registers for and claims.
pub fn all() -> Vec<String> {
    [
        activity_types::LEASED_RESOURCE_CLEANUP,
        activity_types::SESSION_TASK_REAPER,
        activity_types::INVOKE_SCHEDULED_CHANNEL,
        activity_types::INVOKE_AGENT_TRIGGER,
    ]
    .map(String::from)
    .to_vec()
}
