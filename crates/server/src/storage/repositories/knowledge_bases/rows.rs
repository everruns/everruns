// Rows the knowledge bases repository reads and writes.

use crate::kernel_imports::contracts::typed_id::ModelId;
use crate::storage::UpdateField;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct KnowledgeBaseRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub owner_principal_id: Option<String>,
    pub resolved_owner_user_id: Option<Uuid>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    /// Optional embedding model for hybrid retrieval. NULL = keyword search only.
    #[sqlx(default)]
    pub embedding_model_id: Option<ModelId>,
}

#[derive(Debug, Clone)]
pub struct CreateKnowledgeBaseRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub owner_principal_id: Option<String>,
    pub resolved_owner_user_id: Option<Uuid>,
    /// Optional embedding model for hybrid retrieval.
    pub embedding_model_id: Option<ModelId>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateKnowledgeBase {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub status: Option<String>,
    /// Optional update to the embedding model. `None` = unchanged.
    pub embedding_model_id: Option<UpdateField<ModelId>>,
}

#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct KnowledgeEntryRow {
    pub id: Uuid,
    pub kb_id: Uuid,
    pub public_id: String,
    pub title: String,
    pub body: String,
    pub kind: String,
    pub tags: Vec<String>,
    /// Optional OKF `resource` URI identifying the underlying asset.
    #[sqlx(default)]
    pub resource: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateKnowledgeEntryRow {
    pub public_id: String,
    pub title: String,
    pub body: String,
    pub kind: String,
    pub tags: Vec<String>,
    pub resource: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateKnowledgeEntry {
    pub title: Option<String>,
    pub body: Option<String>,
    pub kind: Option<String>,
    pub tags: Option<Vec<String>>,
    /// `None` = unchanged, `Some(None)` = clear, `Some(Some(v))` = set.
    pub resource: Option<Option<String>>,
}
