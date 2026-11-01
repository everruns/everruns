// Cost postings and usage policy debits for one usage record.
//
// Every usage gets exactly one cost posting (`usage_ledger` with no budget),
// carrying where its price came from (`basis`). Usage policies are then
// debited from that cost: dollar policies by the rated amount, token policies
// by the token count. A host (SaaS) that charged for a usage itself reports it
// through `record_host_charge`, which posts the difference as a correction.
//
// See knowledge/security/budgeting.md, "Cost of each usage".

use super::*;

/// Notification kind for a usage policy's notify step.
pub const USAGE_POLICY_NOTICE_KIND: &str = "usage_policy.notice";

/// What a host charged for one usage. See [`BudgetService::record_host_charge`].
#[derive(Debug, Clone, Copy)]
pub struct HostCharge<'a> {
    pub org_id: i64,
    /// The usage's journal source: `event` + the `llm.generation` event id,
    /// `sandbox_span` + the transition id.
    pub source_type: &'a str,
    pub source_id: &'a str,
    pub amount_usd: f64,
    /// Shown as the cost's origin, e.g. "Everruns credits".
    pub label: &'a str,
    /// The host's own service key, kept for reconciliation.
    pub service: &'a str,
}

/// Everything a posting for one journal row shares.
pub(super) struct PostingContext<'a> {
    pub(super) journal_id: uuid::Uuid,
    pub(super) org_id: i64,
    pub(super) scope: &'a BudgetScope,
    pub(super) ref_type: &'static str,
    pub(super) ref_id: uuid::Uuid,
    pub(super) description: Option<String>,
    pub(super) rating_metadata: serde_json::Value,
}

/// A readable name for a policy without one.
fn describe_subject(budget: &BudgetRow) -> String {
    let what = match budget.subject_type.as_str() {
        "org" => "Organization",
        "org_unattended" => "Unattended work",
        "agent" => "Agent",
        "agent_channel" => "Channel",
        "agent_trigger" => "Trigger",
        "user" => "Person",
        "session" => "Session",
        "session_schedule" => "Schedule",
        "eval_run" => "Eval run",
        _ => "Usage policy",
    };
    format!("{what} usage policy")
}

impl BudgetService {
    /// Write the cost posting for one journal row.
    pub(super) async fn post_cost(
        &self,
        ctx: &PostingContext<'_>,
        meter_source: &str,
        rated: &RatedCost,
    ) {
        let mut rating_metadata = ctx.rating_metadata.clone();
        if let Some(object) = rating_metadata.as_object_mut() {
            object.insert("basis".into(), rated.basis.as_str().into());
        }
        let row = CreateUsageLedgerRow {
            journal_id: ctx.journal_id,
            budget_id: None,
            org_id: ctx.org_id,
            session_id: ctx.scope.session_id,
            user_id: ctx.scope.user_id,
            principal_id: ctx.scope.principal_id,
            agent_id: ctx.scope.agent_id,
            harness_id: ctx.scope.harness_id,
            currency: "usd".into(),
            amount: rated.amount.unwrap_or(0.0),
            meter_source: meter_source.into(),
            ref_type: Some(ctx.ref_type.into()),
            ref_id: Some(ctx.ref_id),
            description: ctx.description.clone(),
            rating_metadata: Some(rating_metadata),
            basis: Some(rated.basis.as_str().into()),
            rate_id: rated.rate_id,
        };
        if let Err(error) = self.db.create_usage_ledger_entry(row).await {
            error!(journal_id = %ctx.journal_id, error = %error, "Failed to write cost posting");
        }
    }

    /// Debit every active usage policy in the scope: `usd` policies by the
    /// cost, `tokens` policies by the token count (when the usage has one).
    /// Then apply what the new spend reached: stop marks the policy
    /// exhausted, notify tells owners and admins once per window.
    pub(super) async fn debit_policies(
        &self,
        ctx: &PostingContext<'_>,
        rated: &RatedCost,
        tokens: Option<i64>,
        managed: bool,
    ) {
        let budgets = match self
            .db
            .get_active_budgets_for_subjects(ctx.org_id, ctx.scope.lookup())
            .await
        {
            Ok(b) => b,
            Err(e) => {
                error!(org_id = ctx.org_id, error = %e, "Failed to fetch usage policies");
                return;
            }
        };
        // Period rollover before metering: if a budget's window has elapsed,
        // refresh balance/limit so the new usage is charged against the new
        // period rather than against an exhausted snapshot.
        let timezone = self.org_tz(ctx.org_id).await;
        let mut budgets: Vec<BudgetRow> = futures::future::join_all(
            budgets
                .into_iter()
                .map(|b| self.maybe_reset_period_in(b, timezone)),
        )
        .await;
        budgets.retain(|b| b.status == "active");

        for budget in &budgets {
            let policy = budget.usage_policy();
            let counts = match policy.source {
                PolicySource::All => true,
                PolicySource::Managed => managed,
                PolicySource::OwnKeys => !managed,
            };
            if !counts {
                continue;
            }
            let debit = match budget.currency.as_str() {
                "usd" => rated.amount,
                "tokens" => tokens.map(|t| t as f64),
                // Retired or custom units have no rating: never debit a
                // number in a unit nobody priced.
                _ => None,
            };
            let Some(debit) = debit.filter(|d| *d != 0.0) else {
                continue;
            };

            let ledger_input = CreateUsageLedgerRow {
                journal_id: ctx.journal_id,
                budget_id: Some(budget.id),
                org_id: ctx.org_id,
                session_id: ctx.scope.session_id,
                user_id: ctx.scope.user_id,
                principal_id: ctx.scope.principal_id,
                agent_id: ctx.scope.agent_id,
                harness_id: ctx.scope.harness_id,
                currency: budget.currency.clone(),
                amount: debit,
                meter_source: match ctx.ref_type {
                    "sandbox_span" => "sandbox_time".into(),
                    "host_charge" => "host_correction".into(),
                    _ => "llm_tokens".into(),
                },
                ref_type: Some(ctx.ref_type.into()),
                ref_id: Some(ctx.ref_id),
                description: ctx.description.clone(),
                rating_metadata: Some(ctx.rating_metadata.clone()),
                basis: Some(rated.basis.as_str().into()),
                rate_id: rated.rate_id,
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
                    // The (journal, budget, currency, meter_source) index
                    // refuses a second debit of the same usage.
                    debug!("Usage policy debit not recorded: {}", e);
                    continue;
                }
            };
            self.apply_reached_step(&updated_budget).await;
        }
    }

    /// Act on the most restrictive step a policy's spend reached.
    pub(super) async fn apply_reached_step(&self, budget: &BudgetRow) {
        match self.evaluate_rules(budget) {
            BudgetAction::Continue | BudgetAction::Pause { .. } => {}
            BudgetAction::Warn { message } => {
                self.notify_step(budget, &message).await;
            }
            BudgetAction::Stop { message } => {
                warn!(budget_id = %budget.id, balance = budget.balance, reason = %message, "Usage policy stop reached");
                if let Err(e) = self.db.set_budget_status(budget.id, "exhausted").await {
                    error!(budget_id = %budget.id, error = %e, "Failed to set budget exhausted");
                }
            }
        }
    }

    /// Tell the org's owners and admins that a policy crossed a notify
    /// step, once per step and window.
    pub(super) async fn notify_step(&self, budget: &BudgetRow, message: &str) {
        let policy = budget.usage_policy();
        let spent = self.spent_amount(budget);
        let Some(step) = policy
            .effective_steps(budget.limit, budget.soft_limit)
            .into_iter()
            .rev()
            .find(|step| {
                step.action == PolicyStepAction::Notify
                    && spent >= budget.limit * step.at_percent / 100.0
            })
        else {
            return;
        };
        let window = budget.period_started_at.unwrap_or(budget.created_at);
        match self
            .db
            .claim_usage_policy_notice(budget.id, step.at_percent, window)
            .await
        {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                warn!(budget_id = %budget.id, error = %error, "Failed to record a usage policy notice");
                return;
            }
        }
        let recipients = match self.db.org_owner_and_admin_ids(budget.org_id).await {
            Ok(ids) => ids,
            Err(error) => {
                warn!(org_id = budget.org_id, error = %error, "Failed to list who to notify");
                return;
            }
        };
        let name = policy
            .name
            .clone()
            .unwrap_or_else(|| describe_subject(budget));
        let budget_public_id = BudgetId::from_uuid(budget.id).to_string();
        for user_id in recipients {
            let row = crate::storage::CreateNotificationRow {
                org_id: budget.org_id,
                user_id,
                kind: USAGE_POLICY_NOTICE_KIND.into(),
                title: name.clone(),
                body: message.to_string(),
                target_type: Some("usage_policy".into()),
                target_id: Some(budget_public_id.clone()),
                href: Some("/settings/costs?tab=policies".into()),
                payload: serde_json::json!({
                    "budget_id": budget_public_id,
                    "step_at_percent": step.at_percent,
                    "spent": spent,
                    "limit": budget.limit,
                    "currency": budget.currency,
                }),
                dedupe_key: Some(format!(
                    "usage_policy:{}:{}:{}",
                    budget.id,
                    step.at_percent,
                    window.timestamp()
                )),
                source: Some(crate::storage::NotificationSourceRow {
                    source_type: "system".into(),
                    source_id: None,
                    source_name: Some("Usage policies".into()),
                }),
            };
            if let Err(error) = self.db.create_notification(row).await {
                warn!(error = %error, "Failed to notify about a usage policy step");
            }
        }
    }

    /// Record what a host charged for one usage (Everruns credits on the
    /// hosted platform), replacing the estimate for it. Appends the
    /// difference as a `cost_host` posting, and debits `usd` policies by the
    /// same difference, so a $10 policy means $10 of what the host charged.
    /// Idempotent: a second report for the same usage changes nothing.
    /// Returns `false` when the usage is unknown or already reported.
    pub async fn record_host_charge(&self, charge: HostCharge<'_>) -> anyhow::Result<bool> {
        let Some(journal) = self
            .db
            .usage_journal_by_source(charge.org_id, charge.source_type, charge.source_id)
            .await?
        else {
            return Ok(false);
        };
        let (existing, already) = self.db.journal_cost(journal.id).await?;
        if already {
            return Ok(false);
        }
        let delta = charge.amount_usd - existing.unwrap_or(0.0);
        let scope = BudgetScope {
            session_subject_id: String::new(),
            agent_subject_id: None,
            user_subject_id: None,
            org_subject_id: everruns_core::org_public_id_from_internal(journal.org_id),
            trigger_subject_id: None,
            channel_subject_id: None,
            session_id: journal.session_id,
            user_id: journal.user_id,
            principal_id: journal.principal_id,
            agent_id: journal.agent_id,
            harness_id: journal.harness_id,
            schedule_subject_id: None,
            unattended: false,
            eval_run_subject_id: None,
        };
        // The policies the usage counted against are resolved again from
        // its session, the way the first debit found them.
        let scope = match journal.session_id {
            Some(session_id) => match self
                .db
                .get_session_unscoped(SessionId::from_uuid(session_id))
                .await?
            {
                Some(session) => self
                    .scope_from_session(
                        &session,
                        &SessionId::from_uuid(session_id).to_string(),
                        None,
                    )
                    .await
                    .unwrap_or(scope),
                None => scope,
            },
            None => scope,
        };
        let ctx = PostingContext {
            journal_id: journal.id,
            org_id: journal.org_id,
            scope: &scope,
            ref_type: "host_charge",
            ref_id: journal.id,
            description: Some(format!(
                "{} charged {:.6} USD",
                charge.label, charge.amount_usd
            )),
            rating_metadata: serde_json::json!({
                "ruleset_version": "v2",
                "host_label": charge.label,
                "host_service": charge.service,
                "charged_usd": charge.amount_usd,
                "previous_usd": existing,
            }),
        };
        let rated = RatedCost {
            amount: Some(delta),
            basis: CostBasis::Host,
            rate_id: None,
        };
        self.post_cost(&ctx, "cost_host", &rated).await;
        let managed = journal
            .metadata
            .get("managed")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if delta != 0.0 {
            self.debit_policies(&ctx, &rated, None, managed).await;
        }
        Ok(true)
    }

    /// Meter the closed spans of sandboxes running that the usage record
    /// does not have yet. Run by a background job; each span is journaled
    /// once (the journal's unique source index).
    pub async fn meter_sandbox_spans(&self, limit: i64) -> anyhow::Result<usize> {
        let spans = self.db.unmetered_sandbox_spans(limit).await?;
        let mut metered = 0;
        for span in spans {
            let seconds = (span.end_at - span.start_at).num_milliseconds().max(0) as f64 / 1000.0;
            let rates = self
                .db
                .effective_cost_rates(
                    span.org_id,
                    crate::domains::budgets::costs::METER_SANDBOX_TIME,
                    span.end_at,
                )
                .await
                .unwrap_or_default();
            let rated = crate::domains::budgets::costs::rate_sandbox(
                &span.provider,
                span.profile_name.as_deref(),
                seconds,
                &rates,
            );
            let session = match span.session_id {
                Some(id) => self
                    .db
                    .get_session_unscoped(SessionId::from_uuid(id))
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            let scope = match &session {
                Some(session) => self
                    .scope_from_session(session, &session.id.to_string(), None)
                    .await
                    .ok(),
                None => None,
            };
            let scope = scope.unwrap_or_else(|| BudgetScope {
                session_subject_id: String::new(),
                agent_subject_id: None,
                user_subject_id: None,
                org_subject_id: everruns_core::org_public_id_from_internal(span.org_id),
                trigger_subject_id: None,
                channel_subject_id: None,
                session_id: None,
                user_id: None,
                principal_id: None,
                agent_id: span.agent_id,
                harness_id: None,
                schedule_subject_id: None,
                unattended: false,
                eval_run_subject_id: None,
            });
            let label = match &span.profile_name {
                Some(size) => format!("{} sandboxes ({size})", span.provider),
                None => format!("{} sandboxes", span.provider),
            };
            let journal = match self
                .db
                .create_usage_journal_entry(CreateUsageJournalRow {
                    org_id: span.org_id,
                    kind: "sandbox_time".into(),
                    source_type: Some("sandbox_span".into()),
                    source_id: Some(span.transition_id.to_string()),
                    event_id: None,
                    session_id: scope.session_id,
                    turn_id: None,
                    user_id: scope.user_id,
                    principal_id: scope.principal_id,
                    agent_id: scope.agent_id.or(span.agent_id),
                    harness_id: scope.harness_id,
                    measures: serde_json::json!({
                        "quantity": seconds / 3600.0,
                        "unit": "hour",
                        "seconds": seconds,
                    }),
                    metadata: serde_json::json!({
                        "provider": span.provider,
                        "size": span.profile_name,
                        "sandbox_id": span.sandbox_id,
                        "source_key": format!("sandbox:{}", span.provider),
                        "source_label": label,
                        "started_at": span.start_at,
                        "ended_at": span.end_at,
                    }),
                })
                .await
            {
                Ok(journal) => journal,
                Err(error) => {
                    debug!(error = %error, "Sandbox span already metered");
                    continue;
                }
            };
            let ctx = PostingContext {
                journal_id: journal.id,
                org_id: span.org_id,
                scope: &scope,
                ref_type: "sandbox_span",
                ref_id: span.sandbox_id,
                description: Some(format!("{:.2} h of {label}", seconds / 3600.0)),
                rating_metadata: serde_json::json!({
                    "ruleset_version": "v2",
                    "journal_kind": "sandbox_time",
                    "provider": span.provider,
                    "size": span.profile_name,
                }),
            };
            self.post_cost(&ctx, "cost", &rated).await;
            // Sandboxes are the org's own (or a host's) infrastructure;
            // `own_keys`-only policies count them, built-in-model ones do not.
            self.debit_policies(&ctx, &rated, None, false).await;
            metered += 1;
        }
        Ok(metered)
    }
}
