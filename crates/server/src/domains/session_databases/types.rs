// Session databases domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use everruns_contracts::session_sqldb::DatabaseInfo;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Request body for creating a database.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateDatabaseRequest {
    /// Database name (alphanumeric + underscores, max 64 chars).
    #[schema(example = "refund_history")]
    pub name: String,
}

/// Database info response.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DatabaseInfoResponse {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    pub size_bytes: i64,
    pub page_count: i32,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: String,
    /// Timestamp when this resource was last updated (RFC 3339).
    pub updated_at: String,
}

impl From<DatabaseInfo> for DatabaseInfoResponse {
    fn from(info: DatabaseInfo) -> Self {
        Self {
            name: info.name,
            size_bytes: info.size_bytes,
            page_count: info.page_count,
            created_at: info.created_at.to_rfc3339(),
            updated_at: info.updated_at.to_rfc3339(),
        }
    }
}

/// Schema response for a database.
#[derive(Debug, Serialize, ToSchema)]
pub struct SchemaResponse {
    pub database: String,
    pub tables: Vec<serde_json::Value>,
}
