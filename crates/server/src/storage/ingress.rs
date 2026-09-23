use chrono::{DateTime, Utc};
use everruns_durable::UpdateField;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct CreateAgentChannelRow {
    pub agent_id: Uuid,
    pub public_id: String,
    pub channel_type: String,
    pub channel_config: serde_json::Value,
    pub channel_config_encrypted: Option<Vec<u8>>,
    pub auth: Option<serde_json::Value>,
    pub auth_encrypted: Option<Vec<u8>>,
    pub enabled: bool,
    pub status: String,
    pub agent_identity_id: Option<Uuid>,
    pub agent_version_policy: String,
    pub agent_version_id: Option<Uuid>,
    pub owner_principal_id: Uuid,
    pub resolved_owner_user_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateAgentChannelRow {
    pub channel_type: Option<String>,
    pub channel_config: Option<serde_json::Value>,
    pub channel_config_encrypted: UpdateField<Vec<u8>>,
    pub auth: UpdateField<serde_json::Value>,
    pub auth_encrypted: UpdateField<Vec<u8>>,
    pub enabled: Option<bool>,
    pub status: Option<String>,
}
/// Channel-owned values required to serve ingress without archival App reads.
#[derive(Debug, Clone, FromRow)]
pub struct IngressChannelRow {
    pub channel_id: Uuid,
    pub channel_public_id: String,
    pub legacy_app_id: Option<Uuid>,
    pub legacy_app_public_id: Option<String>,
    pub org_id: i64,
    pub agent_id: Uuid,
    pub agent_public_id: String,
    pub agent_name: String,
    pub agent_description: Option<String>,
    pub harness_id: Uuid,
    pub agent_status: String,
    pub exposures_suspended: bool,
    pub agent_identity_id: Option<Uuid>,
    pub agent_version_policy: String,
    pub agent_version_id: Option<Uuid>,
    pub owner_principal_id: Uuid,
    pub resolved_owner_user_id: Option<Uuid>,
    pub channel_type: String,
    pub channel_config: serde_json::Value,
    pub channel_config_encrypted: Option<Vec<u8>>,
    pub auth: Option<serde_json::Value>,
    pub auth_encrypted: Option<Vec<u8>>,
    pub enabled: bool,
    pub channel_status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
