// In-memory storage: MCP Servers

use super::super::models::*;
use super::InMemoryDatabase;
use super::matches_search_tokens;
use anyhow::Result;
use anyhow::anyhow;
use everruns_core::McpServerId;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // MCP Servers
    // ============================================

    pub async fn create_mcp_server(
        &self,
        org_id: i64,
        input: CreateMcpServerRow,
    ) -> Result<McpServerRow> {
        // Check for duplicate name within org
        if self
            .mcp_servers
            .read()
            .values()
            .any(|s| s.name == input.name && s.org_id == org_id)
        {
            return Err(anyhow!(
                "MCP server with name '{}' already exists",
                input.name
            ));
        }

        let now = Self::now();
        let id = McpServerId::new();
        let api_key_set = input.api_key_encrypted.is_some();

        let row = McpServerRow {
            id,
            org_id,
            name: input.name,
            description: input.description,
            url: input.url,
            transport_type: input.transport_type,
            status: "active".to_string(),
            api_key_encrypted: input.api_key_encrypted,
            api_key_set,
            headers: input.headers.unwrap_or(serde_json::json!({})),
            settings: input.settings.unwrap_or(serde_json::json!({})),
            cached_tools: serde_json::json!([]),
            tools_cached_at: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };

        self.mcp_servers.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create MCP server with a specific ID (for seeding)
    /// Returns None if server already exists with this ID
    /// Create or update MCP server with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_mcp_server_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateMcpServerRow,
    ) -> Result<Option<McpServerRow>> {
        let id = McpServerId::from_uuid(id);
        let mut servers = self.mcp_servers.write();
        let now = Self::now();

        if let Some(existing) = servers.get(&id) {
            if existing.name == input.name
                && existing.description == input.description
                && existing.url == input.url
                && existing.transport_type == input.transport_type
            {
                return Ok(None); // Unchanged
            }
            let row = McpServerRow {
                name: input.name,
                description: input.description,
                url: input.url,
                transport_type: input.transport_type,
                updated_at: now,
                ..existing.clone()
            };
            servers.insert(id, row.clone());
            return Ok(Some(row));
        }

        let api_key_set = input.api_key_encrypted.is_some();
        let row = McpServerRow {
            id,
            org_id,
            name: input.name,
            description: input.description,
            url: input.url,
            transport_type: input.transport_type,
            status: "active".to_string(),
            api_key_encrypted: input.api_key_encrypted,
            api_key_set,
            headers: input.headers.unwrap_or(serde_json::json!({})),
            settings: input.settings.unwrap_or(serde_json::json!({})),
            cached_tools: serde_json::json!([]),
            tools_cached_at: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };

        servers.insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_mcp_server(&self, org_id: i64, id: Uuid) -> Result<Option<McpServerRow>> {
        let id = McpServerId::from_uuid(id);
        Ok(self
            .mcp_servers
            .read()
            .get(&id)
            .filter(|s| s.org_id == org_id)
            .cloned())
    }

    /// Batch fetch multiple MCP servers by IDs.
    pub async fn get_mcp_servers_batch(
        &self,
        org_id: i64,
        ids: &[Uuid],
    ) -> Result<Vec<McpServerRow>> {
        let servers = self.mcp_servers.read();
        Ok(ids
            .iter()
            .filter_map(|id| {
                servers
                    .get(&McpServerId::from_uuid(*id))
                    .filter(|s| s.org_id == org_id)
                    .cloned()
            })
            .collect())
    }

    pub async fn get_mcp_server_by_name(
        &self,
        org_id: i64,
        name: &str,
    ) -> Result<Option<McpServerRow>> {
        Ok(self
            .mcp_servers
            .read()
            .values()
            .find(|s| s.name == name && s.org_id == org_id)
            .cloned())
    }

    pub async fn list_mcp_servers(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<McpServerRow>> {
        let mut servers: Vec<_> = self
            .mcp_servers
            .read()
            .values()
            .filter(|s| s.org_id == org_id)
            .filter(|s| {
                if include_archived {
                    s.status != "deleted"
                } else {
                    s.status != "archived" && s.status != "deleted"
                }
            })
            .filter(|s| {
                matches_search_tokens(search, &[&s.name, s.description.as_deref().unwrap_or("")])
            })
            .cloned()
            .collect();
        servers.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(servers)
    }

    pub async fn list_active_mcp_servers(&self, org_id: i64) -> Result<Vec<McpServerRow>> {
        let mut servers: Vec<_> = self
            .mcp_servers
            .read()
            .values()
            .filter(|s| s.status == "active" && s.org_id == org_id)
            .cloned()
            .collect();
        servers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(servers)
    }

    pub async fn update_mcp_server(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMcpServer,
    ) -> Result<Option<McpServerRow>> {
        let mut servers = self.mcp_servers.write();
        if let Some(server) = servers.get_mut(&id) {
            if server.org_id != org_id {
                return Ok(None);
            }
            if let Some(name) = input.name {
                server.name = name;
            }
            if let Some(description) = input.description {
                server.description = Some(description);
            }
            if let Some(url) = input.url {
                server.url = url;
            }
            if let Some(transport_type) = input.transport_type {
                server.transport_type = transport_type;
            }
            if let Some(status) = input.status {
                server.status = status;
            }
            if let Some(api_key_encrypted) = input.api_key_encrypted {
                server.api_key_encrypted = Some(api_key_encrypted);
                server.api_key_set = true;
            }
            if let Some(headers) = input.headers {
                server.headers = headers;
            }
            if let Some(settings) = input.settings {
                server.settings = settings;
            }
            server.updated_at = Self::now();
            return Ok(Some(server.clone()));
        }
        Ok(None)
    }

    pub async fn delete_mcp_server(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut servers = self.mcp_servers.write();
        if let Some(server) = servers.get_mut(&id) {
            if server.org_id != org_id || !matches!(server.status.as_str(), "active" | "disabled") {
                return Ok(false);
            }
            server.status = "archived".to_string();
            server.archived_at = Some(Self::now());
            server.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn destroy_mcp_server(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let id = McpServerId::from_uuid(id);
        let mut servers = self.mcp_servers.write();
        if let Some(server) = servers.get_mut(&id) {
            if server.org_id != org_id || server.status != "archived" {
                return Ok(false);
            }
            server.status = "deleted".to_string();
            server.deleted_at = Some(Self::now());
            server.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }
}
