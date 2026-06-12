// HTTP DTOs for workspace CRUD endpoints.

use crate::storage::models::WorkspaceRow;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkspaceResponse {
    #[schema(example = "wsp_01933b5a000070008000000000000001")]
    pub id: String,
    #[schema(example = "team-research")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Shared workspace for the Q4 research project")]
    pub description: Option<String>,
    #[schema(example = "active")]
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

pub fn workspace_response(row: WorkspaceRow) -> WorkspaceResponse {
    WorkspaceResponse {
        id: row.public_id,
        name: row.name,
        description: row.description,
        status: row.status,
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateWorkspaceRequest {
    #[schema(example = "team-research")]
    pub name: String,
    #[serde(default)]
    #[schema(example = "Shared workspace for the Q4 research project")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateWorkspaceRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub description: Option<Option<String>>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ListWorkspacesQuery {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub include_archived: bool,
}

fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Ok(Some(Option::deserialize(de)?))
}
