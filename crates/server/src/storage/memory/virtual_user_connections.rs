// In-memory storage: Virtual User Connections

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_contracts::typed_id::{AgentId, VirtualUserId};
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Virtual User Connections
    // ============================================

    pub async fn upsert_virtual_user_connection(
        &self,
        input: CreateVirtualUserConnectionRow,
    ) -> Result<VirtualUserConnectionRow> {
        let now = Self::now();
        let mut connections = self.virtual_user_connections.write();
        let prior = connections
            .values()
            .find(|c| c.virtual_user_id == input.virtual_user_id && c.provider == input.provider)
            .cloned();
        let id = prior.as_ref().map(|c| c.id).unwrap_or_else(Uuid::now_v7);
        connections.retain(|_, c| {
            !(c.virtual_user_id == input.virtual_user_id && c.provider == input.provider)
        });

        let row = VirtualUserConnectionRow {
            id,
            virtual_user_id: input.virtual_user_id,
            provider: input.provider,
            connection_type: input.connection_type,
            provider_user_id: input.provider_user_id,
            provider_username: input.provider_username,
            access_token_encrypted: input.access_token_encrypted,
            refresh_token_encrypted: input.refresh_token_encrypted,
            scopes: input.scopes,
            expires_at: input.expires_at,
            installation_id: input.installation_id,
            provider_metadata: input.provider_metadata,
            created_at: prior.as_ref().map(|c| c.created_at).unwrap_or(now),
            updated_at: now,
        };
        connections.insert(id, row.clone());
        drop(connections);
        for resource in self.leased_resources.write().values_mut().filter(|r| {
            r.owner_user_id == Some(row.virtual_user_id.uuid()) && r.provider == row.provider
        }) {
            if let Some(metadata) = resource.metadata.as_object_mut() {
                metadata.remove("connection_migration_pending");
            }
        }
        Ok(row)
    }

    pub async fn upsert_virtual_user_connection_for_active_agent(
        &self,
        org_id: i64,
        agent_id: AgentId,
        input: CreateVirtualUserConnectionRow,
    ) -> Result<Option<VirtualUserConnectionRow>> {
        let agents = self.agents.read();
        let identities = self.virtual_users.read();
        let is_eligible = agents.get(&agent_id).is_some_and(|agent| {
            agent.org_id == org_id
                && agent.status == "active"
                && agent.virtual_user_id == Some(input.virtual_user_id)
        }) && identities
            .get(&input.virtual_user_id)
            .is_some_and(|identity| {
                identity.org_id == org_id
                    && identity.status == "active"
                    && identity.usage == "service"
            });
        if !is_eligible {
            return Ok(None);
        }

        let now = Self::now();
        let mut connections = self.virtual_user_connections.write();
        let prior = connections
            .values()
            .find(|c| c.virtual_user_id == input.virtual_user_id && c.provider == input.provider)
            .cloned();
        let id = prior.as_ref().map(|c| c.id).unwrap_or_else(Uuid::now_v7);
        connections.retain(|_, connection| {
            !(connection.virtual_user_id == input.virtual_user_id
                && connection.provider == input.provider)
        });
        let row = VirtualUserConnectionRow {
            id,
            virtual_user_id: input.virtual_user_id,
            provider: input.provider,
            connection_type: input.connection_type,
            provider_user_id: input.provider_user_id,
            provider_username: input.provider_username,
            access_token_encrypted: input.access_token_encrypted,
            refresh_token_encrypted: input.refresh_token_encrypted,
            scopes: input.scopes,
            expires_at: input.expires_at,
            installation_id: input.installation_id,
            provider_metadata: input.provider_metadata,
            created_at: prior.as_ref().map(|c| c.created_at).unwrap_or(now),
            updated_at: now,
        };
        connections.insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_virtual_user_connection(
        &self,
        identity_id: VirtualUserId,
        provider: &str,
    ) -> Result<Option<VirtualUserConnectionRow>> {
        Ok(self
            .virtual_user_connections
            .read()
            .values()
            .find(|c| c.virtual_user_id == identity_id && c.provider == provider)
            .cloned())
    }

    pub async fn list_virtual_user_connections(
        &self,
        identity_id: VirtualUserId,
    ) -> Result<Vec<VirtualUserConnectionRow>> {
        let mut connections: Vec<_> = self
            .virtual_user_connections
            .read()
            .values()
            .filter(|c| c.virtual_user_id == identity_id)
            .cloned()
            .collect();
        connections.sort_by_key(|connection| connection.provider.clone());
        Ok(connections)
    }

    pub async fn update_virtual_user_connection_oauth_tokens(
        &self,
        input: UpdateOAuthConnectionTokens,
    ) -> Result<Option<VirtualUserConnectionRow>> {
        let mut connections = self.virtual_user_connections.write();
        let Some(connection) = connections.get_mut(&input.connection_id) else {
            return Ok(None);
        };
        if connection.connection_type != "oauth" {
            return Ok(None);
        }
        connection.access_token_encrypted = Some(input.access_token_encrypted);
        connection.refresh_token_encrypted = Some(input.refresh_token_encrypted);
        connection.expires_at = input.expires_at;
        if input.scopes.is_some() {
            connection.scopes = input.scopes;
        }
        connection.updated_at = Self::now();
        Ok(Some(connection.clone()))
    }

    pub async fn delete_all_virtual_user_connections(
        &self,
        identity_id: VirtualUserId,
    ) -> Result<u64> {
        let mut connections = self.virtual_user_connections.write();
        let before = connections.len();
        connections.retain(|_, connection| connection.virtual_user_id != identity_id);
        Ok((before - connections.len()) as u64)
    }

    pub async fn delete_virtual_user_connection(
        &self,
        identity_id: VirtualUserId,
        provider: &str,
    ) -> Result<bool> {
        let mut connections = self.virtual_user_connections.write();
        let before = connections.len();
        connections.retain(|_, c| !(c.virtual_user_id == identity_id && c.provider == provider));
        Ok(connections.len() < before)
    }

    pub async fn invalidate_mcp_service_connection_if_access_token_matches(
        &self,
        identity_id: VirtualUserId,
        provider: &str,
        expected_access_token_encrypted: &[u8],
        org_id: i64,
        mcp_server_id: Uuid,
        agent_id: Uuid,
    ) -> Result<bool> {
        let mut connections = self.virtual_user_connections.write();
        let before = connections.len();
        connections.retain(|_, connection| {
            connection.virtual_user_id != identity_id
                || connection.provider != provider
                || connection.access_token_encrypted.as_deref()
                    != Some(expected_access_token_encrypted)
        });
        let deleted = connections.len() < before;
        if deleted {
            self.mcp_service_tool_caches
                .write()
                .retain(|key, _| !(key.0 == org_id && key.1 == mcp_server_id && key.2 == agent_id));
        }
        Ok(deleted)
    }
}
