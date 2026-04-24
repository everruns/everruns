// PostgreSQL repository: Sessions (instance of agentic loop), Pinned Sessions

use super::super::models::*;
use super::Database;
use super::build_search_sql;
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::AgentIdentityId;
use everruns_core::PrincipalId;
use everruns_core::typed_id::{AgentId, SessionId};
use uuid::Uuid;

impl Database {
    // ============================================
    // Sessions (instance of agentic loop)
    // ============================================

    pub async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            INSERT INTO sessions (org_id, harness_id, agent_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, blueprint_id, blueprint_config, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, 'started')
            RETURNING id, org_id, harness_id, agent_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, status, created_at, updated_at, started_at, finished_at,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, parent_session_id, subagent_name, subagent_task, subagent_status,
                      blueprint_id, blueprint_config
            "#,
        )
        .bind(input.org_id)
        .bind(input.harness_id.map(|h| h.uuid()))
        .bind(input.agent_id.map(|a| a.uuid()))
        .bind(input.agent_identity_id.map(|a: AgentIdentityId| a.uuid()))
        .bind(input.owner_principal_id)
        .bind(input.resolved_owner_user_id)
        .bind(&input.title)
        .bind(&input.locale)
        .bind(&input.tags)
        .bind(input.model_id)
        .bind(&input.capabilities)
        .bind(&input.tools)
        .bind(&input.mcp_servers)
        .bind(&input.system_prompt)
        .bind(&input.initial_files)
        .bind(&input.hints)
        .bind(&input.network_access)
        .bind(input.max_iterations)
        .bind(&input.blueprint_id)
        .bind(&input.blueprint_config)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get session by org and session id
    pub async fn get_session(&self, org_id: i64, id: SessionId) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, harness_id, agent_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, parent_session_id, subagent_name, subagent_task, subagent_status,
                   blueprint_id, blueprint_config
            FROM sessions
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn set_subagent_metadata(
        &self,
        org_id: i64,
        id: SessionId,
        parent_session_id: SessionId,
        subagent_name: &str,
        subagent_task: &str,
        subagent_status: &str,
    ) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            UPDATE sessions
            SET parent_session_id = $3,
                subagent_name = $4,
                subagent_task = $5,
                subagent_status = $6
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, harness_id, agent_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, status, created_at, updated_at, started_at, finished_at,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, parent_session_id, subagent_name, subagent_task, subagent_status,
                      blueprint_id, blueprint_config
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(parent_session_id)
        .bind(subagent_name)
        .bind(subagent_task)
        .bind(subagent_status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get session without org scoping. For internal system use only (e.g. usage tracking).
    pub async fn get_session_unscoped(&self, id: SessionId) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, harness_id, agent_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, parent_session_id, subagent_name, subagent_task, subagent_status,
                   blueprint_id, blueprint_config
            FROM sessions
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List sessions for an organization with optional agent and search filters.
    /// Returns (sessions, total_count).
    pub async fn list_sessions(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
        search: Option<&str>,
        pagination: crate::api::common::Pagination,
    ) -> Result<(Vec<SessionRow>, u32)> {
        // Build WHERE clause dynamically
        let mut where_clause = "WHERE org_id = $1".to_string();
        let mut param_idx = 2;

        if agent_id.is_some() {
            where_clause.push_str(&format!(" AND agent_id = ${param_idx}"));
            param_idx += 1;
        }

        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(COALESCE(title, ''))", param_idx);
        where_clause.push_str(&search_sql);
        param_idx += patterns.len();

        // Helper: bind org_id, agent_id, and search patterns to a query
        macro_rules! bind_params {
            ($q:expr) => {{
                let mut q = $q.bind(org_id);
                if let Some(aid) = agent_id {
                    q = q.bind(aid);
                }
                for pat in &patterns {
                    q = q.bind(pat);
                }
                q
            }};
        }

        // Get total count
        let count_sql = format!("SELECT COUNT(*) as count FROM sessions {where_clause}");
        let count_query = bind_params!(sqlx::query_as::<_, (i64,)>(&count_sql));
        let total: (i64,) = count_query.fetch_one(&self.pool).await?;

        // Get paginated results
        let limit_idx = param_idx;
        let offset_idx = param_idx + 1;
        let select_sql = format!(
            r#"SELECT id, org_id, harness_id, agent_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, parent_session_id, subagent_name, subagent_task, subagent_status,
                   blueprint_id, blueprint_config
            FROM sessions {where_clause}
            ORDER BY created_at DESC
            LIMIT ${limit_idx} OFFSET ${offset_idx}"#,
        );
        let data_query = bind_params!(sqlx::query_as::<_, SessionRow>(&select_sql));
        let rows: Vec<SessionRow> = data_query
            .bind(pagination.limit as i64)
            .bind(pagination.offset as i64)
            .fetch_all(&self.pool)
            .await?;

        Ok((rows, total.0 as u32))
    }

    /// List child sessions (subagents) for a parent session.
    pub async fn list_child_sessions(
        &self,
        parent_session_id: SessionId,
    ) -> Result<Vec<SessionRow>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, harness_id, agent_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, parent_session_id, subagent_name, subagent_task, subagent_status,
                   blueprint_id, blueprint_config
            FROM sessions
            WHERE parent_session_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(parent_session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Count sessions grouped by status for an organization.
    pub async fn count_sessions_by_status(&self, org_id: i64) -> Result<Vec<(String, i64)>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT status, COUNT(*) as count
            FROM sessions
            WHERE org_id = $1
            GROUP BY status
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Find a single session matching ALL given tags within an org.
    /// Used for singleton patterns like global chat (one session per user per org).
    pub async fn find_session_by_tags(
        &self,
        org_id: i64,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, harness_id, agent_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, parent_session_id, subagent_name, subagent_task, subagent_status,
                   blueprint_id, blueprint_config
            FROM sessions
            WHERE org_id = $1 AND tags @> $2
            ORDER BY created_at ASC
            LIMIT 1
            "#,
        )
        .bind(org_id)
        .bind(tags)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Find a single session matching ALL given tags + owner within an org.
    pub async fn find_session_by_tags_and_owner(
        &self,
        org_id: i64,
        owner_principal_id: PrincipalId,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, harness_id, agent_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, parent_session_id, subagent_name, subagent_task, subagent_status,
                   blueprint_id, blueprint_config
            FROM sessions
            WHERE org_id = $1 AND owner_principal_id = $2 AND tags @> $3
            ORDER BY created_at ASC
            LIMIT 1
            "#,
        )
        .bind(org_id)
        .bind(owner_principal_id)
        .bind(tags)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Find all active sessions with Slack tags (for startup recovery).
    /// Returns sessions where status = 'active' and any tag starts with 'slack:app:'.
    pub async fn find_active_slack_sessions(&self) -> Result<Vec<SessionRow>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, harness_id, agent_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, parent_session_id, subagent_name, subagent_task, subagent_status,
                   blueprint_id, blueprint_config
            FROM sessions
            WHERE status = 'active'
              AND EXISTS (
                  SELECT 1 FROM unnest(tags) AS t
                  WHERE t LIKE 'slack:app:%'
              )
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Find sessions in `waiting_for_tool_results` with updated_at before the
    /// given cutoff. Returns lightweight `(session_id, org_id)` pairs.
    pub async fn list_sessions_waiting_tool_results_before(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<Vec<(SessionId, i64)>> {
        let rows = sqlx::query_as::<_, (SessionId, i64)>(
            r#"
            SELECT id, org_id
            FROM sessions
            WHERE status = 'waiting_for_tool_results'
              AND updated_at < $1
            "#,
        )
        .bind(cutoff)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Update session by org and session id
    pub async fn update_session(
        &self,
        org_id: i64,
        id: SessionId,
        input: UpdateSession,
    ) -> Result<Option<SessionRow>> {
        // Note: updated_at is automatically set by the update_sessions_updated_at trigger
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            UPDATE sessions
            SET
                harness_id = COALESCE($3, harness_id),
                title = COALESCE($4, title),
                agent_identity_id = CASE WHEN $5 THEN $6 ELSE agent_identity_id END,
                owner_principal_id = COALESCE($7, owner_principal_id),
                resolved_owner_user_id = CASE WHEN $8 THEN $9 ELSE resolved_owner_user_id END,
                locale = COALESCE($10, locale),
                tags = COALESCE($11, tags),
                model_id = COALESCE($12, model_id),
                status = COALESCE($13, status),
                started_at = COALESCE($14, started_at),
                finished_at = COALESCE($15, finished_at)
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, harness_id, agent_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, status, created_at, updated_at, started_at, finished_at,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, parent_session_id, subagent_name, subagent_task, subagent_status,
                      blueprint_id, blueprint_config
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(input.harness_id.map(|h| h.uuid()))
        .bind(&input.title)
        .bind(input.agent_identity_id.is_changed())
        .bind(input.agent_identity_id.into_value().map(|a: AgentIdentityId| a.uuid()))
        .bind(input.owner_principal_id)
        .bind(input.resolved_owner_user_id.is_changed())
        .bind(input.resolved_owner_user_id.into_value())
        .bind(&input.locale)
        .bind(&input.tags)
        .bind(input.model_id.map(|m| m.uuid()))
        .bind(&input.status)
        .bind(input.started_at)
        .bind(input.finished_at)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Delete session by org and session id
    pub async fn delete_session(&self, org_id: i64, id: SessionId) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM sessions
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Pinned Sessions
    // ============================================

    /// Pin a session for a user
    pub async fn pin_session(
        &self,
        user_id: Uuid,
        session_id: SessionId,
        org_id: i64,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO pinned_sessions (user_id, session_id, org_id)
            VALUES ($1, $2, $3)
            ON CONFLICT (user_id, session_id) DO NOTHING
            "#,
        )
        .bind(user_id)
        .bind(session_id)
        .bind(org_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Unpin a session for a user
    pub async fn unpin_session(&self, user_id: Uuid, session_id: SessionId) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM pinned_sessions
            WHERE user_id = $1 AND session_id = $2
            "#,
        )
        .bind(user_id)
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Get the set of pinned session IDs for a user in an org
    pub async fn list_pinned_session_ids(
        &self,
        user_id: Uuid,
        org_id: i64,
    ) -> Result<Vec<SessionId>> {
        let rows: Vec<(SessionId,)> = sqlx::query_as(
            r#"
            SELECT session_id
            FROM pinned_sessions
            WHERE user_id = $1 AND org_id = $2
            ORDER BY pinned_at DESC
            "#,
        )
        .bind(user_id)
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }
}
