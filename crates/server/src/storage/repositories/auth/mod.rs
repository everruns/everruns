// PostgreSQL repository: Personal Access Tokens, Refresh Tokens, CLI Auth Sessions

pub(super) mod rows;
use rows::*;

use super::Database;
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_server_macros::sql;
use uuid::Uuid;

/// Seconds a rotated MCP OAuth refresh token may be presented once more.
///
/// Decision: strict single-use rotation turned a lost token response, or two
/// sessions of one MCP host refreshing together, into `invalid_grant` and a
/// full reconnect, and access tokens here live 15 minutes, so refreshes are
/// frequent. One replay inside this window gets a fresh pair; a later replay is
/// treated as a leaked token and deletes the token's whole rotation family
/// (OAuth 2.0 Security BCP, RFC 9700 §4.14.2). The added exposure is one more
/// redemption of an already-used token for at most a minute; an unused stolen
/// token was always redeemable.
pub const REFRESH_REUSE_GRACE_SECS: i64 = 60;

/// What a replay of an already-rotated refresh token means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpentRefresh {
    /// Inside the window and the replay is unused: mint once more.
    Grace,
    /// Inside the window but already replayed: refuse, revoke nothing, so a
    /// third concurrent refresh cannot disconnect the request that won.
    Refuse,
    /// After the window: reuse of a leaked token.
    Reuse,
}

pub fn spent_refresh(
    rotated_at: DateTime<Utc>,
    grace_used: bool,
    now: DateTime<Utc>,
) -> SpentRefresh {
    if (now - rotated_at).num_seconds() > REFRESH_REUSE_GRACE_SECS {
        SpentRefresh::Reuse
    } else if grace_used {
        SpentRefresh::Refuse
    } else {
        SpentRefresh::Grace
    }
}

impl Database {
    // ============================================
    // Personal Access Tokens
    // ============================================

    pub async fn create_personal_access_token(
        &self,
        input: CreatePersonalAccessTokenRow,
    ) -> Result<PersonalAccessTokenRow> {
        let scopes_json = serde_json::to_value(&input.scopes)?;

        let row = sqlx::query_as::<_, PersonalAccessTokenRow>(
            sql!(r#"
            INSERT INTO personal_access_tokens (user_id, name, token_hash, token_prefix, scopes, expires_at, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING {PersonalAccessTokenRow}
            "#),
        )
        .bind(input.user_id)
        .bind(&input.name)
        .bind(&input.token_hash)
        .bind(&input.token_prefix)
        .bind(&scopes_json)
        .bind(input.expires_at)
        .bind(&input.metadata)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_personal_access_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<PersonalAccessTokenRow>> {
        let row = sqlx::query_as::<_, PersonalAccessTokenRow>(sql!(
            r#"
            SELECT {PersonalAccessTokenRow}
            FROM personal_access_tokens
            WHERE token_hash = $1
            "#
        ))
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_personal_access_tokens_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<PersonalAccessTokenRow>> {
        let rows = sqlx::query_as::<_, PersonalAccessTokenRow>(sql!(
            r#"
            SELECT {PersonalAccessTokenRow}
            FROM personal_access_tokens
            WHERE user_id = $1
            ORDER BY created_at DESC
            "#
        ))
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Record a use, at most once a minute per token, like `touch_agent_key`.
    /// Every request authenticates with its token, so an unconditional stamp
    /// was one row lock and one flushed commit per request on a hot row.
    pub async fn update_personal_access_token_last_used(&self, id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE personal_access_tokens SET last_used_at = NOW()
             WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < NOW() - INTERVAL '1 minute')",
        )
        .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn delete_personal_access_token(&self, id: Uuid, user_id: Uuid) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM personal_access_tokens WHERE id = $1 AND user_id = $2")
                .bind(id)
                .bind(user_id)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_personal_access_tokens_for_user(&self, user_id: Uuid) -> Result<u64> {
        let result = sqlx::query("DELETE FROM personal_access_tokens WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    // ============================================
    // Refresh Tokens
    // ============================================

    pub async fn create_refresh_token(
        &self,
        input: CreateRefreshTokenRow,
    ) -> Result<RefreshTokenRow> {
        let row = sqlx::query_as::<_, RefreshTokenRow>(sql!(
            r#"
            INSERT INTO refresh_tokens (user_id, token_hash, expires_at)
            VALUES ($1, $2, $3)
            RETURNING {RefreshTokenRow}
            "#
        ))
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
        let row = sqlx::query_as::<_, RefreshTokenRow>(sql!(
            r#"
            SELECT {RefreshTokenRow}
            FROM refresh_tokens
            WHERE token_hash = $1
            "#
        ))
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

    /// Atomically consume (delete + return) a refresh token by its hash if it
    /// is still valid. Used to enforce single-use rotation.
    ///
    /// EVE-454 / TM-AUTH-018: callers MUST replace the previous
    /// `get_refresh_token_by_hash` + `delete_refresh_token` two-step pattern
    /// with this method. The two-step pattern allowed concurrent refresh
    /// requests with the same token to both pass the existence check before
    /// either delete committed, minting multiple replacement tokens. Doing
    /// the delete in one statement with `RETURNING` ensures only one
    /// concurrent caller can observe the row.
    ///
    /// Also filters expired tokens at the SQL level so the handler does not
    /// need a separate expiry check after consume.
    pub async fn consume_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRow>> {
        let row = sqlx::query_as::<_, RefreshTokenRow>(sql!(
            r#"
            DELETE FROM refresh_tokens
            WHERE token_hash = $1
              AND expires_at > NOW()
            RETURNING {RefreshTokenRow}
            "#
        ))
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
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
    // Password Reset / Email Verification Tokens
    // ============================================

    pub async fn create_password_reset_token(
        &self,
        user_id: Uuid,
        token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO password_reset_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, $3)",
        )
        .bind(user_id)
        .bind(token_hash)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Atomic single-use claim: marks the token used and returns its owner only
    /// if it exists, is unexpired, and was not already used. The UPDATE ...
    /// WHERE used_at IS NULL ensures concurrent claims cannot both succeed.
    pub async fn consume_password_reset_token(&self, token_hash: &str) -> Result<Option<Uuid>> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            r#"
            UPDATE password_reset_tokens
            SET used_at = NOW()
            WHERE token_hash = $1
              AND used_at IS NULL
              AND expires_at > NOW()
            RETURNING user_id
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(user_id,)| user_id))
    }

    pub async fn create_email_verification_token(
        &self,
        user_id: Uuid,
        token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO email_verification_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, $3)",
        )
        .bind(user_id)
        .bind(token_hash)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Atomic single-use claim mirroring `consume_password_reset_token`.
    pub async fn consume_email_verification_token(&self, token_hash: &str) -> Result<Option<Uuid>> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            r#"
            UPDATE email_verification_tokens
            SET used_at = NOW()
            WHERE token_hash = $1
              AND used_at IS NULL
              AND expires_at > NOW()
            RETURNING user_id
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(user_id,)| user_id))
    }

    // ============================================
    // CLI Auth Sessions
    // ============================================

    pub async fn create_cli_auth_session(
        &self,
        input: CreateCliAuthSessionRow,
    ) -> Result<CliAuthSessionRow> {
        let row = sqlx::query_as::<_, CliAuthSessionRow>(sql!(
            r#"
            INSERT INTO cli_auth_sessions (state, exchange_code, redirect_port, expires_at)
            VALUES ($1, $2, $3, $4)
            RETURNING {CliAuthSessionRow}
            "#
        ))
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
        let row = sqlx::query_as::<_, CliAuthSessionRow>(sql!(
            r#"
            SELECT {CliAuthSessionRow}
            FROM cli_auth_sessions
            WHERE state = $1 AND expires_at > NOW()
            "#
        ))
        .bind(state)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_cli_auth_session_by_exchange_code(
        &self,
        code: &str,
    ) -> Result<Option<CliAuthSessionRow>> {
        let row = sqlx::query_as::<_, CliAuthSessionRow>(sql!(
            r#"
            SELECT {CliAuthSessionRow}
            FROM cli_auth_sessions
            WHERE exchange_code = $1 AND expires_at > NOW()
            "#
        ))
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

    pub async fn delete_cli_auth_session(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM cli_auth_sessions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // OAuth Clients (MCP OAuth 2.1)
    // ============================================

    pub async fn create_oauth_client(&self, input: CreateOAuthClientRow) -> Result<OAuthClientRow> {
        let row = sqlx::query_as::<_, OAuthClientRow>(sql!(
            r#"
            INSERT INTO oauth_clients (client_id, client_secret_hash, client_name, redirect_uris)
            VALUES ($1, $2, $3, $4)
            RETURNING {OAuthClientRow}
            "#
        ))
        .bind(&input.client_id)
        .bind(&input.client_secret_hash)
        .bind(&input.client_name)
        .bind(&input.redirect_uris)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_oauth_client_by_client_id(
        &self,
        client_id: &str,
    ) -> Result<Option<OAuthClientRow>> {
        let row = sqlx::query_as::<_, OAuthClientRow>(sql!(
            r#"
            SELECT {OAuthClientRow}
            FROM oauth_clients
            WHERE client_id = $1
            "#
        ))
        .bind(client_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // ============================================
    // OAuth Authorization Codes
    // ============================================

    pub async fn create_oauth_authorization_code(
        &self,
        input: CreateOAuthAuthorizationCodeRow,
    ) -> Result<OAuthAuthorizationCodeRow> {
        let row = sqlx::query_as::<_, OAuthAuthorizationCodeRow>(
            sql!(r#"
            INSERT INTO oauth_authorization_codes (code_hash, client_id, user_id, org_id, redirect_uri, code_challenge, code_challenge_method, scope, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING {OAuthAuthorizationCodeRow}
            "#),
        )
        .bind(&input.code_hash)
        .bind(&input.client_id)
        .bind(input.user_id)
        .bind(input.org_id)
        .bind(&input.redirect_uri)
        .bind(&input.code_challenge)
        .bind(&input.code_challenge_method)
        .bind(&input.scope)
        .bind(input.expires_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_oauth_authorization_code_by_hash(
        &self,
        code_hash: &str,
    ) -> Result<Option<OAuthAuthorizationCodeRow>> {
        let row = sqlx::query_as::<_, OAuthAuthorizationCodeRow>(sql!(
            r#"
            SELECT {OAuthAuthorizationCodeRow}
            FROM oauth_authorization_codes
            WHERE code_hash = $1 AND expires_at > NOW()
            "#
        ))
        .bind(code_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn consume_oauth_authorization_code(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE oauth_authorization_codes SET consumed = TRUE WHERE id = $1 AND consumed = FALSE",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_expired_oauth_authorization_codes(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM oauth_authorization_codes WHERE expires_at < NOW()")
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    // ============================================
    // OAuth Refresh Tokens
    // ============================================

    pub async fn create_oauth_refresh_token(
        &self,
        input: CreateOAuthRefreshTokenRow,
    ) -> Result<OAuthRefreshTokenRow> {
        let row = sqlx::query_as::<_, OAuthRefreshTokenRow>(
            sql!(r#"
            INSERT INTO oauth_refresh_tokens (token_hash, client_id, user_id, org_id, scope, expires_at, grant_id, family_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING {OAuthRefreshTokenRow}
            "#),
        )
        .bind(&input.token_hash)
        .bind(&input.client_id)
        .bind(input.user_id)
        .bind(input.org_id)
        .bind(&input.scope)
        .bind(input.expires_at)
        .bind(input.grant_id)
        .bind(input.family_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_oauth_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<OAuthRefreshTokenRow>> {
        let row = sqlx::query_as::<_, OAuthRefreshTokenRow>(sql!(
            r#"
            SELECT {OAuthRefreshTokenRow}
            FROM oauth_refresh_tokens
            WHERE token_hash = $1 AND expires_at > NOW()
            "#
        ))
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Exchange an MCP OAuth refresh token: mark it rotated if it is live and
    /// unused, else honour one replay inside [`REFRESH_REUSE_GRACE_SECS`], else
    /// refuse, deleting the token's family when the replay comes after the
    /// window. Each winning step is one conditional UPDATE, so concurrent
    /// requests cannot both win it.
    pub async fn use_oauth_refresh_token(&self, token_hash: &str) -> Result<OAuthRefreshUse> {
        let first = sqlx::query_as::<_, OAuthRefreshTokenRow>(sql!(
            r#"
            UPDATE oauth_refresh_tokens
            SET rotated_at = NOW()
            WHERE token_hash = $1
              AND expires_at > NOW()
              AND rotated_at IS NULL
            RETURNING {OAuthRefreshTokenRow}
            "#
        ))
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(row) = first {
            return Ok(OAuthRefreshUse::Mint(row));
        }

        let grace = sqlx::query_as::<_, OAuthRefreshTokenRow>(sql!(
            r#"
            UPDATE oauth_refresh_tokens
            SET grace_used = TRUE
            WHERE token_hash = $1
              AND expires_at > NOW()
              AND rotated_at >= NOW() - make_interval(secs => $2)
              AND NOT grace_used
            RETURNING {OAuthRefreshTokenRow}
            "#
        ))
        .bind(token_hash)
        .bind(REFRESH_REUSE_GRACE_SECS as f64)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(row) = grace {
            return Ok(OAuthRefreshUse::Mint(row));
        }

        let spent = sqlx::query_as::<_, OAuthRefreshTokenRow>(sql!(
            r#"
            SELECT {OAuthRefreshTokenRow}
            FROM oauth_refresh_tokens
            WHERE token_hash = $1
            "#
        ))
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        let Some(spent) = spent else {
            return Ok(OAuthRefreshUse::Refused);
        };
        let Some(rotated_at) = spent.rotated_at else {
            return Ok(OAuthRefreshUse::Refused); // expired, never used
        };
        match spent_refresh(rotated_at, spent.grace_used, Utc::now()) {
            SpentRefresh::Reuse => {
                let family = spent.family();
                sqlx::query("DELETE FROM oauth_refresh_tokens WHERE id = $1 OR family_id = $1")
                    .bind(family)
                    .execute(&self.pool)
                    .await?;
                Ok(OAuthRefreshUse::ReuseDetected)
            }
            SpentRefresh::Grace | SpentRefresh::Refuse => Ok(OAuthRefreshUse::Refused),
        }
    }

    pub async fn delete_oauth_refresh_token(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM oauth_refresh_tokens WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_expired_oauth_refresh_tokens(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM oauth_refresh_tokens WHERE expires_at < NOW()")
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests;
