// File rows (model-input file attachments, e.g. PDFs).

use crate::kernel_imports::contracts::typed_id::FileId;
use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// File row from database
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct FileRow {
    pub id: FileId,
    pub org_id: i64,
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: i64,
    pub data: Vec<u8>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// File info without binary data (for listing)
#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct FileInfoRow {
    pub id: FileId,
    pub org_id: i64,
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: i64,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a file
#[derive(Debug, Clone)]
pub struct CreateFileRow {
    pub org_id: i64,
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: i64,
    pub data: Vec<u8>,
    pub metadata: serde_json::Value,
}
