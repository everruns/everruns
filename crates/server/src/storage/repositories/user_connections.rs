// PostgreSQL repository: User Connections

use super::super::models::*;
use super::Database;
use anyhow::Result;
use everruns_core::typed_id::SessionId;
use uuid::Uuid;

impl Database {
    // ============================================
    // User Connections
    // ============================================

    /// Create or replace a user connection for a provider.
    /// Deletes any existing connection for the same (user_id, provider) first.
    pub async fn upsert_user_connection(
        &self,
        input: CreateUserConnectionRow,
    ) -> Result<UserConnectionRow> {
        // App-level uniqueness: delete existing connection for this user+provider
        sqlx::query("DELETE FROM user_connections WHERE user_id = $1 AND provider = $2")
            .bind(input.user_id)
            .bind(&input.provider)
            .execute(&self.pool)
            .await?;

        let row = sqlx::query_as::<_, UserConnectionRow>(
            r#"
            INSERT INTO user_connections (user_id, provider, connection_type, provider_user_id, provider_username, access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id, user_id, provider, connection_type, provider_user_id, provider_username, access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, created_at, updated_at
            "#,
        )
        .bind(input.user_id)
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

    /// Get a user's connection for a specific provider
    pub async fn get_user_connection(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<UserConnectionRow>> {
        let row = sqlx::query_as::<_, UserConnectionRow>(
            r#"
            SELECT id, user_id, provider, connection_type, provider_user_id, provider_username, access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, created_at, updated_at
            FROM user_connections
            WHERE user_id = $1 AND provider = $2
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List all connections for a user
    pub async fn list_user_connections(&self, user_id: Uuid) -> Result<Vec<UserConnectionRow>> {
        let rows = sqlx::query_as::<_, UserConnectionRow>(
            r#"
            SELECT id, user_id, provider, connection_type, provider_user_id, provider_username, access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, created_at, updated_at
            FROM user_connections
            WHERE user_id = $1
            ORDER BY provider ASC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Get the encrypted connection token for a session.
    ///
    /// Resolution order:
    /// 1. If the session has an `agent_identity_id`, use `agent_identity_connections`.
    /// 2. Otherwise fall back to `user_connections` via org membership.
    ///
    /// Returns None for GitHub App connections (use get_installation_id_for_session instead).
    pub async fn get_connection_token_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Vec<u8>>> {
        let row: Option<(Option<Vec<u8>>,)> = sqlx::query_as(
            r#"
            WITH identity_conn AS (
                SELECT aic.access_token_encrypted
                FROM sessions s
                JOIN agent_identity_connections aic
                    ON aic.agent_identity_id = s.agent_identity_id AND aic.provider = $2
                WHERE s.id = $1
                  AND s.agent_identity_id IS NOT NULL
                  AND aic.access_token_encrypted IS NOT NULL
                LIMIT 1
            ),
            user_conn AS (
                SELECT uc.access_token_encrypted
                FROM sessions s
                JOIN organization_members om ON om.org_id = s.org_id
                JOIN user_connections uc ON uc.user_id = om.user_id AND uc.provider = $2
                WHERE s.id = $1
                  AND uc.access_token_encrypted IS NOT NULL
                  AND NOT EXISTS (SELECT 1 FROM identity_conn)
                LIMIT 1
            )
            SELECT access_token_encrypted FROM identity_conn
            UNION ALL
            SELECT access_token_encrypted FROM user_conn
            LIMIT 1
            "#,
        )
        .bind(session_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|(blob,)| blob))
    }

    /// Resolve the user whose connection would be used for a session/provider pair.
    ///
    /// Returns None when the session uses an agent identity connection (identity
    /// connections are not owned by a user).
    pub async fn get_connection_user_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Uuid>> {
        // If session uses an agent identity with a matching connection, there is
        // no owning user — return None so callers know the credential is identity-owned.
        let has_identity_conn: Option<(i32,)> = sqlx::query_as(
            r#"
            SELECT 1 as v
            FROM sessions s
            JOIN agent_identity_connections aic
                ON aic.agent_identity_id = s.agent_identity_id AND aic.provider = $2
            WHERE s.id = $1
              AND s.agent_identity_id IS NOT NULL
            LIMIT 1
            "#,
        )
        .bind(session_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        if has_identity_conn.is_some() {
            return Ok(None);
        }

        let row: Option<(Uuid,)> = sqlx::query_as(
            r#"
            SELECT uc.user_id
            FROM sessions s
            JOIN organization_members om ON om.org_id = s.org_id
            JOIN user_connections uc ON uc.user_id = om.user_id AND uc.provider = $2
            WHERE s.id = $1
            ORDER BY uc.created_at ASC
            LIMIT 1
            "#,
        )
        .bind(session_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(user_id,)| user_id))
    }

    /// Get the encrypted connection token for a specific user/provider pair.
    pub async fn get_connection_token_for_user(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<Vec<u8>>> {
        let row: Option<(Option<Vec<u8>>,)> = sqlx::query_as(
            r#"
            SELECT access_token_encrypted
            FROM user_connections
            WHERE user_id = $1 AND provider = $2
              AND access_token_encrypted IS NOT NULL
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|(blob,)| blob))
    }

    /// Get the GitHub App installation ID for a session.
    ///
    /// Resolution order:
    /// 1. If the session has an `agent_identity_id`, use `agent_identity_connections`.
    /// 2. Otherwise fall back to `user_connections` via org membership.
    pub async fn get_installation_id_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<i64>> {
        let row: Option<(i64,)> = sqlx::query_as(
            r#"
            WITH identity_conn AS (
                SELECT aic.installation_id
                FROM sessions s
                JOIN agent_identity_connections aic
                    ON aic.agent_identity_id = s.agent_identity_id AND aic.provider = $2
                WHERE s.id = $1
                  AND s.agent_identity_id IS NOT NULL
                  AND aic.installation_id IS NOT NULL
                LIMIT 1
            ),
            user_conn AS (
                SELECT uc.installation_id
                FROM sessions s
                JOIN organization_members om ON om.org_id = s.org_id
                JOIN user_connections uc ON uc.user_id = om.user_id AND uc.provider = $2
                WHERE s.id = $1
                  AND uc.installation_id IS NOT NULL
                  AND NOT EXISTS (SELECT 1 FROM identity_conn)
                LIMIT 1
            )
            SELECT installation_id FROM identity_conn
            UNION ALL
            SELECT installation_id FROM user_conn
            LIMIT 1
            "#,
        )
        .bind(session_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(id,)| id))
    }

    /// Get the GitHub App installation ID for a specific user/provider pair.
    pub async fn get_installation_id_for_user(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<i64>> {
        let row: Option<(i64,)> = sqlx::query_as(
            r#"
            SELECT installation_id
            FROM user_connections
            WHERE user_id = $1 AND provider = $2
              AND installation_id IS NOT NULL
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(id,)| id))
    }

    /// Delete a user's connection for a specific provider
    pub async fn delete_user_connection(&self, user_id: Uuid, provider: &str) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM user_connections WHERE user_id = $1 AND provider = $2")
                .bind(user_id)
                .bind(provider)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() > 0)
    }
}
