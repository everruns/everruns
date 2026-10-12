// Rows the declarative capabilities repository reads and writes.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct DeclarativeCapabilityRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: String,
    pub status: String,
    pub definition: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateDeclarativeCapabilityRow {
    pub public_id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: String,
    pub definition: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateDeclarativeCapability {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub definition: Option<serde_json::Value>,
}
