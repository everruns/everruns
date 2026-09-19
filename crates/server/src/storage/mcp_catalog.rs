use crate::kernel_imports::everruns_provider::typed_id::McpServerId;
use chrono::{DateTime, Utc};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow)]
pub struct McpServerAgentUsageRow {
    pub mcp_server_id: McpServerId,
    pub used_by_agents: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct McpServerAgentNamesRow {
    pub agent_names: Vec<String>,
    pub total_count: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct UserMcpConnectionRow {
    pub provider: String,
    pub provider_username: Option<String>,
    pub scopes: Option<String>,
    pub connected_at: DateTime<Utc>,
    pub server_id: McpServerId,
    pub server_name: String,
    pub server_url: String,
    pub server_status: String,
}
