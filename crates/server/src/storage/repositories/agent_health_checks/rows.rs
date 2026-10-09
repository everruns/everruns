// Rows the agent health checks repository reads and writes.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Agent health check run row from database.
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct AgentHealthCheckRunRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub agent_id: Option<Uuid>,
    pub config_hash: String,
    pub model_id: Option<String>,
    pub status: String,
    pub summary: Option<serde_json::Value>,
    pub results: Option<serde_json::Value>,
    pub error_message: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating an agent health check run.
#[derive(Debug, Clone)]
pub struct CreateAgentHealthCheckRunRow {
    pub public_id: String,
    pub agent_id: Option<Uuid>,
    pub config_hash: String,
    pub model_id: Option<String>,
}

/// Input for updating an agent health check run. `None` fields are left
/// unchanged; `started_at`/`completed_at` are set from `status` transitions.
#[derive(Debug, Clone, Default)]
pub struct UpdateAgentHealthCheckRunRow {
    pub status: Option<String>,
    pub summary: Option<serde_json::Value>,
    pub results: Option<serde_json::Value>,
    pub error_message: Option<String>,
}
