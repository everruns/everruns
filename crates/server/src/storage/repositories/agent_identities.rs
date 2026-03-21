// PostgreSQL repository: Agent identities

use super::super::models::*;
use super::Database;
use super::build_search_sql;
use anyhow::Result;
use everruns_core::AgentIdentityId;

impl Database {
    pub async fn create_agent_identity(
        &self,
        input: CreateAgentIdentityRow,
    ) -> Result<AgentIdentityRow> {
        let row = sqlx::query_as::<_, AgentIdentityRow>(
            r#"
            INSERT INTO agent_identities (org_id, id, name, description, avatar_url, locale, timezone, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'active')
            RETURNING id, org_id, name, description, avatar_url, locale, timezone, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(input.org_id)
        .bind(input.id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.avatar_url)
        .bind(&input.locale)
        .bind(&input.timezone)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent_identity(
        &self,
        org_id: i64,
        id: AgentIdentityId,
    ) -> Result<Option<AgentIdentityRow>> {
        let row = sqlx::query_as::<_, AgentIdentityRow>(
            r#"
            SELECT id, org_id, name, description, avatar_url, locale, timezone, status, created_at, updated_at, archived_at, deleted_at
            FROM agent_identities
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_agent_identities(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<AgentIdentityRow>> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 2);
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status NOT IN ('archived', 'deleted')"
        };
        let sql = format!(
            r#"SELECT id, org_id, name, description, avatar_url, locale, timezone, status, created_at, updated_at, archived_at, deleted_at
                FROM agent_identities
                WHERE org_id = $1{status_sql}{search_sql}
                ORDER BY created_at DESC"#
        );
        let mut query = sqlx::query_as::<_, AgentIdentityRow>(&sql).bind(org_id);
        for pat in &patterns {
            query = query.bind(pat);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn update_agent_identity(
        &self,
        org_id: i64,
        id: AgentIdentityId,
        input: UpdateAgentIdentity,
    ) -> Result<Option<AgentIdentityRow>> {
        let row = sqlx::query_as::<_, AgentIdentityRow>(
            r#"
            UPDATE agent_identities
            SET
                name = COALESCE($3, name),
                description = CASE WHEN $4 THEN $5 ELSE description END,
                avatar_url = CASE WHEN $6 THEN $7 ELSE avatar_url END,
                locale = CASE WHEN $8 THEN $9 ELSE locale END,
                timezone = CASE WHEN $10 THEN $11 ELSE timezone END,
                status = COALESCE($12, status),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, name, description, avatar_url, locale, timezone, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.name)
        .bind(input.description.is_changed())
        .bind(input.description.into_value())
        .bind(input.avatar_url.is_changed())
        .bind(input.avatar_url.into_value())
        .bind(input.locale.is_changed())
        .bind(input.locale.into_value())
        .bind(input.timezone.is_changed())
        .bind(input.timezone.into_value())
        .bind(&input.status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_agent_identity(&self, org_id: i64, id: AgentIdentityId) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE agent_identities
            SET status = 'archived', archived_at = COALESCE(archived_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status = 'active'
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn destroy_agent_identity(&self, org_id: i64, id: AgentIdentityId) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE agent_identities
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
