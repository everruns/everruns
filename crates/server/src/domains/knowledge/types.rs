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
/// A file collection with optional indexing, provider selection and path exclusions.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionInput {
    /// Collection name; `everruns-docs` is reserved for the predefined documentation snapshot.
    pub name: String,
    /// Human-readable purpose of the collection, available during progressive disclosure.
    #[serde(default)]
    pub description: String,
    /// Whether files remain unindexed, use keyword search, or also use embedding retrieval.
    #[serde(default)]
    pub indexing: everruns_contracts::knowledge::IndexMode,
    /// Organization embedding model; required when hybrid indexing is enabled.
    pub embedding_model_id: Option<String>,
    /// Optional organization search account. Omit to use the local index backend.
    pub indexer_id: Option<Uuid>,
    /// Path-glob arrays named `ingest_exclude`, `index_exclude` and `read_exclude`.
    #[serde(default)]
    pub rules: Value,
}
/// A connected repository mounted beneath a non-overlapping collection path.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceInput {
    /// Source adapter: `github` reuses a connection; `git` reads a public HTTPS repository.
    pub kind: String,
    /// Collection-relative root under which the source hierarchy is preserved.
    #[serde(default)]
    pub root_path: String,
    /// Adapter-specific repository, revision and root-folder configuration, without credentials.
    #[serde(default)]
    pub config: Value,
    /// Optional GitHub connection owner; existing connection authorization still applies.
    pub connection_owner: Option<Uuid>,
}
/// An inline managed file; use the streaming upload endpoint for files over 8 MiB.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FileInput {
    /// Normalized absolute path within the collection, preserving its directory hierarchy.
    pub path: String,
    /// UTF-8 original content. Supply exactly one of `content` or `content_base64`.
    pub content: Option<String>,
    /// Base64 original bytes for binary files; mutually exclusive with `content`.
    pub content_base64: Option<String>,
    /// Optional media type used for representation extraction; inferred when omitted.
    pub mime_type: Option<String>,
    /// User annotations retained separately from parsed source/frontmatter metadata.
    #[serde(default)]
    pub metadata: Value,
    /// Optional per-file indexing policy; false retains the original without indexing it.
    #[serde(default)]
    pub indexable: Option<bool>,
    /// Optional agent-read policy. Managers can still download retained originals.
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
    /// List only the immediate children of `path`: its files plus one entry per
    /// subfolder (`kind: "folder"`), for browsing a collection folder by folder.
    pub tree: Option<bool>,
}
/// An organization Turbopuffer account referencing an existing service credential.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexerInput {
    /// Display name for the organization search account.
    pub name: String,
    /// HTTPS regional Turbopuffer API endpoint; embedded credentials are forbidden.
    pub endpoint: String,
    /// Active organization-service credential binding; never a plaintext API key.
    pub credential_binding_id: Uuid,
}

/// Update managed-file annotations and agent visibility without extracting bytes again.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FileMetadataInput {
    /// Existing collection-relative file path.
    pub path: String,
    /// Replace user annotations while preserving metadata parsed from the original file.
    pub metadata: Option<Value>,
    /// Optional indexing override. Omit to preserve the current policy.
    pub indexable: Option<bool>,
    /// Optional agent-read override. Omit to preserve the current policy.
    pub readable: Option<bool>,
}
