// Principal rows.

use crate::kernel_imports::contracts::typed_id::PrincipalId;
use crate::storage::UpdateField;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct PrincipalRow {
    pub id: PrincipalId,
    pub public_id: String,
    pub org_id: i64,
    pub kind: String,
    pub subject_id: Option<Uuid>,
    pub parent_principal_id: Option<PrincipalId>,
    pub resolved_user_id: Option<Uuid>,
    pub metadata: serde_json::Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreatePrincipalRow {
    pub id: PrincipalId,
    pub org_id: i64,
    pub kind: String,
    pub subject_id: Option<Uuid>,
    pub parent_principal_id: Option<PrincipalId>,
    pub resolved_user_id: Option<Uuid>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdatePrincipalRow {
    pub parent_principal_id: UpdateField<PrincipalId>,
    pub resolved_user_id: UpdateField<Uuid>,
    pub metadata: Option<serde_json::Value>,
    pub status: Option<String>,
}
