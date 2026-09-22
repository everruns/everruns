// In-memory storage: Agent Identity Connections

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_provider::typed_id::{AgentId, AgentIdentityId};
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Agent Identity Connections
    // ============================================

    pub async fn upsert_agent_identity_connection(
        &self,
        input: CreateAgentIdentityConnectionRow,
    ) -> Result<AgentIdentityConnectionRow> {
        let now = Self::now();
        let id = Uuid::now_v7();

        let mut connections = self.agent_identity_connections.write();
        connections.retain(|_, c| {
            !(c.agent_identity_id == input.agent_identity_id && c.provider == input.provider)
        });

        let row = AgentIdentityConnectionRow {
            id,
            agent_identity_id: input.agent_identity_id,
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
            created_at: now,
            updated_at: now,
        };
        connections.insert(id, row.clone());
        Ok(row)
    }

    pub async fn upsert_agent_identity_connection_for_active_agent(
        &self,
        org_id: i64,
        agent_id: AgentId,
        input: CreateAgentIdentityConnectionRow,
    ) -> Result<Option<AgentIdentityConnectionRow>> {
        let agents = self.agents.read();
        let identities = self.agent_identities.read();
        let is_eligible = agents.get(&agent_id).is_some_and(|agent| {
            agent.org_id == org_id
                && agent.status == "active"
                && agent.agent_identity_id == Some(input.agent_identity_id)
        }) && identities
            .get(&input.agent_identity_id)
            .is_some_and(|identity| identity.org_id == org_id && identity.status == "active");
        if !is_eligible {
            return Ok(None);
        }

        let now = Self::now();
        let id = Uuid::now_v7();
        let mut connections = self.agent_identity_connections.write();
        connections.retain(|_, connection| {
            !(connection.agent_identity_id == input.agent_identity_id
                && connection.provider == input.provider)
        });
        let row = AgentIdentityConnectionRow {
            id,
            agent_identity_id: input.agent_identity_id,
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
            created_at: now,
            updated_at: now,
        };
        connections.insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_agent_identity_connection(
        &self,
        identity_id: AgentIdentityId,
        provider: &str,
    ) -> Result<Option<AgentIdentityConnectionRow>> {
        Ok(self
            .agent_identity_connections
            .read()
            .values()
            .find(|c| c.agent_identity_id == identity_id && c.provider == provider)
            .cloned())
    }

    pub async fn list_agent_identity_connections(
        &self,
        identity_id: AgentIdentityId,
    ) -> Result<Vec<AgentIdentityConnectionRow>> {
        let mut connections: Vec<_> = self
            .agent_identity_connections
            .read()
            .values()
            .filter(|c| c.agent_identity_id == identity_id)
            .cloned()
            .collect();
        connections.sort_by_key(|connection| connection.provider.clone());
        Ok(connections)
    }

    pub async fn update_agent_identity_connection_oauth_tokens(
        &self,
        input: UpdateOAuthConnectionTokens,
    ) -> Result<Option<AgentIdentityConnectionRow>> {
        let mut connections = self.agent_identity_connections.write();
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

    pub async fn delete_all_agent_identity_connections(
        &self,
        identity_id: AgentIdentityId,
    ) -> Result<u64> {
        let mut connections = self.agent_identity_connections.write();
        let before = connections.len();
        connections.retain(|_, connection| connection.agent_identity_id != identity_id);
        Ok((before - connections.len()) as u64)
    }

    pub async fn delete_agent_identity_connection(
        &self,
        identity_id: AgentIdentityId,
        provider: &str,
    ) -> Result<bool> {
        let mut connections = self.agent_identity_connections.write();
        let before = connections.len();
        connections.retain(|_, c| !(c.agent_identity_id == identity_id && c.provider == provider));
        Ok(connections.len() < before)
    }

    pub async fn invalidate_mcp_service_connection_if_access_token_matches(
        &self,
        identity_id: AgentIdentityId,
        provider: &str,
        expected_access_token_encrypted: &[u8],
        org_id: i64,
        mcp_server_id: Uuid,
        agent_id: Uuid,
    ) -> Result<bool> {
        let mut connections = self.agent_identity_connections.write();
        let before = connections.len();
        connections.retain(|_, connection| {
            connection.agent_identity_id != identity_id
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
