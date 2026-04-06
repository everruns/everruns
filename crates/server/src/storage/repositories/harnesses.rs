// PostgreSQL repository: Harnesses (base configuration for sessions), Harness Capabilities

use super::super::models::*;
use super::Database;
use super::build_search_sql;
use anyhow::Result;
use everruns_core::typed_id::HarnessId;
use uuid::Uuid;

impl Database {
    // ============================================
    // Harnesses (base configuration for sessions)
    // ============================================

    pub async fn create_harness(&self, org_id: i64, input: CreateHarnessRow) -> Result<HarnessRow> {
        let row = sqlx::query_as::<_, HarnessRow>(
            r#"
            INSERT INTO harnesses (org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, network_access, is_built_in, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'active')
            RETURNING id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, network_access, is_built_in, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.parent_harness_id.map(|id| id.uuid()))
        .bind(input.default_model_id.map(|m| m.uuid()))
        .bind(&input.tags)
        .bind(&input.initial_files)
        .bind(&input.network_access)
        .bind(input.is_built_in)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create harness with a specific ID, idempotent (ON CONFLICT DO NOTHING)
    /// Returns None if harness already exists with this ID
    /// Create or update harness with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_harness_with_id(
        &self,
        org_id: i64,
        id: HarnessId,
        input: CreateHarnessRow,
    ) -> Result<Option<HarnessRow>> {
        let row = sqlx::query_as::<_, HarnessRow>(
            r#"
            INSERT INTO harnesses (id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, network_access, is_built_in, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'active')
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                display_name = EXCLUDED.display_name,
                description = EXCLUDED.description,
                system_prompt = EXCLUDED.system_prompt,
                parent_harness_id = EXCLUDED.parent_harness_id,
                tags = EXCLUDED.tags,
                initial_files = EXCLUDED.initial_files,
                network_access = EXCLUDED.network_access,
                is_built_in = EXCLUDED.is_built_in,
                updated_at = NOW()
            WHERE
                harnesses.name IS DISTINCT FROM EXCLUDED.name
                OR harnesses.display_name IS DISTINCT FROM EXCLUDED.display_name
                OR harnesses.description IS DISTINCT FROM EXCLUDED.description
                OR harnesses.system_prompt IS DISTINCT FROM EXCLUDED.system_prompt
                OR harnesses.parent_harness_id IS DISTINCT FROM EXCLUDED.parent_harness_id
                OR harnesses.tags IS DISTINCT FROM EXCLUDED.tags
                OR harnesses.initial_files IS DISTINCT FROM EXCLUDED.initial_files
                OR harnesses.network_access IS DISTINCT FROM EXCLUDED.network_access
                OR harnesses.is_built_in IS DISTINCT FROM EXCLUDED.is_built_in
            RETURNING id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, network_access, is_built_in, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(id.uuid())
        .bind(org_id)
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.parent_harness_id.map(|id| id.uuid()))
        .bind(input.default_model_id.map(|m| m.uuid()))
        .bind(&input.tags)
        .bind(&input.initial_files)
        .bind(&input.network_access)
        .bind(input.is_built_in)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_harness(&self, org_id: i64, id: HarnessId) -> Result<Option<HarnessRow>> {
        let row = sqlx::query_as::<_, HarnessRow>(
            r#"
            SELECT id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, network_access, is_built_in, status, created_at, updated_at, archived_at, deleted_at
            FROM harnesses
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id.uuid())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_harness_by_name(&self, org_id: i64, name: &str) -> Result<Option<HarnessRow>> {
        let row = sqlx::query_as::<_, HarnessRow>(
            r#"
            SELECT id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, network_access, is_built_in, status, created_at, updated_at, archived_at, deleted_at
            FROM harnesses
            WHERE org_id = $1 AND name = $2 AND status != 'deleted'
            "#,
        )
        .bind(org_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_harnesses(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<HarnessRow>> {
        let (search_sql, patterns) = build_search_sql(
            search,
            "LOWER(display_name || ' ' || name || ' ' || COALESCE(description, ''))",
            2,
        );
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status = 'active'"
        };
        let sql = format!(
            r#"SELECT id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, network_access, is_built_in, status, created_at, updated_at, archived_at, deleted_at
                FROM harnesses
                WHERE org_id = $1{status_sql}{search_sql}
                ORDER BY created_at DESC"#
        );
        let mut query = sqlx::query_as::<_, HarnessRow>(&sql).bind(org_id);
        for pat in &patterns {
            query = query.bind(pat);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn update_harness(
        &self,
        org_id: i64,
        id: HarnessId,
        input: UpdateHarness,
    ) -> Result<Option<HarnessRow>> {
        let row = sqlx::query_as::<_, HarnessRow>(
            r#"
            UPDATE harnesses
            SET
                name = COALESCE($3, name),
                display_name = COALESCE($4, display_name),
                description = COALESCE($5, description),
                system_prompt = COALESCE($6, system_prompt),
                parent_harness_id = CASE
                    WHEN $7 THEN $8
                    ELSE parent_harness_id
                END,
                default_model_id = COALESCE($9, default_model_id),
                tags = COALESCE($10, tags),
                initial_files = COALESCE($11, initial_files),
                network_access = CASE WHEN $12 THEN $13 ELSE network_access END,
                status = COALESCE($14, status),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, network_access, is_built_in, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id.uuid())
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.parent_harness_id.is_some())
        .bind(input.parent_harness_id.flatten().map(|id| id.uuid()))
        .bind(input.default_model_id.map(|m| m.uuid()))
        .bind(&input.tags)
        .bind(&input.initial_files)
        .bind(input.network_access.is_some())
        .bind(input.network_access.flatten())
        .bind(&input.status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_child_harnesses(
        &self,
        org_id: i64,
        parent_id: HarnessId,
    ) -> Result<Vec<HarnessRow>> {
        Ok(sqlx::query_as::<_, HarnessRow>(
            r#"
            SELECT id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, network_access, is_built_in, status, created_at, updated_at, archived_at, deleted_at
            FROM harnesses
            WHERE org_id = $1 AND parent_harness_id = $2 AND status != 'deleted'
            ORDER BY created_at DESC
            "#,
        )
        .bind(org_id)
        .bind(parent_id.uuid())
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn delete_harness(&self, org_id: i64, id: HarnessId) -> Result<bool> {
        // Archive instead of hard delete
        let result = sqlx::query(
            r#"
            UPDATE harnesses
            SET status = 'archived', archived_at = COALESCE(archived_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status = 'active'
            "#,
        )
        .bind(org_id)
        .bind(id.uuid())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn destroy_harness(&self, org_id: i64, id: HarnessId) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE harnesses
            SET status = 'deleted', deleted_at = COALESCE(deleted_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status = 'archived'
            "#,
        )
        .bind(org_id)
        .bind(id.uuid())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Harness Capabilities
    // ============================================

    /// Get capabilities for a harness, ordered by position
    pub async fn get_harness_capabilities(
        &self,
        harness_id: Uuid,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        let rows = sqlx::query_as::<_, HarnessCapabilityRow>(
            r#"
            SELECT id, harness_id, capability_id, position, config, created_at
            FROM harness_capabilities
            WHERE harness_id = $1
            ORDER BY position ASC
            "#,
        )
        .bind(harness_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Set capabilities for a harness (replaces existing capabilities)
    /// capabilities: list of (capability_id, position, config) tuples
    pub async fn set_harness_capabilities(
        &self,
        harness_id: Uuid,
        capabilities: Vec<(String, i32, serde_json::Value)>,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM harness_capabilities WHERE harness_id = $1")
            .bind(harness_id)
            .execute(&mut *tx)
            .await?;

        for (capability_id, position, config) in &capabilities {
            sqlx::query(
                r#"
                INSERT INTO harness_capabilities (harness_id, capability_id, position, config)
                VALUES ($1, $2, $3, $4)
                "#,
            )
            .bind(harness_id)
            .bind(capability_id)
            .bind(position)
            .bind(config)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        self.get_harness_capabilities(harness_id).await
    }
}
