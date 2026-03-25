// Memory store module — persistent cross-session memory for agents.
// See specs/memory.md for design rationale.
//
// Decision: MemoryContentPart reuses same shape as message ContentPart (text + image)
//   but is a separate type to exclude tool-call/tool-result variants.
// Decision: Multiple stores per org, selected via capability config.
// Decision: Capacity limits enforced on write (remember tool + REST API).

use crate::typed_id::{MemoryId, MemoryStoreId, OrgId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

// ============================================================================
// Memory Content Parts (multicontent for recall)
// ============================================================================

/// Content part for memory entries — same discriminated-union shape as message
/// `ContentPart` but restricted to text and image variants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MemoryContentPart {
    /// Text content
    Text(MemoryTextPart),
    /// Image content (inline base64)
    Image(MemoryImagePart),
}

/// Text content within a memory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MemoryTextPart {
    pub text: String,
}

/// Image content within a memory (base64-encoded).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MemoryImagePart {
    /// Base64-encoded image data
    pub base64: String,
    /// MIME type (e.g. "image/png")
    pub media_type: String,
}

impl MemoryContentPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(MemoryTextPart { text: text.into() })
    }

    pub fn image(base64: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self::Image(MemoryImagePart {
            base64: base64.into(),
            media_type: media_type.into(),
        })
    }

    /// Estimated size in bytes (for capacity limit checks).
    pub fn estimated_size(&self) -> usize {
        match self {
            Self::Text(t) => t.text.len(),
            Self::Image(i) => i.base64.len(),
        }
    }
}

// ============================================================================
// Memory Kind
// ============================================================================

/// Classification of a memory entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    #[default]
    Fact,
    Preference,
    Correction,
    Procedure,
    Context,
}

impl std::fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fact => write!(f, "fact"),
            Self::Preference => write!(f, "preference"),
            Self::Correction => write!(f, "correction"),
            Self::Procedure => write!(f, "procedure"),
            Self::Context => write!(f, "context"),
        }
    }
}

// ============================================================================
// Memory Entity
// ============================================================================

/// A single memory entry within a store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Memory {
    pub id: MemoryId,
    pub store_id: MemoryStoreId,
    pub content: String,
    pub content_parts: Vec<MemoryContentPart>,
    pub kind: MemoryKind,
    /// 1-10 importance score (higher = more important)
    pub importance: u8,
    pub tags: Vec<String>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Memory Store Entity
// ============================================================================

/// An org-scoped container for memories.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MemoryStoreEntity {
    pub id: MemoryStoreId,
    pub org_id: OrgId,
    pub name: String,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Capacity Limits
// ============================================================================

/// Capacity limits for memory operations.
pub struct MemoryLimits;

impl MemoryLimits {
    /// Max characters in memory content field.
    pub const MAX_CONTENT_LENGTH: usize = 2_000;
    /// Max tags per memory.
    pub const MAX_TAGS: usize = 10;
    /// Max image content parts per memory.
    pub const MAX_IMAGE_PARTS: usize = 4;
    /// Max single image size (5 MB base64).
    pub const MAX_IMAGE_SIZE: usize = 5 * 1024 * 1024;
    /// Max total image data per memory (10 MB base64).
    pub const MAX_TOTAL_IMAGE_SIZE: usize = 10 * 1024 * 1024;
    /// Max active memories per store.
    pub const MAX_MEMORIES_PER_STORE: usize = 10_000;
    /// Max stores per org.
    pub const MAX_STORES_PER_ORG: usize = 50;

    /// Validate a memory before creation/update.
    pub fn validate(
        content: &str,
        tags: &[String],
        parts: &[MemoryContentPart],
    ) -> Result<(), String> {
        if content.len() > Self::MAX_CONTENT_LENGTH {
            return Err(format!(
                "Memory content exceeds {} character limit (got {})",
                Self::MAX_CONTENT_LENGTH,
                content.len()
            ));
        }
        if tags.len() > Self::MAX_TAGS {
            return Err(format!(
                "Too many tags: max {}, got {}",
                Self::MAX_TAGS,
                tags.len()
            ));
        }

        let mut image_count = 0;
        let mut total_image_size = 0;
        for part in parts {
            if let MemoryContentPart::Image(img) = part {
                image_count += 1;
                let size = img.base64.len();
                if size > Self::MAX_IMAGE_SIZE {
                    return Err(format!(
                        "Image exceeds {} byte limit (got {})",
                        Self::MAX_IMAGE_SIZE,
                        size
                    ));
                }
                total_image_size += size;
            }
        }
        if image_count > Self::MAX_IMAGE_PARTS {
            return Err(format!(
                "Too many images: max {}, got {}",
                Self::MAX_IMAGE_PARTS,
                image_count
            ));
        }
        if total_image_size > Self::MAX_TOTAL_IMAGE_SIZE {
            return Err(format!(
                "Total image data exceeds {} byte limit (got {})",
                Self::MAX_TOTAL_IMAGE_SIZE,
                total_image_size
            ));
        }

        Ok(())
    }
}

// ============================================================================
// Store Trait
// ============================================================================

/// Query parameters for recalling memories.
#[derive(Debug, Default)]
pub struct MemoryQuery {
    pub store_id: Option<MemoryStoreId>,
    pub query: Option<String>,
    pub tags: Option<Vec<String>>,
    pub kind: Option<MemoryKind>,
    pub limit: usize,
}

/// Trait for persistent memory storage backends.
#[async_trait]
pub trait MemoryStoreBackend: Send + Sync {
    /// Get or create the default store for an org.
    async fn get_or_create_default_store(
        &self,
        org_id: OrgId,
    ) -> crate::error::Result<MemoryStoreEntity>;

    /// Get a store by ID.
    async fn get_store(
        &self,
        store_id: MemoryStoreId,
    ) -> crate::error::Result<Option<MemoryStoreEntity>>;

    /// Create a memory in a store.
    async fn create_memory(
        &self,
        store_id: MemoryStoreId,
        content: String,
        content_parts: Vec<MemoryContentPart>,
        kind: MemoryKind,
        importance: u8,
        tags: Vec<String>,
    ) -> crate::error::Result<Memory>;

    /// Search/recall memories.
    async fn recall(&self, query: MemoryQuery) -> crate::error::Result<(Vec<Memory>, usize)>;

    /// Soft-delete (deactivate) a memory.
    ///
    /// `store_id` is required so implementations can enforce org-scoped
    /// ownership: the memory must belong to the given store.
    async fn forget(
        &self,
        store_id: MemoryStoreId,
        memory_id: MemoryId,
    ) -> crate::error::Result<bool>;

    /// Count active memories in a store.
    async fn count_active(&self, store_id: MemoryStoreId) -> crate::error::Result<usize>;
}
