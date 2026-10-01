//! Late usage of OpenAI Agents API generations (EVE-1145), in memory.
//!
//! The in-memory backend keeps no ordinary generation rows (only totals), but
//! it keeps pending ones so dev mode and tests reconcile late usage the way
//! PostgreSQL does.

use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::InMemoryDatabase;
use crate::storage::{
    CreatePendingUsageGeneration, GenerationUsageSnapshot, LateGenerationUsage,
    PendingUsageGeneration,
};

#[derive(Clone, Debug)]
pub(crate) struct MemoryUsageGeneration {
    pending: PendingUsageGeneration,
    usage: GenerationUsageSnapshot,
    reconcile_after: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

impl InMemoryDatabase {
    pub async fn create_pending_usage_generation(
        &self,
        input: CreatePendingUsageGeneration,
    ) -> Result<Uuid> {
        let id = Uuid::now_v7();
        let row = MemoryUsageGeneration {
            pending: PendingUsageGeneration {
                id,
                org_id: input.org_id,
                session_id: Some(input.session_id),
                turn_id: input.turn_id,
                event_id: input.event_id,
                model: Some(input.model),
                provider: input.provider,
                provider_response_id: input.provider_response_id,
                provider_session_id: input.provider_session_id,
                provider_config_id: input.provider_config_id,
            },
            usage: GenerationUsageSnapshot {
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                estimated_cost_usd: input.estimated_cost_usd,
                usage_pending: true,
                reconciliation_attempts: 0,
            },
            reconcile_after: None,
            created_at: input.created_at,
        };
        self.usage_generations.write().insert(id, row);
        Ok(id)
    }

    pub async fn list_pending_usage_generations(
        &self,
        max_attempts: i32,
        limit: i64,
    ) -> Result<Vec<PendingUsageGeneration>> {
        let now = Self::now();
        let rows = self.usage_generations.read();
        let mut ready: Vec<&MemoryUsageGeneration> = rows
            .values()
            .filter(|row| row.usage.usage_pending)
            .filter(|row| row.usage.reconciliation_attempts < max_attempts)
            .filter(|row| row.reconcile_after.is_none_or(|after| after <= now))
            .collect();
        ready.sort_by_key(|row| row.created_at);
        Ok(ready
            .into_iter()
            .take(usize::try_from(limit).unwrap_or(0))
            .map(|row| row.pending.clone())
            .collect())
    }

    pub async fn apply_late_generation_usage(
        &self,
        id: Uuid,
        usage: &LateGenerationUsage,
    ) -> Result<bool> {
        let mut rows = self.usage_generations.write();
        let Some(row) = rows.get_mut(&id).filter(|row| row.usage.usage_pending) else {
            return Ok(false);
        };
        row.usage.input_tokens = usage.input_tokens;
        row.usage.output_tokens = usage.output_tokens;
        row.usage.cache_read_tokens = usage.cache_read_tokens;
        if let Some(tokens) = usage.estimated_cost_usd {
            row.usage.estimated_cost_usd =
                Some(row.usage.estimated_cost_usd.unwrap_or(0.0) + tokens);
        }
        row.usage.usage_pending = false;
        row.reconcile_after = None;
        Ok(true)
    }

    pub async fn get_generation_usage(&self, id: Uuid) -> Result<Option<GenerationUsageSnapshot>> {
        Ok(self
            .usage_generations
            .read()
            .get(&id)
            .map(|row| row.usage.clone()))
    }

    /// The pending-row half of `mark_llm_generation_reconciliation_failed`.
    pub(super) fn delay_pending_usage_generation(&self, id: Uuid, retry_after_seconds: i32) {
        if let Some(row) = self
            .usage_generations
            .write()
            .get_mut(&id)
            .filter(|row| row.usage.usage_pending)
        {
            row.usage.reconciliation_attempts += 1;
            row.reconcile_after =
                Some(Self::now() + chrono::Duration::seconds(i64::from(retry_after_seconds)));
        }
    }
}
