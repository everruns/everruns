// Agent script record: a saved shell script an agent owns.
//
// Design: knowledge/runtime-resources/agent-scripts.md. A small, agent-owned
// resource modeled on `AgentTrigger`, with no schedule, webhook or delivery
// machinery. Limits live in the domain (`domains::agent_scripts`).

use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::{AgentId, ScriptId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A saved shell script an agent owns.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentScript {
    /// External identifier (scr_<32-hex>). Shown as `id` in API.
    #[serde(rename = "id")]
    #[schema(value_type = String, example = "scr_01933b5a000070008000000000000001")]
    pub id: ScriptId,
    /// Agent that owns this script.
    #[schema(value_type = String, example = "agent_01933b5a000070008000000000000001")]
    pub agent_id: AgentId,
    /// Unique name among the agent's active scripts.
    #[schema(example = "daily-digest")]
    pub name: String,
    /// One-line description of what the script does.
    pub description: String,
    /// JSON Schema (`type: object`) of the input the script reads on stdin.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub input_schema: Option<serde_json::Value>,
    /// Shell script source.
    pub body: String,
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
