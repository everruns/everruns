use crate::api::common::deserialize_nullable_update_field;
use chrono::{DateTime, Utc};
use everruns_core::typed_id::{KnowledgeBaseId, KnowledgeEntryId};
use everruns_durable::UpdateField;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

pub use crate::storage::models::{
    CreateKnowledgeBaseRow, CreateKnowledgeEntryRow, KnowledgeBaseRow, KnowledgeEntryRow,
    UpdateKnowledgeBase, UpdateKnowledgeEntry,
};

/// Response body for knowledge base.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeBaseResponse {
    #[schema(value_type = String, example = "kb_01933b5a000070008000000000000001")]
    /// Prefixed public identifier (see `specs/id-schema.md`).
    pub id: KnowledgeBaseId,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    #[serde(skip)]
    /// Internal database UUID. Not part of the public identifier surface.
    pub internal_id: Uuid,
    /// Current lifecycle status.
    pub status: String,
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

/// Request body for the `create_knowledge_base` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateKnowledgeBaseRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    #[serde(default)]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
}

/// Request body for the `update_knowledge_base` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateKnowledgeBaseRequest {
    #[serde(default)]
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: UpdateField<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ListKnowledgeBasesQuery {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub include_archived: Option<bool>,
}

pub fn knowledge_base_response(row: KnowledgeBaseRow) -> anyhow::Result<KnowledgeBaseResponse> {
    Ok(KnowledgeBaseResponse {
        id: row.public_id.parse()?,
        name: row.name,
        description: row.description,
        internal_id: row.id,
        status: row.status,
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
    })
}

/// Response body for knowledge entry.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeEntryResponse {
    #[schema(value_type = String, example = "kbe_01933b5a000070008000000000000001")]
    /// Prefixed public identifier (see `specs/id-schema.md`).
    pub id: KnowledgeEntryId,
    #[schema(value_type = String, example = "kb_01933b5a000070008000000000000001")]
    /// Knowledge base's prefixed public identifier.
    pub kb_id: KnowledgeBaseId,
    /// Human-readable title. Safe to render in user-facing messages.
    pub title: String,
    pub body: String,
    /// Discriminator selecting the variant of this resource.
    pub kind: String,
    /// Free-form tags attached to this resource.
    pub tags: Vec<String>,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this resource was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

pub fn knowledge_entry_response(
    row: KnowledgeEntryRow,
    kb_public_id: &str,
) -> anyhow::Result<KnowledgeEntryResponse> {
    Ok(KnowledgeEntryResponse {
        id: row.public_id.parse()?,
        kb_id: kb_public_id.parse()?,
        title: row.title,
        body: row.body,
        kind: row.kind,
        tags: row.tags,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Request body for the `create_knowledge_entry` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateKnowledgeEntryRequest {
    /// Human-readable title. Safe to render in user-facing messages.
    pub title: String,
    pub body: String,
    #[serde(default)]
    /// Discriminator selecting the variant of this resource.
    pub kind: Option<String>,
    #[serde(default)]
    /// Free-form tags attached to this resource.
    pub tags: Option<Vec<String>>,
}

/// Request body for the `update_knowledge_entry` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateKnowledgeEntryRequest {
    #[serde(default)]
    /// Human-readable title. Safe to render in user-facing messages.
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    /// Discriminator selecting the variant of this resource.
    pub kind: Option<String>,
    #[serde(default)]
    /// Free-form tags attached to this resource.
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ListKnowledgeEntriesQuery {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    /// Discriminator selecting the variant of this resource.
    pub kind: Option<String>,
}
