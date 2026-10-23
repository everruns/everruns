use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Collection {
    pub id: Uuid,
    pub org_id: i64,
    pub name: String,
    pub description: String,
    pub indexing: String,
    pub embedding_model_id: Option<Uuid>,
    pub indexer_id: Option<Uuid>,
    pub published_revision: i64,
    pub serving_config: Value,
    pub revision: i64,
    pub status: String,
    pub rules: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
impl Collection {
    pub fn public_id(&self) -> String {
        super::public(self.id, "kn_")
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Source {
    pub id: Uuid,
    pub org_id: i64,
    pub collection_id: Uuid,
    pub kind: String,
    pub root_path: String,
    pub config: Value,
    pub connection_owner: Option<Uuid>,
    pub checkpoint: Option<String>,
    pub sync_status: String,
    pub last_error: Option<String>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct FileVersion {
    pub id: Uuid,
    pub org_id: i64,
    pub file_id: Uuid,
    pub content_hash: String,
    pub size_bytes: i64,
    pub mime_type: String,
    pub raw_key: String,
    pub raw_parts: i32,
    pub text_key: Option<String>,
    pub text_bytes: i32,
    pub metadata: Value,
    pub warnings: Value,
    pub extraction_status: String,
    pub source_uri: String,
    pub source_revision: Option<String>,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct File {
    pub id: Uuid,
    pub org_id: i64,
    pub collection_id: Uuid,
    pub source_id: Uuid,
    pub path: String,
    pub current_version: Option<Uuid>,
    pub indexed_version: Option<Uuid>,
    pub indexed_revision: Option<i64>,
    pub indexable: bool,
    pub readable: bool,
    pub seen_run: Option<Uuid>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Job {
    pub id: Uuid,
    pub org_id: i64,
    pub collection_id: Uuid,
    pub source_id: Option<Uuid>,
    pub file_id: Option<Uuid>,
    pub version_id: Option<Uuid>,
    pub kind: String,
    pub revision: i64,
    pub status: String,
    pub lease_token: Option<Uuid>,
    pub lease_until: Option<DateTime<Utc>>,
    pub attempts: i32,
    pub cursor: String,
    pub last_error: Option<String>,
    pub available_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub indexing: everruns_contracts::knowledge::IndexMode,
    pub embedding_model_id: Option<String>,
    pub indexer_id: Option<Uuid>,
    #[serde(default)]
    pub rules: Value,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceInput {
    pub kind: String,
    #[serde(default)]
    pub root_path: String,
    #[serde(default)]
    pub config: Value,
    pub connection_owner: Option<Uuid>,
}
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FileInput {
    pub path: String,
    pub content: Option<String>,
    pub content_base64: Option<String>,
    pub mime_type: Option<String>,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default)]
    pub indexable: Option<bool>,
    #[serde(default)]
    pub readable: Option<bool>,
}
#[derive(Debug, Default, Deserialize, Serialize, ToSchema, utoipa::IntoParams)]
pub struct PageQuery {
    pub after: Option<String>,
    pub path: Option<String>,
    pub limit: Option<u32>,
    pub extracted: Option<bool>,
    pub version: Option<Uuid>,
    pub offset: Option<u64>,
    pub length: Option<u32>,
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexerInput {
    pub name: String,
    pub endpoint: String,
    pub credential_binding_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FileMetadataInput {
    pub path: String,
    pub metadata: Option<Value>,
    pub indexable: Option<bool>,
    pub readable: Option<bool>,
}
