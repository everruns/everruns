// PostgreSQL repository: Workspace Volume CRUD

use super::super::models::*;
use super::{Database, build_search_sql};
use anyhow::Result;
use uuid::Uuid;

impl Database {
    pub async fn create_volume(&self, org_id: i64, input: CreateVolumeRow) -> Result<VolumeRow> {
        let row = sqlx::query_as::<_, VolumeRow>(
            r#"
            INSERT INTO volumes (
                org_id, public_id, name, description, source_type, source_config,
                is_readonly, sync_status, owner_principal_id, resolved_owner_user_id
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id, org_id, public_id, name, description, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.source_type)
        .bind(&input.source_config)
        .bind(input.is_readonly)
        .bind(&input.sync_status)
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
            SELECT id, org_id, public_id, name, description, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
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
            SELECT id, org_id, public_id, name, description, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
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
            LIMIT 1
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
            SELECT id, org_id, public_id, name, description, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            FROM volumes
            WHERE org_id = $1{status_sql}{search_sql}
            ORDER BY created_at DESC
            "#
        );
        let mut query =
            sqlx::query_as::<_, VolumeRow>(sqlx::AssertSqlSafe(sql.as_str())).bind(org_id);
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
                source_type = COALESCE($7, source_type),
                source_config = COALESCE($8, source_config),
                is_readonly = COALESCE($9, is_readonly),
                sync_status = COALESCE($10, sync_status),
                last_synced_at = CASE WHEN $11 THEN $12 ELSE last_synced_at END,
                last_sync_error = CASE WHEN $13 THEN $14 ELSE last_sync_error END,
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
            RETURNING id, org_id, public_id, name, description, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.name)
        .bind(input.description.is_some())
        .bind(input.description.flatten())
        .bind(&input.status)
        .bind(&input.source_type)
        .bind(&input.source_config)
        .bind(input.is_readonly)
        .bind(&input.sync_status)
        .bind(input.last_synced_at.is_some())
        .bind(input.last_synced_at.flatten())
        .bind(input.last_sync_error.is_some())
        .bind(input.last_sync_error.flatten())
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

    pub async fn claim_next_volume_sync(&self) -> Result<Option<VolumeRow>> {
        let row = sqlx::query_as::<_, VolumeRow>(
            r#"
            UPDATE volumes
            SET sync_status = 'syncing', last_sync_error = NULL, updated_at = NOW()
            WHERE id = (
                SELECT id
                FROM volumes
                WHERE status = 'active'
                  AND source_type != 'manual'
                  AND (
                    sync_status = 'pending'
                    OR (sync_status = 'syncing' AND updated_at < NOW() - INTERVAL '15 minutes')
                    OR (
                        sync_status IN ('synced', 'failed')
                        AND COALESCE((source_config->>'sync_interval_secs')::int, 0) > 0
                        AND COALESCE(last_synced_at, updated_at) < NOW() - make_interval(secs => COALESCE((source_config->>'sync_interval_secs')::int, 0))
                    )
                  )
                ORDER BY updated_at ASC
                FOR UPDATE SKIP LOCKED
                LIMIT 1
            )
            RETURNING id, org_id, public_id, name, description, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn complete_volume_sync(
        &self,
        volume_id: Uuid,
        claimed_at: chrono::DateTime<chrono::Utc>,
        files: Vec<CreateVolumeFileRow>,
    ) -> Result<Option<VolumeRow>> {
        let mut tx = self.pool.begin().await?;

        let volume = sqlx::query_as::<_, VolumeRow>(
            r#"
            UPDATE volumes
            SET sync_status = 'synced',
                last_synced_at = NOW(),
                last_sync_error = NULL,
                updated_at = NOW()
            WHERE id = $1
              AND updated_at = $2
              AND sync_status = 'syncing'
              AND status = 'active'
              AND source_type != 'manual'
            RETURNING id, org_id, public_id, name, description, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(volume_id)
        .bind(claimed_at)
        .fetch_optional(&mut *tx)
        .await?;

        if volume.is_none() {
            tx.commit().await?;
            return Ok(None);
        }

        sqlx::query("DELETE FROM volume_files WHERE volume_id = $1")
            .bind(volume_id)
            .execute(&mut *tx)
            .await?;

        for file in files {
            let size_bytes = file.content.as_ref().map(|c| c.len() as i64).unwrap_or(0);
            sqlx::query(
                r#"
                INSERT INTO volume_files (volume_id, path, content, is_directory, size_bytes, content_hash)
                VALUES ($1, $2, $3, $4, $5, $6)
                "#,
            )
            .bind(volume_id)
            .bind(&file.path)
            .bind(&file.content)
            .bind(file.is_directory)
            .bind(size_bytes)
            .bind(&file.content_hash)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(volume)
    }

    pub async fn fail_volume_sync(
        &self,
        volume_id: Uuid,
        claimed_at: chrono::DateTime<chrono::Utc>,
        error: &str,
    ) -> Result<Option<VolumeRow>> {
        let row = sqlx::query_as::<_, VolumeRow>(
            r#"
            UPDATE volumes
            SET sync_status = 'failed',
                last_sync_error = $2,
                updated_at = NOW()
            WHERE id = $1
              AND updated_at = $3
              AND sync_status = 'syncing'
              AND status = 'active'
              AND source_type != 'manual'
            RETURNING id, org_id, public_id, name, description, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(volume_id)
        .bind(error)
        .bind(claimed_at)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_all_volume_files(&self, volume_id: Uuid) -> Result<Vec<VolumeFileRow>> {
        let rows = sqlx::query_as::<_, VolumeFileRow>(
            r#"
            SELECT id, volume_id, path, content, is_directory, size_bytes, content_hash, created_at, updated_at
            FROM volume_files
            WHERE volume_id = $1
            ORDER BY path ASC
            "#,
        )
        .bind(volume_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }
}
