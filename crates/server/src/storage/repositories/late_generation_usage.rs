//! Late usage of OpenAI Agents API generations (EVE-1145). See
//! `crate::storage::late_generation_usage`.

use anyhow::Result;
use tracing::warn;
use uuid::Uuid;

use super::Database;
use crate::storage::{
    CreatePendingUsageGeneration, GenerationUsageSnapshot, LateGenerationUsage,
    PendingUsageGeneration,
};

impl Database {
    pub async fn create_pending_usage_generation(
        &self,
        input: CreatePendingUsageGeneration,
    ) -> Result<Uuid> {
        let (id,): (Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO llm_generations (
                org_id, session_id, turn_id, event_id, model, provider,
                estimated_cost_usd, duration_ms, finish_reason,
                provider_response_id, provider_session_id, provider_config_id,
                usage_pending, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, TRUE, $13)
            RETURNING id
            "#,
        )
        .bind(input.org_id)
        .bind(input.session_id)
        .bind(input.turn_id)
        .bind(input.event_id)
        .bind(&input.model)
        .bind(&input.provider)
        .bind(input.estimated_cost_usd)
        .bind(input.duration_ms)
        .bind(&input.finish_reason)
        .bind(&input.provider_response_id)
        .bind(&input.provider_session_id)
        .bind(&input.provider_config_id)
        .bind(input.created_at)
        .fetch_one(&self.pool)
        .await?;
        self.enqueue_generation_projection(input.org_id, id, &id.to_string())
            .await;
        Ok(id)
    }

    /// Pending rows ready for another read, oldest first. A row that failed
    /// `max_attempts` times stays an explicit unknown and is no longer read.
    pub async fn list_pending_usage_generations(
        &self,
        max_attempts: i32,
        limit: i64,
    ) -> Result<Vec<PendingUsageGeneration>> {
        Ok(sqlx::query_as(
            r#"
            SELECT id, org_id, session_id, turn_id, event_id, model, provider,
                   provider_response_id, provider_session_id, provider_config_id
            FROM   llm_generations
            WHERE  usage_pending
              AND  provider_response_id IS NOT NULL
              AND  provider_session_id IS NOT NULL
              AND  provider_config_id IS NOT NULL
              AND  reconciliation_attempts < $1
              AND  (reconcile_after IS NULL OR reconcile_after <= NOW())
            ORDER  BY created_at ASC
            LIMIT  $2
            "#,
        )
        .bind(max_attempts)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The `usage_pending` predicate makes this the idempotency guard: of two
    /// concurrent or repeated calls, exactly one updates the row.
    pub async fn apply_late_generation_usage(
        &self,
        id: Uuid,
        usage: &LateGenerationUsage,
    ) -> Result<bool> {
        let row: Option<(i64,)> = sqlx::query_as(
            r#"
            UPDATE llm_generations
            SET
                input_tokens          = $2,
                output_tokens         = $3,
                cache_read_tokens     = $4,
                cache_creation_tokens = $5,
                estimated_cost_usd    = CASE
                    WHEN $6::DOUBLE PRECISION IS NULL THEN estimated_cost_usd
                    ELSE COALESCE(estimated_cost_usd, 0) + $6
                END,
                usage_pending         = FALSE,
                reconcile_after       = NULL,
                reconciled_at         = NOW()
            WHERE id = $1
              AND usage_pending
            RETURNING org_id
            "#,
        )
        .bind(id)
        .bind(usage.input_tokens)
        .bind(usage.output_tokens)
        .bind(usage.cache_read_tokens)
        .bind(usage.cache_creation_tokens)
        .bind(usage.estimated_cost_usd)
        .fetch_optional(&self.pool)
        .await?;
        let Some((org_id,)) = row else {
            return Ok(false);
        };
        self.enqueue_generation_projection(org_id, id, &format!("{id}:late_usage"))
            .await;
        Ok(true)
    }

    pub async fn get_generation_usage(&self, id: Uuid) -> Result<Option<GenerationUsageSnapshot>> {
        Ok(sqlx::query_as(
            r#"
            SELECT input_tokens, output_tokens, cache_read_tokens, estimated_cost_usd,
                   usage_pending, reconciliation_attempts
            FROM   llm_generations
            WHERE  id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Best-effort, as for every generation.
    async fn enqueue_generation_projection(&self, org_id: i64, id: Uuid, version: &str) {
        if let Err(e) = self
            .enqueue_reporting_outbox(
                org_id,
                "llm_generation",
                &id.to_string(),
                Some(version),
                "llm_generation_projection",
            )
            .await
        {
            warn!(
                generation_id = %id,
                org_id,
                error = %e,
                "reporting outbox enqueue failed for llm_generation; projection may remain stale"
            );
        }
    }
}
