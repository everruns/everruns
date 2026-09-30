// Delivery log rows for the agent trigger event pipeline
// (`domains::agent_triggers::events`).

use chrono::{DateTime, Utc};
use everruns_provider::typed_id::TriggerId;
use sqlx::FromRow;
use uuid::Uuid;

/// One event delivery recorded by the trigger event pipeline.
#[derive(Debug, Clone, FromRow)]
pub struct AgentTriggerDeliveryRow {
    pub id: Uuid,
    pub org_id: i64,
    pub trigger_id: TriggerId,
    pub source: String,
    pub event_id: Option<String>,
    pub event_type: Option<String>,
    pub subject: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub session_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateAgentTriggerDeliveryRow {
    pub org_id: i64,
    pub trigger_id: TriggerId,
    pub source: String,
    pub event_id: Option<String>,
    pub event_type: Option<String>,
    pub subject: Option<String>,
    pub status: String,
    pub reason: Option<String>,
}
