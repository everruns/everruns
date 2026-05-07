// PostgreSQL repository: Workspace Volume CRUD

use super::super::models::*;
use super::{Database, build_search_sql};
use anyhow::Result;
use uuid::Uuid;

impl Database {
    pub async fn create_volume(&self, org_id: i64, input: CreateVolumeRow) -> Result<VolumeRow> {
        let row = sqlx::query_as::<_, VolumeRow>(
            r#"
            INSERT INTO volumes (org_id, public_id, name, description, owner_principal_id, resolved_owner_user_id)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, org_id, public_id, name, description, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.owner_principal_id)
        .bind(input.resolved_owner_user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_volume_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<VolumeRow>> {
        let row = sqlx::query_as::<_, VolumeRow>(
            r#"
            SELECT id, org_id, public_id, name, description, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            FROM volumes
            WHERE org_id = $1 AND public_id = $2 AND status != 'deleted'
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_volume_by_id(&self, org_id: i64, id: Uuid) -> Result<Option<VolumeRow>> {
        let row = sqlx::query_as::<_, VolumeRow>(
            r#"
            SELECT id, org_id, public_id, name, description, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            FROM volumes
            WHERE org_id = $1 AND id = $2 AND status != 'deleted'
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_volume_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        let row = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT org_id
            FROM volumes
            WHERE public_id = $1 AND status != 'deleted'
            "#,
        )
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_volumes(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<VolumeRow>> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 2);
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status = 'active'"
        };
        let sql = format!(
            r#"
            SELECT id, org_id, public_id, name, description, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            FROM volumes
            WHERE org_id = $1{status_sql}{search_sql}
            ORDER BY created_at DESC
            "#
        );
        let mut query = sqlx::query_as::<_, VolumeRow>(&sql).bind(org_id);
        for pattern in &patterns {
            query = query.bind(pattern);
        }

        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn update_volume(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateVolume,
    ) -> Result<Option<VolumeRow>> {
        let row = sqlx::query_as::<_, VolumeRow>(
            r#"
            UPDATE volumes
            SET
                name = COALESCE($3, name),
                description = CASE WHEN $4 THEN $5 ELSE description END,
                status = COALESCE($6, status),
                archived_at = CASE
                    WHEN $6 = 'archived' THEN COALESCE(archived_at, NOW())
                    WHEN $6 = 'active' THEN NULL
                    ELSE archived_at
                END,
                deleted_at = CASE
                    WHEN $6 = 'deleted' THEN COALESCE(deleted_at, NOW())
                    ELSE deleted_at
                END,
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status != 'deleted'
            RETURNING id, org_id, public_id, name, description, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.name)
        .bind(input.description.is_some())
        .bind(input.description.flatten())
        .bind(&input.status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn archive_volume(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE volumes
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
}
