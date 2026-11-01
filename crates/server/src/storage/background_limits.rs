//! Limits on unattended background work: per-org overrides on
//! `organization_settings.background_limits`, the `session_schedule` and
//! `org_unattended` budgets, and which schedule a turn's input came from.
//! See knowledge/operations/background-limits.md.
//!
//! Decision: overrides live in one JSONB column read apart from
//! `OrganizationSettingsRow`, like `web_search_defaults`, so the settings
//! patch keeps its shape. NULL means the code defaults.
use super::{BudgetRow, CreateBudgetRow, StorageBackend};
use anyhow::Result;
use everruns_contracts::runtime::session_schedule::BackgroundLimits;
use uuid::Uuid;

/// Budget subject of one schedule's spend, keyed by the schedule's public id.
pub const SESSION_SCHEDULE_SUBJECT: &str = "session_schedule";
/// Budget subject of an org's daily unattended spend, keyed by the org's
/// public id.
pub const ORG_UNATTENDED_SUBJECT: &str = "org_unattended";

const BUDGET_COLUMNS: &str = r#"id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
    balance, period, period_started_at, metadata, policy, status, created_at, updated_at"#;

impl StorageBackend {
    /// The org's background limits; the code defaults when it never set any.
    /// A stored value that no longer parses reads as the defaults, logged.
    pub async fn org_background_limits(&self, org_id: i64) -> Result<BackgroundLimits> {
        let stored: Option<Option<serde_json::Value>> = sqlx::query_scalar(
            "SELECT background_limits FROM organization_settings WHERE org_id = $1",
        )
        .bind(org_id)
        .fetch_optional(self.database().pool())
        .await?;
        Ok(match stored.flatten() {
            Some(value) => serde_json::from_value(value).unwrap_or_else(|error| {
                tracing::warn!(%error, org_id, "Stored background limits do not parse; using defaults");
                BackgroundLimits::default()
            }),
            None => BackgroundLimits::default(),
        })
    }

    /// Replace the org's background limits (already validated) and bring its
    /// live `org_unattended` budget in line with the new daily cap.
    pub async fn set_org_background_limits(
        &self,
        org_id: i64,
        limits: &BackgroundLimits,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO organization_settings (org_id, background_limits) VALUES ($1, $2) \
             ON CONFLICT (org_id) DO UPDATE SET background_limits = EXCLUDED.background_limits, \
             updated_at = NOW()",
        )
        .bind(org_id)
        .bind(serde_json::to_value(limits)?)
        .execute(self.database().pool())
        .await?;
        match limits.org_unattended_daily_usd {
            // Raising the cap also frees a budget that ran out today; the
            // balance moves by the same delta, as any budget limit change does.
            Some(cap) => {
                sqlx::query(
                    r#"UPDATE budgets SET balance = balance + ($2 - "limit"), "limit" = $2,
                       status = CASE WHEN balance + ($2 - "limit") > 0 THEN 'active' ELSE status END,
                       updated_at = NOW()
                       WHERE org_id = $1 AND subject_type = 'org_unattended' AND status <> 'disabled'"#,
                )
                .bind(org_id)
                .bind(cap)
                .execute(self.database().pool())
                .await?;
            }
            None => {
                sqlx::query(
                    "UPDATE budgets SET status = 'disabled', updated_at = NOW() \
                     WHERE org_id = $1 AND subject_type = 'org_unattended' AND status <> 'disabled'",
                )
                .bind(org_id)
                .execute(self.database().pool())
                .await?;
            }
        }
        Ok(())
    }

    /// The org's live unattended budget, created from its limits on first use.
    /// `None` when the org removed the cap.
    pub async fn ensure_org_unattended_budget(&self, org_id: i64) -> Result<Option<BudgetRow>> {
        if let Some(existing) = self.org_unattended_budget(org_id).await? {
            return Ok(Some(existing));
        }
        let Some(cap) = self
            .org_background_limits(org_id)
            .await?
            .org_unattended_daily_usd
        else {
            return Ok(None);
        };
        // The partial unique index makes concurrent first uses converge on one row.
        sqlx::query(
            r#"INSERT INTO budgets (org_id, subject_type, subject_id, currency, "limit", soft_limit,
                                   balance, period, period_started_at, metadata, policy, status)
               VALUES ($1, 'org_unattended', $2, 'usd', $3, NULL, $3, $4, NOW(), $5, '{}'::jsonb, 'active')
               ON CONFLICT (org_id) WHERE subject_type = 'org_unattended' AND status <> 'disabled'
               DO NOTHING"#,
        )
        .bind(org_id)
        .bind(everruns_core::org_public_id_from_internal(org_id))
        .bind(cap)
        .bind(serde_json::json!({"type": "calendar", "unit": "day"}))
        .bind(serde_json::json!({"created_by": "background_limits"}))
        .execute(self.database().pool())
        .await?;
        self.org_unattended_budget(org_id).await
    }

    /// The org's live unattended budget, if one was created.
    pub async fn org_unattended_budget(&self, org_id: i64) -> Result<Option<BudgetRow>> {
        Ok(sqlx::query_as::<_, BudgetRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {BUDGET_COLUMNS} FROM budgets \
             WHERE org_id = $1 AND subject_type = 'org_unattended' AND status <> 'disabled' LIMIT 1"
        )))
        .bind(org_id)
        .fetch_optional(self.database().pool())
        .await?)
    }

    /// Create the budget of a new schedule.
    pub async fn create_schedule_budget(
        &self,
        org_id: i64,
        schedule_public_id: &str,
        limit_usd: f64,
    ) -> Result<BudgetRow> {
        self.database()
            .create_budget(CreateBudgetRow {
                org_id,
                subject_type: SESSION_SCHEDULE_SUBJECT.to_string(),
                subject_id: schedule_public_id.to_string(),
                currency: "usd".to_string(),
                limit: limit_usd,
                soft_limit: None,
                period: None,
                metadata: Some(serde_json::json!({"created_by": "background_limits"})),
            })
            .await
    }

    /// Live budgets of these schedules, keyed by schedule public id.
    pub async fn schedule_budgets(
        &self,
        org_id: i64,
        schedule_public_ids: &[String],
    ) -> Result<Vec<BudgetRow>> {
        if schedule_public_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(sqlx::query_as::<_, BudgetRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {BUDGET_COLUMNS} FROM budgets \
             WHERE org_id = $1 AND subject_type = 'session_schedule' AND subject_id = ANY($2) \
               AND status <> 'disabled'"
        )))
        .bind(org_id)
        .bind(schedule_public_ids)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// Set a schedule's budget to `limit_usd`, creating it when the schedule
    /// has none, and make it active again when the new limit leaves room.
    pub async fn set_schedule_budget_limit(
        &self,
        org_id: i64,
        schedule_public_id: &str,
        limit_usd: f64,
    ) -> Result<BudgetRow> {
        let updated = sqlx::query_as::<_, BudgetRow>(sqlx::AssertSqlSafe(format!(
            r#"UPDATE budgets SET balance = balance + ($3 - "limit"), "limit" = $3,
                   status = CASE WHEN balance + ($3 - "limit") > 0 THEN 'active' ELSE 'exhausted' END,
                   updated_at = NOW()
               WHERE org_id = $1 AND subject_type = 'session_schedule' AND subject_id = $2
                 AND status <> 'disabled'
               RETURNING {BUDGET_COLUMNS}"#
        )))
        .bind(org_id)
        .bind(schedule_public_id)
        .bind(limit_usd)
        .fetch_optional(self.database().pool())
        .await?;
        match updated {
            Some(row) => Ok(row),
            None => {
                self.create_schedule_budget(org_id, schedule_public_id, limit_usd)
                    .await
            }
        }
    }

    /// Record which schedule fired the turn whose input is `input_message_id`.
    pub async fn set_invocation_schedule(
        &self,
        input_message_id: Uuid,
        schedule_id: Uuid,
    ) -> Result<()> {
        sqlx::query("UPDATE runtime_invocations SET schedule_id = $2 WHERE input_message_id = $1")
            .bind(input_message_id)
            .bind(schedule_id)
            .execute(self.database().pool())
            .await?;
        Ok(())
    }

    /// The schedule that fired the turn whose input is `input_message_id`.
    pub async fn invocation_schedule(&self, input_message_id: Uuid) -> Result<Option<Uuid>> {
        let id: Option<Option<Uuid>> = sqlx::query_scalar(
            "SELECT schedule_id FROM runtime_invocations WHERE input_message_id = $1",
        )
        .bind(input_message_id)
        .fetch_optional(self.database().pool())
        .await?;
        Ok(id.flatten())
    }

    /// The schedule that fired the session's latest input, if a schedule did.
    /// Message ids are UUIDv7, so the latest invocation is the turn in flight;
    /// the loop gate has no turn context and uses this.
    pub async fn latest_invocation_schedule(
        &self,
        org_id: i64,
        session_id: Uuid,
    ) -> Result<Option<Uuid>> {
        let id: Option<Option<Uuid>> = sqlx::query_scalar(
            "SELECT schedule_id FROM runtime_invocations WHERE org_id = $1 AND session_id = $2 \
             ORDER BY input_message_id DESC LIMIT 1",
        )
        .bind(org_id)
        .bind(session_id)
        .fetch_optional(self.database().pool())
        .await?;
        Ok(id.flatten())
    }
}
