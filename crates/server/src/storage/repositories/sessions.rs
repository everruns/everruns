// PostgreSQL repository: Sessions (instance of agentic loop), Pinned Sessions

use super::super::models::*;
use super::Database;
use super::build_search_sql;
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::AgentIdentityId;
use everruns_core::PrincipalId;
use everruns_core::typed_id::{AgentId, HarnessId, SessionId};
use tracing::warn;
use uuid::Uuid;

impl Database {
    // ============================================
    // Sessions (instance of agentic loop)
    // ============================================

    pub async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        let session_id = uuid::Uuid::now_v7();
        // Attach to an existing workspace when requested; otherwise auto-create
        // a default workspace whose primary key equals the session's id. That
        // equality invariant lets existing app code that uses session.id as the
        // file-store key keep working unchanged. See specs/workspace.md.
        let workspace_id = input.workspace_id.unwrap_or(session_id);

        let mut tx = self.pool.begin().await?;

        if input.workspace_id.is_none() {
            sqlx::query(
                r#"
                INSERT INTO workspaces (
                    id, org_id, public_id, name, description, status, created_at, updated_at
                )
                VALUES (
                    $1, $2,
                    'wsp_' || replace($1::text, '-', ''),
                    -- Use the full 32-hex so per-org name uniqueness holds under
                    -- bursty creation (UUIDv7 prefixes repeat in short windows).
                    'session-' || replace($1::text, '-', ''),
                    'Default workspace for session ' || $1::text,
                    'active',
                    NOW(),
                    NOW()
                )
                "#,
            )
            .bind(session_id)
            .bind(input.org_id)
            .execute(&mut *tx)
            .await?;
        }

        // EVE-680: resolve this session's delegation-tree root. A top-level
        // session (no parent) is its own root; a subagent child inherits its
        // parent's root. Looked up within the same tx so the denormalized
        // pointer stays consistent with the parent chain. The parent row is
        // guaranteed to exist (parent_session_id is itself an FK) and to carry a
        // root post-migration; the fallbacks are defensive.
        let root_session_id: uuid::Uuid = match input.parent_session_id {
            Some(parent) => sqlx::query_scalar::<_, Option<uuid::Uuid>>(
                "SELECT root_session_id FROM sessions WHERE id = $1",
            )
            .bind(parent.uuid())
            .fetch_optional(&mut *tx)
            .await?
            .flatten()
            .unwrap_or_else(|| parent.uuid()),
            None => session_id,
        };

        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            INSERT INTO sessions (id, org_id, app_id, harness_id, agent_id, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, blueprint_id, blueprint_config, parent_session_id, workspace_id, parallel_tool_calls, root_session_id, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, 'started')
            RETURNING id, org_id, workspace_id, app_id, harness_id, agent_id, agent_version_id, agent_config_hash, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, parallel_tool_calls, status, created_at, updated_at, started_at, finished_at,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd, parent_session_id, root_session_id,
                      blueprint_id, blueprint_config
            "#,
        )
        .bind(session_id)
        .bind(input.org_id)
        .bind(input.app_id)
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
        .bind(input.parent_session_id.map(|id| id.uuid()))
        .bind(workspace_id)
        .bind(input.parallel_tool_calls)
        .bind(root_session_id)
        .fetch_one(&mut *tx)
        .await?;

        if let Some(agent_id) = row.agent_id {
            sqlx::query(
                r#"
                INSERT INTO session_participants (
                    id, org_id, session_id, kind, agent_id, agent_version_id,
                    principal_id, role, joined_at
                )
                VALUES (uuidv7(), $1, $2, 'agent', $3, $4, $5, 'host', $6)
                "#,
            )
            .bind(row.org_id)
            .bind(row.id.uuid())
            .bind(agent_id.uuid())
            .bind(row.agent_version_id.map(|id| id.uuid()))
            .bind(row.owner_principal_id)
            .bind(row.created_at)
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query(
            r#"
            INSERT INTO session_participants (
                id, org_id, session_id, kind, agent_id, agent_version_id,
                principal_id, role, joined_at
            )
            VALUES (uuidv7(), $1, $2, 'user', NULL, NULL, $3, 'member', $4)
            "#,
        )
        .bind(row.org_id)
        .bind(row.id.uuid())
        .bind(row.owner_principal_id)
        .bind(row.created_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        // Reporting outbox enqueue is best-effort (see specs/reporting.md).
        // The canonical session row is durable; no reconciler exists yet
        // (tracked separately), so on failure the corresponding fact will
        // remain stale until reconciliation lands or the session row is
        // updated again and the next enqueue succeeds.
        if let Err(e) = self
            .enqueue_reporting_outbox(
                row.org_id,
                "session",
                &row.id.uuid().to_string(),
                Some(&row.updated_at.to_rfc3339()),
                "session_snapshot",
            )
            .await
        {
            warn!(
                session_id = %row.id.uuid(),
                org_id = row.org_id,
                error = %e,
                "reporting outbox enqueue failed for session create; projection may remain stale until reconciliation lands"
            );
        }

        Ok(row)
    }

    /// Record fork provenance on an already-created session
    /// (specs/forking-sessions.md). Set in a dedicated update so the normal
    /// `create_session` path and its many call sites stay untouched.
    pub async fn set_session_fork_lineage(
        &self,
        session_id: SessionId,
        forked_from_session_id: SessionId,
        forked_from_sequence: Option<i32>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE sessions
            SET forked_from_session_id = $2,
                forked_from_sequence = $3,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(session_id.uuid())
        .bind(forked_from_session_id.uuid())
        .bind(forked_from_sequence)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Get session by org and session id
    pub async fn get_session(&self, org_id: i64, id: SessionId) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, workspace_id, app_id, harness_id, agent_id, agent_version_id, agent_config_hash, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, parallel_tool_calls, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd, parent_session_id,
                   forked_from_session_id, forked_from_sequence,
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

    /// Get session without org scoping. For internal system use only (e.g. usage tracking).
    pub async fn get_session_unscoped(&self, id: SessionId) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, workspace_id, app_id, harness_id, agent_id, agent_version_id, agent_config_hash, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, parallel_tool_calls, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd, parent_session_id,
                   forked_from_session_id, forked_from_sequence,
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
        let count_query = bind_params!(sqlx::query_as::<_, (i64,)>(sqlx::AssertSqlSafe(
            count_sql.as_str()
        )));
        let total: (i64,) = count_query.fetch_one(&self.pool).await?;

        // Get paginated results
        let limit_idx = param_idx;
        let offset_idx = param_idx + 1;
        let select_sql = format!(
            r#"SELECT id, org_id, workspace_id, app_id, harness_id, agent_id, agent_version_id, agent_config_hash, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, parallel_tool_calls, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd, parent_session_id,
                   forked_from_session_id, forked_from_sequence,
                   blueprint_id, blueprint_config
            FROM sessions {where_clause}
            ORDER BY created_at DESC
            LIMIT ${limit_idx} OFFSET ${offset_idx}"#,
        );
        let data_query = bind_params!(sqlx::query_as::<_, SessionRow>(sqlx::AssertSqlSafe(
            select_sql.as_str()
        )));
        let rows: Vec<SessionRow> = data_query
            .bind(pagination.limit as i64)
            .bind(pagination.offset as i64)
            .fetch_all(&self.pool)
            .await?;

        Ok((rows, total.0 as u32))
    }

    pub async fn count_sessions_for_agent(&self, org_id: i64, agent_id: AgentId) -> Result<u64> {
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sessions WHERE org_id = $1 AND agent_id = $2")
                .bind(org_id)
                .bind(agent_id)
                .fetch_one(&self.pool)
                .await?;

        Ok(count as u64)
    }

    pub async fn count_sessions_for_harness(
        &self,
        org_id: i64,
        harness_id: HarnessId,
    ) -> Result<u64> {
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sessions WHERE org_id = $1 AND harness_id = $2")
                .bind(org_id)
                .bind(harness_id)
                .fetch_one(&self.pool)
                .await?;

        Ok(count as u64)
    }

    /// Count sessions in an org (for resource limits). Sessions are hard-deleted
    /// (no soft-delete status), so every stored row counts toward the cap.
    pub async fn count_sessions_for_org(&self, org_id: i64) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sessions WHERE org_id = $1")
            .bind(org_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(count)
    }

    /// List child sessions (subagents) for a parent session.
    pub async fn list_child_sessions(
        &self,
        parent_session_id: SessionId,
    ) -> Result<Vec<SessionRow>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, workspace_id, app_id, harness_id, agent_id, agent_version_id, agent_config_hash, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, parallel_tool_calls, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd, parent_session_id,
                   forked_from_session_id, forked_from_sequence,
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

    /// Count non-finished sessions for an org (EVE-508 concurrent session cap).
    pub async fn count_active_sessions_for_org(&self, org_id: i64) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*)::bigint FROM sessions WHERE org_id = $1 AND status IN ('active', 'idle', 'started', 'waiting_for_tool_results', 'paused')",
        )
        .bind(org_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    /// Count sessions currently executing a turn for an org (EVE-508 active turn cap).
    pub async fn count_active_turns_for_org(&self, org_id: i64) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*)::bigint FROM sessions WHERE org_id = $1 AND status = 'active'",
        )
        .bind(org_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    /// Atomically reserve active-turn capacity by marking the accepted
    /// session active before the user message is persisted.
    pub async fn reserve_active_turn_slot_for_org(
        &self,
        org_id: i64,
        session_id: SessionId,
        max_active_turns: i64,
    ) -> Result<ReserveActiveTurnSlotResult> {
        let mut tx = self.pool.begin().await?;

        // Serialize per-org reservations so the soft cap covers accepted queued
        // turns, not only turns workers have already begun executing.
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(org_id)
            .execute(&mut *tx)
            .await?;

        // Verify the session exists and belongs to the org *before* the capacity
        // check, so a missing/foreign session returns SessionNotFound rather
        // than a misleading AtCapacity, and capture its prior status so the
        // reservation can be released on a later failure.
        let existing: Option<(String,)> =
            sqlx::query_as("SELECT status FROM sessions WHERE org_id = $1 AND id = $2")
                .bind(org_id)
                .bind(session_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((previous_status,)) = existing else {
            tx.commit().await?;
            return Ok(ReserveActiveTurnSlotResult::SessionNotFound);
        };

        let active_turns: (i64,) = sqlx::query_as(
            "SELECT COUNT(*)::bigint FROM sessions WHERE org_id = $1 AND status = 'active'",
        )
        .bind(org_id)
        .fetch_one(&mut *tx)
        .await?;

        if active_turns.0 >= max_active_turns {
            tx.commit().await?;
            return Ok(ReserveActiveTurnSlotResult::AtCapacity {
                active_turns: active_turns.0,
            });
        }

        sqlx::query("UPDATE sessions SET status = 'active' WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(ReserveActiveTurnSlotResult::Reserved { previous_status })
    }

    /// Release a previously reserved active-turn slot by restoring the session's
    /// prior status. Best-effort and idempotent: only reverts a session that is
    /// still `active`, so it never clobbers a status a worker legitimately
    /// advanced to after the reservation.
    pub async fn release_active_turn_slot_for_org(
        &self,
        org_id: i64,
        session_id: SessionId,
        previous_status: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET status = $3 WHERE org_id = $1 AND id = $2 AND status = 'active'",
        )
        .bind(org_id)
        .bind(session_id)
        .bind(previous_status)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Aggregate session and turn execution stats for an optional agent or harness scope.
    pub async fn session_aggregate_stats(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
        harness_id: Option<HarnessId>,
    ) -> Result<SessionAggregateStatsRow> {
        let row = sqlx::query_as::<_, SessionAggregateStatsRow>(
            r#"
            WITH matching_sessions AS (
                SELECT id, status, created_at, updated_at, started_at, finished_at,
                       total_input_tokens, total_output_tokens,
                       total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd
                FROM sessions
                WHERE org_id = $1
                  AND ($2::uuid IS NULL OR agent_id = $2)
                  AND ($3::uuid IS NULL OR harness_id = $3)
            ),
            turn_stats AS (
                SELECT COUNT(*) AS execution_count, MAX(e.ts) AS last_execution_at
                FROM events e
                INNER JOIN matching_sessions s ON s.id = e.session_id
                WHERE e.event_type = 'turn.started'
            )
            SELECT
                COUNT(*)::bigint AS session_count,
                COUNT(*) FILTER (WHERE status = 'active')::bigint AS active_session_count,
                COUNT(*) FILTER (WHERE status = 'idle')::bigint AS idle_session_count,
                COUNT(*) FILTER (WHERE status = 'started')::bigint AS started_session_count,
                COUNT(*) FILTER (WHERE status = 'waiting_for_tool_results')::bigint AS waiting_for_tool_results_session_count,
                COALESCE((SELECT execution_count FROM turn_stats), 0)::bigint AS execution_count,
                COALESCE(SUM(GREATEST((EXTRACT(EPOCH FROM (COALESCE(finished_at, updated_at) - COALESCE(started_at, created_at))) * 1000)::bigint, 0)), 0)::bigint AS total_session_duration_ms,
                COALESCE(SUM(total_input_tokens), 0)::bigint AS total_input_tokens,
                COALESCE(SUM(total_output_tokens), 0)::bigint AS total_output_tokens,
                COALESCE(SUM(total_cache_read_tokens), 0)::bigint AS total_cache_read_tokens,
                COALESCE(SUM(total_cache_creation_tokens), 0)::bigint AS total_cache_creation_tokens,
                COALESCE(SUM(total_actual_cost_usd), 0)::double precision AS total_actual_cost_usd,
                COALESCE(SUM(total_estimated_cost_usd), 0)::double precision AS total_estimated_cost_usd,
                COALESCE(SUM(total_cost_usd), 0)::double precision AS total_cost_usd,
                MIN(created_at) AS first_session_at,
                MAX(created_at) AS last_session_at,
                (SELECT last_execution_at FROM turn_stats) AS last_execution_at
            FROM matching_sessions
            "#,
        )
        .bind(org_id)
        .bind(agent_id.map(|id| id.uuid()))
        .bind(harness_id.map(|id| id.uuid()))
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
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
            SELECT id, org_id, workspace_id, app_id, harness_id, agent_id, agent_version_id, agent_config_hash, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, parallel_tool_calls, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd, parent_session_id,
                   forked_from_session_id, forked_from_sequence,
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

    /// Find a single app-owned session matching ALL given tags within an org.
    pub async fn find_app_session_by_tags(
        &self,
        org_id: i64,
        app_id: Uuid,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, workspace_id, app_id, harness_id, agent_id, agent_version_id, agent_config_hash, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, parallel_tool_calls, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd, parent_session_id,
                   forked_from_session_id, forked_from_sequence,
                   blueprint_id, blueprint_config
            FROM sessions
            WHERE org_id = $1 AND app_id = $2 AND tags @> $3
            ORDER BY created_at ASC
            LIMIT 1
            "#,
        )
        .bind(org_id)
        .bind(app_id)
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
            SELECT id, org_id, workspace_id, app_id, harness_id, agent_id, agent_version_id, agent_config_hash, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, parallel_tool_calls, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd, parent_session_id,
                   forked_from_session_id, forked_from_sequence,
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

    /// Find a single app-owned session matching ALL given tags + owner within an org.
    pub async fn find_app_session_by_tags_and_owner(
        &self,
        org_id: i64,
        app_id: Uuid,
        owner_principal_id: PrincipalId,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, workspace_id, app_id, harness_id, agent_id, agent_version_id, agent_config_hash, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, parallel_tool_calls, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd, parent_session_id,
                   forked_from_session_id, forked_from_sequence,
                   blueprint_id, blueprint_config
            FROM sessions
            WHERE org_id = $1 AND app_id = $2 AND owner_principal_id = $3 AND tags @> $4
            ORDER BY created_at ASC
            LIMIT 1
            "#,
        )
        .bind(org_id)
        .bind(app_id)
        .bind(owner_principal_id)
        .bind(tags)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Find all active sessions with Slack tags (for startup recovery).
    /// Returns active sessions owned by apps with Slack channel configuration.
    pub async fn find_active_slack_sessions(&self) -> Result<Vec<SessionRow>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT s.id, s.org_id, s.workspace_id, s.app_id, s.harness_id, s.agent_id, s.agent_version_id, s.agent_config_hash, s.agent_identity_id, s.owner_principal_id, s.resolved_owner_user_id, s.title, s.locale, s.tags, s.model_id, s.capabilities, s.tools, s.mcp_servers, s.system_prompt, s.initial_files, s.hints, s.network_access, s.max_iterations, s.parallel_tool_calls, s.status, s.created_at, s.updated_at, s.started_at, s.finished_at,
                   s.total_input_tokens, s.total_output_tokens, s.total_cache_read_tokens, s.total_cache_creation_tokens, s.total_cost_usd, s.parent_session_id,
                   s.blueprint_id, s.blueprint_config
            FROM sessions s
            JOIN apps a ON a.id = s.app_id AND a.org_id = s.org_id
            WHERE s.status = 'active'
              AND a.channel_type = 'slack'
              AND a.status != 'deleted'
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
                finished_at = COALESCE($15, finished_at),
                agent_version_id = COALESCE($16, agent_version_id),
                agent_config_hash = COALESCE($17, agent_config_hash)
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, workspace_id, app_id, harness_id, agent_id, agent_version_id, agent_config_hash, agent_identity_id, owner_principal_id, resolved_owner_user_id, title, locale, tags, model_id, capabilities, tools, mcp_servers, system_prompt, initial_files, hints, network_access, max_iterations, parallel_tool_calls, status, created_at, updated_at, started_at, finished_at,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens, total_actual_cost_usd, total_estimated_cost_usd, total_cost_usd, parent_session_id,
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
        .bind(input.agent_version_id.map(|id| id.uuid()))
        .bind(input.agent_config_hash)
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

    /// Unpin a session for a user in an org
    pub async fn unpin_session(
        &self,
        user_id: Uuid,
        session_id: SessionId,
        org_id: i64,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM pinned_sessions
            WHERE user_id = $1 AND session_id = $2 AND org_id = $3
            "#,
        )
        .bind(user_id)
        .bind(session_id)
        .bind(org_id)
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
