// In-memory storage: Agent Identity Connections

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_core::AgentIdentityId;
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
        connections.sort_by(|a, b| a.provider.cmp(&b.provider));
        Ok(connections)
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
}
