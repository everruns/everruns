// Session file rows (virtual filesystem).

use crate::kernel_imports::contracts::typed_id::SessionId;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Session file row from database
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct SessionFileRow {
    pub id: Uuid,
    pub session_id: SessionId,
    pub path: String,
    pub content: Option<Vec<u8>>,
    pub is_directory: bool,
    pub is_readonly: bool,
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a session file
#[derive(Debug, Clone)]
pub struct CreateSessionFileRow {
    pub session_id: SessionId,
    pub path: String,
    pub content: Option<Vec<u8>>,
    pub is_directory: bool,
    pub is_readonly: bool,
}

/// Input for updating a session file
#[derive(Debug, Clone, Default)]
pub struct UpdateSessionFile {
    pub content: Option<Vec<u8>>,
    pub is_readonly: Option<bool>,
}

/// Lightweight file info for listing (without content)
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct SessionFileInfoRow {
    pub id: Uuid,
    pub session_id: SessionId,
    pub path: String,
    pub is_directory: bool,
    pub is_readonly: bool,
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Largest file the control plane loads into memory as a whole.
///
/// Matches the historical per-file write limit, so every file written before
/// streaming existed still loads. Bigger files (recordings, uploads) are only
/// served through [`SessionFileContent`] streams; loading one whole would hold
/// it in memory several times over (bytes, base64, decoded copy).
pub const MAX_LOADED_FILE_BYTES: i64 = 100 * 1024 * 1024;

/// A file was too large to load into memory; stream it instead.
#[derive(Debug, thiserror::Error)]
#[error(
    "File {path} is {size_bytes} bytes, larger than the {limit} bytes that can be read in one piece; download it instead"
)]
pub struct FileTooLargeToLoad {
    pub path: String,
    pub size_bytes: i64,
    pub limit: i64,
}

/// File bytes opened for streaming, optionally a byte range of them.
pub struct SessionFileContent {
    pub info: SessionFileInfoRow,
    /// SHA-256 of the whole file when stored in object storage.
    pub content_sha256: Option<String>,
    /// The requested bytes (the whole file when no range was asked for).
    pub body: crate::storage::blob_store::BlobByteStream,
}
