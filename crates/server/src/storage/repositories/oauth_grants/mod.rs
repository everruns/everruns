// PostgreSQL repository: MCP OAuth grants (connected AI clients).
//
// Spec: knowledge/integrations/mcp-connected-clients.md.
//
// Decision: a revoked grant is never un-revoked. Approving a client whose grant
// was revoked replaces the row with a fresh one (new id), so access tokens that
// named the old grant stay rejected even after the user reconnects the same
// client. An active grant is kept as is. Either way there is one row per client
// and user.

pub(super) mod rows;
use rows::*;

use super::Database;
use anyhow::Result;
use everruns_server_macros::sql;
use uuid::Uuid;

/// Access level written for every grant until the consent page offers a choice.
pub const OAUTH_GRANT_ACCESS_READ_AND_RUN: &str = "read_and_run";

/// Upper bound on one user's listed grants. Each install of a client registers
/// its own client id, so the list grows with installs, not requests.
const MAX_LISTED_GRANTS: i64 = 500;

impl Database {
    /// Record that `user_id` approved `client_id`: create the grant, keep an
    /// active one, or replace a revoked one with a fresh grant.
    pub async fn approve_oauth_grant(
        &self,
        client_id: &str,
        user_id: Uuid,
    ) -> Result<OAuthGrantRow> {
        sqlx::query(
            "DELETE FROM oauth_grants WHERE client_id = $1 AND user_id = $2 AND revoked_at IS NOT NULL",
        )
        .bind(client_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;

        // The no-op update makes RETURNING hand back an existing active grant.
        let row = sqlx::query_as::<_, OAuthGrantRow>(sql!(
            r#"
            INSERT INTO oauth_grants (client_id, user_id, access)
            VALUES ($1, $2, $3)
            ON CONFLICT (client_id, user_id) DO UPDATE SET client_id = EXCLUDED.client_id
            RETURNING {OAuthGrantRow}
            "#
        ))
        .bind(client_id)
        .bind(user_id)
        .bind(OAUTH_GRANT_ACCESS_READ_AND_RUN)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// The client and user's grant, creating an active one only when none
    /// exists. Never revives a revoked grant: that takes a fresh approval.
    ///
    /// The token endpoint uses this for authorization codes and refresh tokens
    /// minted without a grant: before grants existed, or by a replica still on
    /// the previous release during a rolling deploy.
    pub async fn ensure_oauth_grant(
        &self,
        client_id: &str,
        user_id: Uuid,
    ) -> Result<OAuthGrantRow> {
        sqlx::query(
            r#"
            INSERT INTO oauth_grants (client_id, user_id, access)
            VALUES ($1, $2, $3)
            ON CONFLICT (client_id, user_id) DO NOTHING
            "#,
        )
        .bind(client_id)
        .bind(user_id)
        .bind(OAUTH_GRANT_ACCESS_READ_AND_RUN)
        .execute(&self.pool)
        .await?;

        let row = sqlx::query_as::<_, OAuthGrantRow>(sql!(
            r#"
            SELECT {OAuthGrantRow}
            FROM oauth_grants
            WHERE client_id = $1 AND user_id = $2
            "#
        ))
        .bind(client_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_oauth_grant(&self, id: Uuid) -> Result<Option<OAuthGrantRow>> {
        let row = sqlx::query_as::<_, OAuthGrantRow>(sql!(
            r#"
            SELECT {OAuthGrantRow}
            FROM oauth_grants
            WHERE id = $1
            "#
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// The user's active grants with their clients, most recently used first.
    pub async fn list_active_oauth_grants_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<OAuthGrantWithClientRow>> {
        let rows = sqlx::query_as::<_, OAuthGrantWithClientRow>(
            r#"
            SELECT g.id, g.client_id, c.client_name, c.redirect_uris, g.access,
                   g.allowed_org_ids, g.created_at, g.last_used_at
            FROM oauth_grants g
            JOIN oauth_clients c ON c.client_id = g.client_id
            WHERE g.user_id = $1 AND g.revoked_at IS NULL
            ORDER BY COALESCE(g.last_used_at, g.created_at) DESC, g.id DESC
            LIMIT $2
            "#,
        )
        .bind(user_id)
        .bind(MAX_LISTED_GRANTS)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Revoke one of the user's grants and delete every refresh token the
    /// client holds for that user, in one statement. Returns `None` when the
    /// grant does not exist, belongs to someone else, or is already revoked.
    ///
    /// Tokens are deleted by client and user, not only by `grant_id`, so a
    /// refresh token minted without a grant during a rolling deploy goes too.
    pub async fn revoke_oauth_grant(
        &self,
        id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<OAuthGrantRow>> {
        let row = sqlx::query_as::<_, OAuthGrantRow>(sql!(
            r#"
            WITH revoked AS (
                UPDATE oauth_grants
                SET revoked_at = NOW()
                WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL
                RETURNING {OAuthGrantRow}
            ),
            deleted AS (
                DELETE FROM oauth_refresh_tokens rt
                USING revoked r
                WHERE rt.client_id = r.client_id AND rt.user_id = r.user_id
                RETURNING rt.id
            )
            SELECT {OAuthGrantRow} FROM revoked
            "#
        ))
        .bind(id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Stamp a grant as used now. Callers throttle this per grant.
    pub async fn touch_oauth_grant_last_used(&self, id: Uuid) -> Result<()> {
        sqlx::query("UPDATE oauth_grants SET last_used_at = NOW() WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
