//! Org costs: the price list (`cost_rates`), cost postings on `usage_ledger`
//! (rows with no budget), usage policy approvals and notices, the org's
//! timezone, and the sandbox-time meter's source query.
//! See knowledge/security/budgeting.md.
//!
//! Decision: these read and write PostgreSQL directly, like
//! `background_limits`, because costs are a server concern with no embedded
//! or in-memory counterpart (the embedded host keeps no usage ledger).
use super::StorageBackend;
use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// One row of an org's price list.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CostRateRow {
    pub id: Uuid,
    pub org_id: i64,
    pub meter: String,
    #[sqlx(rename = "match")]
    pub match_: serde_json::Value,
    pub unit_price_usd: Option<f64>,
    pub list_price_multiplier: Option<f64>,
    pub unit: String,
    pub effective_from: DateTime<Utc>,
    pub retired: bool,
    pub reason: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// Input for a new price list row.
#[derive(Debug, Clone)]
pub struct CreateCostRateRow {
    pub org_id: i64,
    pub meter: String,
    pub match_: serde_json::Value,
    pub unit_price_usd: Option<f64>,
    pub list_price_multiplier: Option<f64>,
    pub unit: String,
    pub effective_from: DateTime<Utc>,
    pub retired: bool,
    pub reason: Option<String>,
    pub created_by: Option<Uuid>,
}

const RATE_COLUMNS: &str = "id, org_id, meter, match, unit_price_usd, list_price_multiplier, \
    unit, effective_from, retired, reason, created_by, created_at";

/// One aggregated line of the costs summary.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CostSummaryRow {
    pub key: Option<String>,
    pub label: Option<String>,
    pub basis: Option<String>,
    pub amount_usd: f64,
    pub unpriced: i64,
    pub records: i64,
    pub quantity: f64,
}

/// A closed span of a sandbox running, not yet in the usage record.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SandboxSpanRow {
    pub transition_id: i64,
    pub org_id: i64,
    pub sandbox_id: Uuid,
    pub session_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    pub provider: String,
    pub profile_name: Option<String>,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
}

impl StorageBackend {
    // ------------------------------------------------------------------
    // Price list
    // ------------------------------------------------------------------

    /// Every row of the org's price list, newest first (history included).
    pub async fn list_cost_rates(&self, org_id: i64) -> Result<Vec<CostRateRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {RATE_COLUMNS} FROM cost_rates WHERE org_id = $1 \
             ORDER BY effective_from DESC, created_at DESC"
        )))
        .bind(org_id)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// Rates in force for `meter` at `at`: per distinct match, the latest row
    /// that started by then, unless that row retires the match.
    pub async fn effective_cost_rates(
        &self,
        org_id: i64,
        meter: &str,
        at: DateTime<Utc>,
    ) -> Result<Vec<CostRateRow>> {
        let rows: Vec<CostRateRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT DISTINCT ON (match) {RATE_COLUMNS} FROM cost_rates \
             WHERE org_id = $1 AND meter = $2 AND effective_from <= $3 \
             ORDER BY match, effective_from DESC, created_at DESC"
        )))
        .bind(org_id)
        .bind(meter)
        .bind(at)
        .fetch_all(self.database().pool())
        .await?;
        Ok(rows.into_iter().filter(|row| !row.retired).collect())
    }

    pub async fn create_cost_rate(&self, input: CreateCostRateRow) -> Result<CostRateRow> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO cost_rates (org_id, meter, match, unit_price_usd, list_price_multiplier, \
             unit, effective_from, retired, reason, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING {RATE_COLUMNS}"
        )))
        .bind(input.org_id)
        .bind(&input.meter)
        .bind(&input.match_)
        .bind(input.unit_price_usd)
        .bind(input.list_price_multiplier)
        .bind(&input.unit)
        .bind(input.effective_from)
        .bind(input.retired)
        .bind(input.reason.as_deref())
        .bind(input.created_by)
        .fetch_one(self.database().pool())
        .await?)
    }

    pub async fn get_cost_rate(&self, org_id: i64, id: Uuid) -> Result<Option<CostRateRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {RATE_COLUMNS} FROM cost_rates WHERE org_id = $1 AND id = $2"
        )))
        .bind(org_id)
        .bind(id)
        .fetch_optional(self.database().pool())
        .await?)
    }

    // ------------------------------------------------------------------
    // Cost postings
    // ------------------------------------------------------------------

    /// The journal row a host's charge refers to, by the source the journal
    /// was written under (`llm_generation` + event id, `sandbox_span` + id).
    pub async fn usage_journal_by_source(
        &self,
        org_id: i64,
        source_type: &str,
        source_id: &str,
    ) -> Result<Option<super::UsageJournalRow>> {
        Ok(sqlx::query_as(
            "SELECT id, org_id, kind, source_type, source_id, event_id, session_id, turn_id, \
                    user_id, principal_id, agent_id, harness_id, measures, metadata, created_at \
             FROM usage_journal WHERE org_id = $1 AND source_type = $2 AND source_id = $3",
        )
        .bind(org_id)
        .bind(source_type)
        .bind(source_id)
        .fetch_optional(self.database().pool())
        .await?)
    }

    /// Sum of the cost postings on one journal row, and whether a host
    /// already reported its charge.
    pub async fn journal_cost(&self, journal_id: Uuid) -> Result<(Option<f64>, bool)> {
        let row: (Option<f64>, Option<i64>, Option<i64>) = sqlx::query_as(
            "SELECT SUM(amount) FILTER (WHERE basis IS DISTINCT FROM 'unpriced'), \
                    COUNT(*) FILTER (WHERE meter_source = 'cost_host'), \
                    COUNT(*) FILTER (WHERE basis IS DISTINCT FROM 'unpriced') \
             FROM usage_ledger WHERE journal_id = $1 AND budget_id IS NULL",
        )
        .bind(journal_id)
        .fetch_one(self.database().pool())
        .await?;
        let priced = row.2.unwrap_or(0) > 0;
        Ok((
            if priced {
                Some(row.0.unwrap_or(0.0))
            } else {
                None
            },
            row.1.unwrap_or(0) > 0,
        ))
    }

    /// Org cost grouped by one dimension over `[from, to)`. `dimension` is
    /// one of `basis`, `agent`, `user`, `source`, `day`, `started_by`.
    pub async fn cost_summary(
        &self,
        org_id: i64,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        dimension: &str,
        timezone: &str,
    ) -> Result<Vec<CostSummaryRow>> {
        // Decision: one query shape with the grouping key chosen from a fixed
        // list, so no caller text reaches the SQL.
        let (key, label) = match dimension {
            "basis" => ("per.basis", "per.basis"),
            "agent" => ("a.public_id", "a.name"),
            "user" => ("j.user_id::text", "u.name"),
            "source" => (
                "COALESCE(j.metadata->>'source_key', j.kind)",
                "COALESCE(j.metadata->>'source_label', j.kind)",
            ),
            "day" => (
                "to_char(per.at AT TIME ZONE $4::text, 'YYYY-MM-DD')",
                "to_char(per.at AT TIME ZONE $4::text, 'YYYY-MM-DD')",
            ),
            "started_by" => (
                "COALESCE(j.metadata->>'started_by', 'person')",
                "COALESCE(j.metadata->>'started_by', 'person')",
            ),
            other => anyhow::bail!("unknown cost dimension {other}"),
        };
        // Per journal row first: a host's correction (`cost_host`) makes the
        // whole usage "billed by host", not just the difference it posted.
        let sql = format!(
            r#"
            WITH per AS (
                SELECT l.journal_id,
                       MIN(l.created_at) AS at,
                       CASE WHEN bool_or(l.meter_source = 'cost_host') THEN 'host' ELSE MAX(l.basis) END AS basis,
                       SUM(l.amount) FILTER (WHERE l.basis IS DISTINCT FROM 'unpriced') AS amount
                FROM usage_ledger l
                WHERE l.org_id = $1 AND l.budget_id IS NULL AND l.currency = 'usd'
                  AND l.created_at >= $2 AND l.created_at < $3
                  AND $4::text IS NOT NULL
                GROUP BY l.journal_id
            )
            SELECT {key} AS key,
                   MAX({label}) AS label,
                   CASE WHEN COUNT(DISTINCT per.basis) = 1 THEN MAX(per.basis) ELSE 'mixed' END AS basis,
                   COALESCE(SUM(per.amount), 0)::float8 AS amount_usd,
                   COUNT(*) FILTER (WHERE per.basis = 'unpriced') AS unpriced,
                   COUNT(*) AS records,
                   COALESCE(SUM((j.measures->>'quantity')::float8), 0)::float8 AS quantity
            FROM per
            JOIN usage_journal j ON j.id = per.journal_id
            LEFT JOIN agents a ON a.id = j.agent_id
            LEFT JOIN users u ON u.id = j.user_id
            GROUP BY 1
            ORDER BY amount_usd DESC NULLS LAST
            LIMIT 200
            "#
        );
        // Only fixed fragments are formatted in: the dimension picks from a closed list.
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(org_id)
            .bind(from)
            .bind(to)
            .bind(timezone)
            .fetch_all(self.database().pool())
            .await?)
    }

    /// Cost so far in `[from, to)` for each agent, keyed by internal id.
    pub async fn agent_cost_since(
        &self,
        org_id: i64,
        agent_id: Uuid,
        from: DateTime<Utc>,
    ) -> Result<(f64, i64)> {
        let row: (Option<f64>, Option<i64>) = sqlx::query_as(
            "SELECT SUM(amount) FILTER (WHERE basis IS DISTINCT FROM 'unpriced'), \
                    COUNT(*) FILTER (WHERE basis = 'unpriced') \
             FROM usage_ledger WHERE org_id = $1 AND agent_id = $2 AND budget_id IS NULL \
               AND currency = 'usd' AND created_at >= $3",
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(from)
        .fetch_one(self.database().pool())
        .await?;
        Ok((row.0.unwrap_or(0.0), row.1.unwrap_or(0)))
    }

    // ------------------------------------------------------------------
    // Usage policy approvals and notices
    // ------------------------------------------------------------------

    pub async fn create_usage_policy_approval(
        &self,
        org_id: i64,
        budget_id: Uuid,
        session_id: Uuid,
        approved_by: Option<Uuid>,
        window_started_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO usage_policy_approvals (org_id, budget_id, session_id, approved_by, window_started_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(org_id)
        .bind(budget_id)
        .bind(session_id)
        .bind(approved_by)
        .bind(window_started_at)
        .execute(self.database().pool())
        .await?;
        Ok(())
    }

    /// Whether a person let this session past the policy's ask step in the
    /// policy's current window.
    pub async fn usage_policy_approved(
        &self,
        budget_id: Uuid,
        session_id: Uuid,
        window_started_at: Option<DateTime<Utc>>,
    ) -> Result<bool> {
        let found: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM usage_policy_approvals WHERE budget_id = $1 AND session_id = $2 \
             AND window_started_at IS NOT DISTINCT FROM $3 LIMIT 1",
        )
        .bind(budget_id)
        .bind(session_id)
        .bind(window_started_at)
        .fetch_optional(self.database().pool())
        .await?;
        Ok(found.is_some())
    }

    /// Record that a notify step fired for this window. `true` the first time.
    pub async fn claim_usage_policy_notice(
        &self,
        budget_id: Uuid,
        step_at: f64,
        window_started_at: DateTime<Utc>,
    ) -> Result<bool> {
        let result = sqlx::query(
            "INSERT INTO usage_policy_notices (budget_id, step_at, window_started_at) \
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(budget_id)
        .bind(step_at)
        .bind(window_started_at)
        .execute(self.database().pool())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Owners and admins of an org: who hears about org usage policies.
    pub async fn org_owner_and_admin_ids(&self, org_id: i64) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT user_id FROM organization_members WHERE org_id = $1 AND role IN ('owner', 'admin')",
        )
        .bind(org_id)
        .fetch_all(self.database().pool())
        .await?)
    }

    // ------------------------------------------------------------------
    // Org timezone
    // ------------------------------------------------------------------

    /// The org's timezone (IANA name), `None` for UTC.
    pub async fn org_timezone(&self, org_id: i64) -> Result<Option<String>> {
        let stored: Option<Option<String>> =
            sqlx::query_scalar("SELECT timezone FROM organization_settings WHERE org_id = $1")
                .bind(org_id)
                .fetch_optional(self.database().pool())
                .await?;
        Ok(stored.flatten())
    }

    pub async fn set_org_timezone(&self, org_id: i64, timezone: Option<&str>) -> Result<()> {
        sqlx::query(
            "INSERT INTO organization_settings (org_id, timezone) VALUES ($1, $2) \
             ON CONFLICT (org_id) DO UPDATE SET timezone = EXCLUDED.timezone, updated_at = NOW()",
        )
        .bind(org_id)
        .bind(timezone)
        .execute(self.database().pool())
        .await?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Sandbox time meter
    // ------------------------------------------------------------------

    /// Closed spans of sandboxes running (`ready`) that the usage record
    /// does not have yet, oldest first.
    pub async fn unmetered_sandbox_spans(&self, limit: i64) -> Result<Vec<SandboxSpanRow>> {
        Ok(sqlx::query_as(
            r#"
            WITH t AS (
                SELECT tr.id, tr.sandbox_id, tr.org_id, tr.state, tr.at,
                       lead(tr.at) OVER (PARTITION BY tr.sandbox_id ORDER BY tr.at, tr.id) AS next_at
                FROM sandbox_state_transitions tr
                WHERE tr.at > now() - interval '35 days'
            )
            SELECT t.id AS transition_id, t.org_id, t.sandbox_id, s.session_id, s.agent_id,
                   s.provider, s.profile_name, t.at AS start_at, t.next_at AS end_at
            FROM t
            JOIN sandboxes s ON s.id = t.sandbox_id
            WHERE t.state = 'ready' AND t.next_at IS NOT NULL
              AND NOT EXISTS (
                  SELECT 1 FROM usage_journal j
                  WHERE j.source_type = 'sandbox_span' AND j.source_id = t.id::text
              )
            ORDER BY t.at
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(self.database().pool())
        .await?)
    }
}
