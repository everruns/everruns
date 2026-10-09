// PostgreSQL repository: per-agent GitHub Apps (migrations 149 and 153)

pub(super) mod rows;
use rows::*;

use super::Database;
use crate::kernel_imports::contracts::typed_id::VirtualUserId;
use anyhow::Result;
use uuid::Uuid;

const COLUMNS: &str = "id, org_id, virtual_user_id, app_id, slug, name, html_url, owner_login, client_id, client_secret_encrypted, private_key_encrypted, webhook_secret_encrypted, created_by_user_id, created_at, updated_at";

impl Database {
    pub async fn create_github_app(&self, input: CreateGitHubAppRow) -> Result<GitHubAppRow> {
        let sql = format!(
            "INSERT INTO github_apps (id, org_id, virtual_user_id, app_id, slug, name, html_url, owner_login, client_id, client_secret_encrypted, private_key_encrypted, webhook_secret_encrypted, created_by_user_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             RETURNING {COLUMNS}"
        );
        Ok(
            sqlx::query_as::<_, GitHubAppRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(input.id)
                .bind(input.org_id)
                .bind(input.virtual_user_id)
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
        virtual_user_id: VirtualUserId,
    ) -> Result<Option<GitHubAppRow>> {
        let sql =
            format!("SELECT {COLUMNS} FROM github_apps WHERE org_id = $1 AND virtual_user_id = $2");
        Ok(
            sqlx::query_as::<_, GitHubAppRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(org_id)
                .bind(virtual_user_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn delete_github_app(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM github_apps WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
