// Agent trigger models (agent-owned invocation triggers).

use super::*;
use everruns_server_macros::Columns;

#[derive(Debug, Clone, FromRow, Columns)]
pub struct AgentTriggerRow {
    pub id: TriggerId,
    pub org_id: i64,
    pub agent_id: AgentId,
    pub trigger_type: String,
    pub ingress_id: Option<String>,
    pub config: serde_json::Value,
    pub config_encrypted: Option<Vec<u8>>,
    pub enabled: bool,
    pub durable_schedule_id: Option<Uuid>,
    pub execution_harness_id: Option<HarnessId>,
    pub execution_owner_principal_id: Option<PrincipalId>,
    pub execution_resolved_owner_user_id: Option<Uuid>,
    pub execution_virtual_user_id: Option<VirtualUserId>,
    pub execution_app_id: Option<Uuid>,
    pub legacy_alias_id: Option<String>,
    pub legacy_alias_name: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateAgentTriggerRow {
    pub org_id: i64,
    pub id: TriggerId,
    pub agent_id: AgentId,
    pub trigger_type: String,
    pub ingress_id: Option<String>,
    pub config: serde_json::Value,
    pub config_encrypted: Option<Vec<u8>>,
    pub enabled: bool,
    pub durable_schedule_id: Option<Uuid>,
    pub execution_harness_id: Option<HarnessId>,
    pub execution_owner_principal_id: Option<PrincipalId>,
    pub execution_resolved_owner_user_id: Option<Uuid>,
    pub execution_virtual_user_id: Option<VirtualUserId>,
    pub execution_app_id: Option<Uuid>,
    pub legacy_alias_id: Option<String>,
    pub legacy_alias_name: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateAgentTrigger {
    pub trigger_type: Option<String>,
    pub config: Option<serde_json::Value>,
    pub config_encrypted: Option<Vec<u8>>,
    pub enabled: Option<bool>,
    pub durable_schedule_id: UpdateField<Uuid>,
    pub status: Option<String>,
}
