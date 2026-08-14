// PostgreSQL repository: Harnesses (base configuration for sessions), Harness Capabilities

use super::super::models::*;
use super::Database;
use super::build_search_sql;
use anyhow::Result;
use everruns_provider::typed_id::HarnessId;
use std::collections::HashMap;
use uuid::Uuid;

impl Database {
    // ============================================
    // Harnesses (base configuration for sessions)
    // ============================================

    pub async fn create_harness(&self, org_id: i64, input: CreateHarnessRow) -> Result<HarnessRow> {
        let row = sqlx::query_as::<_, HarnessRow>(
            r#"
            INSERT INTO harnesses (org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, mcp_servers, network_access, embedder_metadata, is_built_in, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, 'active')
            RETURNING id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, mcp_servers, network_access, embedder_metadata, is_built_in, status, created_at, updated_at, archived_at, deleted_at
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
        .bind(&input.mcp_servers)
        .bind(&input.network_access)
        .bind(&input.embedder_metadata)
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
            INSERT INTO harnesses (id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, mcp_servers, network_access, embedder_metadata, is_built_in, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, 'active')
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                display_name = EXCLUDED.display_name,
                description = EXCLUDED.description,
                system_prompt = EXCLUDED.system_prompt,
                parent_harness_id = EXCLUDED.parent_harness_id,
                tags = EXCLUDED.tags,
                initial_files = EXCLUDED.initial_files,
                mcp_servers = EXCLUDED.mcp_servers,
                network_access = EXCLUDED.network_access,
                embedder_metadata = EXCLUDED.embedder_metadata,
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
                OR harnesses.mcp_servers IS DISTINCT FROM EXCLUDED.mcp_servers
                OR harnesses.network_access IS DISTINCT FROM EXCLUDED.network_access
                OR harnesses.embedder_metadata IS DISTINCT FROM EXCLUDED.embedder_metadata
                OR harnesses.is_built_in IS DISTINCT FROM EXCLUDED.is_built_in
            RETURNING id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, mcp_servers, network_access, embedder_metadata, is_built_in, status, created_at, updated_at, archived_at, deleted_at
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
        .bind(&input.mcp_servers)
        .bind(&input.network_access)
        .bind(&input.embedder_metadata)
        .bind(input.is_built_in)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Look up the owning org for a harness by its public_id, without scoping
    /// to a caller-supplied org. See knowledge/security/multitenancy.md (Cross-Org Resource
    /// Resolution). Gating by membership happens at the resolver endpoint.
    pub async fn get_harness_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        let Ok(id) = public_id.parse::<HarnessId>() else {
            return Ok(None);
        };
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT org_id FROM harnesses WHERE id = $1 LIMIT 1")
                .bind(id.uuid())
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(org_id,)| org_id))
    }

    pub async fn get_harness(&self, org_id: i64, id: HarnessId) -> Result<Option<HarnessRow>> {
        let row = sqlx::query_as::<_, HarnessRow>(
            r#"
            SELECT id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, mcp_servers, network_access, embedder_metadata, is_built_in, status, created_at, updated_at, archived_at, deleted_at
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

    pub async fn get_harness_ancestry_by_ids(
        &self,
        org_id: i64,
        ids: &[HarnessId],
    ) -> Result<Vec<HarnessRow>> {
        let ids: Vec<Uuid> = ids.iter().map(|id| id.uuid()).collect();
        Ok(sqlx::query_as::<_, HarnessRow>(
            r#"
            WITH RECURSIVE ancestry_ids(id) AS (
                SELECT id
                FROM harnesses
                WHERE org_id = $1 AND id = ANY($2)
                UNION
                SELECT parent.id
                FROM harnesses parent
                JOIN harnesses child ON child.parent_harness_id = parent.id
                JOIN ancestry_ids ancestry ON ancestry.id = child.id
                WHERE parent.org_id = $1
            )
            SELECT h.id, h.org_id, h.name, h.display_name, h.description, h.system_prompt,
                   h.parent_harness_id, h.default_model_id, h.tags, h.status,
                   h.created_at, h.updated_at, h.archived_at, h.deleted_at,
                   h.initial_files, h.mcp_servers, h.network_access, h.embedder_metadata, h.is_built_in
            FROM harnesses h
            JOIN ancestry_ids ancestry ON ancestry.id = h.id
            "#,
        )
        .bind(org_id)
        .bind(&ids)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_harness_by_name(&self, org_id: i64, name: &str) -> Result<Option<HarnessRow>> {
        let row = sqlx::query_as::<_, HarnessRow>(
            r#"
            SELECT id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, mcp_servers, network_access, embedder_metadata, is_built_in, status, created_at, updated_at, archived_at, deleted_at
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
            "LOWER(COALESCE(display_name, name) || ' ' || name || ' ' || COALESCE(description, ''))",
            2,
        );
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status = 'active'"
        };
        let sql = format!(
            r#"SELECT id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, mcp_servers, network_access, embedder_metadata, is_built_in, status, created_at, updated_at, archived_at, deleted_at
                FROM harnesses
                WHERE org_id = $1{status_sql}{search_sql}
                ORDER BY created_at DESC"#
        );
        let mut query =
            sqlx::query_as::<_, HarnessRow>(sqlx::AssertSqlSafe(sql.as_str())).bind(org_id);
        for pat in &patterns {
            query = query.bind(pat);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    /// Count user-created harnesses in an org (for resource limits). Includes
    /// active and archived; excludes soft-deleted rows. Built-in harnesses are
    /// system-seeded into every org and undeletable, so they must not consume a
    /// user's (or SaaS plan's) per-org budget.
    pub async fn count_harnesses_for_org(&self, org_id: i64) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM harnesses WHERE org_id = $1 AND status != 'deleted' AND is_built_in = false",
        )
        .bind(org_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
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
                system_prompt = CASE WHEN $6 THEN $7 ELSE system_prompt END,
                parent_harness_id = CASE
                    WHEN $8 THEN $9
                    ELSE parent_harness_id
                END,
                default_model_id = COALESCE($10, default_model_id),
                tags = COALESCE($11, tags),
                initial_files = COALESCE($12, initial_files),
                mcp_servers = COALESCE($13, mcp_servers),
                network_access = CASE WHEN $14 THEN $15 ELSE network_access END,
                embedder_metadata = COALESCE($16, embedder_metadata),
                status = COALESCE($17, status),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, mcp_servers, network_access, embedder_metadata, is_built_in, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id.uuid())
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(input.system_prompt.is_some())
        .bind(input.system_prompt.clone().flatten())
        .bind(input.parent_harness_id.is_some())
        .bind(input.parent_harness_id.flatten().map(|id| id.uuid()))
        .bind(input.default_model_id.map(|m| m.uuid()))
        .bind(&input.tags)
        .bind(&input.initial_files)
        .bind(&input.mcp_servers)
        .bind(input.network_access.is_some())
        .bind(input.network_access.flatten())
        .bind(&input.embedder_metadata)
        .bind(&input.status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Release built-in flag on the harness with `name` in `org_id`, if it
    /// exists and is currently flagged as built-in. Used during reconciliation
    /// when a previously default-installed harness is moved to the example
    /// catalogue: existing rows are kept (so sessions and agents that
    /// reference them keep working) but become editable, org-owned harnesses.
    /// Returns true when a row was actually flipped.
    pub async fn release_built_in_harness(&self, org_id: i64, name: &str) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE harnesses
            SET is_built_in = false, updated_at = NOW()
            WHERE org_id = $1 AND name = $2 AND is_built_in = true
            "#,
        )
        .bind(org_id)
        .bind(name)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn list_child_harnesses(
        &self,
        org_id: i64,
        parent_id: HarnessId,
    ) -> Result<Vec<HarnessRow>> {
        Ok(sqlx::query_as::<_, HarnessRow>(
            r#"
            SELECT id, org_id, name, display_name, description, system_prompt, parent_harness_id, default_model_id, tags, initial_files, mcp_servers, network_access, embedder_metadata, is_built_in, status, created_at, updated_at, archived_at, deleted_at
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

    pub async fn get_harness_capabilities_by_harness_ids(
        &self,
        org_id: i64,
        harness_ids: &[HarnessId],
    ) -> Result<Vec<HarnessCapabilityRow>> {
        let harness_ids: Vec<Uuid> = harness_ids.iter().map(|id| id.uuid()).collect();
        Ok(sqlx::query_as::<_, HarnessCapabilityRow>(
            r#"
            SELECT hc.id, hc.harness_id, hc.capability_id, hc.position, hc.config, hc.created_at
            FROM harness_capabilities hc
            JOIN harnesses h ON h.id = hc.harness_id
            WHERE h.org_id = $1 AND hc.harness_id = ANY($2)
            ORDER BY hc.harness_id, hc.position ASC
            "#,
        )
        .bind(org_id)
        .bind(&harness_ids)
        .fetch_all(&self.pool)
        .await?)
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

    /// Count how many active harnesses reference each capability ID, scoped to an org.
    /// Skips harnesses that are archived or deleted.
    pub async fn count_harness_capability_references(
        &self,
        org_id: i64,
    ) -> Result<HashMap<String, u64>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT hc.capability_id, COUNT(*) AS count
            FROM harness_capabilities hc
            JOIN harnesses h ON h.id = hc.harness_id
            WHERE h.org_id = $1 AND h.status = 'active'
            GROUP BY hc.capability_id
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, c)| (id, c.max(0) as u64))
            .collect())
    }

    /// Count how many active harnesses reference a single capability ID, scoped to an org.
    pub async fn count_harnesses_for_capability(
        &self,
        org_id: i64,
        capability_id: &str,
    ) -> Result<u64> {
        let (count,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM harness_capabilities hc
            JOIN harnesses h ON h.id = hc.harness_id
            WHERE h.org_id = $1 AND h.status = 'active' AND hc.capability_id = $2
            "#,
        )
        .bind(org_id)
        .bind(capability_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count.max(0) as u64)
    }
}
