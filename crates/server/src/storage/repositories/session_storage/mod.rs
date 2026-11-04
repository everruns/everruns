// PostgreSQL repository: Session Key/Value Storage, Session Secret Storage (Encrypted)

pub(super) mod rows;
use rows::*;

use super::Database;
use anyhow::Result;
use everruns_server_macros::sql;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct McpOAuthSessionCredentialColumns {
    virtual_user_id: Option<everruns_contracts::typed_id::VirtualUserId>,
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
        let row = sqlx::query_as::<_, SessionKeyValueRow>(sql!(
            r#"
            INSERT INTO session_key_values (session_id, key, value)
            VALUES ($1, $2, $3)
            ON CONFLICT (session_id, key) DO UPDATE
            SET value = EXCLUDED.value, updated_at = NOW()
            RETURNING {SessionKeyValueRow}
            "#
        ))
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
        let row = sqlx::query_as::<_, SessionKeyValueRow>(sql!(
            r#"
            SELECT {SessionKeyValueRow}
            FROM session_key_values
            WHERE session_id = $1 AND key = $2
            "#
        ))
        .bind(session_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List all keys for a session (without values)
    pub async fn list_session_keys(&self, session_id: Uuid) -> Result<Vec<SessionKeyInfoRow>> {
        let rows = sqlx::query_as::<_, SessionKeyInfoRow>(sql!(
            r#"
            SELECT {SessionKeyInfoRow}
            FROM session_key_values
            WHERE session_id = $1
            ORDER BY key
            "#
        ))
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

    /// Atomically remove a session key/value and return its value.
    pub async fn take_session_key_value(
        &self,
        session_id: Uuid,
        key: &str,
    ) -> Result<Option<String>> {
        sqlx::query_scalar(
            r#"
            DELETE FROM session_key_values
            WHERE session_id = $1 AND key = $2
            RETURNING value
            "#,
        )
        .bind(session_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    // ============================================
    // Session Secret Storage (Encrypted)
    // ============================================

    /// Upsert a session secret (insert or update)
    pub async fn upsert_session_secret(
        &self,
        input: UpsertSessionSecret,
    ) -> Result<SessionSecretRow> {
        if let Some(binding) = &input.binding {
            let mut tx = self.pool.begin().await?;
            let id:Option<Uuid>=sqlx::query_scalar("SELECT c.id FROM credential_bindings c JOIN resource_access r ON r.id=c.id WHERE r.owner_session_id=$1 AND r.usage='service' AND r.subject_id IS NULL AND c.env_name=$2 FOR UPDATE OF r").bind(input.session_id).bind(&input.name).fetch_optional(&mut *tx).await?;
            let id = id.unwrap_or_else(Uuid::now_v7);
            sqlx::query("INSERT INTO resource_access(id,org_id,kind,scope,owner_session_id,usage) SELECT $1,org_id,'credential_binding','session',id,'service' FROM sessions WHERE id=$2 ON CONFLICT(id) DO NOTHING").bind(id).bind(input.session_id).execute(&mut *tx).await?;
            let row=sqlx::query_as::<_,SessionSecretRow>("INSERT INTO credential_bindings(id,org_id,kind,env_name,binding,label,value_encrypted) SELECT $1,org_id,'http',$3,$4,$3,$5 FROM sessions WHERE id=$2 ON CONFLICT(id) DO UPDATE SET binding=EXCLUDED.binding,value_encrypted=EXCLUDED.value_encrypted,updated_at=now() RETURNING id,$2 AS session_id,env_name AS name,value_encrypted,binding,created_at,updated_at")
            .bind(id).bind(input.session_id).bind(&input.name).bind(sqlx::types::Json(binding)).bind(&input.value_encrypted).fetch_one(&mut *tx).await?;
            sqlx::query("DELETE FROM session_secrets WHERE session_id=$1 AND name=$2")
                .bind(input.session_id)
                .bind(&input.name)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(row);
        }
        // Only the secure management adapter may replace a session-wide binding
        // with a newly supplied ordinary value. Runtime writers refuse bound names.
        sqlx::query("DELETE FROM credential_bindings c USING resource_access r WHERE c.id=r.id AND r.owner_session_id=$1 AND r.usage='service' AND r.subject_id IS NULL AND c.env_name=$2").bind(input.session_id).bind(&input.name).execute(&self.pool).await?;
        let row = sqlx::query_as::<_, SessionSecretRow>(
            r#"
            INSERT INTO session_secrets (session_id, name, value_encrypted, binding)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (session_id, name) DO UPDATE
            SET value_encrypted = EXCLUDED.value_encrypted, binding = EXCLUDED.binding,
                updated_at = NOW()
            RETURNING id, session_id, name, value_encrypted, binding, created_at, updated_at
            "#,
        )
        .bind(input.session_id)
        .bind(&input.name)
        .bind(&input.value_encrypted)
        .bind(input.binding.map(sqlx::types::Json))
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
            SELECT id, session_id, name, value_encrypted, binding, created_at, updated_at
            FROM session_secrets
            WHERE session_id = $1 AND name = $2
            UNION ALL SELECT c.id,$1 AS session_id,c.env_name AS name,c.value_encrypted,c.binding,c.created_at,c.updated_at FROM credential_bindings c JOIN resource_access r ON r.id=c.id WHERE r.owner_session_id=$1 AND r.usage='service' AND r.subject_id IS NULL AND c.env_name=$2 AND r.revoked_at IS NULL
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
        session_id: everruns_contracts::typed_id::SessionId,
        server_id: uuid::Uuid,
    ) -> Result<Option<McpOAuthSessionCredentialsRow>> {
        let access_name = everruns_core::mcp_oauth_session_secret_name(server_id, "access_token");
        let refresh_name = everruns_core::mcp_oauth_session_secret_name(server_id, "refresh_token");
        let expires_name = everruns_core::mcp_oauth_session_secret_name(server_id, "expires_at");
        let row = sqlx::query_as::<_, McpOAuthSessionCredentialColumns>(
            r#"
            SELECT
                (SELECT virtual_user_id FROM session_secrets WHERE session_id=$1 AND name=$2) AS virtual_user_id,
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
                virtual_user_id: row.virtual_user_id,
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
                    INSERT INTO session_secrets (session_id, name, value_encrypted,virtual_user_id)
                    VALUES ($1, $2, $3,$4)
                    ON CONFLICT (session_id, name) DO UPDATE
                    SET value_encrypted = EXCLUDED.value_encrypted, virtual_user_id=EXCLUDED.virtual_user_id, updated_at = NOW()
                    "#,
                )
                .bind(input.session_id)
                .bind(&name)
                .bind(value_encrypted)
                .bind(input.virtual_user_id)
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
        let rows = sqlx::query_as::<_, SessionSecretInfoRow>(sql!(
            r#"
            SELECT {SessionSecretInfoRow}
            FROM session_secrets
            WHERE session_id = $1
            UNION ALL SELECT c.env_name AS name,c.binding,c.created_at,c.updated_at FROM credential_bindings c JOIN resource_access r ON r.id=c.id WHERE r.owner_session_id=$1 AND r.usage='service' AND r.subject_id IS NULL AND r.revoked_at IS NULL
            ORDER BY name
            "#
        ))
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Metadata-only legacy adapter. Runtime resolution requires an invocation.
    pub async fn list_bound_secrets(&self, session_id: Uuid) -> Result<Vec<BoundSecretRow>> {
        Ok(sqlx::query_as("SELECT c.env_name AS name,c.binding,c.value_encrypted,r.scope FROM credential_bindings c JOIN resource_access r ON r.id=c.id JOIN sessions s ON s.org_id=r.org_id WHERE s.id=$1 AND c.kind='http' AND c.value_encrypted IS NOT NULL AND r.revoked_at IS NULL AND (r.owner_session_id=s.id OR r.owner_agent_id=s.agent_id)").bind(session_id).fetch_all(&self.pool).await?)
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
