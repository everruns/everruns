use super::super::models::*;
use super::{Database, build_search_sql};
use anyhow::Result;
use uuid::Uuid;

impl Database {
    pub async fn create_declarative_capability(
        &self,
        org_id: i64,
        input: CreateDeclarativeCapabilityRow,
    ) -> Result<DeclarativeCapabilityRow> {
        Ok(sqlx::query_as::<_, DeclarativeCapabilityRow>(
            r#"
            INSERT INTO declarative_capabilities (org_id, public_id, name, display_name, description, definition)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, org_id, public_id, name, display_name, description, status, definition, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&input.definition)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_declarative_capability(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        Ok(sqlx::query_as::<_, DeclarativeCapabilityRow>(
            r#"
            SELECT id, org_id, public_id, name, display_name, description, status, definition, created_at, updated_at, archived_at, deleted_at
            FROM declarative_capabilities
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn get_declarative_capability_by_name(
        &self,
        org_id: i64,
        name: &str,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        Ok(sqlx::query_as::<_, DeclarativeCapabilityRow>(
            r#"
            SELECT id, org_id, public_id, name, display_name, description, status, definition, created_at, updated_at, archived_at, deleted_at
            FROM declarative_capabilities
            WHERE org_id = $1 AND name = $2
            "#,
        )
        .bind(org_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn get_declarative_capability_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        Ok(sqlx::query_as::<_, DeclarativeCapabilityRow>(
            r#"
            SELECT id, org_id, public_id, name, display_name, description, status, definition, created_at, updated_at, archived_at, deleted_at
            FROM declarative_capabilities
            WHERE org_id = $1 AND public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn list_declarative_capabilities(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<DeclarativeCapabilityRow>> {
        let (search_sql, patterns) = build_search_sql(
            search,
            "LOWER(name || ' ' || COALESCE(display_name, '') || ' ' || COALESCE(description, ''))",
            2,
        );
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status NOT IN ('archived', 'deleted')"
        };
        let sql = format!(
            r#"
            SELECT id, org_id, public_id, name, display_name, description, status, definition, created_at, updated_at, archived_at, deleted_at
            FROM declarative_capabilities
            WHERE org_id = $1{status_sql}{search_sql}
            ORDER BY created_at DESC
            "#
        );
        let mut query = sqlx::query_as::<_, DeclarativeCapabilityRow>(sqlx::AssertSqlSafe(sql.as_str())).bind(org_id);
        for pattern in &patterns {
            query = query.bind(pattern);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn update_declarative_capability(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateDeclarativeCapability,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        Ok(sqlx::query_as::<_, DeclarativeCapabilityRow>(
            r#"
            UPDATE declarative_capabilities
            SET
                name = COALESCE($3, name),
                display_name = CASE
                    WHEN $4::text IS NOT NULL THEN $4
                    WHEN $7::jsonb IS NOT NULL THEN NULL
                    ELSE display_name
                END,
                description = COALESCE($5, description),
                status = COALESCE($6, status),
                definition = COALESCE($7, definition),
                archived_at = CASE WHEN $6 = 'archived' THEN COALESCE(archived_at, NOW()) ELSE archived_at END
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, public_id, name, display_name, description, status, definition, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&input.status)
        .bind(&input.definition)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn delete_declarative_capability(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE declarative_capabilities
            SET status = 'archived', archived_at = COALESCE(archived_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status IN ('active', 'disabled')
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn destroy_declarative_capability(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE declarative_capabilities
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
