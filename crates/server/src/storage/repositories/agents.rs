// PostgreSQL repository: Agents (configuration for agentic loop), Agent Capabilities

use super::super::models::*;
use super::Database;
use super::build_search_sql;
use anyhow::Result;
use everruns_core::AgentIdentityId;
use everruns_core::typed_id::AgentId;
use std::collections::HashMap;
use uuid::Uuid;

impl Database {
    // ============================================
    // Agents (configuration for agentic loop)
    // ============================================

    pub async fn create_agent(&self, org_id: i64, input: CreateAgentRow) -> Result<AgentRow> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            INSERT INTO agents (org_id, public_id, name, display_name, description, system_prompt, default_model_id, harness_id, tags, initial_files, tools, mcp_servers, network_access, max_iterations, parallel_tool_calls, is_built_in, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, 'active')
            RETURNING id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, harness_id, harness_source, agent_identity_id, default_version_id, forked_from_agent_id, forked_from_version_id, root_agent_id, tags, status, is_built_in, created_at, updated_at, archived_at, deleted_at, initial_files, tools, mcp_servers, network_access, max_iterations, parallel_tool_calls,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id)
        .bind(input.harness_id)
        .bind(&input.tags)
        .bind(&input.initial_files)
        .bind(&input.tools)
        .bind(&input.mcp_servers)
        .bind(&input.network_access)
        .bind(input.max_iterations)
        .bind(input.parallel_tool_calls)
        .bind(input.is_built_in)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create or update agent with a specific internal ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_agent_with_id(
        &self,
        org_id: i64,
        id: AgentId,
        input: CreateAgentRow,
    ) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            INSERT INTO agents (id, org_id, public_id, name, display_name, description, system_prompt, default_model_id, harness_id, tags, initial_files, tools, mcp_servers, network_access, max_iterations, parallel_tool_calls, is_built_in, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, 'active')
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                display_name = EXCLUDED.display_name,
                description = EXCLUDED.description,
                system_prompt = EXCLUDED.system_prompt,
                harness_id = EXCLUDED.harness_id,
                tags = EXCLUDED.tags,
                initial_files = EXCLUDED.initial_files,
                tools = EXCLUDED.tools,
                mcp_servers = EXCLUDED.mcp_servers,
                network_access = EXCLUDED.network_access,
                max_iterations = EXCLUDED.max_iterations,
                parallel_tool_calls = EXCLUDED.parallel_tool_calls,
                updated_at = NOW()
            WHERE
                agents.name IS DISTINCT FROM EXCLUDED.name
                OR agents.display_name IS DISTINCT FROM EXCLUDED.display_name
                OR agents.description IS DISTINCT FROM EXCLUDED.description
                OR agents.system_prompt IS DISTINCT FROM EXCLUDED.system_prompt
                OR agents.harness_id IS DISTINCT FROM EXCLUDED.harness_id
                OR agents.tags IS DISTINCT FROM EXCLUDED.tags
                OR agents.initial_files IS DISTINCT FROM EXCLUDED.initial_files
                OR agents.tools IS DISTINCT FROM EXCLUDED.tools
                OR agents.mcp_servers IS DISTINCT FROM EXCLUDED.mcp_servers
                OR agents.network_access IS DISTINCT FROM EXCLUDED.network_access
                OR agents.max_iterations IS DISTINCT FROM EXCLUDED.max_iterations
                OR agents.parallel_tool_calls IS DISTINCT FROM EXCLUDED.parallel_tool_calls
            RETURNING id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, harness_id, harness_source, agent_identity_id, default_version_id, forked_from_agent_id, forked_from_version_id, root_agent_id, tags, status, is_built_in, created_at, updated_at, archived_at, deleted_at, initial_files, tools, mcp_servers, network_access, max_iterations, parallel_tool_calls,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd
            "#,
        )
        .bind(id.uuid())
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id.map(|m| m.uuid()))
        .bind(input.harness_id.uuid())
        .bind(&input.tags)
        .bind(&input.initial_files)
        .bind(&input.tools)
        .bind(&input.mcp_servers)
        .bind(&input.network_access)
        .bind(input.max_iterations)
        .bind(input.parallel_tool_calls)
        .bind(input.is_built_in)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent(&self, org_id: i64, id: AgentId) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, harness_id, harness_source, agent_identity_id, default_version_id, forked_from_agent_id, forked_from_version_id, root_agent_id, tags, status, is_built_in, created_at, updated_at, archived_at, deleted_at, initial_files, tools, mcp_servers, network_access, max_iterations, parallel_tool_calls,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd
            FROM agents
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id.uuid())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agents_by_ids(&self, org_id: i64, ids: &[AgentId]) -> Result<Vec<AgentRow>> {
        let ids: Vec<Uuid> = ids.iter().map(|id| id.uuid()).collect();
        Ok(sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, harness_id, harness_source, agent_identity_id, default_version_id, forked_from_agent_id, forked_from_version_id, root_agent_id, tags, status, is_built_in, created_at, updated_at, archived_at, deleted_at, initial_files, tools, mcp_servers, network_access, max_iterations, parallel_tool_calls,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd
            FROM agents
            WHERE org_id = $1 AND id = ANY($2)
            "#,
        )
        .bind(org_id)
        .bind(&ids)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Look up the owning org for an agent by its public_id, without scoping
    /// to a caller-supplied org. Used exclusively by the cross-org resolver
    /// (see knowledge/security/multitenancy.md). Callers MUST gate the result on user
    /// membership before revealing it — this method does NOT.
    pub async fn get_agent_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT org_id FROM agents WHERE public_id = $1 LIMIT 1")
                .bind(public_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(org_id,)| org_id))
    }

    pub async fn get_agent_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, harness_id, harness_source, agent_identity_id, default_version_id, forked_from_agent_id, forked_from_version_id, root_agent_id, tags, status, is_built_in, created_at, updated_at, archived_at, deleted_at, initial_files, tools, mcp_servers, network_access, max_iterations, parallel_tool_calls,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd
            FROM agents
            WHERE org_id = $1 AND public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_agents(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
        pagination: crate::api::common::Pagination,
    ) -> Result<(Vec<AgentRow>, u32)> {
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
        let param_idx = 1 + patterns.len();

        // Count query
        let count_sql = format!(
            "SELECT COUNT(*) as count FROM agents WHERE org_id = $1{status_sql}{search_sql}"
        );
        let mut count_query =
            sqlx::query_as::<_, (i64,)>(sqlx::AssertSqlSafe(count_sql.as_str())).bind(org_id);
        for pat in &patterns {
            count_query = count_query.bind(pat);
        }
        let total: (i64,) = count_query.fetch_one(&self.pool).await?;

        // Data query with pagination
        let limit_idx = param_idx + 1;
        let offset_idx = param_idx + 2;
        let sql = format!(
            r#"SELECT id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, harness_id, harness_source, agent_identity_id, default_version_id, forked_from_agent_id, forked_from_version_id, root_agent_id, tags, status, is_built_in, created_at, updated_at, archived_at, deleted_at, initial_files, tools, mcp_servers, network_access, max_iterations, parallel_tool_calls,
                       total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd
                FROM agents
                WHERE org_id = $1{status_sql}{search_sql}
                ORDER BY created_at DESC
                LIMIT ${limit_idx} OFFSET ${offset_idx}"#
        );
        let mut query =
            sqlx::query_as::<_, AgentRow>(sqlx::AssertSqlSafe(sql.as_str())).bind(org_id);
        for pat in &patterns {
            query = query.bind(pat);
        }
        let rows = query
            .bind(pagination.limit as i64)
            .bind(pagination.offset as i64)
            .fetch_all(&self.pool)
            .await?;

        Ok((rows, total.0 as u32))
    }

    /// Count non-deleted agents in an org (for resource limits).
    /// Includes active and archived; excludes soft-deleted rows.
    /// Count agents against the per-org limit.
    ///
    /// Built-in agents are platform-supplied, not authored by the org, so they
    /// do not consume the org's quota — same rule as built-in harnesses.
    pub async fn count_agents_for_org(&self, org_id: i64) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM agents WHERE org_id = $1 AND status != 'deleted' AND NOT is_built_in",
        )
        .bind(org_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    pub async fn get_agent_by_name(&self, org_id: i64, name: &str) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, harness_id, harness_source, agent_identity_id, default_version_id, forked_from_agent_id, forked_from_version_id, root_agent_id, tags, status, is_built_in, created_at, updated_at, archived_at, deleted_at, initial_files, tools, mcp_servers, network_access, max_iterations, parallel_tool_calls,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd
            FROM agents
            WHERE org_id = $1 AND name = $2 AND status != 'deleted'
            "#,
        )
        .bind(org_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Flag an agent as platform-supplied. Idempotent; used by org bootstrap.
    pub async fn mark_agent_built_in(&self, org_id: i64, id: AgentId) -> Result<()> {
        sqlx::query("UPDATE agents SET is_built_in = TRUE WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_agent(
        &self,
        org_id: i64,
        id: AgentId,
        input: UpdateAgent,
    ) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            UPDATE agents
            SET
                name = COALESCE($3, name),
                display_name = COALESCE($4, display_name),
                description = COALESCE($5, description),
                system_prompt = COALESCE($6, system_prompt),
                default_model_id = COALESCE($7, default_model_id),
                harness_id = COALESCE($8, harness_id),
                harness_source = COALESCE($24, harness_source),
                tags = COALESCE($9, tags),
                status = COALESCE($10, status),
                initial_files = COALESCE($11, initial_files),
                tools = COALESCE($12, tools),
                mcp_servers = COALESCE($13, mcp_servers),
                network_access = CASE WHEN $14 THEN $15 ELSE network_access END,
                max_iterations = CASE WHEN $16 THEN $17 ELSE max_iterations END,
                default_version_id = COALESCE($18, default_version_id),
                forked_from_agent_id = COALESCE($19, forked_from_agent_id),
                forked_from_version_id = COALESCE($20, forked_from_version_id),
                root_agent_id = COALESCE($21, root_agent_id),
                parallel_tool_calls = CASE WHEN $22 THEN $23 ELSE parallel_tool_calls END,
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, harness_id, harness_source, agent_identity_id, default_version_id, forked_from_agent_id, forked_from_version_id, root_agent_id, tags, status, is_built_in, created_at, updated_at, archived_at, deleted_at, initial_files, tools, mcp_servers, network_access, max_iterations, parallel_tool_calls,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd
            "#,
        )
        .bind(org_id)
        .bind(id.uuid())
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id.map(|m| m.uuid()))
        .bind(input.harness_id.map(|h| h.uuid()))
        .bind(&input.tags)
        .bind(&input.status)
        .bind(&input.initial_files)
        .bind(&input.tools)
        .bind(&input.mcp_servers)
        .bind(input.network_access.is_some())
        .bind(input.network_access.flatten())
        .bind(input.max_iterations.is_some())
        .bind(input.max_iterations.flatten())
        .bind(input.default_version_id)
        .bind(input.forked_from_agent_id)
        .bind(input.forked_from_version_id)
        .bind(input.root_agent_id)
        .bind(input.parallel_tool_calls.is_some())
        .bind(input.parallel_tool_calls.flatten())
        .bind(&input.harness_source)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Link an agent to its lazily-created identity, but only when it is not
    /// already linked (EVE-758). Returns `true` when this call performed the
    /// assignment and `false` when the agent was already linked (e.g. a
    /// concurrent first-fire won the race, or an identity was set explicitly),
    /// so callers never override an existing link.
    pub async fn set_agent_identity_id(
        &self,
        org_id: i64,
        id: AgentId,
        agent_identity_id: AgentIdentityId,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE agents
            SET agent_identity_id = $3, updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND agent_identity_id IS NULL
            "#,
        )
        .bind(org_id)
        .bind(id.uuid())
        .bind(agent_identity_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn has_agent_with_identity(
        &self,
        org_id: i64,
        agent_identity_id: AgentIdentityId,
    ) -> Result<bool> {
        let exists: Option<(i32,)> = sqlx::query_as(
            r#"
            SELECT 1
            FROM agents
            WHERE org_id = $1
              AND agent_identity_id = $2
              AND status != 'deleted'
            LIMIT 1
            "#,
        )
        .bind(org_id)
        .bind(agent_identity_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(exists.is_some())
    }

    pub async fn delete_agent(&self, org_id: i64, id: AgentId) -> Result<bool> {
        // Archive instead of hard delete
        let result = sqlx::query(
            r#"
            UPDATE agents
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

    pub async fn destroy_agent(&self, org_id: i64, id: AgentId) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE agents
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

    /// Upsert agent by public_id. Returns (row, was_created).
    pub async fn upsert_agent(
        &self,
        org_id: i64,
        input: CreateAgentRow,
    ) -> Result<(AgentRow, bool)> {
        // Use CTE to detect insert vs update
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            WITH existing AS (
                SELECT id FROM agents WHERE org_id = $1 AND public_id = $2
            )
            INSERT INTO agents (org_id, public_id, name, display_name, description, system_prompt, default_model_id, harness_id, tags, initial_files, tools, mcp_servers, network_access, max_iterations, parallel_tool_calls, is_built_in, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, 'active')
            ON CONFLICT (org_id, public_id) DO UPDATE SET
                name = EXCLUDED.name,
                display_name = EXCLUDED.display_name,
                description = EXCLUDED.description,
                system_prompt = EXCLUDED.system_prompt,
                default_model_id = EXCLUDED.default_model_id,
                harness_id = EXCLUDED.harness_id,
                tags = EXCLUDED.tags,
                initial_files = EXCLUDED.initial_files,
                tools = EXCLUDED.tools,
                mcp_servers = EXCLUDED.mcp_servers,
                network_access = EXCLUDED.network_access,
                max_iterations = EXCLUDED.max_iterations,
                parallel_tool_calls = EXCLUDED.parallel_tool_calls,
                status = 'active',
                updated_at = NOW()
            RETURNING id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, harness_id, harness_source, agent_identity_id, default_version_id, forked_from_agent_id, forked_from_version_id, root_agent_id, tags, status, is_built_in, created_at, updated_at, archived_at, deleted_at, initial_files, tools, mcp_servers, network_access, max_iterations, parallel_tool_calls,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id)
        .bind(input.harness_id)
        .bind(&input.tags)
        .bind(&input.initial_files)
        .bind(&input.tools)
        .bind(&input.mcp_servers)
        .bind(&input.network_access)
        .bind(input.max_iterations)
        .bind(input.parallel_tool_calls)
        .bind(input.is_built_in)
        .fetch_one(&self.pool)
        .await?;

        // Detect if insert or update: if created_at == updated_at, it was a fresh insert
        let was_created = row.created_at == row.updated_at;
        Ok((row, was_created))
    }

    /// Upsert agent by name within org. Returns (row, was_created).
    pub async fn upsert_agent_by_name(
        &self,
        org_id: i64,
        input: CreateAgentRow,
    ) -> Result<(AgentRow, bool)> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            INSERT INTO agents (org_id, public_id, name, display_name, description, system_prompt, default_model_id, harness_id, tags, initial_files, tools, mcp_servers, network_access, max_iterations, parallel_tool_calls, is_built_in, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, 'active')
            ON CONFLICT (org_id, name) WHERE status != 'deleted' DO UPDATE SET
                display_name = EXCLUDED.display_name,
                description = EXCLUDED.description,
                system_prompt = EXCLUDED.system_prompt,
                default_model_id = EXCLUDED.default_model_id,
                harness_id = EXCLUDED.harness_id,
                tags = EXCLUDED.tags,
                initial_files = EXCLUDED.initial_files,
                tools = EXCLUDED.tools,
                mcp_servers = EXCLUDED.mcp_servers,
                network_access = EXCLUDED.network_access,
                max_iterations = EXCLUDED.max_iterations,
                parallel_tool_calls = EXCLUDED.parallel_tool_calls,
                status = 'active',
                updated_at = NOW()
            RETURNING id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, harness_id, harness_source, agent_identity_id, default_version_id, forked_from_agent_id, forked_from_version_id, root_agent_id, tags, status, is_built_in, created_at, updated_at, archived_at, deleted_at, initial_files, tools, mcp_servers, network_access, max_iterations, parallel_tool_calls,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id)
        .bind(input.harness_id)
        .bind(&input.tags)
        .bind(&input.initial_files)
        .bind(&input.tools)
        .bind(&input.mcp_servers)
        .bind(&input.network_access)
        .bind(input.max_iterations)
        .bind(input.parallel_tool_calls)
        .bind(input.is_built_in)
        .fetch_one(&self.pool)
        .await?;

        let was_created = row.created_at == row.updated_at;
        Ok((row, was_created))
    }

    /// Get agent public_id from internal UUID (for session responses)
    pub async fn get_agent_public_id(&self, org_id: i64, id: AgentId) -> Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT public_id FROM agents WHERE org_id = $1 AND id = $2")
                .bind(org_id)
                .bind(id.uuid())
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.map(|r| r.0))
    }

    pub async fn create_agent_version(
        &self,
        input: CreateAgentVersionRow,
    ) -> Result<AgentVersionRow> {
        Ok(sqlx::query_as::<_, AgentVersionRow>(
            r#"
            INSERT INTO agent_versions (
                id, public_id, org_id, agent_id, version_number,
                semver_major, semver_minor, semver_patch, version, is_published,
                parent_version_id, source_version_id, created_by_principal_id,
                change_kind, summary, config_hash, authored_config, resolved_config
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
            RETURNING id, public_id, org_id, agent_id, version_number,
                semver_major, semver_minor, semver_patch, version, is_published,
                parent_version_id, source_version_id, created_by_principal_id,
                change_kind, summary, config_hash, authored_config, resolved_config, created_at
            "#,
        )
        .bind(input.id)
        .bind(input.public_id)
        .bind(input.org_id)
        .bind(input.agent_id)
        .bind(input.version_number)
        .bind(input.semver_major)
        .bind(input.semver_minor)
        .bind(input.semver_patch)
        .bind(input.version)
        .bind(input.is_published)
        .bind(input.parent_version_id)
        .bind(input.source_version_id)
        .bind(input.created_by_principal_id)
        .bind(input.change_kind)
        .bind(input.summary)
        .bind(input.config_hash)
        .bind(input.authored_config)
        .bind(input.resolved_config)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_agent_versions(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> Result<Vec<AgentVersionRow>> {
        Ok(sqlx::query_as::<_, AgentVersionRow>(
            r#"
            SELECT id, public_id, org_id, agent_id, version_number,
                semver_major, semver_minor, semver_patch, version, is_published,
                parent_version_id, source_version_id, created_by_principal_id,
                change_kind, summary, config_hash, authored_config, resolved_config, created_at
            FROM agent_versions
            WHERE org_id = $1 AND agent_id = $2
            ORDER BY version_number DESC
            "#,
        )
        .bind(org_id)
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_agent_version(
        &self,
        org_id: i64,
        id: everruns_core::AgentVersionId,
    ) -> Result<Option<AgentVersionRow>> {
        Ok(sqlx::query_as::<_, AgentVersionRow>(
            r#"
            SELECT id, public_id, org_id, agent_id, version_number,
                semver_major, semver_minor, semver_patch, version, is_published,
                parent_version_id, source_version_id, created_by_principal_id,
                change_kind, summary, config_hash, authored_config, resolved_config, created_at
            FROM agent_versions
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id.uuid())
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn get_latest_agent_version(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> Result<Option<AgentVersionRow>> {
        Ok(sqlx::query_as::<_, AgentVersionRow>(
            r#"
            SELECT id, public_id, org_id, agent_id, version_number,
                semver_major, semver_minor, semver_patch, version, is_published,
                parent_version_id, source_version_id, created_by_principal_id,
                change_kind, summary, config_hash, authored_config, resolved_config, created_at
            FROM agent_versions
            WHERE org_id = $1 AND agent_id = $2 AND is_published = TRUE
            ORDER BY version_number DESC
            LIMIT 1
            "#,
        )
        .bind(org_id)
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn get_latest_agent_snapshot(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> Result<Option<AgentVersionRow>> {
        Ok(sqlx::query_as::<_, AgentVersionRow>(
            r#"
            SELECT id, public_id, org_id, agent_id, version_number,
                semver_major, semver_minor, semver_patch, version, is_published,
                parent_version_id, source_version_id, created_by_principal_id,
                change_kind, summary, config_hash, authored_config, resolved_config, created_at
            FROM agent_versions
            WHERE org_id = $1 AND agent_id = $2
            ORDER BY version_number DESC
            LIMIT 1
            "#,
        )
        .bind(org_id)
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn prune_agent_auto_snapshots(
        &self,
        org_id: i64,
        agent_id: AgentId,
        keep: i64,
    ) -> Result<u64> {
        let keep = keep.max(0);
        let result = sqlx::query(
            r#"
            DELETE FROM agent_versions
            WHERE id IN (
                SELECT id
                FROM agent_versions
                WHERE org_id = $1
                    AND agent_id = $2
                    AND is_published = FALSE
                    AND change_kind = 'auto'
                ORDER BY version_number DESC
                OFFSET $3
            )
            "#,
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(keep)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    // ============================================
    // Agent Capabilities
    // ============================================

    /// Get capabilities for an agent, ordered by position
    pub async fn get_agent_capabilities(&self, agent_id: Uuid) -> Result<Vec<AgentCapabilityRow>> {
        let rows = sqlx::query_as::<_, AgentCapabilityRow>(
            r#"
            SELECT id, agent_id, capability_id, position, config, created_at
            FROM agent_capabilities
            WHERE agent_id = $1
            ORDER BY position ASC
            "#,
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn get_agent_capabilities_by_agent_ids(
        &self,
        org_id: i64,
        agent_ids: &[AgentId],
    ) -> Result<Vec<AgentCapabilityRow>> {
        let agent_ids: Vec<Uuid> = agent_ids.iter().map(|id| id.uuid()).collect();
        Ok(sqlx::query_as::<_, AgentCapabilityRow>(
            r#"
            SELECT ac.id, ac.agent_id, ac.capability_id, ac.position, ac.config, ac.created_at
            FROM agent_capabilities ac
            JOIN agents a ON a.id = ac.agent_id
            WHERE a.org_id = $1 AND ac.agent_id = ANY($2)
            ORDER BY ac.agent_id, ac.position ASC
            "#,
        )
        .bind(org_id)
        .bind(&agent_ids)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Set capabilities for an agent (replaces existing capabilities)
    /// capabilities: list of (capability_id, position, config) tuples
    pub async fn set_agent_capabilities(
        &self,
        agent_id: Uuid,
        capabilities: Vec<(String, i32, serde_json::Value)>,
    ) -> Result<Vec<AgentCapabilityRow>> {
        // Start a transaction
        let mut tx = self.pool.begin().await?;

        // Delete existing capabilities for this agent
        sqlx::query("DELETE FROM agent_capabilities WHERE agent_id = $1")
            .bind(agent_id)
            .execute(&mut *tx)
            .await?;

        // Insert new capabilities
        for (capability_id, position, config) in &capabilities {
            sqlx::query(
                r#"
                INSERT INTO agent_capabilities (agent_id, capability_id, position, config)
                VALUES ($1, $2, $3, $4)
                "#,
            )
            .bind(agent_id)
            .bind(capability_id)
            .bind(position)
            .bind(config)
            .execute(&mut *tx)
            .await?;
        }

        // Commit transaction
        tx.commit().await?;

        // Return the new capabilities
        self.get_agent_capabilities(agent_id).await
    }

    /// Add a single capability to an agent
    pub async fn add_agent_capability(
        &self,
        input: CreateAgentCapabilityRow,
    ) -> Result<AgentCapabilityRow> {
        let row = sqlx::query_as::<_, AgentCapabilityRow>(
            r#"
            INSERT INTO agent_capabilities (agent_id, capability_id, position, config)
            VALUES ($1, $2, $3, $4)
            RETURNING id, agent_id, capability_id, position, config, created_at
            "#,
        )
        .bind(input.agent_id)
        .bind(&input.capability_id)
        .bind(input.position)
        .bind(&input.config)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Remove a capability from an agent
    pub async fn remove_agent_capability(
        &self,
        agent_id: Uuid,
        capability_id: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM agent_capabilities WHERE agent_id = $1 AND capability_id = $2",
        )
        .bind(agent_id)
        .bind(capability_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Count how many active agents reference each capability ID, scoped to an org.
    /// Skips agents that are archived or deleted.
    pub async fn count_agent_capability_references(
        &self,
        org_id: i64,
    ) -> Result<HashMap<String, u64>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT ac.capability_id, COUNT(*) AS count
            FROM agent_capabilities ac
            JOIN agents a ON a.id = ac.agent_id
            WHERE a.org_id = $1 AND a.status = 'active'
            GROUP BY ac.capability_id
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

    /// Count how many active agents reference a single capability ID, scoped to an org.
    pub async fn count_agents_for_capability(
        &self,
        org_id: i64,
        capability_id: &str,
    ) -> Result<u64> {
        let (count,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM agent_capabilities ac
            JOIN agents a ON a.id = ac.agent_id
            WHERE a.org_id = $1 AND a.status = 'active' AND ac.capability_id = $2
            "#,
        )
        .bind(org_id)
        .bind(capability_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count.max(0) as u64)
    }
}
