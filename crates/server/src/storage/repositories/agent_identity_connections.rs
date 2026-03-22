// PostgreSQL repository: Agent Identity Connections

use super::super::models::*;
use super::Database;
use anyhow::Result;
use everruns_core::AgentIdentityId;

impl Database {
    // ============================================
    // Agent Identity Connections
    // ============================================

    /// Create or replace an identity connection for a provider.
    pub async fn upsert_agent_identity_connection(
        &self,
        input: CreateAgentIdentityConnectionRow,
    ) -> Result<AgentIdentityConnectionRow> {
        sqlx::query(
            "DELETE FROM agent_identity_connections WHERE agent_identity_id = $1 AND provider = $2",
        )
        .bind(input.agent_identity_id)
        .bind(&input.provider)
        .execute(&self.pool)
        .await?;

        let row = sqlx::query_as::<_, AgentIdentityConnectionRow>(
            r#"
            INSERT INTO agent_identity_connections (agent_identity_id, provider, connection_type, provider_user_id, provider_username, access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id, agent_identity_id, provider, connection_type, provider_user_id, provider_username, access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, created_at, updated_at
            "#,
        )
        .bind(input.agent_identity_id)
        .bind(&input.provider)
        .bind(&input.connection_type)
        .bind(&input.provider_user_id)
        .bind(&input.provider_username)
        .bind(&input.access_token_encrypted)
        .bind(&input.refresh_token_encrypted)
        .bind(&input.scopes)
        .bind(input.expires_at)
        .bind(input.installation_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get an identity's connection for a specific provider
    pub async fn get_agent_identity_connection(
        &self,
        identity_id: AgentIdentityId,
        provider: &str,
    ) -> Result<Option<AgentIdentityConnectionRow>> {
        let row = sqlx::query_as::<_, AgentIdentityConnectionRow>(
            r#"
            SELECT id, agent_identity_id, provider, connection_type, provider_user_id, provider_username, access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, created_at, updated_at
            FROM agent_identity_connections
            WHERE agent_identity_id = $1 AND provider = $2
            LIMIT 1
            "#,
        )
        .bind(identity_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List all connections for an agent identity
    pub async fn list_agent_identity_connections(
        &self,
        identity_id: AgentIdentityId,
    ) -> Result<Vec<AgentIdentityConnectionRow>> {
        let rows = sqlx::query_as::<_, AgentIdentityConnectionRow>(
            r#"
            SELECT id, agent_identity_id, provider, connection_type, provider_user_id, provider_username, access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, created_at, updated_at
            FROM agent_identity_connections
            WHERE agent_identity_id = $1
            ORDER BY provider ASC
            "#,
        )
        .bind(identity_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Delete an identity's connection for a specific provider
    pub async fn delete_agent_identity_connection(
        &self,
        identity_id: AgentIdentityId,
        provider: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM agent_identity_connections WHERE agent_identity_id = $1 AND provider = $2",
        )
        .bind(identity_id)
        .bind(provider)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
