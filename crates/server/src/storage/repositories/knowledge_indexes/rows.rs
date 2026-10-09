// Rows the knowledge indexes repository reads and writes.

use crate::kernel_imports::contracts::typed_id::ModelId;
use crate::storage::UpdateField;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct KnowledgeIndexRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub source_type: String,
    pub source_config: serde_json::Value,
    pub embedding_model_id: ModelId,
    pub vector_dim: Option<i32>,
    pub vector_namespace: Option<String>,
    pub owner_principal_id: Option<String>,
    pub resolved_owner_user_id: Option<Uuid>,
    pub status: String,
    pub sync_status: String,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_sync_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateKnowledgeIndexRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub source_type: String,
    pub source_config: serde_json::Value,
    pub embedding_model_id: ModelId,
    /// Vector-store namespace assigned at creation. See `index_namespace`.
    pub vector_namespace: String,
    pub owner_principal_id: Option<String>,
    pub resolved_owner_user_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateKnowledgeIndex {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub source_config: Option<serde_json::Value>,
    pub resolved_owner_user_id: UpdateField<Uuid>,
    /// Optional update to the embedding model. `None` = unchanged. The model is
    /// required on the index, so `Clear` is not representable.
    pub embedding_model_id: Option<ModelId>,
    /// Source or embedding changes invalidate the current projection and must
    /// atomically put the index back on the worker queue.
    pub enqueue_sync: bool,
    pub status: Option<String>,
}

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct KnowledgeIndexDocumentRow {
    pub id: Uuid,
    pub index_id: Uuid,
    pub public_id: String,
    pub source_uri: String,
    pub title: Option<String>,
    pub mime_type: Option<String>,
    pub content_hash: Option<String>,
    pub size_bytes: Option<i64>,
    pub chunk_count: i32,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct KnowledgeIndexChunkRow {
    pub id: Uuid,
    pub document_id: Uuid,
    pub index_id: Uuid,
    pub public_id: String,
    pub ordinal: i32,
    pub text: String,
    pub location: Option<serde_json::Value>,
    pub token_count: Option<i32>,
    pub created_at: DateTime<Utc>,
}

/// A chunk joined with its owning document's citation metadata, used to
/// hydrate `search_index` results from Postgres.
#[derive(Debug, Clone)]
pub struct KnowledgeIndexChunkWithDocument {
    /// Chunk `public_id` (`kchk_…`). The stable citation id.
    pub chunk_public_id: String,
    /// Chunk passage text.
    pub text: String,
    /// Provenance within the document (line / char / page ranges).
    pub location: Option<serde_json::Value>,
    /// Position of the chunk within its document.
    pub ordinal: i32,
    /// Owning document stable locator.
    pub source_uri: String,
    /// Owning document title, if known.
    pub document_title: Option<String>,
}

/// A chunk to persist during a sync pass. The `public_id` (`kchk_…`) is the
/// stable citation id and the vector-store record key.
#[derive(Debug, Clone)]
pub struct CreateKnowledgeIndexChunkRow {
    pub public_id: String,
    pub ordinal: i32,
    pub text: String,
    pub location: Option<serde_json::Value>,
    pub token_count: Option<i32>,
}

/// A document plus its chunks to persist atomically during a sync pass.
#[derive(Debug, Clone)]
pub struct CreateKnowledgeIndexDocumentWithChunks {
    pub public_id: String,
    pub source_uri: String,
    pub title: Option<String>,
    pub mime_type: Option<String>,
    pub content_hash: Option<String>,
    pub size_bytes: Option<i64>,
    pub chunks: Vec<CreateKnowledgeIndexChunkRow>,
}
