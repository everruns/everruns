//! User MCP servers: `mcp_servers` rows a virtual user owns.
//! See knowledge/integrations/user-mcp-servers.md.

use super::*;
use crate::storage::repositories::{OwnedMcpServerRow, UserMcpServerRow};

impl StorageBackend {
    pub async fn get_mcp_server_with_owner(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<OwnedMcpServerRow>> {
        dispatch!(self, get_mcp_server_with_owner, org_id, id)
    }

    pub async fn update_mcp_server_settings_any_owner(
        &self,
        org_id: i64,
        id: Uuid,
        settings: serde_json::Value,
    ) -> Result<bool> {
        dispatch!(
            self,
            update_mcp_server_settings_any_owner,
            org_id,
            id,
            settings
        )
    }

    pub async fn create_user_mcp_server(
        &self,
        org_id: i64,
        owner: Uuid,
        catalog_mcp_server_id: Option<Uuid>,
        input: CreateMcpServerRow,
    ) -> Result<UserMcpServerRow> {
        dispatch!(
            self,
            create_user_mcp_server,
            org_id,
            owner,
            catalog_mcp_server_id,
            input
        )
    }

    pub async fn list_user_mcp_servers(
        &self,
        org_id: i64,
        owner: Uuid,
    ) -> Result<Vec<UserMcpServerRow>> {
        dispatch!(self, list_user_mcp_servers, org_id, owner)
    }

    pub async fn get_user_mcp_server(
        &self,
        org_id: i64,
        owner: Uuid,
        id: Uuid,
    ) -> Result<Option<UserMcpServerRow>> {
        dispatch!(self, get_user_mcp_server, org_id, owner, id)
    }

    pub async fn update_user_mcp_server(
        &self,
        org_id: i64,
        owner: Uuid,
        id: Uuid,
        input: UpdateMcpServer,
    ) -> Result<Option<McpServerRow>> {
        dispatch!(self, update_user_mcp_server, org_id, owner, id, input)
    }

    pub async fn delete_user_mcp_server(&self, org_id: i64, owner: Uuid, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_user_mcp_server, org_id, owner, id)
    }
}
