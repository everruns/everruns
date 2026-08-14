// In-memory budget storage

use crate::kernel_imports::{
    everruns_provider::typed_id::AgentId, everruns_provider::typed_id::SessionId,
};
use anyhow::Result;
use uuid::Uuid;

use super::InMemoryDatabase;
use crate::storage::models::*;
use crate::storage::repositories::BudgetSubjectLookup;

impl InMemoryDatabase {
    // ============================================
    // Budget CRUD
    // ============================================

    pub async fn create_budget(&self, input: CreateBudgetRow) -> Result<BudgetRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = BudgetRow {
            id,
            org_id: input.org_id,
            subject_type: input.subject_type,
            subject_id: input.subject_id,
            currency: input.currency,
            limit: input.limit,
            soft_limit: input.soft_limit,
            balance: input.limit, // start with full balance
            period_started_at: input.period.as_ref().map(|_| now),
            period: input.period,
            metadata: input.metadata,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
        };
        self.budgets.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_budget(&self, org_id: i64, id: Uuid) -> Result<Option<BudgetRow>> {
        let budgets = self.budgets.read();
        Ok(budgets.get(&id).filter(|b| b.org_id == org_id).cloned())
    }

    pub async fn list_budgets(
        &self,
        org_id: i64,
        subject_type: Option<&str>,
        subject_id: Option<&str>,
    ) -> Result<Vec<BudgetRow>> {
        let budgets = self.budgets.read();
        let mut result: Vec<BudgetRow> = budgets
            .values()
            .filter(|b| {
                b.org_id == org_id
                    && b.status != "disabled"
                    && subject_type.is_none_or(|t| b.subject_type == t)
                    && subject_id.is_none_or(|i| b.subject_id == i)
            })
            .cloned()
            .collect();
        result.sort_by_key(|budget| std::cmp::Reverse(budget.created_at));
        result.truncate(200);
        Ok(result)
    }

    pub async fn get_active_budgets_for_session(
        &self,
        org_id: i64,
        session_id: &str,
        agent_id: Option<&str>,
        user_id: Option<&str>,
        org_public_id: Option<&str>,
    ) -> Result<Vec<BudgetRow>> {
        self.get_active_budgets_for_subjects(
            org_id,
            BudgetSubjectLookup {
                session_id: Some(session_id),
                agent_id,
                user_id,
                org_public_id,
                app_id: None,
                app_channel_id: None,
            },
        )
        .await
    }

    pub async fn get_active_budgets_for_subjects(
        &self,
        org_id: i64,
        lookup: BudgetSubjectLookup<'_>,
    ) -> Result<Vec<BudgetRow>> {
        let pairs = lookup.to_pairs();
        if pairs.is_empty() {
            return Ok(vec![]);
        }
        let budgets = self.budgets.read();
        let mut result: Vec<BudgetRow> = budgets
            .values()
            .filter(|b| {
                b.org_id == org_id
                    && b.status == "active"
                    && pairs
                        .iter()
                        .any(|(t, id)| b.subject_type == *t && b.subject_id == *id)
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| {
            a.balance
                .partial_cmp(&b.balance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(result)
    }

    pub async fn reset_budget_period(
        &self,
        id: Uuid,
        period_started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<BudgetRow>> {
        let mut budgets = self.budgets.write();
        let Some(row) = budgets.get_mut(&id) else {
            return Ok(None);
        };
        row.balance = row.limit;
        row.period_started_at = Some(period_started_at);
        if row.status == "paused" || row.status == "exhausted" {
            row.status = "active".to_string();
        }
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn update_budget(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateBudgetRow,
    ) -> Result<Option<BudgetRow>> {
        let mut budgets = self.budgets.write();
        let Some(row) = budgets.get_mut(&id) else {
            return Ok(None);
        };
        if row.org_id != org_id {
            return Ok(None);
        }
        if let Some(new_limit) = input.limit {
            let delta = new_limit - row.limit;
            row.balance += delta;
            row.limit = new_limit;
        }
        if let Some(new_soft) = input.soft_limit {
            row.soft_limit = new_soft;
        }
        if let Some(new_status) = input.status {
            row.status = new_status;
        }
        if let Some(new_metadata) = input.metadata {
            row.metadata = Some(new_metadata);
        }
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn delete_budget(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut budgets = self.budgets.write();
        if let Some(row) = budgets.get_mut(&id) {
            if row.org_id != org_id {
                return Ok(false);
            }
            row.status = "disabled".to_string();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ============================================
    // Usage Journal + Ledger
    // ============================================

    pub async fn create_usage_journal_entry(
        &self,
        input: CreateUsageJournalRow,
    ) -> Result<UsageJournalRow> {
        let now = Self::now();
        let row = UsageJournalRow {
            id: Uuid::now_v7(),
            org_id: input.org_id,
            kind: input.kind,
            source_type: input.source_type,
            source_id: input.source_id,
            event_id: input.event_id,
            session_id: input.session_id,
            turn_id: input.turn_id,
            user_id: input.user_id,
            principal_id: input.principal_id,
            agent_id: input.agent_id,
            harness_id: input.harness_id,
            measures: input.measures,
            metadata: input.metadata,
            created_at: now,
        };
        self.usage_journal.write().insert(row.id, row.clone());
        Ok(row)
    }

    pub async fn get_usage_journal(&self, id: Uuid) -> Result<Option<UsageJournalRow>> {
        Ok(self.usage_journal.read().get(&id).cloned())
    }

    pub async fn create_usage_ledger_entry(
        &self,
        input: CreateUsageLedgerRow,
    ) -> Result<(UsageLedgerRow, Option<BudgetRow>)> {
        let now = Self::now();
        let entry = UsageLedgerRow {
            id: Uuid::now_v7(),
            journal_id: input.journal_id,
            budget_id: input.budget_id,
            org_id: input.org_id,
            session_id: input.session_id,
            user_id: input.user_id,
            principal_id: input.principal_id,
            agent_id: input.agent_id,
            harness_id: input.harness_id,
            currency: input.currency,
            amount: input.amount,
            meter_source: input.meter_source,
            ref_type: input.ref_type,
            ref_id: input.ref_id,
            description: input.description,
            rating_metadata: input.rating_metadata,
            created_at: now,
        };
        self.usage_ledger.write().push(entry.clone());

        let updated_budget = match input.budget_id {
            Some(budget_id) => {
                let mut budgets = self.budgets.write();
                let budget = budgets
                    .get_mut(&budget_id)
                    .ok_or_else(|| anyhow::anyhow!("Budget not found: {}", budget_id))?;
                budget.balance -= input.amount;
                budget.updated_at = now;
                Some(budget.clone())
            }
            None => None,
        };

        self.enqueue_reporting_outbox(
            entry.org_id,
            "usage_ledger",
            &entry.id.to_string(),
            Some(&entry.id.to_string()),
            "budget_posting_projection",
        )
        .await?;

        Ok((entry, updated_budget))
    }

    pub async fn create_budget_ledger_entry(
        &self,
        input: CreateBudgetLedgerRow,
    ) -> Result<(BudgetLedgerRow, BudgetRow)> {
        let budget = self
            .get_budget_for_ledger(input.budget_id)
            .ok_or_else(|| anyhow::anyhow!("Budget not found: {}", input.budget_id))?;
        let (session_id, user_id, agent_id) = self.scope_from_budget(&budget, input.session_id);
        let journal_kind = if input.amount < 0.0 || input.ref_type.as_deref() == Some("top_up") {
            "top_up"
        } else {
            "budget_adjustment"
        };
        let journal = self
            .create_usage_journal_entry(CreateUsageJournalRow {
                org_id: budget.org_id,
                kind: journal_kind.to_string(),
                source_type: Some("budget_ledger_compat".to_string()),
                source_id: Some(Uuid::now_v7().to_string()),
                event_id: None,
                session_id,
                turn_id: None,
                user_id,
                principal_id: None,
                agent_id,
                harness_id: None,
                measures: serde_json::json!({
                    "amount": input.amount,
                    "currency": budget.currency,
                }),
                metadata: serde_json::json!({
                    "budget_id": budget.id,
                    "ref_type": input.ref_type,
                    "ref_id": input.ref_id,
                    "description": input.description,
                }),
            })
            .await?;

        let (entry, updated_budget) = self
            .create_usage_ledger_entry(CreateUsageLedgerRow {
                journal_id: journal.id,
                budget_id: Some(budget.id),
                org_id: budget.org_id,
                session_id,
                user_id,
                principal_id: None,
                agent_id,
                harness_id: None,
                currency: budget.currency.clone(),
                amount: input.amount,
                meter_source: input.meter_source,
                ref_type: input.ref_type,
                ref_id: input.ref_id,
                description: input.description,
                rating_metadata: Some(serde_json::json!({
                    "ruleset_version": "v1",
                    "compatibility_wrapper": true,
                })),
            })
            .await?;

        Ok((
            entry,
            updated_budget.expect("budget ledger compatibility path always updates a budget"),
        ))
    }

    pub async fn list_usage_ledger_for_budget(
        &self,
        budget_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<UsageLedgerRow>> {
        let ledger = self.usage_ledger.read();
        let mut entries: Vec<UsageLedgerRow> = ledger
            .iter()
            .filter(|e| e.budget_id == Some(budget_id))
            .cloned()
            .collect();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
        Ok(entries
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect())
    }

    pub async fn list_budget_ledger(
        &self,
        budget_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<BudgetLedgerRow>> {
        self.list_usage_ledger_for_budget(budget_id, limit, offset)
            .await
    }

    pub async fn set_budget_status(&self, id: Uuid, status: &str) -> Result<Option<BudgetRow>> {
        let mut budgets = self.budgets.write();
        if let Some(row) = budgets.get_mut(&id) {
            row.status = status.to_string();
            row.updated_at = Self::now();
            Ok(Some(row.clone()))
        } else {
            Ok(None)
        }
    }

    fn get_budget_for_ledger(&self, id: Uuid) -> Option<BudgetRow> {
        self.budgets.read().get(&id).cloned()
    }

    fn scope_from_budget(
        &self,
        budget: &BudgetRow,
        explicit_session_id: Option<Uuid>,
    ) -> (Option<Uuid>, Option<Uuid>, Option<Uuid>) {
        let session_id = explicit_session_id.or_else(|| {
            (budget.subject_type == "session")
                .then(|| {
                    SessionId::parse(&budget.subject_id)
                        .ok()
                        .map(|id| id.uuid())
                })
                .flatten()
        });
        let user_id = (budget.subject_type == "user")
            .then(|| Uuid::parse_str(&budget.subject_id).ok())
            .flatten();
        let agent_id = (budget.subject_type == "agent")
            .then(|| AgentId::parse(&budget.subject_id).ok().map(|id| id.uuid()))
            .flatten();
        (session_id, user_id, agent_id)
    }
}
