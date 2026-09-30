// PostgreSQL repository: per-agent GitHub Apps (migration 147)

use super::super::github_app_rows::*;
use super::Database;
use crate::kernel_imports::everruns_provider::typed_id::{AgentIdentityId, SessionId};
use anyhow::Result;
use uuid::Uuid;

const COLUMNS: &str = "id, org_id, agent_identity_id, app_id, slug, name, html_url, owner_login, client_id, client_secret_encrypted, private_key_encrypted, webhook_secret_encrypted, created_by_user_id, created_at, updated_at";

impl Database {
    pub async fn create_github_app(&self, input: CreateGitHubAppRow) -> Result<GitHubAppRow> {
        let sql = format!(
            "INSERT INTO github_apps (id, org_id, agent_identity_id, app_id, slug, name, html_url, owner_login, client_id, client_secret_encrypted, private_key_encrypted, webhook_secret_encrypted, created_by_user_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             RETURNING {COLUMNS}"
        );
        Ok(
            sqlx::query_as::<_, GitHubAppRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(input.id)
                .bind(input.org_id)
                .bind(input.agent_identity_id)
                .bind(input.app_id)
                .bind(&input.slug)
                .bind(&input.name)
                .bind(&input.html_url)
                .bind(&input.owner_login)
                .bind(&input.client_id)
                .bind(&input.client_secret_encrypted)
                .bind(&input.private_key_encrypted)
                .bind(&input.webhook_secret_encrypted)
                .bind(input.created_by_user_id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// Look an App up by its row id alone. Used by unauthenticated GitHub
    /// callbacks (webhook, setup) whose URL carries the id; callers must then
    /// authenticate the request against the row's secrets or org.
    pub async fn get_github_app_unscoped(&self, id: Uuid) -> Result<Option<GitHubAppRow>> {
        let sql = format!("SELECT {COLUMNS} FROM github_apps WHERE id = $1");
        Ok(
            sqlx::query_as::<_, GitHubAppRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn get_github_app_for_identity(
        &self,
        org_id: i64,
        agent_identity_id: AgentIdentityId,
    ) -> Result<Option<GitHubAppRow>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM github_apps WHERE org_id = $1 AND agent_identity_id = $2"
        );
        Ok(
            sqlx::query_as::<_, GitHubAppRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(org_id)
                .bind(agent_identity_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// The identity-owned GitHub App and installation a session resolves to:
    /// the session's agent identity must own an App and have an installed
    /// `github` connection.
    pub async fn get_identity_github_app_for_session(
        &self,
        session_id: SessionId,
    ) -> Result<Option<(GitHubAppRow, i64)>> {
        let sql = format!(
            "SELECT {cols}, aic.installation_id AS installation_id
             FROM sessions s
             JOIN agent_identity_connections aic
                 ON aic.agent_identity_id = s.agent_identity_id AND aic.provider = 'github'
             JOIN github_apps ga ON ga.agent_identity_id = s.agent_identity_id
             WHERE s.id = $1 AND aic.installation_id IS NOT NULL
             LIMIT 1",
            cols = COLUMNS
                .split(", ")
                .map(|column| format!("ga.{column}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        use sqlx::{FromRow, Row};
        let app = GitHubAppRow::from_row(&row)?;
        let installation_id: i64 = row.try_get("installation_id")?;
        Ok(Some((app, installation_id)))
    }

    pub async fn delete_github_app(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM github_apps WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
