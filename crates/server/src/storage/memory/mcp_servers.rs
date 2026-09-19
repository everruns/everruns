// In-memory storage: MCP Servers
//
// Spec: knowledge/integrations/mcp.md (umbrella), knowledge/integrations/mcp-servers.md (detail)

use super::super::mcp_catalog::{McpServerAgentNamesRow, McpServerAgentUsageRow};
use super::super::mcp_tool_cache::*;
use super::super::models::*;
use super::InMemoryDatabase;
use super::matches_search_tokens;
use anyhow::Result;
use anyhow::anyhow;
use everruns_provider::typed_id::McpServerId;
use std::collections::HashSet;
use uuid::Uuid;

/// Statuses that hold a name. Mirrors the partial unique index in
/// `132_mcp_server_name_scope.sql`: archived and deleted servers release their
/// name, a disabled one keeps it because it can be re-enabled (EVE-964).
fn holds_name(status: &str) -> bool {
    matches!(status, "active" | "disabled")
}

impl InMemoryDatabase {
    // ============================================
    // MCP Servers
    // ============================================

    pub async fn create_mcp_server(
        &self,
        org_id: i64,
        input: CreateMcpServerRow,
    ) -> Result<McpServerRow> {
        // Check for duplicate name within org, among rows that still hold one.
        if self
            .mcp_servers
            .read()
            .values()
            .any(|s| s.name == input.name && s.org_id == org_id && holds_name(&s.status))
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

    /// Look up the owning org for an MCP server by its public id.
    pub async fn get_mcp_server_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        let Ok(id) = public_id.parse::<McpServerId>() else {
            return Ok(None);
        };
        Ok(self.mcp_servers.read().get(&id).map(|s| s.org_id))
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
            // Live rows only, matching the Postgres backend: an archived row
            // no longer owns the name and must not shadow the live server.
            .find(|s| s.name == name && s.org_id == org_id && holds_name(&s.status))
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
        servers.sort_by_key(|server| std::cmp::Reverse(server.created_at));
        Ok(servers)
    }

    pub async fn list_mcp_server_agent_usage(
        &self,
        org_id: i64,
    ) -> Result<Vec<McpServerAgentUsageRow>> {
        let servers = self.mcp_servers.read();
        let agents = self.agents.read();
        let mut usage = servers
            .values()
            .filter(|server| server.org_id == org_id && server.status != "deleted")
            .map(|server| {
                let reference = format!("catalog:{}", server.name);
                let used_by_agents = agents
                    .values()
                    .filter(|agent| {
                        agent.org_id == org_id
                            && agent.status == "active"
                            && agent.archived_at.is_none()
                            && agent.deleted_at.is_none()
                            && agent.mcp_servers.as_object().is_some_and(|attachments| {
                                attachments.values().any(|attachment| {
                                    attachment.get("use").and_then(|value| value.as_str())
                                        == Some(reference.as_str())
                                })
                            })
                    })
                    .count() as i64;
                McpServerAgentUsageRow {
                    mcp_server_id: server.id,
                    used_by_agents,
                }
            })
            .collect::<Vec<_>>();
        usage.sort_by_key(|row| row.mcp_server_id.to_string());
        Ok(usage)
    }

    pub async fn get_mcp_server_agent_names(
        &self,
        org_id: i64,
        server_id: McpServerId,
        limit: i64,
    ) -> Result<McpServerAgentNamesRow> {
        let servers = self.mcp_servers.read();
        let Some(server) = servers
            .get(&server_id)
            .filter(|server| server.org_id == org_id && server.status != "deleted")
        else {
            return Ok(McpServerAgentNamesRow {
                agent_names: Vec::new(),
                total_count: 0,
            });
        };
        let reference = format!("catalog:{}", server.name);
        let agents = self.agents.read();
        let mut seen = HashSet::new();
        let mut names = agents
            .values()
            .filter(|agent| {
                agent.org_id == org_id
                    && agent.status == "active"
                    && agent.archived_at.is_none()
                    && agent.deleted_at.is_none()
                    && agent.mcp_servers.as_object().is_some_and(|attachments| {
                        attachments.values().any(|attachment| {
                            attachment.get("use").and_then(|value| value.as_str())
                                == Some(reference.as_str())
                        })
                    })
                    && seen.insert(agent.id)
            })
            .map(|agent| {
                agent
                    .display_name
                    .clone()
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| agent.name.clone())
            })
            .collect::<Vec<_>>();
        names.sort_by_key(|name| name.to_lowercase());
        let total_count = names.len() as i64;
        names.truncate(limit.max(0) as usize);
        Ok(McpServerAgentNamesRow {
            agent_names: names,
            total_count,
        })
    }

    pub async fn list_active_mcp_servers(&self, org_id: i64) -> Result<Vec<McpServerRow>> {
        let mut servers: Vec<_> = self
            .mcp_servers
            .read()
            .values()
            .filter(|s| s.status == "active" && s.org_id == org_id)
            .cloned()
            .collect();
        servers.sort_by_key(|server| server.name.clone());
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
                if api_key_encrypted.is_empty() {
                    server.api_key_encrypted = None;
                    server.api_key_set = false;
                } else {
                    server.api_key_encrypted = Some(api_key_encrypted);
                    server.api_key_set = true;
                }
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

    pub async fn update_mcp_server_tools(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMcpServerTools,
    ) -> Result<Option<McpServerRow>> {
        let id = McpServerId::from_uuid(id);
        let mut servers = self.mcp_servers.write();
        if let Some(server) = servers.get_mut(&id) {
            if server.org_id != org_id {
                return Ok(None);
            }
            server.cached_tools = input.cached_tools;
            server.tools_cached_at = Some(Self::now());
            server.updated_at = Self::now();
            return Ok(Some(server.clone()));
        }
        Ok(None)
    }

    pub async fn clear_mcp_server_tools(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<McpServerRow>> {
        let id = McpServerId::from_uuid(id);
        let mut servers = self.mcp_servers.write();
        if let Some(server) = servers.get_mut(&id) {
            if server.org_id != org_id {
                return Ok(None);
            }
            server.cached_tools = serde_json::json!([]);
            server.tools_cached_at = None;
            server.updated_at = Self::now();
            return Ok(Some(server.clone()));
        }
        Ok(None)
    }

    pub async fn get_mcp_service_tool_cache(
        &self,
        org_id: i64,
        mcp_server_id: Uuid,
        agent_id: Uuid,
        cache_scope: &str,
        credential_hash: &str,
    ) -> Result<Option<McpServiceToolCacheRow>> {
        Ok(self
            .mcp_service_tool_caches
            .read()
            .get(&(
                org_id,
                mcp_server_id,
                agent_id,
                cache_scope.to_string(),
                credential_hash.to_string(),
            ))
            .cloned())
    }

    pub async fn upsert_mcp_service_tool_cache(
        &self,
        input: UpsertMcpServiceToolCache,
    ) -> Result<McpServiceToolCacheRow> {
        let row = McpServiceToolCacheRow {
            org_id: input.org_id,
            mcp_server_id: input.mcp_server_id,
            agent_id: input.agent_id,
            cache_scope: input.cache_scope,
            credential_hash: input.credential_hash,
            cached_tools: input.cached_tools,
            ttl_ms: input.ttl_ms,
            tools_cached_at: Self::now(),
        };
        self.mcp_service_tool_caches.write().insert(
            (
                row.org_id,
                row.mcp_server_id,
                row.agent_id,
                row.cache_scope.clone(),
                row.credential_hash.clone(),
            ),
            row.clone(),
        );
        Ok(row)
    }

    pub async fn delete_mcp_service_tool_caches(
        &self,
        org_id: i64,
        mcp_server_id: Uuid,
        agent_id: Uuid,
    ) -> Result<u64> {
        let mut caches = self.mcp_service_tool_caches.write();
        let before = caches.len();
        caches.retain(|key, _| !(key.0 == org_id && key.1 == mcp_server_id && key.2 == agent_id));
        Ok((before - caches.len()) as u64)
    }

    pub async fn delete_mcp_service_tool_cache(
        &self,
        org_id: i64,
        mcp_server_id: Uuid,
        agent_id: Uuid,
        cache_scope: &str,
        credential_hash: &str,
    ) -> Result<u64> {
        Ok(self
            .mcp_service_tool_caches
            .write()
            .remove(&(
                org_id,
                mcp_server_id,
                agent_id,
                cache_scope.to_string(),
                credential_hash.to_string(),
            ))
            .is_some() as u64)
    }

    pub async fn delete_obsolete_mcp_service_private_tool_caches(
        &self,
        org_id: i64,
        mcp_server_id: Uuid,
        agent_id: Uuid,
        current_credential_hash: &str,
    ) -> Result<u64> {
        let mut caches = self.mcp_service_tool_caches.write();
        let before = caches.len();
        caches.retain(|key, _| {
            !(key.0 == org_id
                && key.1 == mcp_server_id
                && key.2 == agent_id
                && key.3 == "private"
                && key.4 != current_credential_hash)
        });
        Ok((before - caches.len()) as u64)
    }

    /// Test-only: backdate a server's tool-cache timestamp so staleness paths
    /// can be exercised without waiting for the TTL to elapse.
    #[cfg(test)]
    pub(crate) fn set_tools_cached_at_for_test(
        &self,
        id: Uuid,
        cached_at: chrono::DateTime<chrono::Utc>,
    ) {
        let id = McpServerId::from_uuid(id);
        if let Some(server) = self.mcp_servers.write().get_mut(&id) {
            server.tools_cached_at = Some(cached_at);
        }
    }
}
