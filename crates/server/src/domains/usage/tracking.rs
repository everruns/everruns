// Usage Tracking Listener
//
// This listener processes llm.generation events and:
// 1. Inserts records into llm_generations table
// 2. Updates denormalized totals on sessions and agents
//
// Decision: each row records which provider account served the call and
// whether that provider was host-managed at the time. The engine stamps the
// provider id on the event; the managed bit is read here, once, from the
// provider row, so later edits or deletion of the provider do not rewrite
// history. That is what lets usage tell managed spend from the org's own keys.
//
// This replaces the database trigger that was previously used
// (see knowledge/foundations/architecture.md for rationale on no-trigger policy).
//
// Decision: agent totals are added up in memory and written once a second,
// one statement for every agent with new usage. Every session of an agent
// shares its row, so a per-call UPDATE made all of them queue on one row lock
// (the second-largest statement in prod at 500 agents, and 50 ms of lock wait
// per turn in the stress bench). The totals are a display figure;
// `llm_generations` stays the per-call record. A process killed without a
// flush loses at most the last second of an agent's totals, the same kind of
// gap a crash between the generation insert and the per-call UPDATE left.

use async_trait::async_trait;
use everruns_core::{Event, EventData, EventListener, LLM_GENERATION};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tracing::{error, instrument};
use uuid::Uuid;

use crate::storage::StorageBackend;
use crate::storage::repositories::{AgentUsageDelta, GenerationProvider};
use everruns_contracts::typed_id::ProviderId;
use everruns_core::host::openai_agents_api::GENERATION_PROVIDER_ID;

/// Event listener that tracks LLM usage statistics.
///
/// Processes `llm.generation` events and updates:
/// - `llm_generations` table (source of truth for individual generations)
/// - `sessions` table (denormalized totals)
/// - `agents` table (denormalized totals)
pub struct UsageTrackingListener {
    db: Arc<StorageBackend>,
    agent_totals: Arc<AgentTotals>,
}

impl UsageTrackingListener {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            agent_totals: Arc::new(AgentTotals {
                db: db.clone(),
                pending: parking_lot::Mutex::new(HashMap::new()),
                flusher: OnceLock::new(),
            }),
            db,
        }
    }
}

/// How often added-up agent usage is written.
const AGENT_TOTALS_FLUSH: Duration = Duration::from_secs(1);

/// Agent usage not yet written to `agents`, by agent.
struct AgentTotals {
    db: Arc<StorageBackend>,
    pending: parking_lot::Mutex<HashMap<Uuid, AgentUsageDelta>>,
    flusher: OnceLock<()>,
}

impl AgentTotals {
    fn add(self: &Arc<Self>, agent_id: Uuid, delta: AgentUsageDelta) {
        self.pending.lock().entry(agent_id).or_default().add(&delta);
        self.flusher.get_or_init(|| {
            let totals = Arc::downgrade(self);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(AGENT_TOTALS_FLUSH).await;
                    let Some(totals) = totals.upgrade() else {
                        return;
                    };
                    totals.flush().await;
                }
            });
        });
    }

    async fn flush(&self) {
        let batch: Vec<(Uuid, AgentUsageDelta)> = self.pending.lock().drain().collect();
        if batch.is_empty() {
            return;
        }
        if let Err(e) = self.db.add_agent_usage_batch(&batch).await {
            error!(error = %e, agents = batch.len(), "Failed to update agent usage; retrying next flush");
            let mut pending = self.pending.lock();
            for (agent_id, delta) in batch {
                pending.entry(agent_id).or_default().add(&delta);
            }
        }
    }
}

/// Resolve the provider account that served a call.
///
/// A provider id that does not parse is a host without stored providers (an
/// embedded or in-memory host keys providers by name): nothing to record. A
/// provider that no longer exists keeps its id and counts as not managed.
pub(crate) async fn served_by(
    db: &StorageBackend,
    org_id: i64,
    provider_id: Option<&str>,
) -> GenerationProvider {
    let Some(id) = provider_id.and_then(|raw| ProviderId::parse(raw).ok()) else {
        return GenerationProvider::default();
    };
    let managed = match db.get_provider(org_id, id.uuid()).await {
        Ok(row) => row.is_some_and(|row| row.managed),
        Err(e) => {
            error!(error = %e, provider_id = %id, "Failed to read provider for usage tracking");
            false
        }
    };
    GenerationProvider {
        provider_config_id: Some(id.to_string()),
        managed,
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
                // No usage data. An Agents API turn billed before the provider
                // reported its usage is recorded as pending, for the late
                // usage reconciler to apply (EVE-1145); anything else has
                // nothing to track.
                if let Err(e) = super::agents_api::record_pending(&self.db, event, data).await {
                    error!(error = %e, "Failed to record pending Agents API usage");
                }
                return;
            }
        };

        let input_tokens = usage.input_tokens as i64;
        let output_tokens = usage.output_tokens as i64;
        let cache_read_tokens = usage.cache_read_tokens.unwrap_or(0) as i64;
        let cache_creation_tokens = usage.cache_creation_tokens.unwrap_or(0) as i64;
        // Per-generation cost is tracked as two independent figures: the
        // provider's actual cost (e.g. OpenRouter usage.cost) and a price-table
        // estimate. Either may be absent.
        let actual_cost_usd = usage.actual_cost_usd;
        let estimated_cost_usd = usage.estimated_cost_usd;
        // Denormalized totals treat an absent figure as zero. The best-effort
        // total prefers the actual cost and falls back to the estimate.
        let actual_cost_amount = actual_cost_usd.unwrap_or(0.0);
        let estimated_cost_amount = estimated_cost_usd.unwrap_or(0.0);
        let effective_cost_amount = usage.effective_cost_usd().unwrap_or(0.0);

        // Get session to determine org_id (unscoped lookup - internal system use)
        let session = match self.db.get_session_unscoped(event.session_id).await {
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

        // org_id comes directly from the session row
        let org_id = session.org_id;
        // The native engine stamps the provider on the generation; the OpenAI
        // Agents API backend carries it in the event metadata instead.
        let provider_id = data.metadata.provider_id.clone().or_else(|| {
            event
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get(GENERATION_PROVIDER_ID))
                .and_then(|value| value.as_str())
                .map(str::to_string)
        });
        let served_by = served_by(&self.db, org_id, provider_id.as_deref()).await;

        // Insert into llm_generations with org_id
        if let Err(e) = self
            .db
            .create_llm_generation(
                org_id,
                Some(event.session_id.uuid()),
                event.context.turn_id.as_ref().map(|id| id.uuid()),
                Some(event.id.uuid()),
                data.metadata.model.clone(),
                data.metadata.provider.clone(),
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                actual_cost_usd,
                estimated_cost_usd,
                data.metadata.duration_ms.map(|d| d as i32),
                data.metadata
                    .finish_reasons
                    .as_ref()
                    .and_then(|r| r.first().cloned()),
                data.metadata.response_id.clone(),
                served_by,
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
                actual_cost_amount,
                estimated_cost_amount,
                effective_cost_amount,
            )
            .await
        {
            error!("Failed to update session usage: {}", e);
        }

        // Agent totals (skip if no agent), written by the next flush.
        if let Some(agent_id) = session.agent_id {
            self.agent_totals.add(
                agent_id.uuid(),
                AgentUsageDelta {
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                    actual_cost_usd: actual_cost_amount,
                    estimated_cost_usd: estimated_cost_amount,
                    cost_usd: effective_cost_amount,
                },
            );
        }
    }

    async fn flush(&self) -> Result<(), String> {
        self.agent_totals.flush().await;
        Ok(())
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![LLM_GENERATION])
    }

    fn name(&self) -> &'static str {
        "UsageTrackingListener"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateProviderRow, CreateSessionRow, SessionRow};
    use everruns_contracts::typed_id::PrincipalId;
    use everruns_core::events::{EventContext, LlmGenerationData, TokenUsage};

    async fn session(db: &StorageBackend) -> SessionRow {
        db.create_session(CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            title: Some("Usage session".into()),
            capabilities: serde_json::json!({}),
            mcp_servers: serde_json::json!([]),
            initial_files: serde_json::json!({}),
            ..Default::default()
        })
        .await
        .unwrap()
    }

    async fn provider(db: &StorageBackend, managed: bool) -> String {
        let row = db
            .create_provider(
                1,
                CreateProviderRow {
                    name: if managed { "Everruns" } else { "My OpenAI" }.into(),
                    provider_type: "openai".into(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();
        db.set_provider_managed(1, row.id.uuid(), managed)
            .await
            .unwrap();
        row.id.to_string()
    }

    fn generation(session: &SessionRow, provider_id: Option<String>) -> Event {
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 20,
            ..Default::default()
        };
        let data = LlmGenerationData::success(
            vec![],
            vec![],
            Some("ok".into()),
            vec![],
            "gpt-6".into(),
            Some("openai".into()),
            Some(usage),
            None,
            None,
        )
        .with_provider_id(provider_id);
        Event::new(session.id, EventContext::empty(), data)
    }

    async fn recorded(db: &StorageBackend, session: &SessionRow) -> (Option<String>, bool) {
        sqlx::query_as(
            "SELECT provider_config_id, managed FROM llm_generations WHERE session_id = $1",
        )
        .bind(session.id.uuid())
        .fetch_one(db.pool())
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_call_on_a_managed_provider_is_recorded_as_managed() {
        let db = Arc::new(StorageBackend::test_database());
        let session = session(&db).await;
        let managed = provider(&db, true).await;

        UsageTrackingListener::new(db.clone())
            .on_event(&generation(&session, Some(managed.clone())))
            .await;

        assert_eq!(recorded(&db, &session).await, (Some(managed), true));
    }

    #[tokio::test]
    async fn a_call_on_the_orgs_own_key_is_recorded_as_not_managed() {
        let db = Arc::new(StorageBackend::test_database());
        let session = session(&db).await;
        let own = provider(&db, false).await;

        UsageTrackingListener::new(db.clone())
            .on_event(&generation(&session, Some(own.clone())))
            .await;

        assert_eq!(recorded(&db, &session).await, (Some(own), false));
    }

    #[tokio::test]
    async fn a_provider_named_by_key_not_id_records_no_account() {
        let db = Arc::new(StorageBackend::test_database());
        let session = session(&db).await;

        UsageTrackingListener::new(db.clone())
            .on_event(&generation(&session, Some("openai".into())))
            .await;

        assert_eq!(recorded(&db, &session).await, (None, false));
    }

    async fn agent(db: &StorageBackend) -> everruns_contracts::typed_id::AgentId {
        let id = everruns_contracts::typed_id::AgentId::new();
        let harness = everruns_contracts::typed_id::HarnessId::from_uuid(Uuid::nil());
        let mut definition = crate::platform_chat_agent::definition(harness, id);
        // The built-in definition's name is unique per org; these are ordinary agents.
        definition.name = format!("usage-{}", id.uuid().simple());
        definition.is_built_in = false;
        db.create_agent_with_id(1, id, definition)
            .await
            .unwrap()
            .unwrap();
        id
    }

    async fn agent_session(
        db: &StorageBackend,
        agent_id: everruns_contracts::typed_id::AgentId,
    ) -> SessionRow {
        db.create_session(CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            agent_id: Some(agent_id),
            capabilities: serde_json::json!({}),
            mcp_servers: serde_json::json!([]),
            initial_files: serde_json::json!({}),
            ..Default::default()
        })
        .await
        .unwrap()
    }

    async fn agent_tokens(db: &StorageBackend, agent_id: Uuid) -> (i64, i64) {
        sqlx::query_as("SELECT total_input_tokens, total_output_tokens FROM agents WHERE id = $1")
            .bind(agent_id)
            .fetch_one(db.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn agent_totals_add_up_across_sessions_and_are_written_on_flush() {
        let db = Arc::new(StorageBackend::test_database());
        let agent_id = agent(&db).await;
        let (first, second) = (
            agent_session(&db, agent_id).await,
            agent_session(&db, agent_id).await,
        );
        let listener = UsageTrackingListener::new(db.clone());

        for session in [&first, &second, &second] {
            listener.on_event(&generation(session, None)).await;
        }
        listener.flush().await.unwrap();

        assert_eq!(agent_tokens(&db, agent_id.uuid()).await, (300, 60));
        listener.flush().await.unwrap();
        assert_eq!(
            agent_tokens(&db, agent_id.uuid()).await,
            (300, 60),
            "a flush with nothing new adds nothing"
        );
    }

    #[tokio::test]
    async fn a_usage_batch_adds_to_each_agent() {
        let db = StorageBackend::test_database();
        let (a, b) = (agent(&db).await.uuid(), agent(&db).await.uuid());
        let delta = |input| AgentUsageDelta {
            input_tokens: input,
            output_tokens: 1,
            ..Default::default()
        };

        db.add_agent_usage_batch(&[(b, delta(7)), (a, delta(5))])
            .await
            .unwrap();
        db.add_agent_usage_batch(&[(a, delta(10))]).await.unwrap();
        db.add_agent_usage_batch(&[]).await.unwrap();

        assert_eq!(agent_tokens(&db, a).await, (15, 2));
        assert_eq!(agent_tokens(&db, b).await, (7, 1));
    }
}
