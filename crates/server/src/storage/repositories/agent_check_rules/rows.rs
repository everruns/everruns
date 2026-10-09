// Rows the agent check rules repository reads and writes.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Org-configurable agent check rule.
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct AgentCheckRuleRow {
    pub id: Uuid,
    pub org_id: i64,
    pub rule_id: String,
    pub kind: String,
    pub enabled: bool,
    pub severity_override: Option<String>,
    pub config: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Upsert input for an agent check rule (keyed by org_id + rule_id).
#[derive(Debug, Clone)]
pub struct UpsertAgentCheckRuleRow {
    pub rule_id: String,
    pub kind: String,
    pub enabled: bool,
    pub severity_override: Option<String>,
    pub config: Option<serde_json::Value>,
}
