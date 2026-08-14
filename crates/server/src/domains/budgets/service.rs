// Budget Service
//
// Combines: metering (LlmTokenMeter), rules (hard stop, soft pause, warn),
// and evaluation pipeline. Implements EventListener to hook into llm.generation events.
//
// See knowledge/security/budgeting.md for full specification.

use crate::kernel_imports::everruns_provider::user_facing_error::UserFacingError;
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Timelike, Utc};
use everruns_core::EventListener;
use everruns_core::budget::{
    Budget, BudgetAction, BudgetCheckResult, BudgetPeriod, BudgetStatus, BudgetSubjectType,
    LedgerEntry,
};
use everruns_core::events::{Event, EventData, LLM_GENERATION};
use everruns_provider::model_profiles::estimate_cost_usd;
use everruns_provider::provider::DriverId;
use everruns_provider::typed_id::{AgentId, BudgetId, SessionId};
use everruns_provider::user_facing_error::codes as user_facing_error_codes;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, error, info, instrument, warn};

use crate::storage::StorageBackend;
use crate::storage::models::*;
use crate::storage::repositories::BudgetSubjectLookup;

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
    /// Public ID of the owning app, extracted from session tags (`app:<app_id>`)
    /// when the session was created via an app channel. `None` for ad-hoc
    /// sessions that don't belong to an app.
    app_subject_id: Option<String>,
    /// Public ID of the originating app channel, extracted from session tags
    /// (`app_channel:<channel_id>`). `None` for ad-hoc sessions.
    app_channel_subject_id: Option<String>,
    session_id: Option<uuid::Uuid>,
    user_id: Option<uuid::Uuid>,
    principal_id: Option<uuid::Uuid>,
    agent_id: Option<uuid::Uuid>,
    harness_id: Option<uuid::Uuid>,
}

impl BudgetScope {
    fn lookup(&self) -> BudgetSubjectLookup<'_> {
        BudgetSubjectLookup {
            session_id: Some(self.session_subject_id.as_str()),
            agent_id: self.agent_subject_id.as_deref(),
            user_id: self.user_subject_id.as_deref(),
            org_public_id: Some(self.org_subject_id.as_str()),
            app_id: self.app_subject_id.as_deref(),
            app_channel_id: self.app_channel_subject_id.as_deref(),
        }
    }
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
            period_started_at: row.period_started_at,
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
            app_subject_id: None,
            app_channel_subject_id: None,
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
        _session_subject_id: &str,
        agent_id_override: Option<&str>,
    ) -> BudgetScope {
        let (app_subject_id, app_channel_subject_id) = extract_app_subjects(&session.tags);
        let root_session_id = session.root_session_id.unwrap_or(session.id);
        BudgetScope {
            // Session-scoped budgets are owned by the root of a delegation
            // tree, so descendants debit and check the same shared pool.
            session_subject_id: root_session_id.to_string(),
            agent_subject_id: agent_id_override
                .map(ToOwned::to_owned)
                .or_else(|| session.agent_id.map(|id| id.to_string())),
            user_subject_id: session.resolved_owner_user_id.map(|id| id.to_string()),
            org_subject_id: everruns_core::org_public_id_from_internal(session.org_id),
            app_subject_id,
            app_channel_subject_id,
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
            ("app_channel", scope.app_channel_subject_id.as_deref()),
            ("app", scope.app_subject_id.as_deref()),
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
                            // Apply lazy period rollover before returning to callers.
                            let row = self.maybe_reset_period(row).await;
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
    #[allow(clippy::too_many_arguments)]
    async fn process_llm_generation(
        &self,
        event: &Event,
        model: Option<&str>,
        provider: Option<&str>,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        provider_cost_usd: Option<f64>,
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
            .get_active_budgets_for_subjects(session.org_id, scope.lookup())
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

        // Period rollover before metering: if a budget's window has elapsed,
        // refresh balance/limit so the new generation is charged against the
        // new period rather than against an exhausted snapshot.
        let mut budgets: Vec<BudgetRow> =
            futures::future::join_all(budgets.into_iter().map(|b| self.maybe_reset_period(b)))
                .await;
        budgets.retain(|b| b.status == "active");

        if budgets.is_empty() {
            return; // No budgets — nothing to track
        }

        let total_tokens = Self::metered_total_tokens(
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
        );
        // Only persisted events have a sequence allocated by the events table.
        // Synthetic listener inputs preserve traceability via source_id but must
        // not write an FK to usage_journal.event_id.
        let event_id = event.sequence.map(|_| event.id.uuid());
        let journal = match self
            .db
            .create_usage_journal_entry(CreateUsageJournalRow {
                org_id: session.org_id,
                kind: "llm_generation".to_string(),
                source_type: Some("event".to_string()),
                source_id: Some(event.id.to_string()),
                event_id,
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
                    "cache_read_tokens": cache_read_tokens,
                    "cache_creation_tokens": cache_creation_tokens,
                    "provider_cost_usd": provider_cost_usd,
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
                cache_read_tokens,
                cache_creation_tokens,
                model,
                provider,
                provider_cost_usd,
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

    pub(crate) fn metered_total_tokens(
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
    ) -> i64 {
        input_tokens + output_tokens + cache_read_tokens + cache_creation_tokens
    }

    /// Compute the debit amount in the budget's currency.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compute_debit(
        &self,
        currency: &str,
        total_tokens: i64,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        model: Option<&str>,
        provider: Option<&str>,
        provider_cost_usd: Option<f64>,
    ) -> f64 {
        match currency {
            "tokens" => total_tokens as f64,
            "usd" => {
                // Prefer the provider's authoritative cost when present (e.g.
                // OpenRouter's usage.cost). It accounts for provider routing,
                // BYOK pricing, and cache discounts, and covers the long tail of
                // models we do not maintain price profiles for.
                if let Some(cost) = provider_cost_usd.filter(|c| *c > 0.0) {
                    return cost;
                }
                // Fall back to the price-table estimate. This shares the
                // cache-aware accounting used for `estimated_cost_usd` so the
                // meter does not bill cached reads at the full input rate (which
                // overstated cache-heavy runs and could trip a budget early —
                // EVE-599).
                let provider_type = provider
                    .and_then(|p| p.parse::<DriverId>().ok())
                    .unwrap_or(DriverId::OpenAI);
                let model_id = model.unwrap_or("unknown");
                estimate_cost_usd(
                    &provider_type,
                    model_id,
                    input_tokens.max(0) as u32,
                    output_tokens.max(0) as u32,
                    cache_read_tokens.max(0) as u32,
                    cache_creation_tokens.max(0) as u32,
                )
                .unwrap_or_else(|| {
                    // No profile or no cost data — fall back to raw token count.
                    warn!(
                        model = model_id,
                        "No cost data for model, using token count as debit"
                    );
                    total_tokens as f64
                })
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
    // Period rollover
    // ============================================================================

    /// If `budget` has a configured period and the period has elapsed, reset
    /// the balance to the limit and stamp the new `period_started_at`.
    /// Otherwise return the row unchanged. Calendar periods reset on the next
    /// boundary (UTC).
    pub(crate) async fn maybe_reset_period(&self, budget: BudgetRow) -> BudgetRow {
        let Some(period_value) = budget.period.as_ref() else {
            return budget;
        };
        let period = match serde_json::from_value::<BudgetPeriod>(period_value.clone()) {
            Ok(period) => period,
            Err(error) => {
                // Malformed period payloads (e.g. corrupted by manual DB edit
                // or a deserialization-incompatible upgrade) cause the budget
                // to silently never roll over. Log loudly so operators can
                // diagnose; return the stale row to keep enforcement strict.
                warn!(
                    budget_id = %budget.id,
                    raw_period = %period_value,
                    error = %error,
                    "Failed to parse budget period; rollover skipped"
                );
                return budget;
            }
        };
        let now = Utc::now();
        let started = budget.period_started_at.unwrap_or(budget.created_at);
        if !period_elapsed(&period, started, now) {
            return budget;
        }
        match self.db.reset_budget_period(budget.id, now).await {
            Ok(Some(new_row)) => {
                debug!(
                    budget_id = %new_row.id,
                    "Budget period elapsed, balance reset"
                );
                new_row
            }
            Ok(None) => budget,
            Err(error) => {
                warn!(
                    budget_id = %budget.id,
                    error = %error,
                    "Failed to reset budget period; using stale row"
                );
                budget
            }
        }
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
        let cache_read_tokens = usage.cache_read_tokens.unwrap_or(0) as i64;
        let cache_creation_tokens = usage.cache_creation_tokens.unwrap_or(0) as i64;

        self.process_llm_generation(
            event,
            Some(data.metadata.model.as_str()),
            data.metadata.provider.as_deref(),
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            usage.actual_cost_usd,
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

// ============================================================================
// Helpers
// ============================================================================

/// Pull `(app_id, app_channel_id)` from session tags. Both legacy
/// `slack:app:<id>`, `ag_ui:app:<id>`, and the canonical
/// `app:<id>` / `app_channel:<id>` forms are recognised.
fn extract_app_subjects(tags: &[String]) -> (Option<String>, Option<String>) {
    let mut app_id: Option<String> = None;
    let mut channel_id: Option<String> = None;
    for tag in tags {
        if let Some(rest) = tag.strip_prefix("app:") {
            app_id.get_or_insert_with(|| rest.to_string());
        } else if let Some(rest) = tag.strip_prefix("app_channel:") {
            channel_id.get_or_insert_with(|| rest.to_string());
        } else if let Some(rest) = tag.strip_prefix("slack:app:") {
            app_id.get_or_insert_with(|| rest.to_string());
        } else if let Some(rest) = tag.strip_prefix("ag_ui:app:") {
            app_id.get_or_insert_with(|| rest.to_string());
        }
    }
    (app_id, channel_id)
}

/// Decide whether a budget's period has elapsed and the balance should reset.
fn period_elapsed(period: &BudgetPeriod, started: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    if let Some(seconds) = period.duration_seconds() {
        return (now - started).num_seconds() as u64 >= seconds;
    }
    if let BudgetPeriod::Calendar { unit } = period {
        return calendar_boundary_crossed(unit, started, now);
    }
    false
}

fn calendar_boundary_crossed(unit: &str, started: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    match unit.to_ascii_lowercase().as_str() {
        "hour" => started.date_naive() != now.date_naive() || started.hour() != now.hour(),
        "day" => started.date_naive() != now.date_naive(),
        "week" => iso_week_key(started) != iso_week_key(now),
        "month" => (started.year(), started.month()) != (now.year(), now.month()),
        "year" => started.year() != now.year(),
        _ => false,
    }
}

fn iso_week_key(dt: DateTime<Utc>) -> (i32, u32) {
    let iso = dt.iso_week();
    (iso.year(), iso.week())
}

#[cfg(test)]
mod period_tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn duration_period_elapses_after_window() {
        let period = BudgetPeriod::Duration { seconds: 3_600 };
        let started = at(2025, 1, 1, 10, 0);
        assert!(!period_elapsed(&period, started, at(2025, 1, 1, 10, 30)));
        assert!(period_elapsed(&period, started, at(2025, 1, 1, 11, 0)));
    }

    #[test]
    fn rolling_5h_parses_to_5_hours() {
        let period = BudgetPeriod::Rolling {
            window: "5h".into(),
        };
        let started = at(2025, 1, 1, 0, 0);
        assert!(!period_elapsed(&period, started, at(2025, 1, 1, 4, 59)));
        assert!(period_elapsed(&period, started, at(2025, 1, 1, 5, 0)));
    }

    #[test]
    fn calendar_month_resets_at_month_boundary() {
        let period = BudgetPeriod::Calendar {
            unit: "month".into(),
        };
        let started = at(2025, 1, 31, 23, 0);
        assert!(!period_elapsed(&period, started, at(2025, 1, 31, 23, 30)));
        assert!(period_elapsed(&period, started, at(2025, 2, 1, 0, 0)));
    }

    #[test]
    fn calendar_unknown_unit_never_elapses() {
        let period = BudgetPeriod::Calendar {
            unit: "fortnight".into(),
        };
        let started = at(2025, 1, 1, 0, 0);
        assert!(!period_elapsed(&period, started, at(2030, 1, 1, 0, 0)));
    }

    #[test]
    fn extract_subjects_handles_supported_tag_styles() {
        let tags = vec![
            "app:app_abc".to_string(),
            "app_channel:appchan_xyz".to_string(),
        ];
        let (app, ch) = extract_app_subjects(&tags);
        assert_eq!(app.as_deref(), Some("app_abc"));
        assert_eq!(ch.as_deref(), Some("appchan_xyz"));

        let slack_legacy = vec!["slack:app:app_legacy".to_string()];
        let (app, _) = extract_app_subjects(&slack_legacy);
        assert_eq!(app.as_deref(), Some("app_legacy"));

        let ag_ui_legacy = vec!["ag_ui:app:app_ag_ui".to_string()];
        let (app, _) = extract_app_subjects(&ag_ui_legacy);
        assert_eq!(app.as_deref(), Some("app_ag_ui"));
    }
}
