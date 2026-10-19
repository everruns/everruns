// Rows the agent keys repository reads and writes.

use chrono::{DateTime, Utc};
use everruns_server_macros::Columns;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Columns)]
pub struct AgentKeyRow {
    pub id: Uuid,
    pub org_id: i64,
    pub channel_id: Uuid,
    pub name: String,
    pub token_hash: String,
    pub token_prefix: String,
    pub previous_token_hash: Option<String>,
    pub previous_valid_until: Option<DateTime<Utc>>,
    pub permissions: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_by_user_id: Option<Uuid>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateAgentKeyRow {
    pub org_id: i64,
    pub channel_id: Uuid,
    pub name: String,
    pub token_hash: String,
    pub token_prefix: String,
    pub permissions: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_by_user_id: Option<Uuid>,
}
