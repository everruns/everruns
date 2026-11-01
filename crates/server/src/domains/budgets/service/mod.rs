// Budget Service
//
// Combines: metering (LlmTokenMeter), rules (hard stop, soft pause, warn),
// and evaluation pipeline. Implements EventListener to hook into llm.generation events.
//
// See knowledge/security/budgeting.md for full specification.

use crate::kernel_imports::contracts::user_facing_error::UserFacingError;
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Timelike, Utc};
use everruns_contracts::model_profiles::estimate_cost_usd;
use everruns_contracts::provider::DriverId;
use everruns_contracts::typed_id::{AgentId, BudgetId, ScheduleId, SessionId, TriggerId};
use everruns_contracts::user_facing_error::codes as user_facing_error_codes;
use everruns_core::EventListener;
use everruns_core::budget::{
    BudgetAction, BudgetCheckResult, BudgetPeriod, PolicySource, PolicyStepAction,
};
use everruns_core::events::{Event, EventData, LLM_GENERATION};
use std::sync::Arc;
use tracing::{debug, error, instrument, warn};

use crate::storage::StorageBackend;
use crate::storage::repositories::BudgetSubjectLookup;
use crate::storage::*;
use crate::storage::{BudgetRow, CreateUsageJournalRow, CreateUsageLedgerRow};

use super::costs::{CostBasis, RatedCost};

mod postings;
use postings::PostingContext;
pub use postings::{HostCharge, USAGE_POLICY_NOTICE_KIND};

// ============================================================================
// BudgetService
// ============================================================================

pub struct BudgetService {
    db: Arc<StorageBackend>,
}

/// What the session scheduler does with a due schedule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FireGate {
    Fire,
    /// The schedule's own budget is spent: stop the schedule.
    ScheduleBudgetSpent,
    /// The org's unattended spend for today is used up: skip this fire.
    OrgUnattendedSpent {
        /// The org's daily limit, for the health issue.
        limit_usd: f64,
    },
}

struct BudgetScope {
    session_subject_id: String,
    agent_subject_id: Option<String>,
    user_subject_id: Option<String>,
    org_subject_id: String,
    /// Public ID (`trg_`) of the trigger whose ingress started the session,
    /// from the `sessions.trigger_id` column.
    ///
    /// Design Decision: no budget subject is resolved from a session tag. This
    /// was the last one — a webhook trigger's cap was keyed on the
    /// `app_channel:` tag because migration 138 deleted the endpoint row it
    /// had been keyed on and left nothing structural behind. Migration 153
    /// adds `sessions.trigger_id` and re-keys those budgets onto the trigger's
    /// own API id, which is what every other level already does (EVE-1138).
    trigger_subject_id: Option<String>,
    /// Public ID of the endpoint referenced by `sessions.channel_id`.
    channel_subject_id: Option<String>,
    session_id: Option<uuid::Uuid>,
    user_id: Option<uuid::Uuid>,
    principal_id: Option<uuid::Uuid>,
    agent_id: Option<uuid::Uuid>,
    harness_id: Option<uuid::Uuid>,
    /// Public id (`sched_`) of the schedule whose fire started the turn.
    schedule_subject_id: Option<String>,
    /// No person started this turn: a schedule fired it, or an agent trigger
    /// started the session. The org's daily unattended budget binds it.
    unattended: bool,
    /// Public id of the eval run whose case created the session.
    eval_run_subject_id: Option<String>,
}

impl BudgetScope {
    fn lookup(&self) -> BudgetSubjectLookup<'_> {
        BudgetSubjectLookup {
            schedule_id: self.schedule_subject_id.as_deref(),
            org_unattended_id: self.unattended.then_some(self.org_subject_id.as_str()),
            session_id: Some(self.session_subject_id.as_str()),
            agent_id: self.agent_subject_id.as_deref(),
            user_id: self.user_subject_id.as_deref(),
            org_public_id: Some(self.org_subject_id.as_str()),
            trigger_id: self.trigger_subject_id.as_deref(),
            channel_id: self.channel_subject_id.as_deref(),
            eval_run_id: self.eval_run_subject_id.as_deref(),
        }
    }
}

impl BudgetService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
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
            return self
                .scope_from_session(&session, session_id, agent_id)
                .await;
        }

        Ok(BudgetScope {
            session_subject_id: session_id.to_string(),
            agent_subject_id: self.agent_subject_id(org_id, agent_id, None).await,
            user_subject_id: None,
            org_subject_id: everruns_core::org_public_id_from_internal(org_id),
            trigger_subject_id: None,
            channel_subject_id: None,
            session_id: SessionId::parse(session_id).ok().map(|id| id.uuid()),
            user_id: None,
            principal_id: None,
            agent_id: agent_id
                .and_then(|id| AgentId::parse(id).ok())
                .map(|id| id.uuid()),
            harness_id: None,
            schedule_subject_id: None,
            unattended: false,
            eval_run_subject_id: None,
        })
    }

    /// Resolve the `agent` budget subject to the agent's `public_id`.
    ///
    /// Design Decision: every budget subject is keyed by the identifier the API
    /// exposes, because that is the only identifier a caller can put in a
    /// budget (EVE-1136). For an agent that is `agents.public_id`. It is *not*
    /// `agents.id`, which is what a typed `AgentId` taken off a session row or
    /// passed as the worker's override renders — the two are independently
    /// generated UUIDs, so rendering the internal one produced a subject no
    /// stored budget could ever match and agent-scoped budgets silently never
    /// bound. This mirrors the `agent_channel` branch, which has always gone
    /// through `get_agent_channel_public_id`.
    ///
    /// The override is tried first and is looked up the same way, so both the
    /// worker path and the session-derived path land on one identifier. An
    /// override that is already a public id does not resolve as an internal id
    /// and is kept as given.
    async fn agent_subject_id(
        &self,
        org_id: i64,
        agent_id_override: Option<&str>,
        session_agent_id: Option<AgentId>,
    ) -> Option<String> {
        if let Some(raw) = agent_id_override {
            let resolved = match AgentId::parse(raw) {
                Ok(parsed) => self.lookup_agent_public_id(org_id, parsed).await,
                Err(_) => None,
            };
            // Not an internal id — assume the caller already handed us a public
            // one rather than dropping the agent level entirely.
            return Some(resolved.unwrap_or_else(|| raw.to_string()));
        }
        self.lookup_agent_public_id(org_id, session_agent_id?).await
    }

    async fn lookup_agent_public_id(&self, org_id: i64, agent_id: AgentId) -> Option<String> {
        match self.db.get_agent_public_id(org_id, agent_id).await {
            Ok(public_id) => public_id,
            Err(error) => {
                error!(
                    org_id,
                    agent_id = %agent_id,
                    error = %error,
                    "Failed to resolve agent public id for budget scope"
                );
                None
            }
        }
    }

    async fn scope_from_session(
        &self,
        session: &SessionRow,
        _session_subject_id: &str,
        agent_id_override: Option<&str>,
    ) -> Result<BudgetScope, anyhow::Error> {
        let trigger_subject_id = session
            .trigger_id
            .map(|id| TriggerId::from_uuid(id).to_string());
        let channel_subject_id = match session.channel_id {
            Some(channel_id) => {
                self.db
                    .get_agent_channel_public_id(session.org_id, channel_id)
                    .await?
            }
            None => None,
        };
        let root_session_id = session.root_session_id.unwrap_or(session.id);
        // An eval case session (or one it delegated to) also spends from its
        // run's cap. Only eval sessions pay for the lookup.
        let eval_run_subject_id =
            if session.source == crate::domains::sessions::record::SessionSource::Eval.as_str() {
                self.db
                    .eval_run_public_id_for_session(session.org_id, root_session_id.uuid())
                    .await?
            } else {
                None
            };
        Ok(BudgetScope {
            // Session-scoped budgets are owned by the root of a delegation
            // tree, so descendants debit and check the same shared pool.
            session_subject_id: root_session_id.to_string(),
            agent_subject_id: self
                .agent_subject_id(session.org_id, agent_id_override, session.agent_id)
                .await,
            user_subject_id: session.resolved_owner_user_id.map(|id| id.to_string()),
            org_subject_id: everruns_core::org_public_id_from_internal(session.org_id),
            trigger_subject_id,
            channel_subject_id,
            session_id: Some(session.id.uuid()),
            user_id: session.resolved_owner_user_id,
            principal_id: Some(session.owner_principal_id.uuid()),
            agent_id: session.agent_id.map(|id| id.uuid()),
            harness_id: session.harness_id.map(|id| id.uuid()),
            schedule_subject_id: None,
            unattended: session.trigger_id.is_some(),
            eval_run_subject_id,
        })
    }

    /// Add the schedule level when a schedule fired the turn: the turn whose
    /// input is `input_message_id`, or the session's latest input when the
    /// caller has no turn context (the loop gate). Also makes sure the org's
    /// daily unattended budget exists before an unattended turn is checked
    /// or metered (it is created lazily from Settings → Limits).
    async fn attach_unattended(
        &self,
        scope: &mut BudgetScope,
        org_id: i64,
        input_message_id: Option<uuid::Uuid>,
    ) {
        let schedule = match (input_message_id, scope.session_id) {
            (Some(message_id), _) => self.db.invocation_schedule(message_id).await,
            (None, Some(session_id)) => {
                self.db.latest_invocation_schedule(org_id, session_id).await
            }
            (None, None) => Ok(None),
        };
        match schedule {
            Ok(Some(schedule_id)) => {
                scope.schedule_subject_id = Some(ScheduleId::from_uuid(schedule_id).to_string());
                scope.unattended = true;
            }
            Ok(None) => {}
            Err(error) => {
                error!(org_id, error = %error, "Failed to resolve the schedule of a turn for budgets")
            }
        }
        if scope.unattended
            && let Err(error) = self.db.ensure_org_unattended_budget(org_id).await
        {
            error!(org_id, error = %error, "Failed to create the org's unattended budget");
        }
    }

    pub async fn list_budgets_for_session_hierarchy(
        &self,
        org_id: i64,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> Vec<BudgetRow> {
        let mut scope = match self.resolve_scope(org_id, session_id, agent_id).await {
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

        self.attach_unattended(&mut scope, org_id, None).await;
        let rows = match self
            .db
            .list_budgets_for_subjects(org_id, scope.lookup())
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                error!(
                    session_id,
                    error = %error,
                    "Failed to list budgets for session hierarchy"
                );
                return vec![];
            }
        };
        // Apply lazy period rollover before returning to callers.
        let mut budgets =
            futures::future::join_all(rows.into_iter().map(|row| self.maybe_reset_period(row)))
                .await;

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
        source: GenerationSource,
        model: Option<&str>,
        provider: Option<&str>,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        provider_cost_usd: Option<f64>,
        finish_reasons: Option<&[String]>,
        cost_unknown_components: &[String],
    ) {
        // Get session to find subject hierarchy
        let session = match self.db.get_session_unscoped(source.session_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                error!(
                    "Session not found for budget tracking: {}",
                    source.session_id
                );
                return;
            }
            Err(e) => {
                error!("Failed to get session for budget tracking: {}", e);
                return;
            }
        };

        let session_public_id = source.session_id.to_string();
        let mut scope = match self
            .scope_from_session(&session, &session_public_id, None)
            .await
        {
            Ok(scope) => scope,
            Err(error) => {
                error!(
                    session_id = %source.session_id,
                    error = %error,
                    "Failed to resolve session scope for budget tracking"
                );
                return;
            }
        };

        self.attach_unattended(&mut scope, session.org_id, source.input_message_id)
            .await;

        let org_id = session.org_id;
        let total_tokens = Self::metered_total_tokens(
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
        );
        let served = crate::domains::usage::tracking::served_by(
            &self.db,
            org_id,
            source.provider_id.as_deref(),
        )
        .await;
        // The list price is the catalog's cache-aware estimate; the
        // provider's own figure is never mistaken for it.
        let list_estimate_usd = source.estimated_cost_usd.or_else(|| {
            self.compute_debit(
                "usd",
                total_tokens,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                model,
                provider,
                None,
            )
        });
        let rates = match self
            .db
            .effective_cost_rates(org_id, super::costs::METER_MODEL_TOKENS, Utc::now())
            .await
        {
            Ok(rates) => rates,
            Err(error) => {
                warn!(org_id, error = %error, "Failed to read the org's price list; rating without it");
                Vec::new()
            }
        };
        let rated = super::costs::rate_model(
            &super::costs::ModelUsage {
                provider,
                model,
                managed: served.managed,
                total_tokens,
                provider_cost_usd,
                list_estimate_usd,
            },
            &rates,
        );

        let (source_key, source_label) = if served.managed {
            ("managed".to_string(), "Built-in models".to_string())
        } else {
            let provider = provider.unwrap_or("model");
            (format!("own:{provider}"), format!("{provider} (own key)"))
        };
        let journal = match self
            .db
            .create_usage_journal_entry(CreateUsageJournalRow {
                org_id,
                kind: "llm_generation".to_string(),
                source_type: Some(source.source_type.to_string()),
                source_id: Some(source.source_id.clone()),
                event_id: source.event_id,
                session_id: scope.session_id,
                turn_id: source.turn_id,
                user_id: scope.user_id,
                principal_id: scope.principal_id,
                agent_id: scope.agent_id,
                harness_id: scope.harness_id,
                measures: serde_json::json!({
                    "quantity": total_tokens,
                    "unit": "token",
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens,
                    "total_tokens": total_tokens,
                    "cache_read_tokens": cache_read_tokens,
                    "cache_creation_tokens": cache_creation_tokens,
                    "provider_cost_usd": provider_cost_usd,
                    "list_estimate_usd": list_estimate_usd,
                }),
                // Billable components nobody could price (EVE-1125): the
                // totals carry them as an explicit unknown, not a zero.
                metadata: serde_json::json!({
                    "model": model,
                    "provider": provider,
                    "provider_config_id": served.provider_config_id,
                    "managed": served.managed,
                    "source_key": source_key,
                    "source_label": source_label,
                    "started_by": if scope.unattended { "unattended" } else { "person" },
                    "finish_reasons": finish_reasons,
                    "cost_unknown_components": cost_unknown_components,
                }),
            })
            .await
        {
            Ok(journal) => journal,
            Err(error) => {
                // A repeated source (the unique source index) lands here too:
                // the usage is already recorded and charged once.
                debug!(
                    session_id = %source.session_id,
                    error = %error,
                    "Usage journal row not written for llm generation"
                );
                return;
            }
        };

        let ctx = PostingContext {
            journal_id: journal.id,
            org_id,
            scope: &scope,
            ref_type: "llm_generation",
            ref_id: source.ref_id,
            description: model.map(|m| format!("{total_tokens} tokens on {m}")),
            rating_metadata: serde_json::json!({
                "ruleset_version": "v2",
                "journal_kind": "llm_generation",
                "model": model,
                "provider": provider,
                "managed": served.managed,
                "cost_unknown_components": cost_unknown_components,
            }),
        };
        self.post_cost(&ctx, "cost", &rated).await;
        self.debit_policies(&ctx, &rated, Some(total_tokens), served.managed)
            .await;
    }

    /// Meter usage a provider reported after its generation was billed as an
    /// explicit unknown (late Agents API usage, EVE-1145). Only the late
    /// tokens are metered: what the generation's event already debited (its
    /// priced components) is not charged again. The journal is keyed to the
    /// generation record, whose unique `(source_type, source_id)` index admits
    /// it once on PostgreSQL, so a repeated call debits nothing more.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn meter_late_usage(
        &self,
        session_id: SessionId,
        generation_id: uuid::Uuid,
        turn_id: Option<uuid::Uuid>,
        event_id: Option<uuid::Uuid>,
        model: Option<&str>,
        provider: Option<&str>,
        usage: &crate::storage::LateGenerationUsage,
    ) {
        self.process_llm_generation(
            GenerationSource {
                session_id,
                source_type: LATE_USAGE_SOURCE,
                source_id: generation_id.to_string(),
                event_id: None,
                turn_id,
                input_message_id: None,
                ref_id: event_id.unwrap_or(generation_id),
                provider_id: None,
                estimated_cost_usd: usage.estimated_cost_usd,
            },
            model,
            provider,
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.cache_creation_tokens,
            None,
            None,
            &[],
        )
        .await;
    }

    /// Meter what grading one eval result cost against the budgets of its
    /// case session, the run's cap among them. Keyed to the result, so the
    /// journal's unique source index refuses a second charge for one grade.
    /// Grading with no known price is not metered: a dollar budget must not be
    /// debited a token count.
    pub(crate) async fn meter_grading(
        &self,
        session_id: SessionId,
        result_id: uuid::Uuid,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: Option<f64>,
    ) {
        if !cost_usd.is_some_and(|cost| cost > 0.0) {
            return;
        }
        self.process_llm_generation(
            GenerationSource {
                session_id,
                source_type: "eval_grading",
                source_id: result_id.to_string(),
                event_id: None,
                turn_id: None,
                input_message_id: None,
                ref_id: result_id,
                provider_id: None,
                estimated_cost_usd: None,
            },
            None,
            None,
            input_tokens as i64,
            output_tokens as i64,
            0,
            0,
            cost_usd,
            None,
            &[],
        )
        .await;
    }

    pub(crate) fn metered_total_tokens(
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
    ) -> i64 {
        input_tokens + output_tokens + cache_read_tokens + cache_creation_tokens
    }

    /// The amount a usage counts in `currency`: tokens as their count, `usd`
    /// as the provider's figure, else the catalog's cache-aware estimate.
    /// `None` when nothing prices it: an unknown price is unknown, never the
    /// token count read as dollars (that turned a $10 budget into a
    /// 10-token one for any model without a price profile).
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
    ) -> Option<f64> {
        match currency {
            "tokens" => Some(total_tokens as f64),
            "usd" => {
                // Prefer the provider's authoritative cost when present (e.g.
                // OpenRouter's usage.cost). It accounts for provider routing,
                // BYOK pricing, and cache discounts, and covers the long tail of
                // models we do not maintain price profiles for.
                if let Some(cost) = provider_cost_usd.filter(|c| *c > 0.0) {
                    return Some(cost);
                }
                // The price-table estimate shares the cache-aware accounting
                // used for `estimated_cost_usd`, so cached reads are not billed
                // at the full input rate (EVE-599).
                let provider_type = provider
                    .and_then(|p| p.parse::<DriverId>().ok())
                    .unwrap_or(DriverId::OpenAI);
                let model_id = model?;
                estimate_cost_usd(
                    &provider_type,
                    model_id,
                    input_tokens.max(0) as u32,
                    output_tokens.max(0) as u32,
                    cache_read_tokens.max(0) as u32,
                    cache_creation_tokens.max(0) as u32,
                )
            }
            // `credits` is retired (migrated to tokens); custom units have
            // no rating.
            _ => None,
        }
    }

    /// The most restrictive step a policy's spend has reached: notify maps
    /// to `Warn`, ask to `Pause`, stop to `Stop`.
    pub(crate) fn evaluate_rules(&self, budget: &BudgetRow) -> BudgetAction {
        let policy = budget.usage_policy();
        let spent = self.spent_amount_raw(budget);
        match policy.reached(budget.limit, budget.soft_limit, spent) {
            None => BudgetAction::Continue,
            Some(step) => match step.action {
                PolicyStepAction::Stop => BudgetAction::Stop {
                    message: self.format_exhausted_message(budget),
                },
                PolicyStepAction::Ask => BudgetAction::Pause {
                    message: self.format_paused_message(budget),
                },
                PolicyStepAction::Notify => BudgetAction::Warn {
                    message: format!(
                        "{:.2} {} used of the {:.2} {} limit ({:.0}%).",
                        spent,
                        budget.currency,
                        budget.limit,
                        budget.currency,
                        if budget.limit > 0.0 {
                            spent / budget.limit * 100.0
                        } else {
                            100.0
                        }
                    ),
                },
            },
        }
    }

    /// The org's timezone for calendar windows; UTC when unset or unknown.
    pub(crate) async fn org_tz(&self, org_id: i64) -> chrono_tz::Tz {
        match self.db.org_timezone(org_id).await {
            Ok(Some(name)) => name.parse().unwrap_or(chrono_tz::Tz::UTC),
            Ok(None) => chrono_tz::Tz::UTC,
            Err(error) => {
                debug!(org_id, error = %error, "Failed to read the org timezone; using UTC");
                chrono_tz::Tz::UTC
            }
        }
    }

    // ============================================================================
    // Period rollover
    // ============================================================================

    /// If `budget` has a configured period and the period has elapsed, reset
    /// the balance to the limit and stamp the new `period_started_at`.
    /// Otherwise return the row unchanged. Calendar periods reset on the next
    /// boundary (UTC).
    pub(crate) async fn maybe_reset_period(&self, budget: BudgetRow) -> BudgetRow {
        let tz = self.org_tz(budget.org_id).await;
        self.maybe_reset_period_in(budget, tz).await
    }

    /// [`Self::maybe_reset_period`] with the org's timezone already read.
    pub(crate) async fn maybe_reset_period_in(
        &self,
        budget: BudgetRow,
        tz: chrono_tz::Tz,
    ) -> BudgetRow {
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
        if !period_elapsed_in(&period, started, now, tz) {
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

        // "Ask to continue" is answered per session: the session the person
        // sees (the delegation root) or the one asking.
        let asking_sessions = self.approval_sessions(org_id, session_id).await;

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
                        BudgetAction::Pause { message } => {
                            if self.approved_for(budget, &asking_sessions).await {
                                continue;
                            }
                            ("pause", message, 2u8)
                        }
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

    /// Sessions whose approval answers an ask step for `session_id`: itself
    /// and its delegation root.
    async fn approval_sessions(&self, org_id: i64, session_id: &str) -> Vec<uuid::Uuid> {
        let Ok(parsed) = SessionId::parse(session_id) else {
            return Vec::new();
        };
        let mut sessions = vec![parsed.uuid()];
        if let Ok(Some(session)) = self.db.get_session(org_id, parsed).await
            && let Some(root) = session.root_session_id
        {
            sessions.push(root.uuid());
        }
        sessions
    }

    /// Whether a person let one of these sessions past the policy's ask step
    /// in the current window. A read failure counts as not approved, which
    /// asks again rather than spending past the step unasked.
    async fn approved_for(&self, budget: &BudgetRow, sessions: &[uuid::Uuid]) -> bool {
        for session in sessions {
            match self
                .db
                .usage_policy_approved(budget.id, *session, budget.period_started_at)
                .await
            {
                Ok(true) => return true,
                Ok(false) => {}
                Err(error) => {
                    warn!(budget_id = %budget.id, error = %error, "Failed to read usage policy approvals");
                }
            }
        }
        false
    }

    /// Let one session go past a policy's ask step for the policy's current
    /// window. Stop steps still stop.
    pub async fn approve_session(
        &self,
        org_id: i64,
        session_id: SessionId,
        approved_by: Option<uuid::Uuid>,
    ) -> anyhow::Result<usize> {
        let session_public_id = session_id.to_string();
        let budgets = self
            .list_budgets_for_session_hierarchy(org_id, &session_public_id, None)
            .await;
        let mut approved = 0;
        for budget in budgets {
            let asking = matches!(self.evaluate_rules(&budget), BudgetAction::Pause { .. })
                || budget.status == "paused";
            if !asking {
                continue;
            }
            if budget.status == "paused" {
                // A legacy global pause becomes a per-session answer too.
                self.db.set_budget_status(budget.id, "active").await?;
            }
            self.db
                .create_usage_policy_approval(
                    org_id,
                    budget.id,
                    session_id.uuid(),
                    approved_by,
                    budget.period_started_at,
                )
                .await?;
            approved += 1;
        }
        Ok(approved)
    }

    /// Whether a schedule may fire now: its own budget and the org's daily
    /// unattended budget must both have room. Checked by the session
    /// scheduler before it starts a turn, so a spent schedule stops instead of
    /// starting turns the loop gate would end at once. Fails open, like the
    /// loop gate, when the budgets cannot be read.
    pub async fn unattended_fire_gate(&self, org_id: i64, schedule_public_id: &str) -> FireGate {
        let spent = |budget: &BudgetRow| budget.status == "exhausted" || budget.balance <= 0.0;
        match self
            .db
            .schedule_budgets(org_id, &[schedule_public_id.to_string()])
            .await
        {
            Ok(rows) => {
                for row in rows {
                    if spent(&self.maybe_reset_period(row).await) {
                        return FireGate::ScheduleBudgetSpent;
                    }
                }
            }
            Err(error) => error!(org_id, error = %error, "Failed to read a schedule's budget"),
        }
        match self.db.org_unattended_budget(org_id).await {
            Ok(Some(row)) => {
                let row = self.maybe_reset_period(row).await;
                if spent(&row) {
                    FireGate::OrgUnattendedSpent {
                        limit_usd: row.limit,
                    }
                } else {
                    FireGate::Fire
                }
            }
            Ok(None) => FireGate::Fire,
            Err(error) => {
                error!(org_id, error = %error, "Failed to read the org's unattended budget");
                FireGate::Fire
            }
        }
    }

    fn spent_amount(&self, budget: &BudgetRow) -> f64 {
        (budget.limit - budget.balance).max(0.0)
    }

    /// Spend for step evaluation. Unclamped so a balance pushed past the
    /// limit still reads as reaching every step.
    fn spent_amount_raw(&self, budget: &BudgetRow) -> f64 {
        budget.limit - budget.balance
    }

    fn format_exhausted_message(&self, budget: &BudgetRow) -> String {
        self.budget_exhausted_error(budget).fallback_message()
    }

    fn format_paused_message(&self, budget: &BudgetRow) -> String {
        self.budget_paused_error(budget).fallback_message()
    }

    fn budget_exhausted_error(&self, budget: &BudgetRow) -> UserFacingError {
        UserFacingError::new(user_facing_error_codes::BUDGET_EXHAUSTED)
            .with_field("budget_id", BudgetId::from_uuid(budget.id).to_string())
            .with_field("spent", self.spent_amount(budget))
            .with_field("limit", budget.limit)
            .with_field("currency", budget.currency.clone())
    }

    fn budget_paused_error(&self, budget: &BudgetRow) -> UserFacingError {
        UserFacingError::new(user_facing_error_codes::BUDGET_PAUSED)
            .with_field("budget_id", BudgetId::from_uuid(budget.id).to_string())
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

        let Some(meter) = GenerationMeter::from_metadata(&data.metadata) else {
            return;
        };

        self.process_llm_generation(
            GenerationSource::from_event(event),
            Some(data.metadata.model.as_str()),
            data.metadata.provider.as_deref(),
            meter.input_tokens,
            meter.output_tokens,
            meter.cache_read_tokens,
            meter.cache_creation_tokens,
            meter.actual_cost_usd,
            data.metadata.finish_reasons.as_deref(),
            &meter.cost_unknown_components,
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

/// Journal `source_type` of late usage applied to a generation record.
pub(crate) const LATE_USAGE_SOURCE: &str = "llm_generation_late_usage";

/// What a metered generation is journaled against. An `llm.generation` event
/// is one source; usage applied to its record later (late Agents API usage,
/// EVE-1145) is another, journaled under its own source so the two never
/// collide on the journal's unique `(source_type, source_id)` and
/// `(kind, event_id)` indexes.
pub(crate) struct GenerationSource {
    pub session_id: SessionId,
    pub source_type: &'static str,
    pub source_id: String,
    /// FK to the persisted event, when there is one.
    pub event_id: Option<uuid::Uuid>,
    pub turn_id: Option<uuid::Uuid>,
    /// Input message of the turn, which says whether a schedule fired it.
    pub input_message_id: Option<uuid::Uuid>,
    /// Ledger `ref_id` (`ref_type` is `llm_generation`).
    pub ref_id: uuid::Uuid,
    /// Provider row that served the call (`provider_...`), which says
    /// whether the call ran on a host-managed (built-in) model.
    pub provider_id: Option<String>,
    /// The catalog's cache-aware estimate, used as the list price.
    pub estimated_cost_usd: Option<f64>,
}

impl GenerationSource {
    fn from_event(event: &Event) -> Self {
        Self {
            session_id: event.session_id,
            source_type: "event",
            source_id: event.id.to_string(),
            // Only persisted events have a sequence allocated by the events
            // table. Synthetic listener inputs preserve traceability via
            // source_id but must not write an FK to usage_journal.event_id.
            event_id: event.sequence.map(|_| event.id.uuid()),
            turn_id: event.context.turn_id.as_ref().map(|id| id.uuid()),
            input_message_id: event.context.input_message_id.as_ref().map(|id| id.uuid()),
            ref_id: event.id.uuid(),
            provider_id: match &event.data {
                EventData::LlmGeneration(data) => data.metadata.provider_id.clone(),
                _ => None,
            },
            estimated_cost_usd: match &event.data {
                EventData::LlmGeneration(data) => data
                    .metadata
                    .usage
                    .as_ref()
                    .and_then(|usage| usage.estimated_cost_usd),
                _ => None,
            },
        }
    }
}

/// What one `llm.generation` meters.
#[derive(Debug, PartialEq)]
pub(crate) struct GenerationMeter {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cost_usd: Option<f64>,
    /// The provider's own figure for the call (OpenRouter `usage.cost`, or
    /// priced components); `None` when only an estimate exists.
    pub actual_cost_usd: Option<f64>,
    /// `kind:name` of every billable component with an unknown amount.
    pub cost_unknown_components: Vec<String>,
}

impl GenerationMeter {
    /// `None` when the generation bills nothing Everruns knows of. A
    /// generation with unknown token usage but declared cost components
    /// (a managed turn whose usage never arrived, EVE-1125) still meters:
    /// its priced components are debited and the rest recorded as unknown.
    pub(crate) fn from_metadata(
        metadata: &everruns_core::events::LlmGenerationMetadata,
    ) -> Option<Self> {
        let cost_unknown_components = metadata
            .unpriced_cost_components()
            .map(|component| format!("{}:{}", component.kind, component.name))
            .collect();
        match &metadata.usage {
            Some(usage) => Some(Self {
                input_tokens: usage.input_tokens as i64,
                output_tokens: usage.output_tokens as i64,
                cache_read_tokens: usage.cache_read_tokens.unwrap_or(0) as i64,
                cache_creation_tokens: usage.cache_creation_tokens.unwrap_or(0) as i64,
                cost_usd: usage.effective_cost_usd(),
                actual_cost_usd: usage.actual_cost_usd,
                cost_unknown_components,
            }),
            None if !metadata.cost_components.is_empty() => {
                let priced: Vec<f64> = metadata
                    .cost_components
                    .iter()
                    .filter_map(|component| component.cost_usd)
                    .collect();
                Some(Self {
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                    cost_usd: (!priced.is_empty()).then(|| priced.iter().sum()),
                    actual_cost_usd: (!priced.is_empty()).then(|| priced.iter().sum()),
                    cost_unknown_components,
                })
            }
            None => None,
        }
    }
}

/// Decide whether a budget's period has elapsed and the balance should reset.
#[cfg(test)]
fn period_elapsed(period: &BudgetPeriod, started: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    period_elapsed_in(period, started, now, chrono_tz::Tz::UTC)
}

/// Calendar windows (hour, day, week, month, year) turn over at the
/// boundary in the org's timezone; fixed-length windows ignore it.
fn period_elapsed_in(
    period: &BudgetPeriod,
    started: DateTime<Utc>,
    now: DateTime<Utc>,
    tz: chrono_tz::Tz,
) -> bool {
    if let Some(seconds) = period.duration_seconds() {
        return (now - started).num_seconds() as u64 >= seconds;
    }
    if let BudgetPeriod::Calendar { unit } = period {
        return calendar_boundary_crossed(unit, started.with_timezone(&tz), now.with_timezone(&tz));
    }
    false
}

fn calendar_boundary_crossed<T: chrono::TimeZone>(
    unit: &str,
    started: DateTime<T>,
    now: DateTime<T>,
) -> bool {
    match unit.to_ascii_lowercase().as_str() {
        "hour" => started.date_naive() != now.date_naive() || started.hour() != now.hour(),
        "day" => started.date_naive() != now.date_naive(),
        "week" => iso_week_key(&started) != iso_week_key(&now),
        "month" => (started.year(), started.month()) != (now.year(), now.month()),
        "year" => started.year() != now.year(),
        _ => false,
    }
}

fn iso_week_key<T: chrono::TimeZone>(dt: &DateTime<T>) -> (i32, u32) {
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
}
