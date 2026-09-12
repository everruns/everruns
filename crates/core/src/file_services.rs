//! File attachment services (PDFs and other model-input files).
//!
//! Mirrors [`crate::image_services`] for non-image file attachments that are
//! sent to the model (e.g. PDFs via each provider's native file/document
//! input). Files are uploaded via the `/files` API, stored by ID, and
//! resolved to base64 data URLs at prompt-build time.

use async_trait::async_trait;
use everruns_provider::error::Result;
use std::collections::HashMap;
use uuid::Uuid;

/// Stored file info from the files table.
#[derive(Debug, Clone)]
pub struct StoredFileInfo {
    /// Original filename supplied at upload, if any.
    pub filename: Option<String>,
    /// MIME type of the stored bytes.
    pub content_type: String,
    /// Size of the stored bytes.
    pub size_bytes: i64,
}

/// Stored file with data.
#[derive(Debug, Clone)]
pub struct StoredFile {
    /// Metadata describing the stored file.
    pub info: StoredFileInfo,
    /// Raw stored bytes.
    pub data: Vec<u8>,
}

/// Input for creating a stored file.
#[derive(Debug, Clone)]
pub struct CreateStoredFile {
    /// Original filename, if known.
    pub filename: Option<String>,
    /// MIME type of `data`.
    pub content_type: String,
    /// Raw bytes to store.
    pub data: Vec<u8>,
}

/// Storage backend for file attachments.
#[async_trait]
pub trait FileArtifactStore: Send + Sync {
    /// Persist a new file and return its id.
    async fn save_file(&self, input: CreateStoredFile) -> Result<Uuid>;
    /// Load a stored file by id, or `None` when absent.
    async fn get_file(&self, id: Uuid) -> Result<Option<StoredFile>>;
    /// Delete a stored file; reports whether one existed.
    async fn delete_file(&self, id: Uuid) -> Result<bool>;
}

/// A file resolved to base64 for inclusion in an LLM request.
#[derive(Debug, Clone)]
pub struct ResolvedFile {
    /// Base64-encoded file bytes.
    pub base64: String,
    /// MIME type, e.g. `application/pdf`.
    pub media_type: String,
    /// Original filename, when known.
    pub filename: Option<String>,
}

impl ResolvedFile {
    /// Render the resolved bytes as a `data:` URL.
    pub fn to_data_url(&self) -> String {
        format!("data:{};base64,{}", self.media_type, self.base64)
    }
}

/// Abstraction for resolving file IDs to [`ResolvedFile`]s.
#[async_trait]
pub trait FileResolver: Send + Sync {
    /// Resolve stored-file ids to inlineable payloads.
    async fn resolve_files(&self, ids: &[Uuid]) -> Result<HashMap<Uuid, ResolvedFile>>;
}
