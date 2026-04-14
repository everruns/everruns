// MCP Server query helpers — shared by commands, gRPC, and other domains.
//
// No policy checks, no input validation. Pure data access + mapping.

use crate::services::mcp_server::{McpServerService, McpServerSettings};
use crate::storage::StorageBackend;
use everruns_core::{McpServer, McpServerAuthMode, McpServerStatus, McpServerTransportType};
use std::collections::HashMap;
use uuid::Uuid;

use super::types::McpServerRow;

// ============================================================================
// Row mapping
// ============================================================================

pub fn row_to_mcp_server(row: &McpServerRow) -> McpServer {
    let headers: HashMap<String, String> =
        serde_json::from_value(row.headers.clone()).unwrap_or_default();
    let settings = McpServerService::settings_from_row(row);
    let oauth_provider_id = (settings.auth_mode == McpServerAuthMode::OAuth)
        .then(|| McpServerService::oauth_provider_id(row.id.uuid()));

    McpServer {
        id: row.id,
        name: row.name.clone(),
        description: row.description.clone(),
        url: row.url.clone(),
        transport_type: McpServerTransportType::from(row.transport_type.as_str()),
        status: McpServerStatus::from(row.status.as_str()),
        auth_mode: settings.auth_mode,
        oauth_provider_id,
        api_key_set: row.api_key_set,
        headers,
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
    }
}

// ============================================================================
// Data access helpers
// ============================================================================

/// Get a single MCP server by UUID, excluding deleted.
pub async fn get_by_id(
    db: &StorageBackend,
    org_id: i64,
    id: Uuid,
) -> anyhow::Result<Option<McpServer>> {
    let row = db.get_mcp_server(org_id, id).await?;
    Ok(row
        .as_ref()
        .filter(|r| r.status != "deleted")
        .map(row_to_mcp_server))
}

/// Get raw row by UUID (needed for update logic that inspects row-level fields).
pub async fn get_row(
    db: &StorageBackend,
    org_id: i64,
    id: Uuid,
) -> anyhow::Result<Option<McpServerRow>> {
    db.get_mcp_server(org_id, id).await
}

/// Build settings from a row, delegating to McpServerService.
pub fn settings_from_row(row: &McpServerRow) -> McpServerSettings {
    McpServerService::settings_from_row(row)
}
