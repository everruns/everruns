// Rows the memory repository reads and writes.

use crate::kernel_imports::contracts::typed_id::AgentId;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct MemoryRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub scope: String,
    pub owner_agent_id: Option<AgentId>,
    pub owner_user_id: Option<Uuid>,
    pub source_type: String,
    pub source_config: serde_json::Value,
    pub is_readonly: bool,
    pub sync_status: String,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_sync_error: Option<String>,
    pub owner_principal_id: Option<String>,
    pub resolved_owner_user_id: Option<Uuid>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateMemoryRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub scope: String,
    pub owner_agent_id: Option<AgentId>,
    pub owner_user_id: Option<Uuid>,
    pub source_type: String,
    pub source_config: serde_json::Value,
    pub is_readonly: bool,
    pub sync_status: String,
    pub owner_principal_id: Option<String>,
    pub resolved_owner_user_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateMemory {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub status: Option<String>,
    pub source_type: Option<String>,
    pub source_config: Option<serde_json::Value>,
    pub is_readonly: Option<bool>,
    pub sync_status: Option<String>,
    pub last_synced_at: Option<Option<DateTime<Utc>>>,
    pub last_sync_error: Option<Option<String>>,
}

#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct MemoryFileRow {
    pub id: Uuid,
    pub memory_id: Uuid,
    pub path: String,
    pub content: Option<Vec<u8>>,
    pub is_directory: bool,
    pub size_bytes: i64,
    pub content_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateMemoryFileRow {
    pub path: String,
    pub content: Option<Vec<u8>>,
    pub is_directory: bool,
    pub content_hash: Option<String>,
}

/// Input for updating a memory file. Optional fields are left unchanged when None.
#[derive(Debug, Clone, Default)]
pub struct UpdateMemoryFile {
    pub content: Option<Vec<u8>>,
    pub content_hash: Option<Option<String>>,
}

/// Lightweight memory file info for listings (no content).
#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct MemoryFileInfoRow {
    pub id: Uuid,
    pub memory_id: Uuid,
    pub path: String,
    pub is_directory: bool,
    pub size_bytes: i64,
    pub content_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
