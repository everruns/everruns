// Rows the agent mcp secret bindings repository reads and writes.

use crate::kernel_imports::contracts::typed_id::AgentId;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct AgentMcpSecretBindingRow {
    pub id: Uuid,
    pub org_id: i64,
    pub agent_id: AgentId,
    pub mcp_server_name: String,
    pub mcp_server_url: String,
    pub tool_name: String,
    pub parameter_name: String,
    pub label: String,
    pub description: Option<String>,
    pub value_encrypted: Option<Vec<u8>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertAgentMcpSecretBindingRow {
    pub org_id: i64,
    pub agent_id: AgentId,
    pub mcp_server_name: String,
    pub mcp_server_url: String,
    pub tool_name: String,
    pub parameter_name: String,
    pub label: String,
    pub description: Option<String>,
}
