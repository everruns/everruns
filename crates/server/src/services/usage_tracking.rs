// Usage Tracking Listener
//
// This listener processes llm.generation events and:
// 1. Inserts records into llm_generations table
// 2. Updates denormalized totals on sessions and agents
//
// This replaces the database trigger that was previously used
// (see specs/architecture.md for rationale on no-trigger policy).

use async_trait::async_trait;
use everruns_core::{DEFAULT_ORG_ID, Event, EventData, EventListener, LLM_GENERATION};
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

        // Get session to determine org_id
        let session = match self.db.get_session(DEFAULT_ORG_ID, event.session_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                error!("Session not found for usage tracking: {}", event.session_id);
                return;
            }
            Err(e) => {
                error!("Failed to get session for usage tracking: {}", e);
                return;
            }
        };

        // Get org_id from agent (sessions don't have direct org_id)
        let org_id = if let Some(agent_id) = session.agent_id {
            match self.db.get_agent(DEFAULT_ORG_ID, agent_id).await {
                Ok(Some(agent)) => agent.org_id,
                Ok(None) => {
                    error!("Agent not found for usage tracking: {}", agent_id);
                    DEFAULT_ORG_ID
                }
                Err(e) => {
                    error!("Failed to get agent for usage tracking: {}", e);
                    DEFAULT_ORG_ID
                }
            }
        } else {
            DEFAULT_ORG_ID
        };

        // Insert into llm_generations with org_id
        if let Err(e) = self
            .db
            .create_llm_generation(
                org_id,
                event.session_id.uuid(),
                event.context.turn_id.as_ref().map(|id| id.uuid()),
                Some(event.id.uuid()),
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
                event.session_id.uuid(),
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            )
            .await
        {
            error!("Failed to update session usage: {}", e);
        }

        // Update agent totals (skip if no agent)
        if let Some(agent_id) = session.agent_id
            && let Err(e) = self
                .db
                .increment_agent_usage(
                    agent_id.uuid(),
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
