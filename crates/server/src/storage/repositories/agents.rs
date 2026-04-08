// PostgreSQL repository: Agents (configuration for agentic loop), Agent Capabilities

use super::super::models::*;
use super::Database;
use super::build_search_sql;
use anyhow::Result;
use everruns_core::typed_id::AgentId;
use uuid::Uuid;

impl Database {
    // ============================================
    // Agents (configuration for agentic loop)
    // ============================================

    pub async fn create_agent(&self, org_id: i64, input: CreateAgentRow) -> Result<AgentRow> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            INSERT INTO agents (org_id, public_id, name, display_name, description, system_prompt, default_model_id, tags, initial_files, tools, network_access, max_iterations, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'active')
            RETURNING id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, archived_at, deleted_at, initial_files, tools, network_access, max_iterations,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id)
        .bind(&input.tags)
        .bind(&input.initial_files)
        .bind(&input.tools)
        .bind(&input.network_access)
        .bind(input.max_iterations)
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
            INSERT INTO agents (id, org_id, public_id, name, display_name, description, system_prompt, default_model_id, tags, initial_files, tools, network_access, max_iterations, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, 'active')
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                display_name = EXCLUDED.display_name,
                description = EXCLUDED.description,
                system_prompt = EXCLUDED.system_prompt,
                tags = EXCLUDED.tags,
                initial_files = EXCLUDED.initial_files,
                tools = EXCLUDED.tools,
                network_access = EXCLUDED.network_access,
                max_iterations = EXCLUDED.max_iterations,
                updated_at = NOW()
            WHERE
                agents.name IS DISTINCT FROM EXCLUDED.name
                OR agents.display_name IS DISTINCT FROM EXCLUDED.display_name
                OR agents.description IS DISTINCT FROM EXCLUDED.description
                OR agents.system_prompt IS DISTINCT FROM EXCLUDED.system_prompt
                OR agents.tags IS DISTINCT FROM EXCLUDED.tags
                OR agents.initial_files IS DISTINCT FROM EXCLUDED.initial_files
                OR agents.tools IS DISTINCT FROM EXCLUDED.tools
                OR agents.network_access IS DISTINCT FROM EXCLUDED.network_access
                OR agents.max_iterations IS DISTINCT FROM EXCLUDED.max_iterations
            RETURNING id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, archived_at, deleted_at, initial_files, tools, network_access, max_iterations,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
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
        .bind(&input.tags)
        .bind(&input.initial_files)
        .bind(&input.tools)
        .bind(&input.network_access)
        .bind(input.max_iterations)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent(&self, org_id: i64, id: AgentId) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, archived_at, deleted_at, initial_files, tools, network_access, max_iterations,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
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

    pub async fn get_agent_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, archived_at, deleted_at, initial_files, tools, network_access, max_iterations,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
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
        let mut count_query = sqlx::query_as::<_, (i64,)>(&count_sql).bind(org_id);
        for pat in &patterns {
            count_query = count_query.bind(pat);
        }
        let total: (i64,) = count_query.fetch_one(&self.pool).await?;

        // Data query with pagination
        let limit_idx = param_idx + 1;
        let offset_idx = param_idx + 2;
        let sql = format!(
            r#"SELECT id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, archived_at, deleted_at, initial_files, tools, network_access, max_iterations,
                       total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
                FROM agents
                WHERE org_id = $1{status_sql}{search_sql}
                ORDER BY created_at DESC
                LIMIT ${limit_idx} OFFSET ${offset_idx}"#
        );
        let mut query = sqlx::query_as::<_, AgentRow>(&sql).bind(org_id);
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

    pub async fn get_agent_by_name(&self, org_id: i64, name: &str) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, archived_at, deleted_at, initial_files, tools, network_access, max_iterations,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
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
                tags = COALESCE($8, tags),
                status = COALESCE($9, status),
                initial_files = COALESCE($10, initial_files),
                tools = COALESCE($11, tools),
                network_access = CASE WHEN $12 THEN $13 ELSE network_access END,
                max_iterations = CASE WHEN $14 THEN $15 ELSE max_iterations END,
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, archived_at, deleted_at, initial_files, tools, network_access, max_iterations,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            "#,
        )
        .bind(org_id)
        .bind(id.uuid())
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id.map(|m| m.uuid()))
        .bind(&input.tags)
        .bind(&input.status)
        .bind(&input.initial_files)
        .bind(&input.tools)
        .bind(input.network_access.is_some())
        .bind(input.network_access.flatten())
        .bind(input.max_iterations.is_some())
        .bind(input.max_iterations.flatten())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
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
            INSERT INTO agents (org_id, public_id, name, display_name, description, system_prompt, default_model_id, tags, initial_files, tools, network_access, max_iterations, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'active')
            ON CONFLICT (org_id, public_id) DO UPDATE SET
                name = EXCLUDED.name,
                display_name = EXCLUDED.display_name,
                description = EXCLUDED.description,
                system_prompt = EXCLUDED.system_prompt,
                default_model_id = EXCLUDED.default_model_id,
                tags = EXCLUDED.tags,
                initial_files = EXCLUDED.initial_files,
                tools = EXCLUDED.tools,
                network_access = EXCLUDED.network_access,
                max_iterations = EXCLUDED.max_iterations,
                status = 'active',
                updated_at = NOW()
            RETURNING id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, archived_at, deleted_at, initial_files, tools, network_access, max_iterations,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id)
        .bind(&input.tags)
        .bind(&input.initial_files)
        .bind(&input.tools)
        .bind(&input.network_access)
        .bind(input.max_iterations)
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
            INSERT INTO agents (org_id, public_id, name, display_name, description, system_prompt, default_model_id, tags, initial_files, tools, network_access, max_iterations, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'active')
            ON CONFLICT (org_id, name) WHERE status != 'deleted' DO UPDATE SET
                display_name = EXCLUDED.display_name,
                description = EXCLUDED.description,
                system_prompt = EXCLUDED.system_prompt,
                default_model_id = EXCLUDED.default_model_id,
                tags = EXCLUDED.tags,
                initial_files = EXCLUDED.initial_files,
                tools = EXCLUDED.tools,
                network_access = EXCLUDED.network_access,
                max_iterations = EXCLUDED.max_iterations,
                status = 'active',
                updated_at = NOW()
            RETURNING id, public_id, org_id, name, display_name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, archived_at, deleted_at, initial_files, tools, network_access, max_iterations,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id)
        .bind(&input.tags)
        .bind(&input.initial_files)
        .bind(&input.tools)
        .bind(&input.network_access)
        .bind(input.max_iterations)
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
}
