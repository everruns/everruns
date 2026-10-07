// PostgreSQL repository: Virtual User Connections

use super::super::models::*;
use super::Database;
use anyhow::Result;
use everruns_contracts::typed_id::{AgentId, VirtualUserId};
use everruns_server_macros::sql;

#[derive(Debug, thiserror::Error)]
#[error("organization connection is assigned to a Sandbox Template or Session")]
pub struct OrganizationConnectionInUse;

impl Database {
    // ============================================
    // Virtual User Connections
    // ============================================

    /// Create or replace an identity connection for a provider.
    /// Uses ON CONFLICT to atomically upsert, avoiding data loss from a
    /// DELETE+INSERT race if the INSERT were to fail after the DELETE.
    pub async fn upsert_virtual_user_connection(
        &self,
        input: CreateVirtualUserConnectionRow,
    ) -> Result<VirtualUserConnectionRow> {
        let row = sqlx::query_as::<_, VirtualUserConnectionRow>(
            sql!(r#"
            INSERT INTO virtual_user_connections
                (virtual_user_id, provider, connection_type, provider_user_id, provider_username,
                 access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, provider_metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (virtual_user_id, provider) DO UPDATE SET
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
            RETURNING {VirtualUserConnectionRow}
            "#),
        )
        .bind(input.virtual_user_id)
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

        sqlx::query("UPDATE leased_resources SET pending_connection_id=NULL,metadata=metadata-'connection_migration_pending' WHERE owner_user_id=$1 AND provider=$2").bind(row.virtual_user_id).bind(&row.provider).execute(&self.pool).await?;
        Ok(row)
    }

    pub async fn upsert_virtual_user_connection_for_active_agent(
        &self,
        org_id: i64,
        agent_id: AgentId,
        input: CreateVirtualUserConnectionRow,
    ) -> Result<Option<VirtualUserConnectionRow>> {
        let row = sqlx::query_as::<_, VirtualUserConnectionRow>(
            sql!(r#"
            WITH eligible AS (
                SELECT a.virtual_user_id
                FROM agents AS a
                JOIN virtual_users AS i ON i.id = a.virtual_user_id
                WHERE a.org_id = $1
                  AND a.id = $2
                  AND a.status = 'active'
                  AND a.virtual_user_id = $3
                  AND i.org_id = $1
                  AND i.status = 'active'
                  AND i.usage = 'service'
                FOR UPDATE OF a, i
            )
            INSERT INTO virtual_user_connections
                (virtual_user_id, provider, connection_type, provider_user_id, provider_username,
                 access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, provider_metadata)
            SELECT virtual_user_id, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13
            FROM eligible
            ON CONFLICT (virtual_user_id, provider) DO UPDATE SET
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
            RETURNING {VirtualUserConnectionRow}
            "#),
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(input.virtual_user_id)
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
    // (`get_virtual_user_connection`, `list_virtual_user_connections`,
    // `update_virtual_user_connection_oauth_tokens`,
    // `delete_all_virtual_user_connections`,
    // `delete_virtual_user_connection`, and
    // `invalidate_mcp_service_connection_if_access_token_matches`) are scoped
    // only by `virtual_user_id` and return/mutate `access_token_encrypted` /
    // `refresh_token_encrypted` (OAuth secrets). `virtual_user_connections`
    // has no `org_id` column, so these methods cannot self-enforce tenant
    // isolation. Every caller MUST derive the identity or connection from an
    // org-scoped agent, identity, or session first. Do not call these with an
    // `VirtualUserId` or connection id that was not org-validated.

    /// Get an identity's connection for a specific provider
    pub async fn get_virtual_user_connection(
        &self,
        identity_id: VirtualUserId,
        provider: &str,
    ) -> Result<Option<VirtualUserConnectionRow>> {
        let row = sqlx::query_as::<_, VirtualUserConnectionRow>(sql!(
            r#"
            SELECT {VirtualUserConnectionRow}
            FROM virtual_user_connections
            WHERE virtual_user_id = $1 AND provider = $2
            LIMIT 1
            "#
        ))
        .bind(identity_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Resolve one exact connection after the caller has already selected and
    /// tenant-validated its virtual-user owner.
    pub async fn get_virtual_user_connection_by_id(
        &self,
        identity_id: VirtualUserId,
        connection_id: uuid::Uuid,
        provider: &str,
    ) -> Result<Option<VirtualUserConnectionRow>> {
        let row = sqlx::query_as::<_, VirtualUserConnectionRow>(sql!(
            r#"
            SELECT {VirtualUserConnectionRow}
            FROM virtual_user_connections
            WHERE virtual_user_id = $1 AND id = $2 AND provider = $3
            LIMIT 1
            "#
        ))
        .bind(identity_id)
        .bind(connection_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Create an organization-owned account in the canonical encrypted
    /// connection store. A dedicated hidden virtual user preserves the
    /// existing one-account-per-provider invariant for ordinary identities.
    pub async fn create_organization_connection(
        &self,
        input: CreateOrganizationConnectionRow,
    ) -> Result<VirtualUserConnectionRow> {
        let mut tx = self.pool.begin().await?;
        let virtual_user_id = VirtualUserId::from_uuid(uuid::Uuid::new_v4());
        sqlx::query(
            r#"INSERT INTO virtual_users
                (org_id, id, name, description, status, usage)
               VALUES ($1, $2, $3, 'Hidden owner for an organization connection', 'active', 'organization')"#,
        )
        .bind(input.org_id)
        .bind(virtual_user_id)
        .bind(format!("Sandbox account: {}", input.name))
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query_as::<_, VirtualUserConnectionRow>(sql!(
            r#"
            INSERT INTO virtual_user_connections
                (virtual_user_id, provider, connection_type, access_token_encrypted,
                 owner_scope, name, provider_username, provider_metadata)
            VALUES ($1, $2, 'api_key', $3, 'organization', $4, $5, $6)
            RETURNING {VirtualUserConnectionRow}
        "#
        ))
        .bind(virtual_user_id)
        .bind(input.provider)
        .bind(input.access_token_encrypted)
        .bind(input.name)
        .bind(input.provider_username)
        .bind(input.provider_metadata)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn list_organization_connections(
        &self,
        org_id: i64,
    ) -> Result<Vec<VirtualUserConnectionRow>> {
        Ok(sqlx::query_as::<_, VirtualUserConnectionRow>(sql!(
            r#"
            SELECT {VirtualUserConnectionRow}
            FROM virtual_user_connections
            WHERE virtual_user_id IN (
                SELECT id FROM virtual_users WHERE org_id = $1 AND usage = 'organization'
            ) AND owner_scope = 'organization'
            ORDER BY lower(name), created_at
        "#
        ))
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_organization_connection(
        &self,
        org_id: i64,
        connection_id: uuid::Uuid,
    ) -> Result<Option<VirtualUserConnectionRow>> {
        Ok(sqlx::query_as::<_, VirtualUserConnectionRow>(sql!(
            r#"
            SELECT {VirtualUserConnectionRow}
            FROM virtual_user_connections
            WHERE virtual_user_id IN (
                SELECT id FROM virtual_users WHERE org_id = $1 AND usage = 'organization'
            ) AND owner_scope = 'organization' AND id = $2
            LIMIT 1
        "#
        ))
        .bind(org_id)
        .bind(connection_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn update_organization_connection(
        &self,
        org_id: i64,
        connection_id: uuid::Uuid,
        name: &str,
        access_token_encrypted: &[u8],
        provider_username: Option<&str>,
        provider_metadata: Option<&serde_json::Value>,
    ) -> Result<Option<VirtualUserConnectionRow>> {
        Ok(sqlx::query_as::<_, VirtualUserConnectionRow>(sql!(
            r#"
            UPDATE virtual_user_connections c
            SET name = $3, access_token_encrypted = $4, provider_username = $5,
                provider_metadata = $6, updated_at = NOW()
            WHERE c.virtual_user_id IN (
                SELECT id FROM virtual_users WHERE org_id = $1 AND usage = 'organization'
            ) AND c.owner_scope = 'organization' AND c.id = $2
            RETURNING {VirtualUserConnectionRow}
        "#
        ))
        .bind(org_id)
        .bind(connection_id)
        .bind(name)
        .bind(access_token_encrypted)
        .bind(provider_username)
        .bind(provider_metadata)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn delete_organization_connection(
        &self,
        org_id: i64,
        connection_id: uuid::Uuid,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let owner: Option<(VirtualUserId,)> = sqlx::query_as(
            r#"SELECT c.virtual_user_id FROM virtual_user_connections c
               JOIN virtual_users v ON v.id = c.virtual_user_id
               WHERE c.id = $1 AND v.org_id = $2 AND v.usage = 'organization'
                 AND c.owner_scope = 'organization' FOR UPDATE"#,
        )
        .bind(connection_id)
        .bind(org_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((owner,)) = owner else {
            return Ok(false);
        };
        let referenced: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                SELECT 1
                FROM execution_environments e
                JOIN execution_environment_revisions r ON r.id = e.current_revision_id
                WHERE e.status = 'active'
                  AND r.profile #>> '{target,credential,connection_id}' = $1
                UNION ALL
                SELECT 1 FROM sandboxes
                WHERE desired_state <> 'deleted'
                  AND profile_snapshot #>> '{target,credential,connection_id}' = $1
                UNION ALL
                SELECT 1 FROM agents
                WHERE status = 'active' AND environments::text LIKE '%' || $1 || '%'
                UNION ALL
                SELECT 1 FROM leased_resources
                WHERE connection_id = $2 AND status <> 'released'
            )"#,
        )
        .bind(connection_id.to_string())
        .bind(connection_id)
        .fetch_one(&mut *tx)
        .await?;
        if referenced {
            return Err(OrganizationConnectionInUse.into());
        }
        sqlx::query("DELETE FROM virtual_user_connections WHERE id = $1")
            .bind(connection_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM virtual_users WHERE id = $1")
            .bind(owner)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// List all connections for an virtual user
    pub async fn list_virtual_user_connections(
        &self,
        identity_id: VirtualUserId,
    ) -> Result<Vec<VirtualUserConnectionRow>> {
        let rows = sqlx::query_as::<_, VirtualUserConnectionRow>(sql!(
            r#"
            SELECT {VirtualUserConnectionRow}
            FROM virtual_user_connections
            WHERE virtual_user_id = $1
            ORDER BY provider ASC
            "#
        ))
        .bind(identity_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_virtual_user_connection_oauth_tokens(
        &self,
        input: UpdateOAuthConnectionTokens,
    ) -> Result<Option<VirtualUserConnectionRow>> {
        let row = sqlx::query_as::<_, VirtualUserConnectionRow>(sql!(
            r#"
            UPDATE virtual_user_connections
            SET access_token_encrypted = $2,
                refresh_token_encrypted = $3,
                expires_at = $4,
                scopes = COALESCE($5, scopes),
                updated_at = NOW()
            WHERE id = $1 AND connection_type = 'oauth'
            RETURNING {VirtualUserConnectionRow}
            "#
        ))
        .bind(input.connection_id)
        .bind(input.access_token_encrypted)
        .bind(input.refresh_token_encrypted)
        .bind(input.expires_at)
        .bind(input.scopes)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn delete_all_virtual_user_connections(
        &self,
        identity_id: VirtualUserId,
    ) -> Result<u64> {
        let result = sqlx::query("DELETE FROM virtual_user_connections WHERE virtual_user_id = $1")
            .bind(identity_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Delete an identity's connection for a specific provider
    pub async fn delete_virtual_user_connection(
        &self,
        identity_id: VirtualUserId,
        provider: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM virtual_user_connections WHERE virtual_user_id = $1 AND provider = $2",
        )
        .bind(identity_id)
        .bind(provider)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn invalidate_mcp_service_connection_if_access_token_matches(
        &self,
        identity_id: VirtualUserId,
        provider: &str,
        expected_access_token_encrypted: &[u8],
        org_id: i64,
        mcp_server_id: uuid::Uuid,
        agent_id: uuid::Uuid,
    ) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            r#"
            DELETE FROM virtual_user_connections
            WHERE virtual_user_id = $1
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
