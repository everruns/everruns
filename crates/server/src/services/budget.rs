// Budget Service
//
// Combines: metering (LlmTokenMeter), rules (hard stop, soft pause, warn),
// and evaluation pipeline. Implements EventListener to hook into llm.generation events.
//
// See specs/budgeting.md for full specification.

use async_trait::async_trait;
use everruns_core::EventListener;
use everruns_core::budget::{
    Budget, BudgetAction, BudgetCheckResult, BudgetStatus, BudgetSubjectType, LedgerEntry,
};
use everruns_core::events::{Event, EventData, LLM_GENERATION};
use everruns_core::llm_model_profiles::get_model_profile;
use everruns_core::llm_models::LlmProviderType;
use everruns_core::typed_id::{BudgetId, SessionId};
use std::sync::Arc;
use tracing::{error, info, instrument, warn};

use crate::storage::StorageBackend;
use crate::storage::models::*;

// ============================================================================
// BudgetService
// ============================================================================

pub struct BudgetService {
    db: Arc<StorageBackend>,
}

impl BudgetService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    /// Convert a storage row to the API-facing Budget DTO.
    pub fn row_to_budget(row: &BudgetRow) -> Budget {
        Budget {
            id: BudgetId::from_uuid(row.id),
            organization_id: everruns_core::org_public_id_from_internal(row.org_id),
            subject_type: BudgetSubjectType::from(row.subject_type.as_str()),
            subject_id: row.subject_id.clone(),
            currency: row.currency.clone(),
            limit: row.limit,
            soft_limit: row.soft_limit,
            balance: row.balance,
            period: row
                .period
                .as_ref()
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
            metadata: row.metadata.clone(),
            status: BudgetStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    pub fn row_to_ledger_entry(row: &BudgetLedgerRow) -> LedgerEntry {
        LedgerEntry {
            id: format!("ledger_{}", row.id.to_string().replace('-', "")),
            budget_id: BudgetId::from_uuid(row.budget_id),
            amount: row.amount,
            meter_source: row.meter_source.clone(),
            ref_type: row.ref_type.clone(),
            ref_id: row.ref_id.map(|id| id.to_string()),
            session_id: row.session_id.map(SessionId::from_uuid),
            description: row.description.clone(),
            created_at: row.created_at,
        }
    }

    // ============================================================================
    // Evaluation pipeline: meter → ledger → rules
    // ============================================================================

    /// Process an LLM generation event: meter tokens, record ledger entries,
    /// evaluate rules, and return the most restrictive action.
    async fn process_llm_generation(
        &self,
        event: &Event,
        model: Option<&str>,
        provider: Option<&str>,
        input_tokens: i64,
        output_tokens: i64,
    ) {
        // Get session to find subject hierarchy
        let session = match self.db.get_session_unscoped(event.session_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                error!(
                    "Session not found for budget tracking: {}",
                    event.session_id
                );
                return;
            }
            Err(e) => {
                error!("Failed to get session for budget tracking: {}", e);
                return;
            }
        };

        let session_public_id = event.session_id.to_string();
        let agent_public_id = session.agent_id.map(|a| a.to_string());

        // Find all active budgets in the subject hierarchy
        let budgets = match self
            .db
            .get_active_budgets_for_session(
                session.org_id,
                &session_public_id,
                agent_public_id.as_deref(),
                // TODO: user_id and org_public_id not yet available in event context;
                // user/org-scoped budgets will require plumbing these through session/turn context
                None,
                None,
            )
            .await
        {
            Ok(b) => b,
            Err(e) => {
                error!(
                    "Failed to fetch budgets for session {}: {}",
                    event.session_id, e
                );
                return;
            }
        };

        if budgets.is_empty() {
            return; // No budgets — nothing to track
        }

        let total_tokens = input_tokens + output_tokens;

        // For each budget, compute the debit amount in budget currency and record it
        for budget in &budgets {
            let debit = self.compute_debit(
                &budget.currency,
                total_tokens,
                input_tokens,
                output_tokens,
                model,
                provider,
            );

            if debit <= 0.0 {
                continue;
            }

            // Record ledger entry and get updated balance
            let ledger_input = CreateBudgetLedgerRow {
                budget_id: budget.id,
                amount: debit,
                meter_source: "llm_tokens".into(),
                ref_type: Some("llm_generation".into()),
                ref_id: Some(event.id.uuid()),
                session_id: Some(event.session_id.uuid()),
                description: model.map(|m| format!("{} tokens on {}", total_tokens, m)),
            };

            let updated_budget = match self.db.create_budget_ledger_entry(ledger_input).await {
                Ok((_entry, budget)) => budget,
                Err(e) => {
                    error!("Failed to record budget ledger entry: {}", e);
                    continue;
                }
            };

            // Evaluate rules
            let action = self.evaluate_rules(&updated_budget);

            match action {
                BudgetAction::Continue => {}
                BudgetAction::Warn { message } => {
                    info!(
                        budget_id = %updated_budget.id,
                        balance = updated_budget.balance,
                        "Budget warning: {}",
                        message
                    );
                    // Budget warning events would be emitted via EventEmitter
                    // which is not available in EventListener context.
                    // The worker checks budget status between atoms instead.
                }
                BudgetAction::Pause { message } => {
                    warn!(
                        budget_id = %updated_budget.id,
                        balance = updated_budget.balance,
                        "Budget pause triggered: {}",
                        message
                    );
                    // Mark budget as paused
                    if let Err(e) = self.db.set_budget_status(updated_budget.id, "paused").await {
                        error!("Failed to set budget paused: {}", e);
                    }
                }
                BudgetAction::Stop { message } => {
                    warn!(
                        budget_id = %updated_budget.id,
                        balance = updated_budget.balance,
                        "Budget exhausted: {}",
                        message
                    );
                    // Mark budget as exhausted
                    if let Err(e) = self
                        .db
                        .set_budget_status(updated_budget.id, "exhausted")
                        .await
                    {
                        error!("Failed to set budget exhausted: {}", e);
                    }
                }
            }
        }
    }

    /// Compute the debit amount in the budget's currency.
    pub(crate) fn compute_debit(
        &self,
        currency: &str,
        total_tokens: i64,
        input_tokens: i64,
        output_tokens: i64,
        model: Option<&str>,
        provider: Option<&str>,
    ) -> f64 {
        match currency {
            "tokens" => total_tokens as f64,
            "usd" => {
                // Look up model cost from profiles
                let provider_type = provider
                    .and_then(|p| p.parse::<LlmProviderType>().ok())
                    .unwrap_or(LlmProviderType::Openai);
                let model_id = model.unwrap_or("unknown");
                if let Some(profile) = get_model_profile(&provider_type, model_id) {
                    if let Some(cost) = profile.cost {
                        // Cost is per million tokens
                        let input_cost = (input_tokens as f64 / 1_000_000.0) * cost.input;
                        let output_cost = (output_tokens as f64 / 1_000_000.0) * cost.output;
                        input_cost + output_cost
                    } else {
                        // No cost data — fall back to token count
                        warn!(
                            model = model_id,
                            "No cost data for model, using token count as debit"
                        );
                        total_tokens as f64
                    }
                } else {
                    warn!(
                        model = model_id,
                        "No profile for model, using token count as debit"
                    );
                    total_tokens as f64
                }
            }
            "credits" => {
                // 1 credit = 1000 tokens (default rate, customizable via metadata)
                total_tokens as f64 / 1000.0
            }
            _ => {
                // Unknown currency — use raw token count
                total_tokens as f64
            }
        }
    }

    /// Evaluate budget rules and return the most restrictive action.
    pub(crate) fn evaluate_rules(&self, budget: &BudgetRow) -> BudgetAction {
        // Rule 1: Hard stop at 0
        if budget.balance <= 0.0 {
            return BudgetAction::Stop {
                message: format!(
                    "Budget exhausted. {:.2} {} spent of {:.2} {} limit.",
                    budget.limit - budget.balance,
                    budget.currency,
                    budget.limit,
                    budget.currency
                ),
            };
        }

        // Rule 2: Soft limit pause
        if let Some(soft_limit) = budget.soft_limit
            && budget.balance <= (budget.limit - soft_limit)
        {
            return BudgetAction::Pause {
                message: format!(
                    "Soft limit reached. {:.2} {} spent of {:.2} {} soft limit.",
                    budget.limit - budget.balance,
                    budget.currency,
                    soft_limit,
                    budget.currency
                ),
            };
        }

        // Rule 3: Warn at 80% of limit
        let warn_threshold = budget.limit * 0.2; // warn when 20% remaining
        if budget.balance <= warn_threshold {
            return BudgetAction::Warn {
                message: format!(
                    "Budget running low. {:.2} {} remaining of {:.2} {} limit.",
                    budget.balance, budget.currency, budget.limit, budget.currency
                ),
            };
        }

        BudgetAction::Continue
    }

    // ============================================================================
    // Public API for checking budgets (called by worker between atoms)
    // ============================================================================

    /// Check all active budgets for a session and return the most restrictive action.
    /// This is called by the worker between atoms to decide whether to continue.
    pub async fn check_budgets_for_session(
        &self,
        org_id: i64,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> BudgetCheckResult {
        // Query session-scoped budgets
        let mut all_matching = match self
            .db
            .list_budgets(org_id, Some("session"), Some(session_id))
            .await
        {
            Ok(b) => b,
            Err(e) => {
                error!("Failed to check session budgets: {}", e);
                return BudgetCheckResult::ok();
            }
        };

        // Query agent-scoped budgets if agent_id is provided
        if let Some(aid) = agent_id {
            match self.db.list_budgets(org_id, Some("agent"), Some(aid)).await {
                Ok(b) => all_matching.extend(b),
                Err(e) => error!("Failed to check agent budgets: {}", e),
            }
        }

        // TODO: user_id and org_public_id not yet available in event context;
        // user/org-scoped budgets will require plumbing these through session/turn context

        // Evaluate ALL matching budgets and keep the most restrictive result.
        // Priority: stop > pause > warn > continue
        let mut most_restrictive = BudgetCheckResult::ok();
        let mut most_restrictive_priority: u8 = 0; // 0=continue, 1=warn, 2=pause, 3=stop

        for budget in &all_matching {
            let (action_str, message, priority) =
                if budget.status == "exhausted" || budget.balance <= 0.0 {
                    (
                        "stop",
                        format!(
                            "Budget exhausted ({} {})",
                            budget.currency,
                            BudgetId::from_uuid(budget.id)
                        ),
                        3u8,
                    )
                } else if budget.status == "paused" {
                    (
                        "pause",
                        format!(
                            "Budget paused ({} {})",
                            budget.currency,
                            BudgetId::from_uuid(budget.id)
                        ),
                        2u8,
                    )
                } else {
                    // Active budget — evaluate rules
                    match self.evaluate_rules(budget) {
                        BudgetAction::Stop { message } => ("stop", message, 3u8),
                        BudgetAction::Pause { message } => ("pause", message, 2u8),
                        BudgetAction::Warn { message } => ("warn", message, 1u8),
                        BudgetAction::Continue => continue,
                    }
                };

            if priority > most_restrictive_priority {
                most_restrictive_priority = priority;
                most_restrictive = BudgetCheckResult {
                    action: action_str.into(),
                    message: Some(message),
                    budget_id: Some(BudgetId::from_uuid(budget.id)),
                    balance: Some(budget.balance),
                    currency: Some(budget.currency.clone()),
                };
            }
        }

        most_restrictive
    }
}

// ============================================================================
// EventListener — hooks into llm.generation events
// ============================================================================

#[async_trait]
impl EventListener for BudgetService {
    #[instrument(skip(self, event), fields(event_id = %event.id, session_id = %event.session_id))]
    async fn on_event(&self, event: &Event) {
        let EventData::LlmGeneration(data) = &event.data else {
            return;
        };

        let usage = match &data.metadata.usage {
            Some(u) => u,
            None => return,
        };

        let input_tokens = usage.input_tokens as i64;
        let output_tokens = usage.output_tokens as i64;

        self.process_llm_generation(
            event,
            Some(data.metadata.model.as_str()),
            data.metadata.provider.as_deref(),
            input_tokens,
            output_tokens,
        )
        .await;
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![LLM_GENERATION])
    }

    fn name(&self) -> &'static str {
        "BudgetService"
    }
}
