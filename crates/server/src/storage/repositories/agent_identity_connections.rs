// PostgreSQL repository: Agent Identity Connections

use super::super::models::*;
use super::Database;
use anyhow::Result;
use everruns_provider::typed_id::{AgentId, AgentIdentityId};

impl Database {
    // ============================================
    // Agent Identity Connections
    // ============================================

    /// Create or replace an identity connection for a provider.
    /// Uses ON CONFLICT to atomically upsert, avoiding data loss from a
    /// DELETE+INSERT race if the INSERT were to fail after the DELETE.
    pub async fn upsert_agent_identity_connection(
        &self,
        input: CreateAgentIdentityConnectionRow,
    ) -> Result<AgentIdentityConnectionRow> {
        let row = sqlx::query_as::<_, AgentIdentityConnectionRow>(
            r#"
            INSERT INTO agent_identity_connections
                (agent_identity_id, provider, connection_type, provider_user_id, provider_username,
                 access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, provider_metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (agent_identity_id, provider) DO UPDATE SET
                connection_type = EXCLUDED.connection_type,
                provider_user_id = EXCLUDED.provider_user_id,
                provider_username = EXCLUDED.provider_username,
                access_token_encrypted = EXCLUDED.access_token_encrypted,
                refresh_token_encrypted = EXCLUDED.refresh_token_encrypted,
                scopes = EXCLUDED.scopes,
                expires_at = EXCLUDED.expires_at,
                installation_id = EXCLUDED.installation_id,
                provider_metadata = EXCLUDED.provider_metadata,
                updated_at = NOW()
            RETURNING id, agent_identity_id, provider, connection_type, provider_user_id,
                      provider_username, access_token_encrypted, refresh_token_encrypted,
                      scopes, expires_at, installation_id, provider_metadata, created_at, updated_at
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
        .bind(&input.provider_metadata)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn upsert_agent_identity_connection_for_active_agent(
        &self,
        org_id: i64,
        agent_id: AgentId,
        input: CreateAgentIdentityConnectionRow,
    ) -> Result<Option<AgentIdentityConnectionRow>> {
        let row = sqlx::query_as::<_, AgentIdentityConnectionRow>(
            r#"
            WITH eligible AS (
                SELECT a.agent_identity_id
                FROM agents AS a
                JOIN agent_identities AS i ON i.id = a.agent_identity_id
                WHERE a.org_id = $1
                  AND a.id = $2
                  AND a.status = 'active'
                  AND a.agent_identity_id = $3
                  AND i.org_id = $1
                  AND i.status = 'active'
                FOR UPDATE OF a, i
            )
            INSERT INTO agent_identity_connections
                (agent_identity_id, provider, connection_type, provider_user_id, provider_username,
                 access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, provider_metadata)
            SELECT agent_identity_id, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13
            FROM eligible
            ON CONFLICT (agent_identity_id, provider) DO UPDATE SET
                connection_type = EXCLUDED.connection_type,
                provider_user_id = EXCLUDED.provider_user_id,
                provider_username = EXCLUDED.provider_username,
                access_token_encrypted = EXCLUDED.access_token_encrypted,
                refresh_token_encrypted = EXCLUDED.refresh_token_encrypted,
                scopes = EXCLUDED.scopes,
                expires_at = EXCLUDED.expires_at,
                installation_id = EXCLUDED.installation_id,
                provider_metadata = EXCLUDED.provider_metadata,
                updated_at = NOW()
            RETURNING id, agent_identity_id, provider, connection_type, provider_user_id,
                      provider_username, access_token_encrypted, refresh_token_encrypted,
                      scopes, expires_at, installation_id, provider_metadata, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(agent_id)
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
        .bind(&input.provider_metadata)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // THREAT[TM-TENANT-012]: the connection accessors and mutators below
    // (`get_agent_identity_connection`, `list_agent_identity_connections`,
    // `update_agent_identity_connection_oauth_tokens`,
    // `delete_all_agent_identity_connections`,
    // `delete_agent_identity_connection`, and
    // `invalidate_mcp_service_connection_if_access_token_matches`) are scoped
    // only by `agent_identity_id` and return/mutate `access_token_encrypted` /
    // `refresh_token_encrypted` (OAuth secrets). `agent_identity_connections`
    // has no `org_id` column, so these methods cannot self-enforce tenant
    // isolation. Every caller MUST derive the identity or connection from an
    // org-scoped agent, identity, or session first. Do not call these with an
    // `AgentIdentityId` or connection id that was not org-validated.

    /// Get an identity's connection for a specific provider
    pub async fn get_agent_identity_connection(
        &self,
        identity_id: AgentIdentityId,
        provider: &str,
    ) -> Result<Option<AgentIdentityConnectionRow>> {
        let row = sqlx::query_as::<_, AgentIdentityConnectionRow>(
            r#"
            SELECT id, agent_identity_id, provider, connection_type, provider_user_id, provider_username, access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, provider_metadata, created_at, updated_at
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
            SELECT id, agent_identity_id, provider, connection_type, provider_user_id, provider_username, access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, provider_metadata, created_at, updated_at
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

    pub async fn update_agent_identity_connection_oauth_tokens(
        &self,
        input: UpdateOAuthConnectionTokens,
    ) -> Result<Option<AgentIdentityConnectionRow>> {
        let row = sqlx::query_as::<_, AgentIdentityConnectionRow>(
            r#"
            UPDATE agent_identity_connections
            SET access_token_encrypted = $2,
                refresh_token_encrypted = $3,
                expires_at = $4,
                scopes = COALESCE($5, scopes),
                updated_at = NOW()
            WHERE id = $1 AND connection_type = 'oauth'
            RETURNING id, agent_identity_id, provider, connection_type, provider_user_id,
                      provider_username, access_token_encrypted, refresh_token_encrypted,
                      scopes, expires_at, installation_id, provider_metadata, created_at, updated_at
            "#,
        )
        .bind(input.connection_id)
        .bind(input.access_token_encrypted)
        .bind(input.refresh_token_encrypted)
        .bind(input.expires_at)
        .bind(input.scopes)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn delete_all_agent_identity_connections(
        &self,
        identity_id: AgentIdentityId,
    ) -> Result<u64> {
        let result =
            sqlx::query("DELETE FROM agent_identity_connections WHERE agent_identity_id = $1")
                .bind(identity_id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected())
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

    pub async fn invalidate_mcp_service_connection_if_access_token_matches(
        &self,
        identity_id: AgentIdentityId,
        provider: &str,
        expected_access_token_encrypted: &[u8],
        org_id: i64,
        mcp_server_id: uuid::Uuid,
        agent_id: uuid::Uuid,
    ) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            r#"
            DELETE FROM agent_identity_connections
            WHERE agent_identity_id = $1
              AND provider = $2
              AND access_token_encrypted = $3
            "#,
        )
        .bind(identity_id)
        .bind(provider)
        .bind(expected_access_token_encrypted)
        .execute(&mut *transaction)
        .await?;
        let deleted = result.rows_affected() > 0;
        if deleted {
            sqlx::query(
                r#"
                DELETE FROM mcp_service_tool_caches
                WHERE org_id = $1 AND mcp_server_id = $2 AND agent_id = $3
                "#,
            )
            .bind(org_id)
            .bind(mcp_server_id)
            .bind(agent_id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;

        Ok(deleted)
    }
}
