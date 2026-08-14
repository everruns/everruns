// PostgreSQL repository: Workspace Memory CRUD

use super::super::models::*;
use super::{Database, build_search_sql};
use anyhow::Result;
use uuid::Uuid;

impl Database {
    pub async fn create_memory(&self, org_id: i64, input: CreateMemoryRow) -> Result<MemoryRow> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            INSERT INTO memories (
                org_id, public_id, name, description, scope, owner_agent_id, owner_user_id, source_type, source_config,
                is_readonly, sync_status, owner_principal_id, resolved_owner_user_id
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            RETURNING id, org_id, public_id, name, description, scope, owner_agent_id, owner_user_id, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.scope)
        .bind(input.owner_agent_id.map(|id| id.uuid()))
        .bind(input.owner_user_id)
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

    pub async fn get_memory_by_scope_owner(
        &self,
        org_id: i64,
        scope: &str,
        owner_agent_id: Option<everruns_provider::typed_id::AgentId>,
        owner_user_id: Option<Uuid>,
    ) -> Result<Option<MemoryRow>> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT id, org_id, public_id, name, description, scope, owner_agent_id, owner_user_id, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            FROM memories
            WHERE org_id = $1
              AND scope = $2
              AND (($3::uuid IS NULL AND owner_agent_id IS NULL) OR owner_agent_id = $3)
              AND (($4::uuid IS NULL AND owner_user_id IS NULL) OR owner_user_id = $4)
              AND status != 'deleted'
            "#,
        )
        .bind(org_id)
        .bind(scope)
        .bind(owner_agent_id.map(|id| id.uuid()))
        .bind(owner_user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_memory_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<MemoryRow>> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT id, org_id, public_id, name, description, scope, owner_agent_id, owner_user_id, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            FROM memories
            WHERE org_id = $1 AND public_id = $2 AND status != 'deleted'
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_memory_by_id(&self, org_id: i64, id: Uuid) -> Result<Option<MemoryRow>> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT id, org_id, public_id, name, description, scope, owner_agent_id, owner_user_id, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            FROM memories
            WHERE org_id = $1 AND id = $2 AND status != 'deleted'
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_memory_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        let row = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT org_id
            FROM memories
            WHERE public_id = $1 AND status != 'deleted'
            LIMIT 1
            "#,
        )
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_memories(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<MemoryRow>> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 2);
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status = 'active'"
        };
        let sql = format!(
            r#"
            SELECT id, org_id, public_id, name, description, scope, owner_agent_id, owner_user_id, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            FROM memories
            WHERE org_id = $1 AND scope = 'org'{status_sql}{search_sql}
            ORDER BY created_at DESC
            "#
        );
        let mut query =
            sqlx::query_as::<_, MemoryRow>(sqlx::AssertSqlSafe(sql.as_str())).bind(org_id);
        for pattern in &patterns {
            query = query.bind(pattern);
        }

        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn update_memory(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMemory,
    ) -> Result<Option<MemoryRow>> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            UPDATE memories
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
            RETURNING id, org_id, public_id, name, description, scope, owner_agent_id, owner_user_id, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
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

    pub async fn archive_memory(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE memories
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

    pub async fn claim_next_memory_sync(&self) -> Result<Option<MemoryRow>> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            UPDATE memories
            SET sync_status = 'syncing', last_sync_error = NULL, updated_at = NOW()
            WHERE id = (
                SELECT id
                FROM memories
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
            RETURNING id, org_id, public_id, name, description, scope, owner_agent_id, owner_user_id, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn complete_memory_sync(
        &self,
        memory_id: Uuid,
        claimed_at: chrono::DateTime<chrono::Utc>,
        files: Vec<CreateMemoryFileRow>,
    ) -> Result<Option<MemoryRow>> {
        let mut tx = self.pool.begin().await?;

        let memory = sqlx::query_as::<_, MemoryRow>(
            r#"
            UPDATE memories
            SET sync_status = 'synced',
                last_synced_at = NOW(),
                last_sync_error = NULL,
                updated_at = NOW()
            WHERE id = $1
              AND updated_at = $2
              AND sync_status = 'syncing'
              AND status = 'active'
              AND source_type != 'manual'
            RETURNING id, org_id, public_id, name, description, scope, owner_agent_id, owner_user_id, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(memory_id)
        .bind(claimed_at)
        .fetch_optional(&mut *tx)
        .await?;

        if memory.is_none() {
            tx.commit().await?;
            return Ok(None);
        }

        sqlx::query("DELETE FROM memory_files WHERE memory_id = $1")
            .bind(memory_id)
            .execute(&mut *tx)
            .await?;

        for file in files {
            let size_bytes = file.content.as_ref().map(|c| c.len() as i64).unwrap_or(0);
            sqlx::query(
                r#"
                INSERT INTO memory_files (memory_id, path, content, is_directory, size_bytes, content_hash)
                VALUES ($1, $2, $3, $4, $5, $6)
                "#,
            )
            .bind(memory_id)
            .bind(&file.path)
            .bind(&file.content)
            .bind(file.is_directory)
            .bind(size_bytes)
            .bind(&file.content_hash)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(memory)
    }

    pub async fn fail_memory_sync(
        &self,
        memory_id: Uuid,
        claimed_at: chrono::DateTime<chrono::Utc>,
        error: &str,
    ) -> Result<Option<MemoryRow>> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            UPDATE memories
            SET sync_status = 'failed',
                last_sync_error = $2,
                updated_at = NOW()
            WHERE id = $1
              AND updated_at = $3
              AND sync_status = 'syncing'
              AND status = 'active'
              AND source_type != 'manual'
            RETURNING id, org_id, public_id, name, description, scope, owner_agent_id, owner_user_id, source_type, source_config, is_readonly, sync_status, last_synced_at, last_sync_error, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(memory_id)
        .bind(error)
        .bind(claimed_at)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_all_memory_files(&self, memory_id: Uuid) -> Result<Vec<MemoryFileRow>> {
        let rows = sqlx::query_as::<_, MemoryFileRow>(
            r#"
            SELECT id, memory_id, path, content, is_directory, size_bytes, content_hash, created_at, updated_at
            FROM memory_files
            WHERE memory_id = $1
            ORDER BY path ASC
            "#,
        )
        .bind(memory_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // ============================================
    // Memory Files CRUD (virtual filesystem under a Memory)
    // ============================================

    pub async fn create_memory_file(
        &self,
        memory_id: Uuid,
        input: CreateMemoryFileRow,
    ) -> Result<MemoryFileRow> {
        let size_bytes = input.content.as_ref().map(|c| c.len() as i64).unwrap_or(0);

        let row = sqlx::query_as::<_, MemoryFileRow>(
            r#"
            INSERT INTO memory_files (memory_id, path, content, is_directory, size_bytes, content_hash)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, memory_id, path, content, is_directory, size_bytes, content_hash, created_at, updated_at
            "#,
        )
        .bind(memory_id)
        .bind(&input.path)
        .bind(&input.content)
        .bind(input.is_directory)
        .bind(size_bytes)
        .bind(&input.content_hash)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_memory_file(
        &self,
        memory_id: Uuid,
        path: &str,
    ) -> Result<Option<MemoryFileRow>> {
        let row = sqlx::query_as::<_, MemoryFileRow>(
            r#"
            SELECT id, memory_id, path, content, is_directory, size_bytes, content_hash, created_at, updated_at
            FROM memory_files
            WHERE memory_id = $1 AND path = $2
            "#,
        )
        .bind(memory_id)
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_memory_file_info(
        &self,
        memory_id: Uuid,
        path: &str,
    ) -> Result<Option<MemoryFileInfoRow>> {
        let row = sqlx::query_as::<_, MemoryFileInfoRow>(
            r#"
            SELECT id, memory_id, path, is_directory, size_bytes, content_hash, created_at, updated_at
            FROM memory_files
            WHERE memory_id = $1 AND path = $2
            "#,
        )
        .bind(memory_id)
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_memory_files(
        &self,
        memory_id: Uuid,
        parent_path: &str,
    ) -> Result<Vec<MemoryFileInfoRow>> {
        let pattern = if parent_path == "/" {
            "^/[^/]+$".to_string()
        } else {
            format!("^{}/[^/]+$", regex::escape(parent_path))
        };

        let rows = sqlx::query_as::<_, MemoryFileInfoRow>(
            r#"
            SELECT id, memory_id, path, is_directory, size_bytes, content_hash, created_at, updated_at
            FROM memory_files
            WHERE memory_id = $1 AND path ~ $2
            ORDER BY is_directory DESC, path ASC
            "#,
        )
        .bind(memory_id)
        .bind(&pattern)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_memory_file(
        &self,
        memory_id: Uuid,
        path: &str,
        input: UpdateMemoryFile,
    ) -> Result<Option<MemoryFileRow>> {
        let size_bytes = input.content.as_ref().map(|c| c.len() as i64);
        let (hash_set_explicit, hash_value): (bool, Option<String>) = match input.content_hash {
            Some(inner) => (true, inner),
            None => (false, None),
        };

        let row = sqlx::query_as::<_, MemoryFileRow>(
            r#"
            UPDATE memory_files
            SET
                content = COALESCE($3, content),
                size_bytes = COALESCE($4, size_bytes),
                content_hash = CASE WHEN $5 THEN $6 ELSE content_hash END,
                updated_at = NOW()
            WHERE memory_id = $1 AND path = $2 AND is_directory = FALSE
            RETURNING id, memory_id, path, content, is_directory, size_bytes, content_hash, created_at, updated_at
            "#,
        )
        .bind(memory_id)
        .bind(path)
        .bind(&input.content)
        .bind(size_bytes)
        .bind(hash_set_explicit)
        .bind(hash_value)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_memory_file(&self, memory_id: Uuid, path: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM memory_files WHERE memory_id = $1 AND path = $2")
            .bind(memory_id)
            .bind(path)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_memory_file_recursive(&self, memory_id: Uuid, path: &str) -> Result<u64> {
        let pattern = if path == "/" {
            "^/".to_string()
        } else {
            format!("^{}(/|$)", regex::escape(path))
        };

        let result = sqlx::query("DELETE FROM memory_files WHERE memory_id = $1 AND path ~ $2")
            .bind(memory_id)
            .bind(&pattern)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    pub async fn grep_memory_files(
        &self,
        memory_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
        max_file_bytes: i64,
    ) -> Result<Vec<MemoryFileInfoRow>> {
        // The service applies the shared glob matcher. Avoid treating glob syntax
        // as a PostgreSQL regex and, critically, avoid scanning content until the
        // service has narrowed these metadata candidates by path.
        let rows = if path_pattern.is_some() {
            sqlx::query_as::<_, MemoryFileInfoRow>(
                r#"
                SELECT id, memory_id, path, is_directory, size_bytes, content_hash, created_at, updated_at
                FROM memory_files
                WHERE memory_id = $1
                    AND is_directory = FALSE
                    AND size_bytes <= $2
                ORDER BY path ASC
                "#,
            )
            .bind(memory_id)
            .bind(max_file_bytes)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, MemoryFileInfoRow>(
                r#"
                SELECT id, memory_id, path, is_directory, size_bytes, content_hash, created_at, updated_at
                FROM memory_files
                WHERE memory_id = $1
                    AND is_directory = FALSE
                    AND size_bytes <= $2
                    AND convert_from(content, 'UTF8') ~ $3
                ORDER BY path ASC
                "#,
            )
            .bind(memory_id)
            .bind(max_file_bytes)
            .bind(pattern)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }

    pub async fn memory_file_exists(&self, memory_id: Uuid, path: &str) -> Result<bool> {
        let result: Option<(bool,)> =
            sqlx::query_as("SELECT TRUE FROM memory_files WHERE memory_id = $1 AND path = $2")
                .bind(memory_id)
                .bind(path)
                .fetch_optional(&self.pool)
                .await?;

        Ok(result.is_some())
    }

    pub async fn memory_directory_has_children(&self, memory_id: Uuid, path: &str) -> Result<bool> {
        let pattern = if path == "/" {
            "^/[^/]+".to_string()
        } else {
            format!("^{}/[^/]+", regex::escape(path))
        };

        let result: Option<(bool,)> = sqlx::query_as(
            "SELECT TRUE FROM memory_files WHERE memory_id = $1 AND path ~ $2 LIMIT 1",
        )
        .bind(memory_id)
        .bind(&pattern)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }
}
