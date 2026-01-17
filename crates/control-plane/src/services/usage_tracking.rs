// Usage Tracking Listener
//
// This listener processes llm.generation events and:
// 1. Inserts records into llm_generations table
// 2. Updates denormalized totals on sessions and agents
//
// This replaces the database trigger that was previously used
// (see specs/architecture.md for rationale on no-trigger policy).

use async_trait::async_trait;
use everruns_core::{Event, EventData, EventListener, LLM_GENERATION};
use std::sync::Arc;
use tracing::{error, instrument};

use crate::storage::StorageBackend;

/// Event listener that tracks LLM usage statistics.
///
/// Processes `llm.generation` events and updates:
/// - `llm_generations` table (source of truth for individual generations)
/// - `sessions` table (denormalized totals)
/// - `agents` table (denormalized totals)
pub struct UsageTrackingListener {
    db: Arc<StorageBackend>,
}

impl UsageTrackingListener {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl EventListener for UsageTrackingListener {
    #[instrument(skip(self, event), fields(event_id = %event.id, session_id = %event.session_id))]
    async fn on_event(&self, event: &Event) {
        // Extract LLM generation data
        let EventData::LlmGeneration(data) = &event.data else {
            return;
        };

        // Extract usage data
        let usage = match &data.metadata.usage {
            Some(u) => u,
            None => {
                // No usage data, nothing to track
                return;
            }
        };

        let input_tokens = usage.input_tokens as i64;
        let output_tokens = usage.output_tokens as i64;
        let cache_read_tokens = usage.cache_read_tokens.unwrap_or(0) as i64;
        let cache_creation_tokens = usage.cache_creation_tokens.unwrap_or(0) as i64;

        // Insert into llm_generations
        if let Err(e) = self
            .db
            .create_llm_generation(
                event.session_id,
                event.context.turn_id,
                Some(event.id),
                data.metadata.model.clone(),
                data.metadata.provider.clone(),
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                data.metadata.duration_ms.map(|d| d as i32),
                data.metadata
                    .finish_reasons
                    .as_ref()
                    .and_then(|r| r.first().cloned()),
                event.ts,
            )
            .await
        {
            error!("Failed to insert llm_generation: {}", e);
            return;
        }

        // Update session totals
        if let Err(e) = self
            .db
            .increment_session_usage(
                event.session_id,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            )
            .await
        {
            error!("Failed to update session usage: {}", e);
        }

        // Update agent totals (need to get agent_id from session)
        if let Ok(Some(session)) = self.db.get_session(event.session_id).await
            && let Err(e) = self
                .db
                .increment_agent_usage(
                    session.agent_id,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                )
                .await
        {
            error!("Failed to update agent usage: {}", e);
        }
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![LLM_GENERATION])
    }

    fn name(&self) -> &'static str {
        "UsageTrackingListener"
    }
}
