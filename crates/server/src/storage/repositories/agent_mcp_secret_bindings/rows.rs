// Rows the agent mcp secret bindings repository reads and writes.

use crate::kernel_imports::contracts::typed_id::AgentId;
use chrono::{DateTime, Utc};
use everruns_contracts::runtime::SecretBinding;
use sqlx::FromRow;
use sqlx::types::Json;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct AgentMcpSecretBindingRow {
    pub id: Uuid,
    pub org_id: i64,
    pub agent_id: AgentId,
    /// `mcp_parameter` (the MCP columns are set) or `http` (`env_name` and
    /// `binding` are set); the table's check constraint keeps them apart.
    pub kind: String,
    pub mcp_server_name: Option<String>,
    pub mcp_server_url: Option<String>,
    pub tool_name: Option<String>,
    pub parameter_name: Option<String>,
    pub env_name: Option<String>,
    pub binding: Option<Json<SecretBinding>>,
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

/// An agent credential the shell's HTTP layer injects (`kind = 'http'`).
#[derive(Debug, Clone)]
pub struct UpsertAgentHttpCredentialRow {
    pub org_id: i64,
    pub agent_id: AgentId,
    pub env_name: String,
    pub binding: SecretBinding,
    pub label: String,
    pub description: Option<String>,
}
