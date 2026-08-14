use crate::api::common::deserialize_nullable_update_field;
use chrono::{DateTime, Utc};
use everruns_durable::UpdateField;
use everruns_provider::typed_id::{KnowledgeIndexDocumentId, KnowledgeIndexId, ModelId};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

pub use crate::storage::models::{
    CreateKnowledgeIndexRow, KnowledgeIndexDocumentRow, KnowledgeIndexRow, UpdateKnowledgeIndex,
};

/// Response body for a knowledge index.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeIndexResponse {
    #[schema(value_type = String, example = "kidx_01933b5a000070008000000000000001")]
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: KnowledgeIndexId,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    #[serde(skip)]
    /// Internal database UUID. Not part of the public identifier surface.
    pub internal_id: Uuid,
    /// External source type. One of `github`, `git`.
    pub source_type: String,
    /// Non-secret source coordinates. Never holds credentials.
    pub source_config: serde_json::Value,
    #[schema(value_type = String)]
    /// Embedding model used to embed chunks. Required.
    pub embedding_model_id: ModelId,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Embedding dimension, recorded on first successful sync.
    pub vector_dim: Option<i32>,
    /// Number of documents currently retained by this index.
    pub document_count: usize,
    /// Current lifecycle status.
    pub status: String,
    /// Syncout pipeline state. One of `idle`, `pending`, `syncing`, `synced`, `failed`.
    pub sync_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Timestamp of the last successful sync, if any (RFC 3339).
    pub last_synced_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Sanitized failure reason from the last failed sync, if any.
    pub last_sync_error: Option<String>,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this resource was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Timestamp when this resource was archived, if any (RFC 3339).
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Timestamp when this resource was soft-deleted, if any (RFC 3339).
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Request body for the `create_knowledge_index` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateKnowledgeIndexRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "product-docs")]
    pub name: String,
    #[serde(default)]
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Synced product documentation")]
    pub description: Option<String>,
    #[serde(default)]
    /// External source type. One of `github`, `git`. Defaults to `github`.
    #[schema(example = "github")]
    pub source_type: Option<String>,
    #[serde(default)]
    /// Non-secret source coordinates. GitHub repositories accept `owner/repo`
    /// or canonical `https://github.com/owner/repo[.git]` URLs. Never include credentials.
    #[schema(example = json!({"provider": "github", "repository": "owner/repo", "branch": "main"}))]
    pub source_config: Option<serde_json::Value>,
    #[schema(value_type = String)]
    /// Embedding model used to embed chunks. Required.
    pub embedding_model_id: ModelId,
}

/// Request body for the `update_knowledge_index` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateKnowledgeIndexRequest {
    #[serde(default)]
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "product-docs-v2")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    /// Human-readable description. Set to null to clear.
    pub description: UpdateField<String>,
    #[serde(default)]
    /// Non-secret source coordinates. Never include credentials.
    pub source_config: Option<serde_json::Value>,
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    /// Embedding model used to embed chunks. Required; cannot be cleared.
    pub embedding_model_id: Option<ModelId>,
}

/// Query parameters for `GET /v1/knowledge-indexes` — optional name/desc
/// search plus a flag to include archived indexes.
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ListKnowledgeIndexesQuery {
    /// Substring filter applied to index name and description.
    #[serde(default)]
    #[schema(example = "docs")]
    pub search: Option<String>,
    /// When `true`, also returns archived knowledge indexes.
    #[serde(default)]
    #[schema(example = false)]
    pub include_archived: Option<bool>,
}

pub fn knowledge_index_response(
    row: KnowledgeIndexRow,
    document_count: usize,
) -> anyhow::Result<KnowledgeIndexResponse> {
    Ok(KnowledgeIndexResponse {
        id: row.public_id.parse()?,
        name: row.name,
        description: row.description,
        internal_id: row.id,
        source_type: row.source_type,
        source_config: row.source_config,
        embedding_model_id: row.embedding_model_id,
        vector_dim: row.vector_dim,
        document_count,
        status: row.status,
        sync_status: row.sync_status,
        last_synced_at: row.last_synced_at,
        last_sync_error: row.last_sync_error,
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
    })
}

/// Response body for a knowledge index document.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeIndexDocumentResponse {
    #[schema(value_type = String, example = "kidoc_01933b5a000070008000000000000001")]
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: KnowledgeIndexDocumentId,
    #[schema(value_type = String, example = "kidx_01933b5a000070008000000000000001")]
    /// Parent index's prefixed public identifier.
    pub index_id: KnowledgeIndexId,
    /// Stable per-source locator (e.g. `github://owner/repo@main/docs/x.md`).
    pub source_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Document title, if known.
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Document MIME type, if known.
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Content hash driving incremental re-embedding.
    pub content_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Document size in bytes, if known.
    pub size_bytes: Option<i64>,
    /// Denormalized chunk count for this document.
    pub chunk_count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Timestamp this document was last seen during a sync pass (RFC 3339).
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this resource was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

pub fn knowledge_index_document_response(
    row: KnowledgeIndexDocumentRow,
    index_public_id: &str,
) -> anyhow::Result<KnowledgeIndexDocumentResponse> {
    Ok(KnowledgeIndexDocumentResponse {
        id: row.public_id.parse()?,
        index_id: index_public_id.parse()?,
        source_uri: row.source_uri,
        title: row.title,
        mime_type: row.mime_type,
        content_hash: row.content_hash,
        size_bytes: row.size_bytes,
        chunk_count: row.chunk_count,
        last_seen_at: row.last_seen_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}
