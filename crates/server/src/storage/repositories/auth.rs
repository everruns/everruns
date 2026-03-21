// PostgreSQL repository: API Keys, Refresh Tokens, CLI Auth Sessions

use super::super::models::*;
use super::Database;
use anyhow::Result;
use uuid::Uuid;

impl Database {
    // ============================================
    // API Keys
    // ============================================

    pub async fn create_api_key(&self, input: CreateApiKeyRow) -> Result<ApiKeyRow> {
        let scopes_json = serde_json::to_value(&input.scopes)?;

        let row = sqlx::query_as::<_, ApiKeyRow>(
            r#"
            INSERT INTO api_keys (user_id, name, key_hash, key_prefix, scopes, expires_at, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, org_id, user_id, name, key_hash, key_prefix, scopes, expires_at, last_used_at, created_at, metadata
            "#,
        )
        .bind(input.user_id)
        .bind(&input.name)
        .bind(&input.key_hash)
        .bind(&input.key_prefix)
        .bind(&scopes_json)
        .bind(input.expires_at)
        .bind(&input.metadata)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKeyRow>> {
        let row = sqlx::query_as::<_, ApiKeyRow>(
            r#"
            SELECT id, org_id, user_id, name, key_hash, key_prefix, scopes, expires_at, last_used_at, created_at, metadata
            FROM api_keys
            WHERE key_hash = $1
            "#,
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_api_keys_for_user(&self, user_id: Uuid) -> Result<Vec<ApiKeyRow>> {
        let rows = sqlx::query_as::<_, ApiKeyRow>(
            r#"
            SELECT id, org_id, user_id, name, key_hash, key_prefix, scopes, expires_at, last_used_at, created_at, metadata
            FROM api_keys
            WHERE user_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_api_key_last_used(&self, id: Uuid) -> Result<()> {
        sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn delete_api_key(&self, id: Uuid, user_id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM api_keys WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Refresh Tokens
    // ============================================

    pub async fn create_refresh_token(
        &self,
        input: CreateRefreshTokenRow,
    ) -> Result<RefreshTokenRow> {
        let row = sqlx::query_as::<_, RefreshTokenRow>(
            r#"
            INSERT INTO refresh_tokens (user_id, token_hash, expires_at)
            VALUES ($1, $2, $3)
            RETURNING id, user_id, token_hash, expires_at, created_at
            "#,
        )
        .bind(input.user_id)
        .bind(&input.token_hash)
        .bind(input.expires_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRow>> {
        let row = sqlx::query_as::<_, RefreshTokenRow>(
            r#"
            SELECT id, user_id, token_hash, expires_at, created_at
            FROM refresh_tokens
            WHERE token_hash = $1
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_refresh_token(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM refresh_tokens WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_expired_refresh_tokens(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM refresh_tokens WHERE expires_at < NOW()")
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    pub async fn delete_user_refresh_tokens(&self, user_id: Uuid) -> Result<u64> {
        let result = sqlx::query("DELETE FROM refresh_tokens WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    // ============================================
    // CLI Auth Sessions
    // ============================================

    pub async fn create_cli_auth_session(
        &self,
        input: CreateCliAuthSessionRow,
    ) -> Result<CliAuthSessionRow> {
        let row = sqlx::query_as::<_, CliAuthSessionRow>(
            r#"
            INSERT INTO cli_auth_sessions (state, exchange_code, redirect_port, expires_at)
            VALUES ($1, $2, $3, $4)
            RETURNING id, state, exchange_code, user_id, redirect_port, completed, expires_at, created_at
            "#,
        )
        .bind(&input.state)
        .bind(&input.exchange_code)
        .bind(input.redirect_port)
        .bind(input.expires_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_cli_auth_session_by_state(
        &self,
        state: &str,
    ) -> Result<Option<CliAuthSessionRow>> {
        let row = sqlx::query_as::<_, CliAuthSessionRow>(
            r#"
            SELECT id, state, exchange_code, user_id, redirect_port, completed, expires_at, created_at
            FROM cli_auth_sessions
            WHERE state = $1 AND expires_at > NOW()
            "#,
        )
        .bind(state)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_cli_auth_session_by_exchange_code(
        &self,
        code: &str,
    ) -> Result<Option<CliAuthSessionRow>> {
        let row = sqlx::query_as::<_, CliAuthSessionRow>(
            r#"
            SELECT id, state, exchange_code, user_id, redirect_port, completed, expires_at, created_at
            FROM cli_auth_sessions
            WHERE exchange_code = $1 AND expires_at > NOW()
            "#,
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn complete_cli_auth_session(&self, id: Uuid, user_id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE cli_auth_sessions SET user_id = $2, completed = TRUE WHERE id = $1 AND completed = FALSE",
        )
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_expired_cli_auth_sessions(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM cli_auth_sessions WHERE expires_at < NOW()")
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }
}
