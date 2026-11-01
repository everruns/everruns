//! Costs API: the org's price list, the cost summary behind Settings →
//! Costs, and the org timezone calendar windows use.
//! See knowledge/security/budgeting.md.

use super::costs::{METER_MODEL_TOKENS, METER_SANDBOX_TIME};
use super::{BUDGET_MANAGE, BUDGET_VIEW};
use crate::domains::common::*;
use crate::storage::costs::{CostRateRow, CostSummaryRow, CreateCostRateRow};
use chrono::{DateTime, Datelike, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// One row of the org's price list.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CostRate {
    /// Rate id (`rate_...`).
    pub id: String,
    /// What it prices: `model.tokens` or `sandbox.time`.
    pub meter: String,
    /// Which usage it applies to. Model tokens: `provider`, `model`,
    /// `source` (`managed` or `own_keys`). Sandbox time: `provider`, `size`.
    /// Empty applies to the whole meter.
    #[schema(value_type = Object)]
    pub r#match: serde_json::Value,
    /// Fixed price per unit, in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price_usd: Option<f64>,
    /// Multiple of the provider's or catalog's price (0.8 is a 20%
    /// discount, 0 counts usage at no cost).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_price_multiplier: Option<f64>,
    /// Unit of `unit_price_usd`: `1m_tokens` or `hour`.
    pub unit: String,
    pub effective_from: DateTime<Utc>,
    /// This row ends the rates it matches from `effective_from`.
    pub retired: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

fn rate_public_id(id: uuid::Uuid) -> String {
    format!("rate_{}", id.simple())
}

fn parse_rate_id(raw: &str) -> Result<uuid::Uuid, CommandError> {
    raw.strip_prefix("rate_")
        .and_then(|hex| uuid::Uuid::parse_str(hex).ok())
        .ok_or_else(|| CommandError::bad_request("Invalid rate id"))
}

impl From<&CostRateRow> for CostRate {
    fn from(row: &CostRateRow) -> Self {
        CostRate {
            id: rate_public_id(row.id),
            meter: row.meter.clone(),
            r#match: row.match_.clone(),
            unit_price_usd: row.unit_price_usd,
            list_price_multiplier: row.list_price_multiplier,
            unit: row.unit.clone(),
            effective_from: row.effective_from,
            retired: row.retired,
            reason: row.reason.clone(),
            created_at: row.created_at,
        }
    }
}

// Decision: a rate's "why" is the change reason every mutating call already
// carries (`--reason`, the `Everruns-Change-Reason` header), not a param of its
// own; the price list shows it beside the row it explains.
fn change_reason(ctx: &Ctx) -> Option<String> {
    crate::domains::change_history::effective_intent(ctx).reason
}

fn require_manage(ctx: &Ctx) -> Result<(), CommandError> {
    BUDGET_MANAGE
        .evaluate(&ctx.caller)
        .map_err(|e| CommandError::forbidden(e.message))
}

/// List the org's price list, history included, newest first.
#[derive(Debug, Default, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListCostRates {}

#[command(
    name = "list_cost_rates",
    category = "budgets",
    description = "List the organization's price list, history included.",
    method = "GET",
    path = "/v1/cost-rates",
    policy = BUDGET_VIEW,
    http = plain,
    params(ListCostRates),
    cli = CliRoute::new(&["costs", "rates"], "list").with_examples(&[CliExample::new("See which rates price your agents' usage", "everruns costs rates list")]),
)]
impl Command for ListCostRates {
    type Output = Vec<CostRate>;
    async fn execute(self, ctx: &Ctx) -> Result<Vec<CostRate>, CommandError> {
        let rows = ctx.db.list_cost_rates(ctx.org_id()).await?;
        Ok(rows.iter().map(CostRate::from).collect())
    }
}

/// Add a rate to the org's price list. Rates are never edited: a change is
/// a new rate from a later date.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateCostRate {
    /// `model.tokens` or `sandbox.time`.
    pub meter: String,
    /// Which usage it applies to (see `CostRate.match`).
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub r#match: Option<serde_json::Value>,
    /// Fixed USD per million tokens (model tokens) or per hour (sandboxes).
    #[serde(default)]
    pub unit_price_usd: Option<f64>,
    /// Multiple of the provider's or catalog price. Model tokens only.
    #[serde(default)]
    pub list_price_multiplier: Option<f64>,
    /// When it starts; now when absent.
    #[serde(default)]
    pub effective_from: Option<DateTime<Utc>>,
}

const MATCH_KEYS_MODEL: &[&str] = &["provider", "model", "source"];
const MATCH_KEYS_SANDBOX: &[&str] = &["provider", "size"];

fn validate_match(meter: &str, value: &serde_json::Value) -> Result<(), CommandError> {
    let allowed = if meter == METER_MODEL_TOKENS {
        MATCH_KEYS_MODEL
    } else {
        MATCH_KEYS_SANDBOX
    };
    let Some(map) = value.as_object() else {
        return Err(CommandError::bad_request("match must be an object"));
    };
    for (key, v) in map {
        if !allowed.contains(&key.as_str()) {
            return Err(CommandError::bad_request(format!(
                "match key \"{key}\" does not apply to {meter} (allowed: {})",
                allowed.join(", ")
            )));
        }
        match v.as_str() {
            Some(s) if !s.trim().is_empty() && s.len() <= 200 => {}
            _ => {
                return Err(CommandError::bad_request(
                    "match values must be non-empty text",
                ));
            }
        }
        if key == "source" && !matches!(v.as_str(), Some("managed" | "own_keys")) {
            return Err(CommandError::bad_request(
                "source must be \"managed\" or \"own_keys\"",
            ));
        }
    }
    Ok(())
}

#[command(
    name = "create_cost_rate",
    category = "budgets",
    description = "Add a rate to the organization's price list. A rate is a fixed price per unit or a multiple of the list price; rates are never edited, a change is a new rate from a later date.",
    method = "POST",
    path = "/v1/cost-rates",
    policy = BUDGET_MANAGE,
    http = created,
    request_body(CreateCostRate),
    cli = CliRoute::new(&["costs", "rates"], "create").with_examples(&[CliExample::new("Record a negotiated 20% discount on OpenAI", "everruns costs rates create --meter model.tokens --match '{\"provider\":\"openai\"}' --list-price-multiplier 0.8 --reason 'Enterprise discount'")]),
)]
impl Command for CreateCostRate {
    type Output = CostRate;
    async fn execute(self, ctx: &Ctx) -> Result<CostRate, CommandError> {
        require_manage(ctx)?;
        let unit = match self.meter.as_str() {
            METER_MODEL_TOKENS => "1m_tokens",
            METER_SANDBOX_TIME => "hour",
            _ => {
                return Err(CommandError::bad_request(
                    "meter must be \"model.tokens\" or \"sandbox.time\"",
                ));
            }
        };
        let match_ = self.r#match.unwrap_or_else(|| serde_json::json!({}));
        validate_match(&self.meter, &match_)?;
        match (self.unit_price_usd, self.list_price_multiplier) {
            (Some(price), None) if price >= 0.0 && price.is_finite() => {}
            (None, Some(m)) if m >= 0.0 && m.is_finite() && self.meter == METER_MODEL_TOKENS => {}
            (None, Some(0.0)) => {}
            _ => {
                return Err(CommandError::bad_request(
                    "Set exactly one of unit_price_usd or list_price_multiplier, not negative; sandbox rates take a unit price",
                ));
            }
        }
        let row = ctx
            .db
            .create_cost_rate(CreateCostRateRow {
                org_id: ctx.org_id(),
                meter: self.meter,
                match_,
                unit_price_usd: self.unit_price_usd,
                list_price_multiplier: self.list_price_multiplier,
                unit: unit.into(),
                effective_from: self.effective_from.unwrap_or_else(Utc::now),
                retired: false,
                reason: change_reason(ctx),
                created_by: ctx.caller.user_id,
            })
            .await?;
        Ok(CostRate::from(&row))
    }
}

/// End a rate: adds a row that retires its match from a date.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RetireCostRate {
    /// Rate id (`rate_...`) whose match to retire.
    pub rate_id: String,
    /// When it ends; now when absent.
    #[serde(default)]
    pub effective_from: Option<DateTime<Utc>>,
}

#[command(
    name = "retire_cost_rate",
    category = "budgets",
    description = "End a rate from a date. Past costs keep the rate they used; usage after the date falls back to the provider's or catalog price.",
    method = "POST",
    path = "/v1/cost-rates/{rate_id}/retire",
    policy = BUDGET_MANAGE,
    http = created,
    responses((status = 404, description = "Rate not found")),
    cli = CliRoute::new(&["costs", "rates"], "retire").with_examples(&[CliExample::new("Stop applying a discount that expired", "everruns costs rates retire --rate-id rate_01h9 --reason 'Contract ended'")]),
)]
impl Command for RetireCostRate {
    type Output = CostRate;
    async fn execute(self, ctx: &Ctx) -> Result<CostRate, CommandError> {
        require_manage(ctx)?;
        let id = parse_rate_id(&self.rate_id)?;
        let existing = ctx
            .db
            .get_cost_rate(ctx.org_id(), id)
            .await?
            .ok_or_else(|| CommandError::not_found("Rate"))?;
        let row = ctx
            .db
            .create_cost_rate(CreateCostRateRow {
                org_id: ctx.org_id(),
                meter: existing.meter,
                match_: existing.match_,
                unit_price_usd: None,
                list_price_multiplier: None,
                unit: existing.unit,
                effective_from: self.effective_from.unwrap_or_else(Utc::now),
                retired: true,
                reason: change_reason(ctx),
                created_by: ctx.caller.user_id,
            })
            .await?;
        Ok(CostRate::from(&row))
    }
}

/// One line of the costs summary.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CostSummaryLine {
    /// Group key: an agent id, a user id, a source key, a basis, a day.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Where the amounts came from: host, rate, provider, list_price,
    /// unpriced, or mixed.
    pub basis: String,
    /// Priced cost in USD. Unpriced usage is not in it.
    pub cost_usd: f64,
    /// Usage records with no price.
    pub unpriced_records: i64,
    pub records: i64,
    /// Tokens for models, hours for sandboxes, summed as recorded.
    pub quantity: f64,
}

impl From<CostSummaryRow> for CostSummaryLine {
    fn from(row: CostSummaryRow) -> Self {
        CostSummaryLine {
            key: row.key,
            label: row.label,
            basis: row.basis.unwrap_or_else(|| "unpriced".into()),
            cost_usd: row.amount_usd,
            unpriced_records: row.unpriced,
            records: row.records,
            quantity: row.quantity,
        }
    }
}

/// Org cost over a window, grouped several ways.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CostSummary {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    /// The org's timezone; calendar windows and days use it.
    pub timezone: String,
    pub cost_usd: f64,
    pub unpriced_records: i64,
    pub by_basis: Vec<CostSummaryLine>,
    pub by_source: Vec<CostSummaryLine>,
    pub by_agent: Vec<CostSummaryLine>,
    pub by_user: Vec<CostSummaryLine>,
    pub by_started_by: Vec<CostSummaryLine>,
    pub by_day: Vec<CostSummaryLine>,
}

/// Org cost over a window. Defaults to this month in the org's timezone.
#[derive(Debug, Default, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct GetCostSummary {
    /// Window start (inclusive). Default: the start of this month.
    pub from: Option<DateTime<Utc>>,
    /// Window end (exclusive). Default: now.
    pub to: Option<DateTime<Utc>>,
    /// Only this agent (`agent_...`).
    pub agent_id: Option<String>,
}

/// Start of the calendar month containing `now`, in `tz`.
pub(crate) fn month_start(now: DateTime<Utc>, tz: chrono_tz::Tz) -> DateTime<Utc> {
    let local = now.with_timezone(&tz);
    tz.with_ymd_and_hms(local.year(), local.month(), 1, 0, 0, 0)
        .earliest()
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(now)
}

#[command(
    name = "get_cost_summary",
    category = "budgets",
    description = "Summarize what usage cost the organization.",
    method = "GET",
    path = "/v1/costs/summary",
    policy = BUDGET_VIEW,
    http = plain,
    params(GetCostSummary),
    cli = CliRoute::new(&["costs", "summary"], "get").with_examples(&[CliExample::new("See this month's cost by agent and source", "everruns costs summary get")]),
)]
impl Command for GetCostSummary {
    type Output = CostSummary;
    async fn execute(self, ctx: &Ctx) -> Result<CostSummary, CommandError> {
        let tz = super::BudgetService::new(ctx.db.clone())
            .org_tz(ctx.org_id())
            .await;
        let now = Utc::now();
        let to = self.to.unwrap_or(now);
        let from = self.from.unwrap_or_else(|| month_start(now, tz));
        if from >= to {
            return Err(CommandError::bad_request("from must be before to"));
        }
        if self.agent_id.is_some() {
            return Err(CommandError::bad_request(
                "agent_id filtering is served by GET /v1/agents/{agent_id}/costs",
            ));
        }
        let tz_name = tz.name().to_string();
        let mut groups = Vec::new();
        for dimension in ["basis", "source", "agent", "user", "started_by", "day"] {
            let rows = ctx
                .db
                .cost_summary(ctx.org_id(), from, to, dimension, &tz_name)
                .await?;
            groups.push(
                rows.into_iter()
                    .map(CostSummaryLine::from)
                    .collect::<Vec<_>>(),
            );
        }
        let by_day = groups.pop().unwrap_or_default();
        let by_started_by = groups.pop().unwrap_or_default();
        let by_user = groups.pop().unwrap_or_default();
        let by_agent = groups.pop().unwrap_or_default();
        let by_source = groups.pop().unwrap_or_default();
        let by_basis = groups.pop().unwrap_or_default();
        Ok(CostSummary {
            from,
            to,
            timezone: tz_name,
            cost_usd: by_basis.iter().map(|l| l.cost_usd).sum(),
            unpriced_records: by_basis.iter().map(|l| l.unpriced_records).sum(),
            by_basis,
            by_source,
            by_agent,
            by_user,
            by_started_by,
            by_day,
        })
    }
}

/// An agent's cost this month, for the agent page.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentCost {
    pub agent_id: String,
    pub from: DateTime<Utc>,
    pub cost_usd: f64,
    pub unpriced_records: i64,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetAgentCost {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
}

#[command(
    name = "get_agent_cost",
    category = "budgets",
    description = "What one agent cost the organization this month.",
    method = "GET",
    path = "/v1/agents/{agent_id}/costs",
    policy = BUDGET_VIEW,
    positional = "agent_id",
    http = plain,
    responses((status = 404, description = "Agent not found")),
    cli = CliRoute::new(&["agents", "costs"], "get").with_args(&[CliArg::new("agent_id").at(1)]).with_examples(&[CliExample::new("See what an agent cost this month", "everruns agents costs get agent_01h9")]),
)]
impl Command for GetAgentCost {
    type Output = AgentCost;
    async fn execute(self, ctx: &Ctx) -> Result<AgentCost, CommandError> {
        let agent = ctx
            .db
            .get_agent_by_public_id(ctx.org_id(), &self.agent_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Agent"))?;
        let tz = super::BudgetService::new(ctx.db.clone())
            .org_tz(ctx.org_id())
            .await;
        let from = month_start(Utc::now(), tz);
        let (cost_usd, unpriced_records) = ctx
            .db
            .agent_cost_since(ctx.org_id(), agent.id.uuid(), from)
            .await?;
        Ok(AgentCost {
            agent_id: self.agent_id,
            from,
            cost_usd,
            unpriced_records,
        })
    }
}

/// The org's cost settings.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CostSettings {
    /// IANA timezone (e.g. `America/Chicago`) for calendar windows: daily,
    /// weekly and monthly usage policies reset at midnight there. `null` is
    /// UTC.
    pub timezone: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct GetCostSettings {}

#[command(
    name = "get_cost_settings",
    category = "budgets",
    description = "Read the organization's cost settings.",
    method = "GET",
    path = "/v1/costs/settings",
    policy = BUDGET_VIEW,
    http = plain,
    params(GetCostSettings),
    cli = CliRoute::new(&["costs", "settings"], "get").with_examples(&[CliExample::new("See which timezone daily and monthly limits reset in", "everruns costs settings get")]),
)]
impl Command for GetCostSettings {
    type Output = CostSettings;
    async fn execute(self, ctx: &Ctx) -> Result<CostSettings, CommandError> {
        Ok(CostSettings {
            timezone: ctx.db.org_timezone(ctx.org_id()).await?,
        })
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateCostSettings {
    /// IANA timezone, or null for UTC.
    pub timezone: Option<String>,
}

#[command(
    name = "update_cost_settings",
    category = "budgets",
    description = "Set the timezone usage policy windows reset in. Windows already running keep their start; the next boundary uses the new timezone.",
    method = "PATCH",
    path = "/v1/costs/settings",
    policy = BUDGET_MANAGE,
    http = plain,
    request_body(UpdateCostSettings),
    cli = CliRoute::new(&["costs", "settings"], "update").with_examples(&[CliExample::new("Reset monthly limits at midnight in Austin", "everruns costs settings update --timezone America/Chicago --reason 'Team is in Austin'")]),
)]
impl Command for UpdateCostSettings {
    type Output = CostSettings;
    async fn execute(self, ctx: &Ctx) -> Result<CostSettings, CommandError> {
        require_manage(ctx)?;
        let timezone = match self.timezone.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(name) => {
                let tz: chrono_tz::Tz = name
                    .parse()
                    .map_err(|_| CommandError::bad_request("Unknown timezone"))?;
                Some(tz.name().to_string())
            }
        };
        ctx.db
            .set_org_timezone(ctx.org_id(), timezone.as_deref())
            .await?;
        Ok(CostSettings { timezone })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_start_follows_the_org_timezone() {
        let now = Utc.with_ymd_and_hms(2026, 11, 1, 3, 0, 0).unwrap();
        // 03:00 UTC on Nov 1 is still Oct 31 in Austin.
        let chicago: chrono_tz::Tz = "America/Chicago".parse().unwrap();
        assert_eq!(
            month_start(now, chicago),
            Utc.with_ymd_and_hms(2026, 10, 1, 5, 0, 0).unwrap()
        );
        assert_eq!(
            month_start(now, chrono_tz::Tz::UTC),
            Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap()
        );
    }

    #[test]
    fn rate_match_keys_are_checked_per_meter() {
        assert!(
            validate_match(
                METER_MODEL_TOKENS,
                &serde_json::json!({"provider": "openai"})
            )
            .is_ok()
        );
        assert!(validate_match(METER_MODEL_TOKENS, &serde_json::json!({"size": "small"})).is_err());
        assert!(validate_match(METER_SANDBOX_TIME, &serde_json::json!({"size": "small"})).is_ok());
        assert!(
            validate_match(
                METER_MODEL_TOKENS,
                &serde_json::json!({"source": "elsewhere"})
            )
            .is_err()
        );
    }
}
