use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;
/// Endpoint-owned values required to serve ingress without archival App reads.
#[derive(Debug, Clone, FromRow)]
pub struct IngressEndpointRow {
    pub endpoint_id: Uuid,
    pub endpoint_public_id: String,
    pub legacy_app_id: Uuid,
    pub legacy_app_public_id: String,
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
    pub endpoint_status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
