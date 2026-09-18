use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
pub struct McpServiceToolCacheRow {
    pub org_id: i64,
    pub mcp_server_id: Uuid,
    pub agent_id: Uuid,
    pub cache_scope: String,
    pub credential_hash: String,
    pub cached_tools: sqlx::types::JsonValue,
    pub ttl_ms: i64,
    pub tools_cached_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertMcpServiceToolCache {
    pub org_id: i64,
    pub mcp_server_id: Uuid,
    pub agent_id: Uuid,
    pub cache_scope: String,
    pub credential_hash: String,
    pub cached_tools: serde_json::Value,
    pub ttl_ms: i64,
}
