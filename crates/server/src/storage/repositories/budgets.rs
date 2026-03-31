// Budget storage (PostgreSQL)

use anyhow::Result;
use uuid::Uuid;

use crate::storage::Database;
use crate::storage::models::*;

impl Database {
    // ============================================
    // Budget CRUD
    // ============================================

    pub async fn create_budget(&self, input: CreateBudgetRow) -> Result<BudgetRow> {
        let public_id = format!("bdgt_{}", Uuid::now_v7().simple());
        let row = sqlx::query_as::<_, BudgetRow>(
            r#"
            INSERT INTO budgets (org_id, subject_type, subject_id, currency, "limit", soft_limit,
                                balance, period, metadata, status)
            VALUES ($1, $2, $3, $4, $5, $6, $5, $7, $8, 'active')
            RETURNING id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                      balance, period, metadata, status, created_at, updated_at
            "#,
        )
        .bind(input.org_id)
        .bind(&input.subject_type)
        .bind(&input.subject_id)
        .bind(&input.currency)
        .bind(input.limit)
        .bind(input.soft_limit)
        .bind(&input.period)
        .bind(&input.metadata)
        .fetch_one(&self.pool)
        .await?;
        // Store public_id — we don't have a column for it in this simplified version,
        // we use the UUID directly. The BudgetId typed wrapper handles formatting.
        let _ = public_id;
        Ok(row)
    }

    pub async fn get_budget(&self, org_id: i64, id: Uuid) -> Result<Option<BudgetRow>> {
        let row = sqlx::query_as::<_, BudgetRow>(
            r#"
            SELECT id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                   balance, period, metadata, status, created_at, updated_at
            FROM budgets
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_budgets(
        &self,
        org_id: i64,
        subject_type: Option<&str>,
        subject_id: Option<&str>,
    ) -> Result<Vec<BudgetRow>> {
        let rows = sqlx::query_as::<_, BudgetRow>(
            r#"
            SELECT id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                   balance, period, metadata, status, created_at, updated_at
            FROM budgets
            WHERE org_id = $1
              AND ($2::TEXT IS NULL OR subject_type = $2)
              AND ($3::TEXT IS NULL OR subject_id = $3)
              AND status != 'disabled'
            ORDER BY created_at DESC
            "#,
        )
        .bind(org_id)
        .bind(subject_type)
        .bind(subject_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Get all active budgets applicable to a session (session + agent + user + org budgets).
    pub async fn get_active_budgets_for_session(
        &self,
        org_id: i64,
        session_id: &str,
        agent_id: Option<&str>,
        user_id: Option<&str>,
        org_public_id: Option<&str>,
    ) -> Result<Vec<BudgetRow>> {
        // Build a list of (subject_type, subject_id) pairs to look up
        let mut subject_pairs: Vec<(&str, &str)> = vec![("session", session_id)];
        if let Some(aid) = agent_id {
            subject_pairs.push(("agent", aid));
        }
        if let Some(uid) = user_id {
            subject_pairs.push(("user", uid));
        }
        if let Some(oid) = org_public_id {
            subject_pairs.push(("org", oid));
        }

        // Use a single query with ANY for each pair
        let types: Vec<&str> = subject_pairs.iter().map(|(t, _)| *t).collect();
        let ids: Vec<&str> = subject_pairs.iter().map(|(_, i)| *i).collect();

        let rows = sqlx::query_as::<_, BudgetRow>(
            r#"
            SELECT id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                   balance, period, metadata, status, created_at, updated_at
            FROM budgets
            WHERE org_id = $1
              AND status = 'active'
              AND (subject_type, subject_id) IN (
                  SELECT UNNEST($2::TEXT[]), UNNEST($3::TEXT[])
              )
            ORDER BY balance ASC
            "#,
        )
        .bind(org_id)
        .bind(&types)
        .bind(&ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn update_budget(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateBudgetRow,
    ) -> Result<Option<BudgetRow>> {
        // If limit increased, also increase balance by the delta
        let row = sqlx::query_as::<_, BudgetRow>(
            r#"
            UPDATE budgets
            SET
                "limit" = COALESCE($3, "limit"),
                soft_limit = CASE WHEN $4 THEN $5 ELSE soft_limit END,
                balance = CASE
                    WHEN $3 IS NOT NULL THEN balance + ($3 - "limit")
                    ELSE balance
                END,
                status = COALESCE($6, status),
                metadata = COALESCE($7, metadata)
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                      balance, period, metadata, status, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(input.limit)
        .bind(input.soft_limit.is_some()) // flag: should we update soft_limit?
        .bind(input.soft_limit.flatten())
        .bind(input.status.as_deref())
        .bind(&input.metadata)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn delete_budget(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE budgets SET status = 'disabled'
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Budget Ledger
    // ============================================

    /// Record a ledger entry and atomically update the budget balance.
    /// Returns the updated budget row.
    pub async fn create_budget_ledger_entry(
        &self,
        input: CreateBudgetLedgerRow,
    ) -> Result<(BudgetLedgerRow, BudgetRow)> {
        let mut tx = self.pool.begin().await?;

        // Lock budget row
        let _budget = sqlx::query_as::<_, BudgetRow>(
            r#"
            SELECT id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                   balance, period, metadata, status, created_at, updated_at
            FROM budgets
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(input.budget_id)
        .fetch_one(&mut *tx)
        .await?;

        // Insert ledger entry
        let entry = sqlx::query_as::<_, BudgetLedgerRow>(
            r#"
            INSERT INTO budget_ledger (budget_id, amount, meter_source, ref_type, ref_id, session_id, description)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, budget_id, amount, meter_source, ref_type, ref_id, session_id, description, created_at
            "#,
        )
        .bind(input.budget_id)
        .bind(input.amount)
        .bind(&input.meter_source)
        .bind(input.ref_type.as_deref())
        .bind(input.ref_id)
        .bind(input.session_id)
        .bind(input.description.as_deref())
        .fetch_one(&mut *tx)
        .await?;

        // Update balance (subtract debit, add credit)
        let updated_budget = sqlx::query_as::<_, BudgetRow>(
            r#"
            UPDATE budgets
            SET balance = balance - $2
            WHERE id = $1
            RETURNING id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                      balance, period, metadata, status, created_at, updated_at
            "#,
        )
        .bind(input.budget_id)
        .bind(input.amount)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok((entry, updated_budget))
    }

    pub async fn list_budget_ledger(
        &self,
        budget_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<BudgetLedgerRow>> {
        let rows = sqlx::query_as::<_, BudgetLedgerRow>(
            r#"
            SELECT id, budget_id, amount, meter_source, ref_type, ref_id, session_id,
                   description, created_at
            FROM budget_ledger
            WHERE budget_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(budget_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Update budget status (used by rules engine).
    pub async fn set_budget_status(&self, id: Uuid, status: &str) -> Result<Option<BudgetRow>> {
        let row = sqlx::query_as::<_, BudgetRow>(
            r#"
            UPDATE budgets SET status = $2
            WHERE id = $1
            RETURNING id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                      balance, period, metadata, status, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(status)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }
}
