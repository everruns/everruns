use crate::api::common::deserialize_nullable_update_field;
use chrono::{DateTime, Utc};
use everruns_durable::UpdateField;
use everruns_provider::typed_id::{AgentId, MemoryId};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

pub use crate::storage::models::{CreateMemoryRow, MemoryRow, UpdateMemory};

/// Response body for memory.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = Memory)]
pub struct MemoryResponse {
    #[schema(value_type = String, example = "mem_01933b5a000070008000000000000001")]
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: MemoryId,
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "design-docs")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Living design documents synced from GitHub")]
    pub description: Option<String>,
    /// Ownership scope (`org`, `agent`, or `user`).
    #[schema(example = "org")]
    pub scope: String,
    /// Owning agent when `scope = agent`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "agent_01933b5a000070008000000000000001")]
    pub owner_agent_id: Option<AgentId>,
    /// Owning user when `scope = user`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<Uuid>,
    /// Source kind discriminator (`manual`, `git`, `github`). Determines which `source` variant is populated.
    #[schema(example = "github")]
    pub source_type: String,
    /// Source-specific configuration (git remote, github repo, manual upload).
    pub source: MemorySourceResponse,
    /// Whether the memory is mounted read-only into sessions. Read-only memories accept no writes from the session sandbox.
    #[schema(example = false)]
    pub is_readonly: bool,
    /// Current sync status (`idle`, `syncing`, `succeeded`, `failed`). Only meaningful when `source_type` is `git` or `github`.
    #[schema(example = "succeeded")]
    pub sync_status: String,
    /// Timestamp of the most recent successful sync (RFC 3339). `None` if never synced.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "2026-05-25T08:00:00Z")]
    pub last_synced_at: Option<DateTime<Utc>>,
    /// Most recent sync error message; cleared on the next successful sync.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "ssh: connect to host github.com port 22: Connection timed out")]
    pub last_sync_error: Option<String>,
    #[serde(skip)]
    /// Internal database UUID. Not part of the public identifier surface.
    pub internal_id: Uuid,
    /// Current lifecycle status.
    #[schema(example = "active")]
    pub status: String,
    /// Timestamp when this resource was created (RFC 3339).
    #[schema(example = "2026-04-01T10:00:00Z")]
    pub created_at: DateTime<Utc>,
    /// Timestamp when this resource was last updated (RFC 3339).
    #[schema(example = "2026-05-25T08:00:00Z")]
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Timestamp when this resource was archived, if any (RFC 3339).
    #[schema(example = "2026-05-26T00:00:00Z")]
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Timestamp when this resource was soft-deleted, if any (RFC 3339).
    #[schema(example = "2026-05-26T00:00:00Z")]
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Request body for the `create_memory` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateMemoryRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "design-docs")]
    pub name: String,
    #[serde(default)]
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Living design documents synced from GitHub")]
    pub description: Option<String>,
    /// Source configuration. Example shape is defined on `CreateMemorySourceRequest`.
    #[serde(default)]
    pub source: Option<CreateMemorySourceRequest>,
}

/// Request body for the `update_memory` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateMemoryRequest {
    #[serde(default)]
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "design-docs")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: UpdateField<String>,
    /// Source configuration. Example shape is defined on `CreateMemorySourceRequest`.
    #[serde(default)]
    pub source: Option<CreateMemorySourceRequest>,
}

/// Query parameters for `GET /v1/memories` — optional name search and a
/// flag to include archived memories in the listing.
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ListMemoriesQuery {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub include_archived: Option<bool>,
}

/// Request body for the `create_memory_source` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
#[schema(example = json!({"type": "github", "repository": "acme/design-docs", "branch": "main", "root_folder": "docs/"}))]
pub enum CreateMemorySourceRequest {
    Github(GitHubMemorySourceRequest),
    Git(GitMemorySourceRequest),
}

/// Response body for memory source.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum MemorySourceResponse {
    Manual(ManualMemorySourceResponse),
    Github(GitHubMemorySourceResponse),
    Git(GitMemorySourceResponse),
}

/// Response body for manual memory source.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ManualMemorySourceResponse {}

/// Response body for GitHub memory source.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GitHubMemorySourceResponse {
    pub repository: String,
    pub branch: String,
    #[serde(default)]
    pub root_folder: Option<String>,
    /// Automatic resync interval in seconds. Omit or set 0 for manual-only; scheduled sync accepts 300 through 604800.
    #[serde(default)]
    #[schema(maximum = 604800, minimum = 0)]
    pub sync_interval_secs: Option<u32>,
}

/// Response body for git memory source.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GitMemorySourceResponse {
    pub url: String,
    pub branch: String,
    #[serde(default)]
    pub root_folder: Option<String>,
    /// Automatic resync interval in seconds. Omit or set 0 for manual-only; scheduled sync accepts 300 through 604800.
    #[serde(default)]
    #[schema(maximum = 604800, minimum = 0)]
    pub sync_interval_secs: Option<u32>,
}

/// Request body for GitHub memory source.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct GitHubMemorySourceRequest {
    /// GitHub repository in `owner/repo` form.
    #[schema(example = "acme/design-docs")]
    pub repository: String,
    /// Branch to sync. Defaults to the repository's default branch when omitted.
    #[serde(default)]
    #[schema(example = "main")]
    pub branch: Option<String>,
    /// Sub-directory within the repository to sync. Empty / omitted = whole tree.
    #[serde(default)]
    #[schema(example = "docs/")]
    pub root_folder: Option<String>,
    /// Automatic resync interval in seconds. Omit or set 0 for manual-only; scheduled sync accepts 300 through 604800.
    #[serde(default)]
    #[schema(maximum = 604800, minimum = 0, example = 3600)]
    pub sync_interval_secs: Option<u32>,
}

/// Request body for git memory source.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct GitMemorySourceRequest {
    /// Clonable git URL (SSH or HTTPS).
    #[schema(example = "https://github.com/acme/design-docs.git")]
    pub url: String,
    /// Branch to sync. Defaults to the repository's default branch when omitted.
    #[serde(default)]
    #[schema(example = "main")]
    pub branch: Option<String>,
    /// Sub-directory within the repository to sync. Empty / omitted = whole tree.
    #[serde(default)]
    #[schema(example = "docs/")]
    pub root_folder: Option<String>,
    /// Automatic resync interval in seconds. Omit or set 0 for manual-only; scheduled sync accepts 300 through 604800.
    #[serde(default)]
    #[schema(maximum = 604800, minimum = 0, example = 3600)]
    pub sync_interval_secs: Option<u32>,
}

fn memory_source_response(row: &MemoryRow) -> anyhow::Result<MemorySourceResponse> {
    if row.source_type == "manual" {
        return Ok(MemorySourceResponse::Manual(ManualMemorySourceResponse {}));
    }

    let source: MemorySourceResponse = serde_json::from_value(row.source_config.clone())?;
    match (&source, row.source_type.as_str()) {
        (MemorySourceResponse::Github(_), "github") | (MemorySourceResponse::Git(_), "git") => {
            Ok(source)
        }
        _ => anyhow::bail!("memory source_type does not match source config provider"),
    }
}

pub fn memory_response(row: MemoryRow) -> anyhow::Result<MemoryResponse> {
    let source = memory_source_response(&row)?;
    Ok(MemoryResponse {
        id: row.public_id.parse()?,
        name: row.name,
        description: row.description,
        scope: row.scope,
        owner_agent_id: row.owner_agent_id,
        owner_user_id: row.owner_user_id,
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
