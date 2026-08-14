use crate::api::common::deserialize_nullable_update_field;
use chrono::{DateTime, Utc};
use everruns_durable::UpdateField;
use everruns_provider::typed_id::{KnowledgeBaseId, KnowledgeEntryId, ModelId};
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
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    /// Optional embedding model for hybrid retrieval. `null` = keyword search only.
    pub embedding_model_id: Option<ModelId>,
}

/// Request body for the `create_knowledge_base` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateKnowledgeBaseRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "support-runbooks")]
    pub name: String,
    #[serde(default)]
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Runbooks for the support team")]
    pub description: Option<String>,
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    /// Optional embedding model for hybrid retrieval. Omit or null for keyword search only.
    pub embedding_model_id: Option<ModelId>,
}

/// Request body for the `update_knowledge_base` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateKnowledgeBaseRequest {
    #[serde(default)]
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "support-runbooks-archive")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: UpdateField<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    /// Optional embedding model for hybrid retrieval. Set to null to clear.
    pub embedding_model_id: UpdateField<ModelId>,
}

/// Query parameters for `GET /v1/knowledge-bases` — optional name/desc
/// search plus a flag to include archived knowledge bases.
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ListKnowledgeBasesQuery {
    /// Substring filter applied to knowledge-base name and description.
    #[serde(default)]
    #[schema(example = "runbook")]
    pub search: Option<String>,
    /// When `true`, also returns archived knowledge bases.
    #[serde(default)]
    #[schema(example = false)]
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
        embedding_model_id: row.embedding_model_id,
    })
}

/// Response body for knowledge entry.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeEntryResponse {
    #[schema(value_type = String, example = "kbe_01933b5a000070008000000000000001")]
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional OKF resource URI identifying the underlying asset.
    #[schema(example = "https://console.cloud.google.com/bigquery?p=acme&d=sales&t=orders")]
    pub resource: Option<String>,
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
        resource: row.resource,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Request body for the `create_knowledge_entry` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateKnowledgeEntryRequest {
    /// Human-readable title. Safe to render in user-facing messages.
    #[schema(example = "Refund a payment past 30 days")]
    pub title: String,
    /// Entry body. Markdown is rendered when displayed.
    #[schema(
        example = "Use the `/v1/payments/{id}/refund` endpoint with `reason: \"past_window\"`. Only the on-call billing engineer can authorize this."
    )]
    pub body: String,
    #[serde(default)]
    /// Discriminator selecting the variant of this resource. One of `note`,
    /// `table`, `business`, `query`, `runbook`.
    #[schema(example = "runbook")]
    pub kind: Option<String>,
    #[serde(default)]
    /// Free-form tags attached to this resource.
    #[schema(example = json!(["billing", "refunds"]))]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    /// Optional OKF resource URI identifying the underlying asset.
    #[schema(example = "https://console.cloud.google.com/bigquery?p=acme&d=sales&t=orders")]
    pub resource: Option<String>,
}

/// Request body for the `update_knowledge_entry` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateKnowledgeEntryRequest {
    #[serde(default)]
    /// Human-readable title. Safe to render in user-facing messages.
    #[schema(example = "Refund a payment past 60 days")]
    pub title: Option<String>,
    /// Updated entry body. Markdown is rendered when displayed.
    #[serde(default)]
    #[schema(
        example = "Use the `/v1/payments/{id}/refund` endpoint with `reason: \"past_window\"`. Now requires VP approval."
    )]
    pub body: Option<String>,
    #[serde(default)]
    /// Discriminator selecting the variant of this resource. One of `note`,
    /// `table`, `business`, `query`, `runbook`.
    #[schema(example = "runbook")]
    pub kind: Option<String>,
    #[serde(default)]
    /// Free-form tags attached to this resource.
    #[schema(example = json!(["billing", "refunds", "vp-approval"]))]
    pub tags: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    /// Optional OKF resource URI. Set to null to clear.
    pub resource: UpdateField<String>,
}

/// Query parameters for listing entries inside a knowledge base — optional
/// text search and tag filter.
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ListKnowledgeEntriesQuery {
    /// Substring filter applied to entry title and body.
    #[serde(default)]
    #[schema(example = "refund")]
    pub search: Option<String>,
    #[serde(default)]
    /// Discriminator selecting the variant of this resource. One of `note`,
    /// `table`, `business`, `query`, `runbook`.
    #[schema(example = "runbook")]
    pub kind: Option<String>,
}
