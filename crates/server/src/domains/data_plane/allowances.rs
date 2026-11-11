//! `POST /dp/v1/tenants/{org}/allowances`: the grant ledger
//! (cp-dp-protocol.md, "Allowances").
//!
//! Decisions:
//! - What may be granted is what the org's usage policies still allow
//!   before their first stopping step (`stop`, or `ask`: a data plane turn
//!   has no one to ask), the tightest policy winning. An org with no such
//!   policy is not capped here.
//! - Money is reserved when granted and released only by a closed
//!   settlement, never by time; the arithmetic is in `dp_allowance`.
//! - A suspended org is granted nothing (`suspended`).
//! - Grants are short (a minute to spend), so a data plane cut off from the
//!   control plane runs out within a minute of its last grant.

use std::time::Duration;

use everruns_contracts::dataplane::{AllowanceGrant, AllowanceRequest};
use everruns_core::budget::PolicyStepAction;

use super::ControlPlane;
use crate::storage::{DpAllowanceAsk, DpSettlement};

/// How long a grant may be spent.
pub const SPEND_FOR: Duration = Duration::from_secs(60);

/// What `org_id`'s usage policies still allow, in micros (`None`: no cap).
pub async fn cap_micros(cp: &ControlPlane, org_id: i64) -> anyhow::Result<Option<i64>> {
    let public = everruns_core::org_public_id_from_internal(org_id);
    let budgets = cp
        .db
        .get_active_budgets_for_subjects(
            org_id,
            crate::storage::repositories::BudgetSubjectLookup {
                session_id: None,
                agent_id: None,
                user_id: None,
                org_public_id: Some(&public),
                trigger_id: None,
                channel_id: None,
                schedule_id: None,
                org_unattended_id: None,
                eval_run_id: None,
            },
        )
        .await?;
    let mut cap: Option<i64> = None;
    for budget in budgets.iter().filter(|b| b.currency == "usd") {
        let policy = budget.usage_policy();
        let Some(stop) = policy
            .effective_steps(budget.limit, budget.soft_limit)
            .into_iter()
            .find(|step| matches!(step.action, PolicyStepAction::Stop | PolicyStepAction::Ask))
        else {
            continue;
        };
        let spent = budget.limit - budget.balance;
        let left = budget.limit * stop.at_percent / 100.0 - spent;
        let micros = (left.max(0.0) * 1_000_000.0).floor() as i64;
        cap = Some(cap.map_or(micros, |c| c.min(micros)));
    }
    Ok(cap)
}

/// Answer `request` for `org_id`, once.
pub async fn answer(
    cp: &ControlPlane,
    org_id: i64,
    request: &AllowanceRequest,
) -> anyhow::Result<AllowanceGrant> {
    let org = cp
        .db
        .dp_org(org_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("org {org_id} is not on a data plane"))?;
    let cap = cap_micros(cp, org_id).await?;
    let answer = cp
        .db
        .dp_allowance(DpAllowanceAsk {
            org_id,
            request_id: &request.request_id,
            cell: &request.cell,
            want_micros: request.want_micros.max(0),
            settlements: request
                .settlements
                .iter()
                .map(|s| DpSettlement {
                    grant_id: &s.grant_id,
                    used_micros: s.used_micros,
                    reserved_micros: s.reserved_micros,
                    closed: s.closed,
                })
                .collect(),
            cap_micros: cap,
            refuse: (org.state == "suspended").then_some("suspended"),
            spend_until: chrono::Utc::now() + SPEND_FOR,
            per_call_micros: None,
        })
        .await?;
    Ok(serde_json::from_value(answer)?)
}
