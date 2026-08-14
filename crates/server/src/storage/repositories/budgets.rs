// Budget storage (PostgreSQL)

use crate::kernel_imports::{
    everruns_provider::typed_id::AgentId, everruns_provider::typed_id::SessionId,
};
use anyhow::Result;
use uuid::Uuid;

use crate::storage::Database;
use crate::storage::models::*;

/// Subject identifiers used to look up the active budgets that cascade onto
/// a single LLM generation. Any field set to `None` is silently skipped, so
/// callers can pass exactly the layers they have resolved.
#[derive(Debug, Clone, Default)]
pub struct BudgetSubjectLookup<'a> {
    pub session_id: Option<&'a str>,
    pub agent_id: Option<&'a str>,
    pub user_id: Option<&'a str>,
    pub org_public_id: Option<&'a str>,
    pub app_id: Option<&'a str>,
    pub app_channel_id: Option<&'a str>,
}

impl<'a> BudgetSubjectLookup<'a> {
    /// Flatten into `(subject_type, subject_id)` pairs, preserving hierarchy
    /// order from most specific (session) to most general (org).
    pub fn to_pairs(&self) -> Vec<(&'a str, &'a str)> {
        let mut pairs = Vec::new();
        if let Some(id) = self.session_id {
            pairs.push(("session", id));
        }
        if let Some(id) = self.app_channel_id {
            pairs.push(("app_channel", id));
        }
        if let Some(id) = self.app_id {
            pairs.push(("app", id));
        }
        if let Some(id) = self.agent_id {
            pairs.push(("agent", id));
        }
        if let Some(id) = self.user_id {
            pairs.push(("user", id));
        }
        if let Some(id) = self.org_public_id {
            pairs.push(("org", id));
        }
        pairs
    }
}

impl Database {
    // ============================================
    // Budget CRUD
    // ============================================

    pub async fn create_budget(&self, input: CreateBudgetRow) -> Result<BudgetRow> {
        let public_id = format!("bdgt_{}", Uuid::now_v7().simple());
        let period_started_at = input.period.as_ref().map(|_| chrono::Utc::now());
        let row = sqlx::query_as::<_, BudgetRow>(
            r#"
            INSERT INTO budgets (org_id, subject_type, subject_id, currency, "limit", soft_limit,
                                balance, period, period_started_at, metadata, status)
            VALUES ($1, $2, $3, $4, $5, $6, $5, $7, $8, $9, 'active')
            RETURNING id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                      balance, period, period_started_at, metadata, status, created_at, updated_at
            "#,
        )
        .bind(input.org_id)
        .bind(&input.subject_type)
        .bind(&input.subject_id)
        .bind(&input.currency)
        .bind(input.limit)
        .bind(input.soft_limit)
        .bind(&input.period)
        .bind(period_started_at)
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
                   balance, period, period_started_at, metadata, status, created_at, updated_at
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
                   balance, period, period_started_at, metadata, status, created_at, updated_at
            FROM budgets
            WHERE org_id = $1
              AND ($2::TEXT IS NULL OR subject_type = $2)
              AND ($3::TEXT IS NULL OR subject_id = $3)
              AND status != 'disabled'
            ORDER BY created_at DESC
            LIMIT 200
            "#,
        )
        .bind(org_id)
        .bind(subject_type)
        .bind(subject_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Get all active budgets applicable to a session, walking the full
    /// `session → app_channel → app → agent → user → org` hierarchy.
    /// Empty/missing layers are silently skipped.
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

    /// Generalised hierarchy lookup — supports app and app_channel subjects in
    /// addition to the legacy session/agent/user/org levels.
    pub async fn get_active_budgets_for_subjects(
        &self,
        org_id: i64,
        lookup: BudgetSubjectLookup<'_>,
    ) -> Result<Vec<BudgetRow>> {
        let pairs = lookup.to_pairs();
        if pairs.is_empty() {
            return Ok(vec![]);
        }

        let types: Vec<&str> = pairs.iter().map(|(t, _)| *t).collect();
        let ids: Vec<&str> = pairs.iter().map(|(_, i)| *i).collect();

        let rows = sqlx::query_as::<_, BudgetRow>(
            r#"
            SELECT id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                   balance, period, period_started_at, metadata, status, created_at, updated_at
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

    /// Reset a budget's window: set balance back to limit and stamp
    /// `period_started_at`. Used by the budget service when a `Duration`
    /// or `Calendar` window has elapsed.
    pub async fn reset_budget_period(
        &self,
        id: Uuid,
        period_started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<BudgetRow>> {
        let row = sqlx::query_as::<_, BudgetRow>(
            r#"
            UPDATE budgets
               SET balance = "limit",
                   period_started_at = $2,
                   status = CASE
                       WHEN status IN ('paused', 'exhausted') THEN 'active'
                       ELSE status
                   END
             WHERE id = $1
         RETURNING id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                   balance, period, period_started_at, metadata, status, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(period_started_at)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
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
                      balance, period, period_started_at, metadata, status, created_at, updated_at
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
    // Usage Journal + Ledger
    // ============================================

    pub async fn create_usage_journal_entry(
        &self,
        input: CreateUsageJournalRow,
    ) -> Result<UsageJournalRow> {
        let row = sqlx::query_as::<_, UsageJournalRow>(
            r#"
            INSERT INTO usage_journal (
                org_id, kind, source_type, source_id, event_id, session_id, turn_id,
                user_id, principal_id, agent_id, harness_id, measures, metadata
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7,
                $8, $9, $10, $11, $12, $13
            )
            RETURNING id, org_id, kind, source_type, source_id, event_id, session_id, turn_id,
                      user_id, principal_id, agent_id, harness_id, measures, metadata, created_at
            "#,
        )
        .bind(input.org_id)
        .bind(&input.kind)
        .bind(input.source_type.as_deref())
        .bind(input.source_id.as_deref())
        .bind(input.event_id)
        .bind(input.session_id)
        .bind(input.turn_id)
        .bind(input.user_id)
        .bind(input.principal_id)
        .bind(input.agent_id)
        .bind(input.harness_id)
        .bind(&input.measures)
        .bind(&input.metadata)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_usage_journal(&self, id: Uuid) -> Result<Option<UsageJournalRow>> {
        let row = sqlx::query_as::<_, UsageJournalRow>(
            r#"
            SELECT id, org_id, kind, source_type, source_id, event_id, session_id, turn_id,
                   user_id, principal_id, agent_id, harness_id, measures, metadata, created_at
            FROM usage_journal
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Record a rated ledger entry and update any attached budget snapshot atomically.
    pub async fn create_usage_ledger_entry(
        &self,
        input: CreateUsageLedgerRow,
    ) -> Result<(UsageLedgerRow, Option<BudgetRow>)> {
        let mut tx = self.pool.begin().await?;

        let updated_budget = if let Some(budget_id) = input.budget_id {
            let _budget = sqlx::query_as::<_, BudgetRow>(
                r#"
                SELECT id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                       balance, period, period_started_at, metadata, status, created_at, updated_at
                FROM budgets
                WHERE id = $1
                FOR UPDATE
                "#,
            )
            .bind(budget_id)
            .fetch_one(&mut *tx)
            .await?;

            Some(
                sqlx::query_as::<_, BudgetRow>(
                    r#"
                    UPDATE budgets
                    SET balance = balance - $2
                    WHERE id = $1
                    RETURNING id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                              balance, period, period_started_at, metadata, status, created_at, updated_at
                    "#,
                )
                .bind(budget_id)
                .bind(input.amount)
                .fetch_one(&mut *tx)
                .await?,
            )
        } else {
            None
        };

        let entry = sqlx::query_as::<_, UsageLedgerRow>(
            r#"
            INSERT INTO usage_ledger (
                journal_id, budget_id, org_id, session_id, user_id, principal_id,
                agent_id, harness_id, currency, amount, meter_source, ref_type,
                ref_id, description, rating_metadata
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10, $11, $12,
                $13, $14, $15
            )
            RETURNING id, journal_id, budget_id, org_id, session_id, user_id, principal_id,
                      agent_id, harness_id, currency, amount, meter_source, ref_type, ref_id,
                      description, rating_metadata, created_at
            "#,
        )
        .bind(input.journal_id)
        .bind(input.budget_id)
        .bind(input.org_id)
        .bind(input.session_id)
        .bind(input.user_id)
        .bind(input.principal_id)
        .bind(input.agent_id)
        .bind(input.harness_id)
        .bind(&input.currency)
        .bind(input.amount)
        .bind(&input.meter_source)
        .bind(input.ref_type.as_deref())
        .bind(input.ref_id)
        .bind(input.description.as_deref())
        .bind(&input.rating_metadata)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO reporting_outbox (
                org_id, source_type, source_id, source_version, reason, status, next_attempt_at
            )
            VALUES ($1, 'usage_ledger', $2, $2, 'budget_posting_projection', 'pending', NOW())
            ON CONFLICT (org_id, source_type, source_id, source_version, reason)
            DO NOTHING
            "#,
        )
        .bind(entry.org_id)
        .bind(entry.id.to_string())
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok((entry, updated_budget))
    }

    /// Record a ledger entry and atomically update the budget balance.
    /// Returns the updated budget row.
    pub async fn create_budget_ledger_entry(
        &self,
        input: CreateBudgetLedgerRow,
    ) -> Result<(BudgetLedgerRow, BudgetRow)> {
        let budget = sqlx::query_as::<_, BudgetRow>(
            r#"
            SELECT id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                   balance, period, period_started_at, metadata, status, created_at, updated_at
            FROM budgets
            WHERE id = $1
            "#,
        )
        .bind(input.budget_id)
        .fetch_one(&self.pool)
        .await?;
        let (session_id, user_id, agent_id) = scope_from_budget(&budget, input.session_id);
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
        let rows = sqlx::query_as::<_, UsageLedgerRow>(
            r#"
            SELECT id, journal_id, budget_id, org_id, session_id, user_id, principal_id,
                   agent_id, harness_id, currency, amount, meter_source, ref_type, ref_id,
                   description, rating_metadata, created_at
            FROM usage_ledger
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

    pub async fn list_budget_ledger(
        &self,
        budget_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<BudgetLedgerRow>> {
        self.list_usage_ledger_for_budget(budget_id, limit, offset)
            .await
    }

    /// Update budget status (used by rules engine).
    pub async fn set_budget_status(&self, id: Uuid, status: &str) -> Result<Option<BudgetRow>> {
        let row = sqlx::query_as::<_, BudgetRow>(
            r#"
            UPDATE budgets SET status = $2
            WHERE id = $1
            RETURNING id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
                      balance, period, period_started_at, metadata, status, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(status)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }
}

fn scope_from_budget(
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
