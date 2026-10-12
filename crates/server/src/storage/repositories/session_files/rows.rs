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
