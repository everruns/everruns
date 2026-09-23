// Project domain types — shared by the project commands and the REST adapter.

use crate::storage::models::ProjectRow;
use serde::Serialize;
use utoipa::ToSchema;

/// Project DTO returned by the API and MCP commands.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectResponse {
    /// External identifier (proj_<32-hex-chars>).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether this is the org's default project.
    pub is_default: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<ProjectRow> for ProjectResponse {
    fn from(row: ProjectRow) -> Self {
        Self {
            id: row.public_id,
            name: row.name,
            description: row.description,
            is_default: row.is_default,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Response for the delete-project command.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeleteProjectResponse {
    pub success: bool,
}
