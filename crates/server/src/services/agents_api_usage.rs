// Late usage of OpenAI Agents API turns (EVE-1145).
//
// The Agents API fills a turn's usage after `turn.completed`. The durable
// driver re-reads the turn a few times; a turn whose usage is still null is
// billed as an explicit unknown (a `model_tokens` cost component with no
// amount). Here that generation is recorded as `usage_pending`, and a
// periodic pass reads the turn again (by `response_id` in the provider
// session, with the provider credentials that ran it) and applies the usage
// to the record, the session and agent totals, and the budget ledger.
//
// Decision: no double debit, without a distributed lock. Every replica may
// run the pass. The budget debit comes first and is journaled under the
// generation record (`llm_generation_late_usage`, generation id), which the
// journal's unique `(source_type, source_id)` index admits once. The usage is
// then applied by an UPDATE guarded on `usage_pending`, which exactly one
// caller wins, and only the winner adds to the session and agent totals. A
// crash between the debit and the update re-runs both; the journal index
// refuses the second debit. The in-memory dev backend has no such index, so
// there the `usage_pending` claim alone keeps sequential passes from
// double-debiting.
//
// Decision: the read leaves the platform only through the official OpenAI
// API with the turn's own provider key (TM-LLM-043), as the driver does. A
// provider that is gone, changed type, or moved off the official API keeps
// the generation an explicit unknown rather than sending its ids elsewhere.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use everruns_contracts::driver_registry::{DriverId, ProviderConfig};
use everruns_contracts::model_profiles::estimate_cost_usd;
use everruns_contracts::runtime_provider::ProviderKey;
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::correlation::{PROVIDER_SESSION_ID, RUNTIME_BACKEND};
use everruns_core::events::{Event, LlmCostComponent, LlmGenerationData};
use everruns_host::openai_agents_api::backend::official_endpoint;
use everruns_host::openai_agents_api::{AgentsApiClient, GENERATION_PROVIDER_ID, usage_from};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::domains::budgets::BudgetService;
use crate::services::ProviderResolverService;
use crate::storage::{
    CreatePendingUsageGeneration, EncryptionService, LateGenerationUsage, PendingUsageGeneration,
    StorageBackend,
};

/// `runtime_backend` of events the Agents API driver projects.
const AGENTS_API_BACKEND: &str = "openai_agents_api";
const BATCH: i64 = 50;
/// The provider fills usage within moments of `turn.completed`; a turn still
/// without usage after this many reads (about an hour apart from the first)
/// stays an explicit unknown.
const MAX_ATTEMPTS: i32 = 30;
const RETRY_AFTER_SECONDS: i32 = 2 * 60;
const MISSING_PROVIDER_RETRY_AFTER_SECONDS: i32 = 60 * 60;
const INTERVAL_ENV: &str = "AGENTS_API_USAGE_RECONCILE_INTERVAL_SECS";
const DEFAULT_INTERVAL_SECS: u64 = 60;

/// The pending record of an Agents API generation billed before its usage
/// arrived, or `None` for any other generation.
pub(crate) fn pending_generation(
    event: &Event,
    data: &LlmGenerationData,
    org_id: i64,
) -> Option<CreatePendingUsageGeneration> {
    let metadata = event.metadata.as_ref()?;
    let text = |key: &str| {
        metadata
            .get(key)
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    if text(RUNTIME_BACKEND).as_deref() != Some(AGENTS_API_BACKEND) || data.metadata.usage.is_some()
    {
        return None;
    }
    let tokens_unknown = data.metadata.cost_components.iter().any(|component| {
        component.kind == LlmCostComponent::MODEL_TOKENS
            && component.quantity.is_none()
            && component.cost_usd.is_none()
    });
    if !tokens_unknown {
        return None;
    }
    Some(CreatePendingUsageGeneration {
        org_id,
        session_id: event.session_id.uuid(),
        turn_id: event.context.turn_id.as_ref().map(|id| id.uuid()),
        event_id: Some(event.id.uuid()),
        model: data.metadata.model.clone(),
        provider: data.metadata.provider.clone(),
        estimated_cost_usd: priced_components_usd(data),
        duration_ms: data.metadata.duration_ms.map(|ms| ms as i32),
        finish_reason: data
            .metadata
            .finish_reasons
            .as_ref()
            .and_then(|reasons| reasons.first().cloned()),
        provider_response_id: data.metadata.response_id.clone()?,
        provider_session_id: text(PROVIDER_SESSION_ID)?,
        provider_config_id: text(GENERATION_PROVIDER_ID)?,
        created_at: event.ts,
    })
}

/// What the generation's priced components (per-call tools) cost, which the
/// budget already debited when the event was metered.
fn priced_components_usd(data: &LlmGenerationData) -> Option<f64> {
    let priced: Vec<f64> = data
        .metadata
        .cost_components
        .iter()
        .filter_map(|component| component.cost_usd)
        .collect();
    (!priced.is_empty()).then(|| priced.iter().sum())
}

/// Record an Agents API generation whose usage is still unknown, so the
/// reconciler can apply it later. The session and agent totals take the
/// priced components now, matching what the budget debited.
pub(crate) async fn record_pending(
    db: &StorageBackend,
    event: &Event,
    data: &LlmGenerationData,
) -> Result<bool> {
    let Some(session) = db.get_session_unscoped(event.session_id).await? else {
        return Ok(false);
    };
    let Some(pending) = pending_generation(event, data, session.org_id) else {
        return Ok(false);
    };
    let priced = pending.estimated_cost_usd.unwrap_or(0.0);
    db.create_pending_usage_generation(pending).await?;
    if priced > 0.0 {
        add_to_totals(
            db,
            event.session_id,
            session.agent_id.map(|id| id.uuid()),
            &zero_usage(),
            priced,
        )
        .await;
    }
    Ok(true)
}

fn zero_usage() -> LateGenerationUsage {
    LateGenerationUsage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        estimated_cost_usd: None,
    }
}

async fn add_to_totals(
    db: &StorageBackend,
    session_id: SessionId,
    agent_id: Option<Uuid>,
    usage: &LateGenerationUsage,
    cost_usd: f64,
) {
    if let Err(error) = db
        .increment_session_usage(
            session_id.uuid(),
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.cache_creation_tokens,
            0.0,
            cost_usd,
            cost_usd,
        )
        .await
    {
        error!(%session_id, %error, "failed to add late usage to session totals");
    }
    if let Some(agent_id) = agent_id
        && let Err(error) = db
            .increment_agent_usage(
                agent_id,
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_creation_tokens,
                0.0,
                cost_usd,
                cost_usd,
            )
            .await
    {
        error!(%agent_id, %error, "failed to add late usage to agent totals");
    }
}

/// How one pending generation fared in a pass.
#[derive(Debug, PartialEq)]
enum Outcome {
    /// Usage applied by this pass.
    Applied,
    /// Usage already applied by another pass; nothing metered here.
    AlreadyApplied,
    /// Not readable yet; retried later.
    Delayed,
}

/// Applies usage the OpenAI Agents API reported after its turn was billed.
pub struct AgentsApiUsageReconciler {
    db: Arc<StorageBackend>,
    resolver: ProviderResolverService,
    budgets: BudgetService,
    /// Tests point the client at a fake server instead of the official API.
    base_url_override: Option<String>,
}

impl AgentsApiUsageReconciler {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self {
            budgets: BudgetService::new(db.clone()),
            resolver: ProviderResolverService::new(db.clone(), encryption),
            db,
            base_url_override: None,
        }
    }

    #[cfg(test)]
    fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url_override = Some(base_url.into());
        self
    }

    /// One pass over the generations whose usage is pending. Returns how many
    /// this pass applied.
    pub async fn reconcile(&self) -> Result<usize> {
        let rows = self
            .db
            .list_pending_usage_generations(MAX_ATTEMPTS, BATCH)
            .await?;
        let mut clients: HashMap<(i64, String), Option<AgentsApiClient>> = HashMap::new();
        let mut applied = 0;
        for row in rows {
            let key = (row.org_id, row.provider_config_id.clone());
            if !clients.contains_key(&key) {
                let client = self.client(row.org_id, &row.provider_config_id).await;
                clients.insert(key.clone(), client);
            }
            let outcome = match clients.get(&key).and_then(Option::as_ref) {
                Some(client) => self.reconcile_one(client, &row).await,
                None => {
                    debug!(
                        generation_id = %row.id,
                        org_id = row.org_id,
                        "no official OpenAI provider for late Agents API usage; delaying"
                    );
                    self.delay(row.id, MISSING_PROVIDER_RETRY_AFTER_SECONDS)
                        .await;
                    Outcome::Delayed
                }
            };
            if outcome == Outcome::Applied {
                applied += 1;
            }
        }
        Ok(applied)
    }

    async fn reconcile_one(
        &self,
        client: &AgentsApiClient,
        row: &PendingUsageGeneration,
    ) -> Outcome {
        let turn = match client
            .retrieve_turn(&row.provider_session_id, &row.provider_response_id)
            .await
        {
            Ok(turn) => turn,
            Err(error) => {
                warn!(
                    generation_id = %row.id,
                    error = %error,
                    "Agents API turn lookup for late usage failed; delaying"
                );
                self.delay(row.id, RETRY_AFTER_SECONDS).await;
                return Outcome::Delayed;
            }
        };
        let Some(usage) = usage_from(&turn) else {
            self.delay(row.id, RETRY_AFTER_SECONDS).await;
            return Outcome::Delayed;
        };
        let model = row.model.as_deref().unwrap_or_default();
        let late = LateGenerationUsage {
            input_tokens: i64::from(usage.input_tokens),
            output_tokens: i64::from(usage.output_tokens),
            cache_read_tokens: i64::from(usage.cache_read_tokens.unwrap_or(0)),
            cache_creation_tokens: i64::from(usage.cache_creation_tokens.unwrap_or(0)),
            // Priced as the driver prices tokens it read in time.
            estimated_cost_usd: estimate_cost_usd(
                &DriverId::OpenAI,
                model,
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens.unwrap_or(0),
                usage.cache_creation_tokens.unwrap_or(0),
            ),
        };
        let Some(session_uuid) = row.session_id else {
            return Outcome::Delayed;
        };
        let session_id = SessionId::from_uuid(session_uuid);
        // Debit first: journaled under the generation, admitted once.
        self.budgets
            .meter_late_usage(
                session_id,
                row.id,
                row.turn_id,
                row.event_id,
                row.model.as_deref(),
                row.provider.as_deref(),
                &late,
            )
            .await;
        match self.db.apply_late_generation_usage(row.id, &late).await {
            Ok(true) => {}
            Ok(false) => return Outcome::AlreadyApplied,
            Err(error) => {
                error!(generation_id = %row.id, %error, "failed to apply late Agents API usage");
                return Outcome::Delayed;
            }
        }
        let agent_id = match self.db.get_session_unscoped(session_id).await {
            Ok(session) => session.and_then(|s| s.agent_id).map(|id| id.uuid()),
            Err(error) => {
                error!(%session_id, %error, "failed to load session for late usage totals");
                None
            }
        };
        add_to_totals(
            &self.db,
            session_id,
            agent_id,
            &late,
            late.estimated_cost_usd.unwrap_or(0.0),
        )
        .await;
        info!(
            generation_id = %row.id,
            input_tokens = late.input_tokens,
            output_tokens = late.output_tokens,
            "applied late Agents API usage"
        );
        Outcome::Applied
    }

    async fn delay(&self, id: Uuid, retry_after_seconds: i32) {
        if let Err(error) = self
            .db
            .mark_llm_generation_reconciliation_failed(id, retry_after_seconds)
            .await
        {
            error!(generation_id = %id, %error, "failed to delay late usage reconciliation");
        }
    }

    /// A client over the turn's own provider, on the official API only.
    async fn client(&self, org_id: i64, provider_config_id: &str) -> Option<AgentsApiClient> {
        let resolved = match self
            .resolver
            .resolve_runtime_provider_config(org_id, provider_config_id)
            .await
        {
            Ok(resolved) => resolved?,
            Err(error) => {
                warn!(org_id, %error, "provider lookup for late Agents API usage failed");
                return None;
            }
        };
        let mut config = ProviderConfig::for_provider(
            ProviderKey::new(provider_config_id),
            resolved
                .provider_type
                .to_ascii_lowercase()
                .parse()
                .unwrap_or(DriverId::OpenAI),
        );
        config.api_key = resolved.api_key;
        config.base_url = resolved.base_url;
        // THREAT[TM-LLM-043]: provider ids and the key go only to the
        // official API, exactly as for turns.
        let endpoint = official_endpoint(&config)?;
        let client = AgentsApiClient::from_endpoint(endpoint);
        Some(match &self.base_url_override {
            Some(base) => client.with_base_url(base.clone()),
            None => client,
        })
    }
}

/// Run the reconciler every `AGENTS_API_USAGE_RECONCILE_INTERVAL_SECS`
/// (default 60; 0 disables).
pub fn spawn_agents_api_usage_reconciler(
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
) -> Option<JoinHandle<()>> {
    let interval = std::env::var(INTERVAL_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS);
    if interval == 0 {
        info!("Agents API late usage reconciler disabled ({INTERVAL_ENV}=0)");
        return None;
    }
    let reconciler = AgentsApiUsageReconciler::new(db, encryption);
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(interval));
        ticker.tick().await;
        loop {
            ticker.tick().await;
            match reconciler.reconcile().await {
                Ok(0) => {}
                Ok(applied) => debug!(applied, "Agents API late usage pass completed"),
                Err(error) => error!(%error, "Agents API late usage pass failed"),
            }
        }
    }))
}

#[cfg(test)]
#[path = "agents_api_usage_tests.rs"]
mod tests;
