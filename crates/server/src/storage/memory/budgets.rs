// In-memory budget storage

use anyhow::Result;
use uuid::Uuid;

use super::InMemoryDatabase;
use crate::storage::models::*;

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
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
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
        let budgets = self.budgets.read();
        let mut result: Vec<BudgetRow> = budgets
            .values()
            .filter(|b| {
                b.org_id == org_id
                    && b.status == "active"
                    && ((b.subject_type == "session" && b.subject_id == session_id)
                        || (b.subject_type == "agent"
                            && agent_id.is_some_and(|a| b.subject_id == a))
                        || (b.subject_type == "user" && user_id.is_some_and(|u| b.subject_id == u))
                        || (b.subject_type == "org"
                            && org_public_id.is_some_and(|o| b.subject_id == o)))
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
    // Budget Ledger
    // ============================================

    pub async fn create_budget_ledger_entry(
        &self,
        input: CreateBudgetLedgerRow,
    ) -> Result<(BudgetLedgerRow, BudgetRow)> {
        let now = Self::now();
        let entry_id = Uuid::now_v7();

        // Update budget balance
        let updated_budget = {
            let mut budgets = self.budgets.write();
            let budget = budgets
                .get_mut(&input.budget_id)
                .ok_or_else(|| anyhow::anyhow!("Budget not found: {}", input.budget_id))?;
            budget.balance -= input.amount;
            budget.updated_at = now;
            budget.clone()
        };

        let entry = BudgetLedgerRow {
            id: entry_id,
            budget_id: input.budget_id,
            amount: input.amount,
            meter_source: input.meter_source,
            ref_type: input.ref_type,
            ref_id: input.ref_id,
            session_id: input.session_id,
            description: input.description,
            created_at: now,
        };
        self.budget_ledger.write().push(entry.clone());

        Ok((entry, updated_budget))
    }

    pub async fn list_budget_ledger(
        &self,
        budget_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<BudgetLedgerRow>> {
        let ledger = self.budget_ledger.read();
        let mut entries: Vec<BudgetLedgerRow> = ledger
            .iter()
            .filter(|e| e.budget_id == budget_id)
            .cloned()
            .collect();
        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        let result = entries
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();
        Ok(result)
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
}
