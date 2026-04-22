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
use everruns_core::typed_id::{AgentId, BudgetId, SessionId};
use everruns_core::{UserFacingError, user_facing_error_codes};
use std::collections::HashSet;
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

struct BudgetScope {
    session_subject_id: String,
    agent_subject_id: Option<String>,
    user_subject_id: Option<String>,
    org_subject_id: String,
    session_id: Option<uuid::Uuid>,
    user_id: Option<uuid::Uuid>,
    principal_id: Option<uuid::Uuid>,
    agent_id: Option<uuid::Uuid>,
    harness_id: Option<uuid::Uuid>,
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
            budget_id: BudgetId::from_uuid(
                row.budget_id
                    .expect("budget ledger rows returned from budget APIs always have a budget_id"),
            ),
            amount: row.amount,
            meter_source: row.meter_source.clone(),
            ref_type: row.ref_type.clone(),
            ref_id: row.ref_id.map(|id| id.to_string()),
            session_id: row.session_id.map(SessionId::from_uuid),
            description: row.description.clone(),
            created_at: row.created_at,
        }
    }

    async fn resolve_scope(
        &self,
        org_id: i64,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> Result<BudgetScope, anyhow::Error> {
        if let Ok(parsed_session_id) = SessionId::parse(session_id)
            && let Some(session) = self.db.get_session(org_id, parsed_session_id).await?
        {
            return Ok(Self::scope_from_session(&session, session_id, agent_id));
        }

        Ok(BudgetScope {
            session_subject_id: session_id.to_string(),
            agent_subject_id: agent_id.map(ToOwned::to_owned),
            user_subject_id: None,
            org_subject_id: everruns_core::org_public_id_from_internal(org_id),
            session_id: SessionId::parse(session_id).ok().map(|id| id.uuid()),
            user_id: None,
            principal_id: None,
            agent_id: agent_id
                .and_then(|id| AgentId::parse(id).ok())
                .map(|id| id.uuid()),
            harness_id: None,
        })
    }

    fn scope_from_session(
        session: &SessionRow,
        session_subject_id: &str,
        agent_id_override: Option<&str>,
    ) -> BudgetScope {
        BudgetScope {
            session_subject_id: session_subject_id.to_string(),
            agent_subject_id: agent_id_override
                .map(ToOwned::to_owned)
                .or_else(|| session.agent_id.map(|id| id.to_string())),
            user_subject_id: session.resolved_owner_user_id.map(|id| id.to_string()),
            org_subject_id: everruns_core::org_public_id_from_internal(session.org_id),
            session_id: Some(session.id.uuid()),
            user_id: session.resolved_owner_user_id,
            principal_id: Some(session.owner_principal_id.uuid()),
            agent_id: session.agent_id.map(|id| id.uuid()),
            harness_id: session.harness_id.map(|id| id.uuid()),
        }
    }

    pub async fn list_budgets_for_session_hierarchy(
        &self,
        org_id: i64,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> Vec<BudgetRow> {
        let scope = match self.resolve_scope(org_id, session_id, agent_id).await {
            Ok(scope) => scope,
            Err(error) => {
                error!(
                    session_id,
                    error = %error,
                    "Failed to resolve session scope for budget hierarchy"
                );
                return vec![];
            }
        };

        let mut seen = HashSet::new();
        let mut budgets = Vec::new();
        for (subject_type, subject_id) in [
            ("session", Some(scope.session_subject_id.as_str())),
            ("agent", scope.agent_subject_id.as_deref()),
            ("user", scope.user_subject_id.as_deref()),
            ("org", Some(scope.org_subject_id.as_str())),
        ] {
            let Some(subject_id) = subject_id else {
                continue;
            };
            match self
                .db
                .list_budgets(org_id, Some(subject_type), Some(subject_id))
                .await
            {
                Ok(rows) => {
                    for row in rows {
                        if seen.insert(row.id) {
                            budgets.push(row);
                        }
                    }
                }
                Err(error) => {
                    error!(
                        subject_type,
                        subject_id,
                        error = %error,
                        "Failed to list budgets for hierarchy subject"
                    );
                }
            }
        }

        budgets.sort_by(|a, b| {
            a.balance
                .partial_cmp(&b.balance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        budgets
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
        finish_reasons: Option<&[String]>,
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
        let scope = Self::scope_from_session(&session, &session_public_id, None);

        // Find all active budgets in the subject hierarchy
        let budgets = match self
            .db
            .get_active_budgets_for_session(
                session.org_id,
                &scope.session_subject_id,
                scope.agent_subject_id.as_deref(),
                scope.user_subject_id.as_deref(),
                Some(scope.org_subject_id.as_str()),
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
        let journal = match self
            .db
            .create_usage_journal_entry(CreateUsageJournalRow {
                org_id: session.org_id,
                kind: "llm_generation".to_string(),
                source_type: Some("event".to_string()),
                source_id: Some(event.id.to_string()),
                event_id: Some(event.id.uuid()),
                session_id: scope.session_id,
                turn_id: event.context.turn_id.as_ref().map(|id| id.uuid()),
                user_id: scope.user_id,
                principal_id: scope.principal_id,
                agent_id: scope.agent_id,
                harness_id: scope.harness_id,
                measures: serde_json::json!({
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens,
                    "total_tokens": total_tokens,
                }),
                metadata: serde_json::json!({
                    "model": model,
                    "provider": provider,
                    "finish_reasons": finish_reasons,
                }),
            })
            .await
        {
            Ok(journal) => journal,
            Err(error) => {
                error!(
                    session_id = %event.session_id,
                    error = %error,
                    "Failed to create usage journal row for llm generation"
                );
                return;
            }
        };

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

            let ledger_input = CreateUsageLedgerRow {
                journal_id: journal.id,
                budget_id: Some(budget.id),
                org_id: session.org_id,
                session_id: scope.session_id,
                user_id: scope.user_id,
                principal_id: scope.principal_id,
                agent_id: scope.agent_id,
                harness_id: scope.harness_id,
                currency: budget.currency.clone(),
                amount: debit,
                meter_source: "llm_tokens".into(),
                ref_type: Some("llm_generation".into()),
                ref_id: Some(event.id.uuid()),
                description: model.map(|m| format!("{} tokens on {}", total_tokens, m)),
                rating_metadata: Some(serde_json::json!({
                    "ruleset_version": "v1",
                    "journal_kind": "llm_generation",
                    "model": model,
                    "provider": provider,
                })),
            };

            let updated_budget = match self.db.create_usage_ledger_entry(ledger_input).await {
                Ok((_entry, Some(budget))) => budget,
                Ok((_entry, None)) => {
                    error!(
                        "Usage ledger row for budget {} did not update a budget",
                        budget.id
                    );
                    continue;
                }
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
                message: self.format_exhausted_message(budget),
            };
        }

        // Rule 2: Soft limit pause
        if let Some(soft_limit) = budget.soft_limit
            && budget.balance <= (budget.limit - soft_limit)
        {
            return BudgetAction::Pause {
                message: self.format_paused_message(budget),
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
        let all_matching = self
            .list_budgets_for_session_hierarchy(org_id, session_id, agent_id)
            .await;

        // Evaluate ALL matching budgets and keep the most restrictive result.
        // Priority: stop > pause > warn > continue
        let mut most_restrictive = BudgetCheckResult::ok();
        let mut most_restrictive_priority: u8 = 0; // 0=continue, 1=warn, 2=pause, 3=stop

        for budget in &all_matching {
            let (action_str, message, priority) =
                if budget.status == "exhausted" || budget.balance <= 0.0 {
                    ("stop", self.format_exhausted_message(budget), 3u8)
                } else if budget.status == "paused" {
                    ("pause", self.format_paused_message(budget), 2u8)
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
                let user_error = self.user_facing_error_for_action(budget, action_str);
                most_restrictive = BudgetCheckResult {
                    action: action_str.into(),
                    message: Some(message),
                    budget_id: Some(BudgetId::from_uuid(budget.id)),
                    balance: Some(budget.balance),
                    currency: Some(budget.currency.clone()),
                    error_code: user_error.as_ref().map(|error| error.code.clone()),
                    error_fields: user_error.and_then(|error| error.error_fields()),
                };
            }
        }

        most_restrictive
    }

    fn spent_amount(&self, budget: &BudgetRow) -> f64 {
        (budget.limit - budget.balance).max(0.0)
    }

    fn format_exhausted_message(&self, budget: &BudgetRow) -> String {
        self.budget_exhausted_error(budget).fallback_message()
    }

    fn format_paused_message(&self, budget: &BudgetRow) -> String {
        self.budget_paused_error(budget).fallback_message()
    }

    fn budget_exhausted_error(&self, budget: &BudgetRow) -> UserFacingError {
        UserFacingError::new(user_facing_error_codes::BUDGET_EXHAUSTED)
            .with_field("spent", self.spent_amount(budget))
            .with_field("limit", budget.limit)
            .with_field("currency", budget.currency.clone())
    }

    fn budget_paused_error(&self, budget: &BudgetRow) -> UserFacingError {
        UserFacingError::new(user_facing_error_codes::BUDGET_PAUSED)
            .with_field("spent", self.spent_amount(budget))
            .with_optional_field("soft_limit", budget.soft_limit)
            .with_field("currency", budget.currency.clone())
    }

    fn user_facing_error_for_action(
        &self,
        budget: &BudgetRow,
        action: &str,
    ) -> Option<UserFacingError> {
        match action {
            "stop" => Some(self.budget_exhausted_error(budget)),
            "pause" => Some(self.budget_paused_error(budget)),
            _ => None,
        }
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
            data.metadata.finish_reasons.as_deref(),
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
