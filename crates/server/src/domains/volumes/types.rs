use crate::api::common::deserialize_nullable_update_field;
use chrono::{DateTime, Utc};
use everruns_core::typed_id::VolumeId;
use everruns_durable::UpdateField;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

pub use crate::storage::models::{CreateVolumeRow, UpdateVolume, VolumeRow};

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VolumeResponse {
    #[schema(value_type = String, example = "vol_01933b5a000070008000000000000001")]
    pub id: VolumeId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub source_type: String,
    pub source: VolumeSourceResponse,
    pub is_readonly: bool,
    pub sync_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_error: Option<String>,
    #[serde(skip)]
    pub internal_id: Uuid,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateVolumeRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub source: Option<CreateVolumeSourceRequest>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateVolumeRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub description: UpdateField<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ListVolumesQuery {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub include_archived: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CreateVolumeSourceRequest {
    Github(GitHubVolumeSourceRequest),
    Git(GitVolumeSourceRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum VolumeSourceResponse {
    Manual(ManualVolumeSourceResponse),
    Github(GitHubVolumeSourceResponse),
    Git(GitVolumeSourceResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ManualVolumeSourceResponse {}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GitHubVolumeSourceResponse {
    pub repository: String,
    pub branch: String,
    #[serde(default)]
    pub root_folder: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GitVolumeSourceResponse {
    pub url: String,
    pub branch: String,
    #[serde(default)]
    pub root_folder: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct GitHubVolumeSourceRequest {
    pub repository: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub root_folder: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct GitVolumeSourceRequest {
    pub url: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub root_folder: Option<String>,
}

fn volume_source_response(row: &VolumeRow) -> anyhow::Result<VolumeSourceResponse> {
    if row.source_type == "manual" {
        return Ok(VolumeSourceResponse::Manual(ManualVolumeSourceResponse {}));
    }

    let source: VolumeSourceResponse = serde_json::from_value(row.source_config.clone())?;
    match (&source, row.source_type.as_str()) {
        (VolumeSourceResponse::Github(_), "github") | (VolumeSourceResponse::Git(_), "git") => {
            Ok(source)
        }
        _ => anyhow::bail!("volume source_type does not match source config provider"),
    }
}

pub fn volume_response(row: VolumeRow) -> anyhow::Result<VolumeResponse> {
    let source = volume_source_response(&row)?;
    Ok(VolumeResponse {
        id: row.public_id.parse()?,
        name: row.name,
        description: row.description,
        source_type: row.source_type,
        source,
        is_readonly: row.is_readonly,
        sync_status: row.sync_status,
        last_synced_at: row.last_synced_at,
        last_sync_error: row.last_sync_error,
        internal_id: row.id,
        status: row.status,
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
    })
}
