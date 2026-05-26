use chrono::{DateTime, Utc};
use everruns_core::typed_id::{MemoryId, MemoryStoreId};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

pub use crate::storage::models::{MemoryDbRow, MemoryStoreDbRow};

/// Response body for memory store.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemoryStoreResponse {
    #[schema(value_type = String, example = "mst_01933b5a000070008000000000000001")]
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: MemoryStoreId,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    pub is_default: bool,
    pub active_memory_count: i64,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this resource was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

/// Request body for the `create_memory_store` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateMemoryStoreRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "customer-preferences")]
    pub name: String,
    /// When `true`, sets this store as the org's default for newly-created memories.
    #[serde(default)]
    #[schema(example = false)]
    pub is_default: bool,
}

/// Request body for the `update_memory_store` operation.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct UpdateMemoryStoreRequest {
    #[serde(default)]
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "customer-preferences-archive")]
    pub name: Option<String>,
    /// Toggle this store as the org default.
    #[serde(default)]
    #[schema(example = true)]
    pub is_default: Option<bool>,
}

/// Response body for memory.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemoryResponse {
    #[schema(value_type = String, example = "mem_01933b5a000070008000000000000001")]
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: MemoryId,
    #[schema(value_type = String, example = "mst_01933b5a000070008000000000000001")]
    pub store_id: MemoryStoreId,
    pub content: String,
    pub content_parts: serde_json::Value,
    /// Discriminator selecting the variant of this resource.
    pub kind: String,
    pub importance: i16,
    /// Free-form tags attached to this resource.
    pub tags: Vec<String>,
    pub active: bool,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this resource was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

/// Response body for the `list_memories` operation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ListMemoriesResponse {
    /// Page of items returned by this query.
    pub data: Vec<MemoryResponse>,
    /// Total number of items matching the query, across all pages.
    pub total: i64,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams, ToSchema)]
pub struct ListMemoriesQuery {
    /// Substring filter applied to memory content.
    #[serde(default)]
    #[schema(example = "preferred timezone")]
    pub query: Option<String>,
    #[serde(default)]
    /// Discriminator selecting the variant of this resource. One of `fact`,
    /// `preference`, `correction`, `procedure`, `context`.
    #[schema(example = "preference")]
    pub kind: Option<String>,
    /// Filter to memories tagged with at least one of these labels.
    #[serde(default)]
    #[schema(example = json!(["onboarding", "billing"]))]
    pub tag: Option<Vec<String>>,
    /// When `true`, also returns inactive (deactivated) memories.
    #[serde(default)]
    #[schema(example = false)]
    pub include_inactive: Option<bool>,
    #[serde(default)]
    /// Maximum number of items returned in this page.
    #[schema(example = 50)]
    pub limit: Option<usize>,
}

pub fn store_response(
    row: MemoryStoreDbRow,
    active_memory_count: i64,
) -> anyhow::Result<MemoryStoreResponse> {
    Ok(MemoryStoreResponse {
        id: row.public_id.parse()?,
        name: row.name,
        is_default: row.is_default,
        active_memory_count,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub fn memory_response(row: MemoryDbRow) -> anyhow::Result<MemoryResponse> {
    Ok(MemoryResponse {
        id: row.public_id.parse()?,
        store_id: row.store_public_id.parse()?,
        content: row.content,
        content_parts: row.content_parts,
        kind: row.kind,
        importance: row.importance,
        tags: row.tags,
        active: row.active,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}
