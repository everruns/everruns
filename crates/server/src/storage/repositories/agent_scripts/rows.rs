// Rows the agent scripts repository reads and writes.

use crate::kernel_imports::contracts::typed_id::AgentId;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::ScriptId;
use everruns_server_macros::Columns;
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, Columns)]
pub struct AgentScriptRow {
    pub id: ScriptId,
    pub org_id: i64,
    pub agent_id: AgentId,
    pub name: String,
    pub description: String,
    pub input_schema: Option<serde_json::Value>,
    pub body: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateAgentScriptRow {
    pub org_id: i64,
    pub id: ScriptId,
    pub agent_id: AgentId,
    pub name: String,
    pub description: String,
    pub input_schema: Option<serde_json::Value>,
    pub body: String,
}

/// Name is immutable; only provided fields change.
#[derive(Debug, Clone, Default)]
pub struct UpdateAgentScript {
    pub description: Option<String>,
    pub input_schema: Option<serde_json::Value>,
    pub body: Option<String>,
}
