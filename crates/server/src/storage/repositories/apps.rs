// PostgreSQL repository: App CRUD

use super::super::models::*;
use super::Database;
use super::build_search_sql;
use anyhow::Result;
use everruns_provider::typed_id::{AgentId, HarnessId};
use uuid::Uuid;

impl Database {
    // ============================================
    // App CRUD
    // ============================================

    pub async fn create_app(&self, org_id: i64, input: CreateAppRow) -> Result<AppRow> {
        let row = sqlx::query_as::<_, AppRow>(
            r#"
            INSERT INTO apps (org_id, public_id, name, description, harness_id, agent_id, agent_version_policy, agent_version_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, channel_type, channel_config, channel_config_encrypted, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, 'draft')
            RETURNING id, org_id, public_id, name, description, harness_id, agent_id, agent_version_policy, agent_version_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, channel_type, channel_config, channel_config_encrypted, status, published_at, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(input.harness_id)
        .bind(input.agent_id)
        .bind(&input.agent_version_policy)
        .bind(input.agent_version_id)
        .bind(input.agent_identity_id)
        .bind(input.owner_principal_id)
        .bind(input.resolved_owner_user_id)
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .bind(&input.channel_config_encrypted)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_app_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AppRow>> {
        let row = sqlx::query_as::<_, AppRow>(
            r#"
            SELECT id, org_id, public_id, name, description, harness_id, agent_id, agent_version_policy, agent_version_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, channel_type, channel_config, channel_config_encrypted, status, published_at, created_at, updated_at, archived_at, deleted_at
            FROM apps
            WHERE org_id = $1 AND public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_app_by_id(&self, org_id: i64, id: Uuid) -> Result<Option<AppRow>> {
        let row = sqlx::query_as::<_, AppRow>(
            r#"
            SELECT id, org_id, public_id, name, description, harness_id, agent_id, agent_version_policy, agent_version_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, channel_type, channel_config, channel_config_encrypted, status, published_at, created_at, updated_at, archived_at, deleted_at
            FROM apps
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Look up the owning org for an app by its public_id. See
    /// knowledge/security/multitenancy.md (Cross-Org Resource Resolution).
    pub async fn get_app_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT org_id FROM apps WHERE public_id = $1 LIMIT 1")
                .bind(public_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(org_id,)| org_id))
    }

    /// Lookup app by public_id without org scoping (for unauthenticated webhooks).
    pub async fn get_app_by_public_id_unscoped(&self, public_id: &str) -> Result<Option<AppRow>> {
        let row = sqlx::query_as::<_, AppRow>(
            r#"
            SELECT id, org_id, public_id, name, description, harness_id, agent_id, agent_version_policy, agent_version_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, channel_type, channel_config, channel_config_encrypted, status, published_at, created_at, updated_at, archived_at, deleted_at
            FROM apps
            WHERE public_id = $1
            "#,
        )
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_apps(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<AppRow>> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 2);
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status NOT IN ('archived', 'deleted')"
        };
        let sql = format!(
            r#"SELECT id, org_id, public_id, name, description, harness_id, agent_id, agent_version_policy, agent_version_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, channel_type, channel_config, channel_config_encrypted, status, published_at, created_at, updated_at, archived_at, deleted_at
                FROM apps
                WHERE org_id = $1{status_sql}{search_sql}
                ORDER BY created_at DESC"#
        );
        let mut query = sqlx::query_as::<_, AppRow>(sqlx::AssertSqlSafe(sql.as_str())).bind(org_id);
        for pat in &patterns {
            query = query.bind(pat);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn count_apps_for_agent(&self, org_id: i64, agent_id: AgentId) -> Result<u64> {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM apps WHERE org_id = $1 AND agent_id = $2 AND status != 'deleted'",
        )
        .bind(org_id)
        .bind(agent_id.uuid())
        .fetch_one(&self.pool)
        .await?;

        Ok(count as u64)
    }

    pub async fn count_apps_for_harness(&self, org_id: i64, harness_id: HarnessId) -> Result<u64> {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM apps WHERE org_id = $1 AND harness_id = $2 AND status != 'deleted'",
        )
        .bind(org_id)
        .bind(harness_id.uuid())
        .fetch_one(&self.pool)
        .await?;

        Ok(count as u64)
    }

    pub async fn count_apps_for_harnesses(
        &self,
        org_id: i64,
        harness_ids: &[HarnessId],
    ) -> Result<Vec<(HarnessId, i64)>> {
        let harness_ids = harness_ids.iter().map(|id| id.uuid()).collect::<Vec<_>>();
        Ok(sqlx::query_as(
            r#"
            SELECT harness_id, COUNT(*)::bigint
            FROM apps
            WHERE org_id = $1 AND harness_id = ANY($2) AND status != 'deleted'
            GROUP BY harness_id
            "#,
        )
        .bind(org_id)
        .bind(&harness_ids)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn update_app(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateApp,
    ) -> Result<Option<AppRow>> {
        let row = sqlx::query_as::<_, AppRow>(
            r#"
            UPDATE apps
            SET
                name = COALESCE($3, name),
                description = COALESCE($4, description),
                harness_id = COALESCE($5, harness_id),
                agent_id = COALESCE($6, agent_id),
                agent_version_policy = COALESCE($7, agent_version_policy),
                agent_version_id = CASE WHEN $8 THEN $9 ELSE agent_version_id END,
                agent_identity_id = CASE WHEN $10 THEN $11 ELSE agent_identity_id END,
                owner_principal_id = COALESCE($12, owner_principal_id),
                resolved_owner_user_id = CASE WHEN $13 THEN $14 ELSE resolved_owner_user_id END,
                channel_type = COALESCE($15, channel_type),
                channel_config = COALESCE($16, channel_config),
                channel_config_encrypted = COALESCE($17, channel_config_encrypted),
                status = COALESCE($18, status),
                published_at = CASE WHEN $19 THEN $20 ELSE published_at END,
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, public_id, name, description, harness_id, agent_id, agent_version_policy, agent_version_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, channel_type, channel_config, channel_config_encrypted, status, published_at, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(input.harness_id)
        .bind(input.agent_id)
        .bind(&input.agent_version_policy)
        .bind(input.agent_version_id.is_changed())
        .bind(input.agent_version_id.into_value())
        .bind(input.agent_identity_id.is_changed())
        .bind(input.agent_identity_id.into_value())
        .bind(input.owner_principal_id)
        .bind(input.resolved_owner_user_id.is_changed())
        .bind(input.resolved_owner_user_id.into_value())
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .bind(&input.channel_config_encrypted)
        .bind(&input.status)
        .bind(input.published_at.is_changed())
        .bind(input.published_at.into_value())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_app(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE apps
            SET status = 'archived', archived_at = COALESCE(archived_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status IN ('draft', 'published')
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn destroy_app(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE apps
            SET status = 'deleted', deleted_at = COALESCE(deleted_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status = 'archived'
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
