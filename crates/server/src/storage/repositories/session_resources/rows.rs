// Rows the session resources repository reads and writes.

use crate::kernel_imports::contracts::typed_id::SessionId;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct SessionResourceRow {
    pub id: Uuid,
    pub session_id: SessionId,
    pub resource_id: String,
    pub kind: String,
    pub display_name: String,
    pub status: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertSessionResourceRow {
    pub session_id: SessionId,
    pub resource_id: String,
    pub kind: String,
    pub display_name: String,
    pub status: String,
    pub metadata: serde_json::Value,
}
