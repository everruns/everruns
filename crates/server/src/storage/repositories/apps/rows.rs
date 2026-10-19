// App rows (deployable agent+harness bundles).

use crate::kernel_imports::contracts::typed_id::PrincipalId;
use crate::storage::UpdateField;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// App row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct AppRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub harness_id: Uuid,
    pub agent_id: Option<Uuid>,
    pub virtual_user_id: Option<Uuid>,
    pub owner_principal_id: PrincipalId,
    #[sqlx(default)]
    pub resolved_owner_user_id: Option<Uuid>,
    pub channel_type: Option<String>,
    pub channel_config: serde_json::Value,
    pub channel_config_encrypted: Option<Vec<u8>>,
    pub status: String,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Input for creating an app
#[derive(Debug, Clone)]
pub struct CreateAppRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub harness_id: Uuid,
    pub agent_id: Option<Uuid>,
    pub virtual_user_id: Option<Uuid>,
    pub owner_principal_id: PrincipalId,
    pub resolved_owner_user_id: Option<Uuid>,
    pub channel_type: Option<String>,
    pub channel_config: serde_json::Value,
    /// Encrypted channel_config bytes (envelope-encrypted JSON).
    pub channel_config_encrypted: Option<Vec<u8>>,
}

/// Input for updating an app
#[derive(Debug, Clone, Default)]
pub struct UpdateApp {
    pub name: Option<String>,
    pub description: Option<String>,
    pub harness_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    pub virtual_user_id: UpdateField<Uuid>,
    pub owner_principal_id: Option<PrincipalId>,
    pub resolved_owner_user_id: UpdateField<Uuid>,
    pub channel_type: Option<String>,
    pub channel_config: Option<serde_json::Value>,
    /// Encrypted channel_config bytes (envelope-encrypted JSON).
    pub channel_config_encrypted: Option<Vec<u8>>,
    pub status: Option<String>,
    pub published_at: UpdateField<DateTime<Utc>>,
}
