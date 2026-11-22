// Session spend: what a session spent, for the session itself
// (`get_session_info`). See knowledge/execution/session-spend.md.
//
// Decisions:
// - Internal command (`worker_get_session_spend`): a v1 worker and a shard
//   executor ask through their internal command channel (a host call on a
//   shard), as for knowledge and tokens. It reads only the session that asks
//   and the subagents it started, never the org's other spend.
// - Model calls are counted as the Cost tab counts them (provider-reported
//   cost, else the price-table estimate). A v1 session's come from
//   `llm_generations`; a v2 session's from its shard's log (`spend` read),
//   priced here the same way (`session_spend::summarize`).
// - Sandboxes are priced from their lifecycle spans: the org's rate for the
//   provider wins, else the provider's list price on the pinned profile's
//   resources. Stopped time adds only a provider's stopped-disk charge
//   (Daytona), and only at list price: an org rate is per running hour.
// - Subagents add their own whole spend, two levels deep and at most 100
//   per session, so a deep tree cannot make one read unbounded.

use std::collections::HashMap;

use chrono::Utc;
use everruns_contracts::runtime::session_spend::{
    SandboxSpend, SessionSpend, SpendBasis, SpendCall, SpendInputs, SubagentSpend, summarize,
};
use everruns_contracts::sandbox_prices::sandbox_resources;
use everruns_contracts::session_commands;
use everruns_contracts::typed_id::{SessionId, TurnId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domains::budgets::costs::{
    CostBasis, METER_SANDBOX_TIME, rate_sandbox, sandbox_list_usd,
};
use crate::domains::common::*;
use crate::domains::sandboxes::spans_by_sandbox;
use crate::domains::sessions::SESSION_VIEW;

/// Subagent levels counted below the asking session.
const MAX_DEPTH: usize = 2;
/// Subagents counted per session.
const MAX_CHILDREN: usize = 100;

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetSessionSpend {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// The turn that asks, for its share (`current_turn_usd`).
    #[serde(default)]
    pub turn_id: Option<String>,
}

#[command(
    name = "worker_get_session_spend",
    category = "sessions",
    description = "Internal: what the session spent so far (models, background calls, images, sandboxes, subagents), for the session itself.",
    method = "GET",
    path = "/internal/sessions/{session_id}/spend",
    policy = SESSION_VIEW
)]
impl Command for WorkerGetSessionSpend {
    type Output = SessionSpend;

    async fn execute(self, ctx: &Ctx) -> Result<SessionSpend, CommandError> {
        let session_id = crate::domains::user_connections::commands::worker::owned_session(
            ctx,
            &self.session_id,
        )
        .await?;
        let turn = match self.turn_id.as_deref() {
            Some(turn) => Some(
                turn.parse::<TurnId>()
                    .map_err(|e| CommandError::bad_request(format!("Invalid turn ID: {e}")))?
                    .uuid(),
            ),
            None => None,
        };
        session_spend(ctx, session_id, turn, 0).await
    }
}

/// `session_id`'s spend; the caller checked it is the org's.
pub(crate) fn session_spend<'a>(
    ctx: &'a Ctx,
    session_id: SessionId,
    turn: Option<Uuid>,
    depth: usize,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<SessionSpend, CommandError>> + Send + 'a>,
> {
    Box::pin(async move {
        let org = ctx.org_id();
        let inputs = spend_inputs(ctx, session_id).await?;
        let sandboxes = priced_sandboxes(ctx, session_id).await?;
        let mut subagents = SubagentSpend::default();
        if depth < MAX_DEPTH {
            for child in inputs.children.iter().take(MAX_CHILDREN) {
                let Ok(child) = child.parse::<SessionId>() else {
                    continue;
                };
                // A child of another org cannot be named here, but check.
                if ctx.db.get_session(org, child).await?.is_none() {
                    continue;
                }
                let spend = session_spend(ctx, child, None, depth + 1).await?;
                subagents.sessions += 1;
                subagents.usd += spend.total_usd;
            }
        }
        Ok(summarize(
            session_id.to_string(),
            &inputs,
            sandboxes,
            subagents,
            turn,
        ))
    })
}

/// Model calls, failed turns, image results and children: from the shard
/// for a v2 session, from the control plane's tables for v1.
async fn spend_inputs(ctx: &Ctx, session_id: SessionId) -> Result<SpendInputs, CommandError> {
    let org = ctx.org_id();
    let cp = crate::domains::data_plane::ControlPlane::from_env(ctx.db.clone());
    if let Some(plane) =
        crate::domains::data_plane::session_plane::SessionPlane::with(&cp, org, session_id)
            .await
            .map_err(CommandError::internal)?
    {
        let value = plane
            .read(&session_commands::SPEND)
            .await
            .map_err(CommandError::internal)?;
        return serde_json::from_value(value).map_err(|e| CommandError::internal(e.into()));
    }
    let id = session_id.uuid();
    ctx.db.database().catch_up_session_trace_fully(id).await?;
    let calls = ctx
        .db
        .session_cost_generations(org, id)
        .await?
        .into_iter()
        .map(|row| SpendCall {
            turn_id: row.turn_id,
            provider: row.provider,
            model: row.model,
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            cache_read_tokens: row.cache_read_tokens,
            cache_creation_tokens: row.cache_creation_tokens,
            actual_cost_usd: row.actual_cost_usd,
            estimated_cost_usd: row.estimated_cost_usd,
        })
        .collect();
    let turns = ctx
        .db
        .session_cost_turns(id)
        .await?
        .into_iter()
        .map(|turn| (turn.turn_id, turn.status))
        .collect();
    let image_results = ctx
        .db
        .session_image_tool_events(id)
        .await?
        .iter()
        .filter_map(everruns_contracts::runtime::session_spend::image_tool_result)
        .collect();
    let children = ctx
        .db
        .session_child_ids(org, id)
        .await?
        .into_iter()
        .map(|child| SessionId::from_uuid(child).to_string())
        .collect();
    Ok(SpendInputs {
        calls,
        turns,
        image_results,
        children,
    })
}

/// The session's sandboxes with their time and cost.
pub(crate) async fn priced_sandboxes(
    ctx: &Ctx,
    session_id: SessionId,
) -> Result<Vec<SandboxSpend>, CommandError> {
    let org = ctx.org_id();
    let id = session_id.uuid();
    let rows = ctx.db.session_cost_sandboxes(org, id).await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut spans = spans_by_sandbox(ctx.db.session_cost_sandbox_transitions(org, id).await?);
    let rates = ctx
        .db
        .effective_cost_rates(org, METER_SANDBOX_TIME, Utc::now())
        .await
        .unwrap_or_default();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let spans = spans.remove(&row.id).unwrap_or_default();
        let mut seconds: HashMap<&str, i64> = HashMap::new();
        for span in &spans {
            *seconds.entry(span.state.as_str()).or_default() +=
                (span.end - span.start).num_seconds();
        }
        let running = seconds.get("running").copied().unwrap_or(0);
        let stopped = seconds.get("paused").copied().unwrap_or(0);
        let options = row.options.clone().unwrap_or(serde_json::Value::Null);
        let size = options
            .get("size")
            .and_then(|s| s.as_str())
            .map(str::to_owned);
        let at = spans.last().map_or(row.created_at, |span| span.end);
        let list_running = sandbox_list_usd(&row.provider, &options, running as f64, 0.0, at);
        let list_stopped = sandbox_list_usd(&row.provider, &options, 0.0, stopped as f64, at);
        let rated = rate_sandbox(
            &row.provider,
            row.profile_name.as_deref(),
            running as f64,
            list_running,
            &rates,
        );
        let (usd, basis) = match rated.amount {
            Some(amount) => {
                let basis = match rated.basis {
                    CostBasis::Rate => SpendBasis::PriceList,
                    _ => SpendBasis::ListPrice,
                };
                // Stopped disk is a list-price charge; a running-hour org
                // rate does not cover it.
                let stopped_usd = match rated.basis {
                    CostBasis::Rate => 0.0,
                    _ => list_stopped.unwrap_or(0.0),
                };
                (Some(amount + stopped_usd), basis)
            }
            None => (None, SpendBasis::Unpriced),
        };
        out.push(SandboxSpend {
            provider: row.provider.clone(),
            size,
            resources: sandbox_resources(&row.provider, &options),
            running_seconds: running,
            stopped_seconds: stopped,
            usd,
            basis,
        });
    }
    Ok(out)
}
