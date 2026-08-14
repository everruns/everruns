// PostgreSQL repository: Session Key/Value Storage, Session Secret Storage (Encrypted)

use super::super::models::*;
use super::Database;
use anyhow::Result;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct McpOAuthSessionCredentialColumns {
    access_token_encrypted: Option<Vec<u8>>,
    refresh_token_encrypted: Option<Vec<u8>>,
    expires_at_encrypted: Option<Vec<u8>>,
}

impl Database {
    // ============================================
    // Session Key/Value Storage
    // ============================================

    /// Upsert a session key/value (insert or update)
    pub async fn upsert_session_key_value(
        &self,
        input: UpsertSessionKeyValue,
    ) -> Result<SessionKeyValueRow> {
        let row = sqlx::query_as::<_, SessionKeyValueRow>(
            r#"
            INSERT INTO session_key_values (session_id, key, value)
            VALUES ($1, $2, $3)
            ON CONFLICT (session_id, key) DO UPDATE
            SET value = EXCLUDED.value, updated_at = NOW()
            RETURNING id, session_id, key, value, created_at, updated_at
            "#,
        )
        .bind(input.session_id)
        .bind(&input.key)
        .bind(&input.value)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get a session key/value by key
    pub async fn get_session_key_value(
        &self,
        session_id: Uuid,
        key: &str,
    ) -> Result<Option<SessionKeyValueRow>> {
        let row = sqlx::query_as::<_, SessionKeyValueRow>(
            r#"
            SELECT id, session_id, key, value, created_at, updated_at
            FROM session_key_values
            WHERE session_id = $1 AND key = $2
            "#,
        )
        .bind(session_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List all keys for a session (without values)
    pub async fn list_session_keys(&self, session_id: Uuid) -> Result<Vec<SessionKeyInfoRow>> {
        let rows = sqlx::query_as::<_, SessionKeyInfoRow>(
            r#"
            SELECT key, created_at, updated_at
            FROM session_key_values
            WHERE session_id = $1
            ORDER BY key
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Delete a session key/value by key
    pub async fn delete_session_key_value(&self, session_id: Uuid, key: &str) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM session_key_values
            WHERE session_id = $1 AND key = $2
            "#,
        )
        .bind(session_id)
        .bind(key)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Session Secret Storage (Encrypted)
    // ============================================

    /// Upsert a session secret (insert or update)
    pub async fn upsert_session_secret(
        &self,
        input: UpsertSessionSecret,
    ) -> Result<SessionSecretRow> {
        let row = sqlx::query_as::<_, SessionSecretRow>(
            r#"
            INSERT INTO session_secrets (session_id, name, value_encrypted)
            VALUES ($1, $2, $3)
            ON CONFLICT (session_id, name) DO UPDATE
            SET value_encrypted = EXCLUDED.value_encrypted, updated_at = NOW()
            RETURNING id, session_id, name, value_encrypted, created_at, updated_at
            "#,
        )
        .bind(input.session_id)
        .bind(&input.name)
        .bind(&input.value_encrypted)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get a session secret by name
    pub async fn get_session_secret(
        &self,
        session_id: Uuid,
        name: &str,
    ) -> Result<Option<SessionSecretRow>> {
        let row = sqlx::query_as::<_, SessionSecretRow>(
            r#"
            SELECT id, session_id, name, value_encrypted, created_at, updated_at
            FROM session_secrets
            WHERE session_id = $1 AND name = $2
            "#,
        )
        .bind(session_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Read a session-scoped MCP OAuth grant as one database snapshot.
    pub async fn get_mcp_oauth_session_credentials(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        server_id: uuid::Uuid,
    ) -> Result<Option<McpOAuthSessionCredentialsRow>> {
        let access_name = everruns_core::mcp_oauth_session_secret_name(server_id, "access_token");
        let refresh_name = everruns_core::mcp_oauth_session_secret_name(server_id, "refresh_token");
        let expires_name = everruns_core::mcp_oauth_session_secret_name(server_id, "expires_at");
        let row = sqlx::query_as::<_, McpOAuthSessionCredentialColumns>(
            r#"
            SELECT
                (SELECT value_encrypted FROM session_secrets WHERE session_id = $1 AND name = $2)
                    AS access_token_encrypted,
                (SELECT value_encrypted FROM session_secrets WHERE session_id = $1 AND name = $3)
                    AS refresh_token_encrypted,
                (SELECT value_encrypted FROM session_secrets WHERE session_id = $1 AND name = $4)
                    AS expires_at_encrypted
            "#,
        )
        .bind(session_id)
        .bind(access_name)
        .bind(refresh_name)
        .bind(expires_name)
        .fetch_one(&self.pool)
        .await?;

        Ok(row
            .access_token_encrypted
            .map(|access_token_encrypted| McpOAuthSessionCredentialsRow {
                access_token_encrypted,
                refresh_token_encrypted: row.refresh_token_encrypted,
                expires_at_encrypted: row.expires_at_encrypted,
            }))
    }

    /// Atomically replace all fields of a session-scoped MCP OAuth grant.
    pub async fn upsert_mcp_oauth_session_credentials(
        &self,
        input: UpsertMcpOAuthSessionCredentials,
    ) -> Result<()> {
        let access_name =
            everruns_core::mcp_oauth_session_secret_name(input.server_id, "access_token");
        let refresh_name =
            everruns_core::mcp_oauth_session_secret_name(input.server_id, "refresh_token");
        let expires_name =
            everruns_core::mcp_oauth_session_secret_name(input.server_id, "expires_at");
        let mut tx = self.pool.begin().await?;

        for (name, value) in [
            (access_name, Some(input.access_token_encrypted)),
            (refresh_name, input.refresh_token_encrypted),
            (expires_name, input.expires_at_encrypted),
        ] {
            if let Some(value_encrypted) = value {
                sqlx::query(
                    r#"
                    INSERT INTO session_secrets (session_id, name, value_encrypted)
                    VALUES ($1, $2, $3)
                    ON CONFLICT (session_id, name) DO UPDATE
                    SET value_encrypted = EXCLUDED.value_encrypted, updated_at = NOW()
                    "#,
                )
                .bind(input.session_id)
                .bind(&name)
                .bind(value_encrypted)
                .execute(&mut *tx)
                .await?;
            } else {
                sqlx::query("DELETE FROM session_secrets WHERE session_id = $1 AND name = $2")
                    .bind(input.session_id)
                    .bind(&name)
                    .execute(&mut *tx)
                    .await?;
            }
        }

        tx.commit().await?;
        Ok(())
    }

    /// List all secret names for a session (without encrypted values)
    pub async fn list_session_secrets(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionSecretInfoRow>> {
        let rows = sqlx::query_as::<_, SessionSecretInfoRow>(
            r#"
            SELECT name, created_at, updated_at
            FROM session_secrets
            WHERE session_id = $1
            ORDER BY name
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Delete a session secret by name
    pub async fn delete_session_secret(&self, session_id: Uuid, name: &str) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM session_secrets
            WHERE session_id = $1 AND name = $2
            "#,
        )
        .bind(session_id)
        .bind(name)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
