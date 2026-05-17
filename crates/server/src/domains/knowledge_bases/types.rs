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
    pub id: KnowledgeBaseId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip)]
    pub internal_id: Uuid,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Request body for the `create_knowledge_base` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateKnowledgeBaseRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Request body for the `update_knowledge_base` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateKnowledgeBaseRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
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
    pub id: KnowledgeEntryId,
    #[schema(value_type = String, example = "kb_01933b5a000070008000000000000001")]
    pub kb_id: KnowledgeBaseId,
    pub title: String,
    pub body: String,
    pub kind: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
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
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

/// Request body for the `update_knowledge_entry` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateKnowledgeEntryRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ListKnowledgeEntriesQuery {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}
