use anyhow::{Context, Result};
use sqlx::PgPool;
use uuid::Uuid;

use super::models::ReportingOutboxRow;
use crate::domains::reporting::types::ProjectorRunResult;

#[derive(Clone)]
pub struct PostgresReportingProjector {
    pool: PgPool,
}

impl PostgresReportingProjector {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn run_once(&self, limit: i64) -> Result<ProjectorRunResult> {
        let rows = self.claim_pending(limit).await?;
        let claimed = rows.len();
        let mut completed = 0;
        let mut failed = 0;

        for row in rows {
            match self.project_row(&row).await {
                Ok(()) => {
                    completed += 1;
                    self.mark_completed(row.id).await?;
                }
                Err(error) => {
                    failed += 1;
                    self.mark_failed(row.id, row.attempts, &error.to_string())
                        .await?;
                }
            }
        }

        Ok(ProjectorRunResult {
            claimed,
            completed,
            failed,
        })
    }

    async fn claim_pending(&self, limit: i64) -> Result<Vec<ReportingOutboxRow>> {
        let rows = sqlx::query_as::<_, ReportingOutboxRow>(
            r#"
            UPDATE reporting_outbox
               SET status = 'processing',
                   attempts = attempts + 1,
                   claimed_at = NOW(),
                   updated_at = NOW()
             WHERE id IN (
                SELECT id
                  FROM reporting_outbox
                 WHERE status IN ('pending', 'failed')
                   AND next_attempt_at <= NOW()
                 ORDER BY created_at ASC
                 LIMIT $1
                 FOR UPDATE SKIP LOCKED
             )
         RETURNING id, org_id, source_type, source_id, source_version, reason, status,
                   attempts, next_attempt_at, last_error, created_at, updated_at
            "#,
        )
        .bind(limit.clamp(1, 1_000))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn mark_completed(&self, id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE reporting_outbox
               SET status = 'completed',
                   last_error = NULL,
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn mark_failed(&self, id: Uuid, attempts: i32, error: &str) -> Result<()> {
        let delay_seconds = (2_i64.pow(attempts.clamp(1, 8) as u32) * 30).min(3_600);
        sqlx::query(
            r#"
            UPDATE reporting_outbox
               SET status = 'failed',
                   next_attempt_at = NOW() + ($2::TEXT || ' seconds')::INTERVAL,
                   last_error = LEFT($3, 2000),
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(delay_seconds.to_string())
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn project_row(&self, row: &ReportingOutboxRow) -> Result<()> {
        match row.source_type.as_str() {
            "llm_generation" => {
                self.project_llm_generation(row.org_id, &row.source_id)
                    .await
            }
            "event" => self.project_event(row.org_id, &row.source_id).await,
            "session" => self.project_session(row.org_id, &row.source_id).await,
            "usage_ledger" => {
                self.project_budget_posting(row.org_id, &row.source_id)
                    .await
            }
            source => {
                tracing::debug!(
                    source_type = source,
                    "Skipping unsupported reporting source"
                );
                Ok(())
            }
        }
    }

    async fn project_llm_generation(&self, org_id: i64, source_id: &str) -> Result<()> {
        let id = Uuid::parse_str(source_id).context("invalid llm_generation source id")?;
        sqlx::query(
            r#"
            INSERT INTO fact_llm_generation (
                org_id, source_key, session_id, turn_id, user_id, principal_id, agent_id,
                agent_name_snapshot, harness_id, harness_name_snapshot, app_id, blueprint_id,
                created_at, time_bucket_day, model, provider, finish_reason,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                total_tokens, duration_ms, time_to_first_token_ms, success_count, error_count,
                projected_at
            )
            SELECT
                lg.org_id,
                'llm_generation:' || lg.id::TEXT,
                lg.session_id,
                lg.turn_id::TEXT,
                s.resolved_owner_user_id,
                s.owner_principal_id,
                s.agent_id,
                a.name,
                s.harness_id,
                h.name,
                s.app_id,
                s.blueprint_id,
                lg.created_at,
                (lg.created_at AT TIME ZONE 'UTC')::DATE,
                lg.model,
                lg.provider,
                lg.finish_reason,
                lg.input_tokens,
                lg.output_tokens,
                lg.cache_read_tokens,
                lg.cache_creation_tokens,
                lg.input_tokens + lg.output_tokens + lg.cache_read_tokens + lg.cache_creation_tokens,
                COALESCE(lg.duration_ms, 0),
                0,
                1,
                0,
                NOW()
            FROM llm_generations lg
            JOIN sessions s ON s.id = lg.session_id AND s.org_id = lg.org_id
            LEFT JOIN agents a ON a.id = s.agent_id AND a.org_id = lg.org_id
            LEFT JOIN harnesses h ON h.id = s.harness_id AND h.org_id = lg.org_id
            WHERE lg.id = $1 AND lg.org_id = $2
            ON CONFLICT (org_id, source_key) DO UPDATE SET
                session_id = EXCLUDED.session_id,
                turn_id = EXCLUDED.turn_id,
                user_id = EXCLUDED.user_id,
                principal_id = EXCLUDED.principal_id,
                agent_id = EXCLUDED.agent_id,
                agent_name_snapshot = EXCLUDED.agent_name_snapshot,
                harness_id = EXCLUDED.harness_id,
                harness_name_snapshot = EXCLUDED.harness_name_snapshot,
                app_id = EXCLUDED.app_id,
                blueprint_id = EXCLUDED.blueprint_id,
                created_at = EXCLUDED.created_at,
                time_bucket_day = EXCLUDED.time_bucket_day,
                model = EXCLUDED.model,
                provider = EXCLUDED.provider,
                finish_reason = EXCLUDED.finish_reason,
                input_tokens = EXCLUDED.input_tokens,
                output_tokens = EXCLUDED.output_tokens,
                cache_read_tokens = EXCLUDED.cache_read_tokens,
                cache_creation_tokens = EXCLUDED.cache_creation_tokens,
                total_tokens = EXCLUDED.total_tokens,
                duration_ms = EXCLUDED.duration_ms,
                projected_at = NOW()
            "#,
        )
        .bind(id)
        .bind(org_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn project_event(&self, org_id: i64, source_id: &str) -> Result<()> {
        let id = Uuid::parse_str(source_id).context("invalid event source id")?;
        let event_type: Option<(String,)> = sqlx::query_as(
            "SELECT e.event_type FROM events e JOIN sessions s ON s.id = e.session_id WHERE e.id = $1 AND s.org_id = $2",
        )
        .bind(id)
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;
        match event_type.as_ref().map(|row| row.0.as_str()) {
            Some("tool.completed") => self.project_tool_event(org_id, id).await,
            Some("capability.usage") => self.project_capability_usage_event(org_id, id).await,
            Some("output.message.replaced") => {
                self.project_guardrail_effect_event(org_id, id).await
            }
            Some("turn.completed" | "turn.failed" | "turn.cancelled") => {
                self.project_turn_event(org_id, id).await
            }
            _ => Ok(()),
        }
    }

    async fn project_tool_event(&self, org_id: i64, id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO fact_tool_call (
                org_id, source_key, session_id, turn_id, user_id, principal_id, agent_id,
                agent_name_snapshot, harness_id, harness_name_snapshot, app_id, blueprint_id,
                created_at, time_bucket_day, tool_call_id, tool_name,
                tool_display_name_snapshot, tool_status, capability_id,
                capability_name_snapshot, call_count, success_count, error_count,
                timeout_count, cancelled_count, duration_ms, projected_at
            )
            SELECT
                s.org_id,
                'event:' || e.id::TEXT,
                e.session_id,
                COALESCE(e.context->>'turn_id', e.data->>'turn_id'),
                s.resolved_owner_user_id,
                s.owner_principal_id,
                s.agent_id,
                a.name,
                s.harness_id,
                h.name,
                s.app_id,
                s.blueprint_id,
                e.ts,
                (e.ts AT TIME ZONE 'UTC')::DATE,
                e.data->>'tool_call_id',
                e.data->>'tool_name',
                e.data->>'display_name',
                COALESCE(e.data->>'status', CASE WHEN e.data->>'success' = 'true' THEN 'success' ELSE 'error' END),
                e.data->>'capability_id',
                e.data->>'capability_name',
                1,
                CASE WHEN COALESCE(e.data->>'status', '') = 'success' OR e.data->>'success' = 'true' THEN 1 ELSE 0 END,
                CASE WHEN COALESCE(e.data->>'status', '') = 'error' OR e.data->>'success' = 'false' THEN 1 ELSE 0 END,
                CASE WHEN e.data->>'status' = 'timeout' THEN 1 ELSE 0 END,
                CASE WHEN e.data->>'status' = 'cancelled' THEN 1 ELSE 0 END,
                CASE WHEN COALESCE(e.data->>'duration_ms', '') ~ '^[0-9]+$'
                     THEN (e.data->>'duration_ms')::BIGINT
                     ELSE 0
                END,
                NOW()
            FROM events e
            JOIN sessions s ON s.id = e.session_id
            LEFT JOIN agents a ON a.id = s.agent_id AND a.org_id = s.org_id
            LEFT JOIN harnesses h ON h.id = s.harness_id AND h.org_id = s.org_id
            WHERE e.id = $1 AND s.org_id = $2
            ON CONFLICT (org_id, source_key) DO UPDATE SET
                turn_id = EXCLUDED.turn_id,
                user_id = EXCLUDED.user_id,
                principal_id = EXCLUDED.principal_id,
                agent_id = EXCLUDED.agent_id,
                agent_name_snapshot = EXCLUDED.agent_name_snapshot,
                harness_id = EXCLUDED.harness_id,
                harness_name_snapshot = EXCLUDED.harness_name_snapshot,
                app_id = EXCLUDED.app_id,
                blueprint_id = EXCLUDED.blueprint_id,
                created_at = EXCLUDED.created_at,
                time_bucket_day = EXCLUDED.time_bucket_day,
                tool_call_id = EXCLUDED.tool_call_id,
                tool_name = EXCLUDED.tool_name,
                tool_display_name_snapshot = EXCLUDED.tool_display_name_snapshot,
                tool_status = EXCLUDED.tool_status,
                capability_id = EXCLUDED.capability_id,
                capability_name_snapshot = EXCLUDED.capability_name_snapshot,
                success_count = EXCLUDED.success_count,
                error_count = EXCLUDED.error_count,
                timeout_count = EXCLUDED.timeout_count,
                cancelled_count = EXCLUDED.cancelled_count,
                duration_ms = EXCLUDED.duration_ms,
                projected_at = NOW()
            "#,
        )
        .bind(id)
        .bind(org_id)
        .execute(&self.pool)
        .await?;
        self.project_tool_invoked_usage(org_id, id).await?;
        Ok(())
    }

    async fn project_tool_invoked_usage(&self, org_id: i64, id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO fact_capability_usage (
                org_id, source_key, session_id, turn_id, user_id, principal_id, agent_id,
                agent_name_snapshot, harness_id, harness_name_snapshot, app_id, blueprint_id,
                created_at, time_bucket_day, capability_id, capability_name_snapshot,
                capability_usage_kind, tool_name, usage_count, duration_ms, projected_at
            )
            SELECT
                s.org_id,
                'event:' || e.id::TEXT || ':invoked',
                e.session_id,
                COALESCE(e.context->>'turn_id', e.data->>'turn_id'),
                s.resolved_owner_user_id,
                s.owner_principal_id,
                s.agent_id,
                a.name,
                s.harness_id,
                h.name,
                s.app_id,
                s.blueprint_id,
                e.ts,
                (e.ts AT TIME ZONE 'UTC')::DATE,
                e.data->>'capability_id',
                e.data->>'capability_name',
                'invoked',
                e.data->>'tool_name',
                1,
                CASE WHEN COALESCE(e.data->>'duration_ms', '') ~ '^[0-9]+$'
                     THEN (e.data->>'duration_ms')::BIGINT
                     ELSE 0
                END,
                NOW()
            FROM events e
            JOIN sessions s ON s.id = e.session_id
            LEFT JOIN agents a ON a.id = s.agent_id AND a.org_id = s.org_id
            LEFT JOIN harnesses h ON h.id = s.harness_id AND h.org_id = s.org_id
            WHERE e.id = $1 AND s.org_id = $2 AND e.data ? 'capability_id'
            ON CONFLICT (org_id, source_key) DO UPDATE SET
                turn_id = EXCLUDED.turn_id,
                user_id = EXCLUDED.user_id,
                principal_id = EXCLUDED.principal_id,
                agent_id = EXCLUDED.agent_id,
                agent_name_snapshot = EXCLUDED.agent_name_snapshot,
                harness_id = EXCLUDED.harness_id,
                harness_name_snapshot = EXCLUDED.harness_name_snapshot,
                app_id = EXCLUDED.app_id,
                blueprint_id = EXCLUDED.blueprint_id,
                created_at = EXCLUDED.created_at,
                time_bucket_day = EXCLUDED.time_bucket_day,
                capability_id = EXCLUDED.capability_id,
                capability_name_snapshot = EXCLUDED.capability_name_snapshot,
                tool_name = EXCLUDED.tool_name,
                usage_count = EXCLUDED.usage_count,
                duration_ms = EXCLUDED.duration_ms,
                projected_at = NOW()
            "#,
        )
        .bind(id)
        .bind(org_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn project_capability_usage_event(&self, org_id: i64, id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO fact_capability_usage (
                org_id, source_key, session_id, turn_id, user_id, principal_id, agent_id,
                agent_name_snapshot, harness_id, harness_name_snapshot, app_id, blueprint_id,
                created_at, time_bucket_day, capability_id, capability_name_snapshot,
                capability_usage_kind, tool_name, usage_count, duration_ms, projected_at
            )
            SELECT
                s.org_id,
                'event:' || e.id::TEXT || ':capability_usage:' || usage.ordinality::TEXT,
                e.session_id,
                COALESCE(e.context->>'turn_id', e.data->>'turn_id'),
                s.resolved_owner_user_id,
                s.owner_principal_id,
                s.agent_id,
                a.name,
                s.harness_id,
                h.name,
                s.app_id,
                s.blueprint_id,
                e.ts,
                (e.ts AT TIME ZONE 'UTC')::DATE,
                usage.record->>'capability_id',
                usage.record->>'capability_name',
                usage.record->>'usage_kind',
                usage.record->>'tool_name',
                CASE WHEN COALESCE(usage.record->>'usage_count', '') ~ '^[0-9]+$'
                     THEN (usage.record->>'usage_count')::BIGINT
                     ELSE 1
                END,
                CASE WHEN COALESCE(usage.record->>'duration_ms', '') ~ '^[0-9]+$'
                     THEN (usage.record->>'duration_ms')::BIGINT
                     ELSE 0
                END,
                NOW()
            FROM events e
            JOIN sessions s ON s.id = e.session_id
            LEFT JOIN agents a ON a.id = s.agent_id AND a.org_id = s.org_id
            LEFT JOIN harnesses h ON h.id = s.harness_id AND h.org_id = s.org_id
            CROSS JOIN LATERAL jsonb_array_elements(COALESCE(e.data->'records', '[]'::jsonb))
                WITH ORDINALITY AS usage(record, ordinality)
            WHERE e.id = $1
              AND s.org_id = $2
              AND usage.record ? 'capability_id'
              AND usage.record ? 'usage_kind'
            ON CONFLICT (org_id, source_key) DO UPDATE SET
                turn_id = EXCLUDED.turn_id,
                user_id = EXCLUDED.user_id,
                principal_id = EXCLUDED.principal_id,
                agent_id = EXCLUDED.agent_id,
                agent_name_snapshot = EXCLUDED.agent_name_snapshot,
                harness_id = EXCLUDED.harness_id,
                harness_name_snapshot = EXCLUDED.harness_name_snapshot,
                app_id = EXCLUDED.app_id,
                blueprint_id = EXCLUDED.blueprint_id,
                created_at = EXCLUDED.created_at,
                time_bucket_day = EXCLUDED.time_bucket_day,
                capability_id = EXCLUDED.capability_id,
                capability_name_snapshot = EXCLUDED.capability_name_snapshot,
                capability_usage_kind = EXCLUDED.capability_usage_kind,
                tool_name = EXCLUDED.tool_name,
                usage_count = EXCLUDED.usage_count,
                duration_ms = EXCLUDED.duration_ms,
                projected_at = NOW()
            "#,
        )
        .bind(id)
        .bind(org_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn project_guardrail_effect_event(&self, org_id: i64, id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO fact_capability_usage (
                org_id, source_key, session_id, turn_id, user_id, principal_id, agent_id,
                agent_name_snapshot, harness_id, harness_name_snapshot, app_id, blueprint_id,
                created_at, time_bucket_day, capability_id, capability_name_snapshot,
                capability_usage_kind, tool_name, usage_count, duration_ms, projected_at
            )
            SELECT
                s.org_id,
                'event:' || e.id::TEXT || ':effect_ran',
                e.session_id,
                COALESCE(e.context->>'turn_id', e.data->>'turn_id'),
                s.resolved_owner_user_id,
                s.owner_principal_id,
                s.agent_id,
                a.name,
                s.harness_id,
                h.name,
                s.app_id,
                s.blueprint_id,
                e.ts,
                (e.ts AT TIME ZONE 'UTC')::DATE,
                e.data->>'guardrail_capability_id',
                e.data->>'guardrail_capability_id',
                'effect_ran',
                NULL,
                1,
                0,
                NOW()
            FROM events e
            JOIN sessions s ON s.id = e.session_id
            LEFT JOIN agents a ON a.id = s.agent_id AND a.org_id = s.org_id
            LEFT JOIN harnesses h ON h.id = s.harness_id AND h.org_id = s.org_id
            WHERE e.id = $1 AND s.org_id = $2 AND e.data ? 'guardrail_capability_id'
            ON CONFLICT (org_id, source_key) DO UPDATE SET
                turn_id = EXCLUDED.turn_id,
                user_id = EXCLUDED.user_id,
                principal_id = EXCLUDED.principal_id,
                agent_id = EXCLUDED.agent_id,
                agent_name_snapshot = EXCLUDED.agent_name_snapshot,
                harness_id = EXCLUDED.harness_id,
                harness_name_snapshot = EXCLUDED.harness_name_snapshot,
                app_id = EXCLUDED.app_id,
                blueprint_id = EXCLUDED.blueprint_id,
                created_at = EXCLUDED.created_at,
                time_bucket_day = EXCLUDED.time_bucket_day,
                capability_id = EXCLUDED.capability_id,
                capability_name_snapshot = EXCLUDED.capability_name_snapshot,
                usage_count = EXCLUDED.usage_count,
                duration_ms = EXCLUDED.duration_ms,
                projected_at = NOW()
            "#,
        )
        .bind(id)
        .bind(org_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn project_turn_event(&self, org_id: i64, id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO fact_turn (
                org_id, source_key, session_id, turn_id, user_id, principal_id, agent_id,
                agent_name_snapshot, harness_id, harness_name_snapshot, app_id, blueprint_id,
                created_at, time_bucket_day, turn_status, turn_count, duration_ms,
                iterations, input_tokens, output_tokens, cache_read_tokens,
                cache_creation_tokens, total_tokens, tool_call_count, success_count,
                error_count, projected_at
            )
            SELECT
                s.org_id,
                'event:' || e.id::TEXT,
                e.session_id,
                COALESCE(e.data->>'turn_id', e.context->>'turn_id'),
                s.resolved_owner_user_id,
                s.owner_principal_id,
                s.agent_id,
                a.name,
                s.harness_id,
                h.name,
                s.app_id,
                s.blueprint_id,
                e.ts,
                (e.ts AT TIME ZONE 'UTC')::DATE,
                CASE e.event_type
                    WHEN 'turn.completed' THEN 'completed'
                    WHEN 'turn.failed' THEN 'failed'
                    WHEN 'turn.cancelled' THEN 'cancelled'
                    ELSE e.event_type
                END,
                1,
                CASE WHEN COALESCE(e.data->>'duration_ms', '') ~ '^[0-9]+$'
                     THEN (e.data->>'duration_ms')::BIGINT
                     ELSE 0
                END,
                CASE WHEN COALESCE(e.data->>'iterations', '') ~ '^[0-9]+$'
                     THEN (e.data->>'iterations')::BIGINT
                     ELSE 0
                END,
                CASE WHEN COALESCE(e.data#>>'{usage,input_tokens}', '') ~ '^[0-9]+$'
                     THEN (e.data#>>'{usage,input_tokens}')::BIGINT
                     ELSE 0
                END,
                CASE WHEN COALESCE(e.data#>>'{usage,output_tokens}', '') ~ '^[0-9]+$'
                     THEN (e.data#>>'{usage,output_tokens}')::BIGINT
                     ELSE 0
                END,
                CASE WHEN COALESCE(e.data#>>'{usage,cache_read_tokens}', '') ~ '^[0-9]+$'
                     THEN (e.data#>>'{usage,cache_read_tokens}')::BIGINT
                     ELSE 0
                END,
                CASE WHEN COALESCE(e.data#>>'{usage,cache_creation_tokens}', '') ~ '^[0-9]+$'
                     THEN (e.data#>>'{usage,cache_creation_tokens}')::BIGINT
                     ELSE 0
                END,
                CASE WHEN COALESCE(e.data#>>'{usage,input_tokens}', '') ~ '^[0-9]+$'
                     THEN (e.data#>>'{usage,input_tokens}')::BIGINT
                     ELSE 0
                END
                + CASE WHEN COALESCE(e.data#>>'{usage,output_tokens}', '') ~ '^[0-9]+$'
                       THEN (e.data#>>'{usage,output_tokens}')::BIGINT
                       ELSE 0
                  END,
                (
                    SELECT COUNT(*)
                    FROM events tool_events
                    WHERE tool_events.session_id = e.session_id
                      AND tool_events.event_type = 'tool.completed'
                      AND COALESCE(tool_events.context->>'turn_id', tool_events.data->>'turn_id') =
                          COALESCE(e.data->>'turn_id', e.context->>'turn_id')
                ),
                CASE WHEN e.event_type = 'turn.completed' THEN 1 ELSE 0 END,
                CASE WHEN e.event_type IN ('turn.failed', 'turn.cancelled') THEN 1 ELSE 0 END,
                NOW()
            FROM events e
            JOIN sessions s ON s.id = e.session_id
            LEFT JOIN agents a ON a.id = s.agent_id AND a.org_id = s.org_id
            LEFT JOIN harnesses h ON h.id = s.harness_id AND h.org_id = s.org_id
            WHERE e.id = $1 AND s.org_id = $2
            ON CONFLICT (org_id, source_key) DO UPDATE SET
                turn_id = EXCLUDED.turn_id,
                user_id = EXCLUDED.user_id,
                principal_id = EXCLUDED.principal_id,
                agent_id = EXCLUDED.agent_id,
                agent_name_snapshot = EXCLUDED.agent_name_snapshot,
                harness_id = EXCLUDED.harness_id,
                harness_name_snapshot = EXCLUDED.harness_name_snapshot,
                app_id = EXCLUDED.app_id,
                blueprint_id = EXCLUDED.blueprint_id,
                created_at = EXCLUDED.created_at,
                time_bucket_day = EXCLUDED.time_bucket_day,
                turn_status = EXCLUDED.turn_status,
                duration_ms = EXCLUDED.duration_ms,
                iterations = EXCLUDED.iterations,
                input_tokens = EXCLUDED.input_tokens,
                output_tokens = EXCLUDED.output_tokens,
                cache_read_tokens = EXCLUDED.cache_read_tokens,
                cache_creation_tokens = EXCLUDED.cache_creation_tokens,
                total_tokens = EXCLUDED.total_tokens,
                tool_call_count = EXCLUDED.tool_call_count,
                success_count = EXCLUDED.success_count,
                error_count = EXCLUDED.error_count,
                projected_at = NOW()
            "#,
        )
        .bind(id)
        .bind(org_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn project_session(&self, org_id: i64, source_id: &str) -> Result<()> {
        let id = Uuid::parse_str(source_id).context("invalid session source id")?;
        sqlx::query(
            r#"
            INSERT INTO fact_session (
                org_id, source_key, session_id, user_id, principal_id, agent_id,
                agent_name_snapshot, harness_id, harness_name_snapshot, app_id,
                blueprint_id, created_at, time_bucket_day, session_status, started_at,
                finished_at, last_active_at, session_count, duration_ms, turn_count,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                total_tokens, tool_call_count, projected_at
            )
            SELECT
                s.org_id,
                'session:' || s.id::TEXT,
                s.id,
                s.resolved_owner_user_id,
                s.owner_principal_id,
                s.agent_id,
                a.name,
                s.harness_id,
                h.name,
                s.app_id,
                s.blueprint_id,
                s.created_at,
                (s.created_at AT TIME ZONE 'UTC')::DATE,
                s.status,
                s.started_at,
                s.finished_at,
                s.updated_at,
                1,
                CASE
                    WHEN s.started_at IS NOT NULL AND s.finished_at IS NOT NULL
                    THEN GREATEST(EXTRACT(EPOCH FROM (s.finished_at - s.started_at)) * 1000, 0)::BIGINT
                    ELSE 0
                END,
                (
                    SELECT COUNT(*)
                    FROM events e
                    WHERE e.session_id = s.id
                      AND e.event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')
                ),
                s.total_input_tokens,
                s.total_output_tokens,
                s.total_cache_read_tokens,
                s.total_cache_creation_tokens,
                s.total_input_tokens + s.total_output_tokens
                    + s.total_cache_read_tokens + s.total_cache_creation_tokens,
                (
                    SELECT COUNT(*)
                    FROM events e
                    WHERE e.session_id = s.id
                      AND e.event_type = 'tool.completed'
                ),
                NOW()
            FROM sessions s
            LEFT JOIN agents a ON a.id = s.agent_id AND a.org_id = s.org_id
            LEFT JOIN harnesses h ON h.id = s.harness_id AND h.org_id = s.org_id
            WHERE s.id = $1 AND s.org_id = $2
            ON CONFLICT (org_id, source_key) DO UPDATE SET
                user_id = EXCLUDED.user_id,
                principal_id = EXCLUDED.principal_id,
                agent_id = EXCLUDED.agent_id,
                agent_name_snapshot = EXCLUDED.agent_name_snapshot,
                harness_id = EXCLUDED.harness_id,
                harness_name_snapshot = EXCLUDED.harness_name_snapshot,
                app_id = EXCLUDED.app_id,
                blueprint_id = EXCLUDED.blueprint_id,
                session_status = EXCLUDED.session_status,
                started_at = EXCLUDED.started_at,
                finished_at = EXCLUDED.finished_at,
                last_active_at = EXCLUDED.last_active_at,
                duration_ms = EXCLUDED.duration_ms,
                turn_count = EXCLUDED.turn_count,
                input_tokens = EXCLUDED.input_tokens,
                output_tokens = EXCLUDED.output_tokens,
                cache_read_tokens = EXCLUDED.cache_read_tokens,
                cache_creation_tokens = EXCLUDED.cache_creation_tokens,
                total_tokens = EXCLUDED.total_tokens,
                tool_call_count = EXCLUDED.tool_call_count,
                projected_at = NOW()
            "#,
        )
        .bind(id)
        .bind(org_id)
        .execute(&self.pool)
        .await?;
        self.project_configured_capabilities(org_id, id).await?;
        Ok(())
    }

    async fn project_configured_capabilities(&self, org_id: i64, id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            DELETE FROM fact_capability_usage
             WHERE org_id = $1
               AND session_id = $2
               AND capability_usage_kind = 'configured'
               AND source_key LIKE ('session:' || $2::TEXT || ':configured:%')
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            WITH session_row AS (
                SELECT s.*
                  FROM sessions s
                 WHERE s.id = $1 AND s.org_id = $2
            ),
            configured AS (
                SELECT ac.capability_id, 'agent' AS source_scope
                  FROM session_row s
                  JOIN agent_capabilities ac ON ac.agent_id = s.agent_id
                UNION ALL
                SELECT hc.capability_id, 'harness' AS source_scope
                  FROM session_row s
                  JOIN harness_capabilities hc ON hc.harness_id = s.harness_id
                UNION ALL
                SELECT COALESCE(cap.value->>'ref', cap.value #>> '{}') AS capability_id,
                       'session' AS source_scope
                  FROM session_row s
                  CROSS JOIN LATERAL jsonb_array_elements(
                      CASE WHEN jsonb_typeof(s.capabilities) = 'array'
                           THEN s.capabilities
                           ELSE '[]'::jsonb
                      END
                  ) AS cap(value)
            )
            INSERT INTO fact_capability_usage (
                org_id, source_key, session_id, turn_id, user_id, principal_id, agent_id,
                agent_name_snapshot, harness_id, harness_name_snapshot, app_id, blueprint_id,
                created_at, time_bucket_day, capability_id, capability_name_snapshot,
                capability_usage_kind, tool_name, usage_count, duration_ms, projected_at
            )
            SELECT
                s.org_id,
                'session:' || s.id::TEXT || ':configured:' || configured.source_scope || ':' || configured.capability_id,
                s.id,
                NULL,
                s.resolved_owner_user_id,
                s.owner_principal_id,
                s.agent_id,
                a.name,
                s.harness_id,
                h.name,
                s.app_id,
                s.blueprint_id,
                s.created_at,
                (s.created_at AT TIME ZONE 'UTC')::DATE,
                configured.capability_id,
                COALESCE(mcp.name, skill.name, declarative.display_name, declarative.name, configured.capability_id),
                'configured',
                NULL,
                1,
                0,
                NOW()
            FROM session_row s
            JOIN configured ON configured.capability_id IS NOT NULL AND configured.capability_id <> ''
            LEFT JOIN agents a ON a.id = s.agent_id AND a.org_id = s.org_id
            LEFT JOIN harnesses h ON h.id = s.harness_id AND h.org_id = s.org_id
            LEFT JOIN mcp_servers mcp
                ON configured.capability_id = ('mcp:' || mcp.id::TEXT)
               AND mcp.org_id = s.org_id
            LEFT JOIN skills skill
                ON configured.capability_id = ('skill:' || skill.id::TEXT)
               AND skill.org_id = s.org_id
            LEFT JOIN declarative_capabilities declarative
                ON configured.capability_id = ('declarative:' || declarative.name)
               AND declarative.org_id = s.org_id
            ON CONFLICT (org_id, source_key) DO UPDATE SET
                user_id = EXCLUDED.user_id,
                principal_id = EXCLUDED.principal_id,
                agent_id = EXCLUDED.agent_id,
                agent_name_snapshot = EXCLUDED.agent_name_snapshot,
                harness_id = EXCLUDED.harness_id,
                harness_name_snapshot = EXCLUDED.harness_name_snapshot,
                app_id = EXCLUDED.app_id,
                blueprint_id = EXCLUDED.blueprint_id,
                created_at = EXCLUDED.created_at,
                time_bucket_day = EXCLUDED.time_bucket_day,
                capability_name_snapshot = EXCLUDED.capability_name_snapshot,
                usage_count = EXCLUDED.usage_count,
                projected_at = NOW()
            "#,
        )
        .bind(id)
        .bind(org_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn project_budget_posting(&self, org_id: i64, source_id: &str) -> Result<()> {
        let id = Uuid::parse_str(source_id).context("invalid usage_ledger source id")?;
        sqlx::query(
            r#"
            INSERT INTO fact_budget_posting (
                org_id, source_key, session_id, user_id, principal_id, agent_id,
                agent_name_snapshot, harness_id, harness_name_snapshot, app_id,
                blueprint_id, created_at, time_bucket_day, budget_id,
                budget_subject_type, budget_subject_id, currency, meter_source,
                ref_type, amount, debit_count, credit_count, projected_at
            )
            SELECT
                ul.org_id,
                'usage_ledger:' || ul.id::TEXT,
                ul.session_id,
                ul.user_id,
                ul.principal_id,
                ul.agent_id,
                a.name,
                ul.harness_id,
                h.name,
                s.app_id,
                s.blueprint_id,
                ul.created_at,
                (ul.created_at AT TIME ZONE 'UTC')::DATE,
                ul.budget_id,
                b.subject_type,
                b.subject_id,
                ul.currency,
                ul.meter_source,
                ul.ref_type,
                ul.amount,
                CASE WHEN ul.amount >= 0 THEN 1 ELSE 0 END,
                CASE WHEN ul.amount < 0 THEN 1 ELSE 0 END,
                NOW()
            FROM usage_ledger ul
            LEFT JOIN budgets b ON b.id = ul.budget_id AND b.org_id = ul.org_id
            LEFT JOIN sessions s ON s.id = ul.session_id AND s.org_id = ul.org_id
            LEFT JOIN agents a ON a.id = ul.agent_id AND a.org_id = ul.org_id
            LEFT JOIN harnesses h ON h.id = ul.harness_id AND h.org_id = ul.org_id
            WHERE ul.id = $1 AND ul.org_id = $2
            ON CONFLICT (org_id, source_key) DO UPDATE SET
                session_id = EXCLUDED.session_id,
                user_id = EXCLUDED.user_id,
                principal_id = EXCLUDED.principal_id,
                agent_id = EXCLUDED.agent_id,
                agent_name_snapshot = EXCLUDED.agent_name_snapshot,
                harness_id = EXCLUDED.harness_id,
                harness_name_snapshot = EXCLUDED.harness_name_snapshot,
                app_id = EXCLUDED.app_id,
                blueprint_id = EXCLUDED.blueprint_id,
                created_at = EXCLUDED.created_at,
                time_bucket_day = EXCLUDED.time_bucket_day,
                budget_id = EXCLUDED.budget_id,
                budget_subject_type = EXCLUDED.budget_subject_type,
                budget_subject_id = EXCLUDED.budget_subject_id,
                currency = EXCLUDED.currency,
                meter_source = EXCLUDED.meter_source,
                ref_type = EXCLUDED.ref_type,
                amount = EXCLUDED.amount,
                debit_count = EXCLUDED.debit_count,
                credit_count = EXCLUDED.credit_count,
                projected_at = NOW()
            "#,
        )
        .bind(id)
        .bind(org_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
