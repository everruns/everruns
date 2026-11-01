// GetSessionCost: the read behind a session's Cost tab.
// See knowledge/ui/session-cost.md.
//
// Decision: a model call's cost is split across categories (uncached prompt,
// cache read, cache write, output) by the model's price table, then scaled so
// the parts add up to the call's recorded cost (provider-reported where there
// is one). `llm_generations` stores only the total, and the price table is the
// only per-bucket rate there is; scaling keeps every total on the page equal to
// what the session was billed. A call with a reported cost but no price table
// keeps its cost in `unattributed_usd` instead of guessing a split.
//
// Decision: hours are UTC and the UI rolls them up into days in the viewer's
// timezone, so the server needs no timezone input and one response serves
// every viewer.

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

use chrono::{DateTime, DurationRound, TimeDelta, Utc};
use everruns_contracts::model_profiles::{CostBreakdown, estimate_cost_breakdown_usd};
use everruns_contracts::provider::DriverId;
use everruns_contracts::typed_id::{SandboxId, SessionId, TurnId};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::types::*;
use crate::domains::common::*;
use crate::domains::sandboxes::{running_seconds, spans_by_sandbox};
use crate::domains::sessions::SESSION_VIEW;
use crate::storage::backend::session_cost::{
    CostGenerationRow, CostScheduleRow, CostToolRow, CostTurnOriginRow, CostTurnRow,
};

/// Turns returned at most; a longer session returns its newest turns.
pub const MAX_TURNS: usize = 5000;

/// Category keys, in display order.
pub const UNCACHED_INPUT: &str = "uncached_input";
pub const CACHE_READ: &str = "cache_read";
pub const CACHE_WRITE: &str = "cache_write";
pub const OUTPUT: &str = "output";

/// A turn whose input message names no schedule, trigger or channel.
pub const USER_SOURCE: &str = "user";

#[derive(Debug, Deserialize, ToSchema, IntoParams, Serialize)]
#[into_params(parameter_in = Query)]
pub struct GetSessionCost {
    /// Session's prefixed public identifier (a path parameter).
    #[param(ignore)]
    pub session_id: String,
}

#[command(
    name = "get_session_cost",
    category = "sessions",
    description = "Show what a session spent: totals by cost category (uncached prompt, cache read, cache write, output), spend per hour and per turn, what started the turns, tool calls, and sandbox running time.",
    summary = "Show what a session spent",
    method = "GET",
    path = "/v1/sessions/{session_id}/cost",
    policy = SESSION_VIEW,
    cli = CliRoute::new(&["sessions", "cost"], "get").with_args(&[CliArg::new("session_id").at(1)]).with_examples(&[CliExample::new("See where a session's spend went", "everruns sessions cost get session_01h9")]),
    positional = "session_id",
    http = plain,
    params(GetSessionCost),
)]
impl Command for GetSessionCost {
    type Output = SessionCost;

    async fn execute(self, ctx: &Ctx) -> Result<SessionCost, CommandError> {
        let session_id: SessionId = self
            .session_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid session ID: {e}")))?;
        if ctx
            .db
            .get_session(ctx.org_id(), session_id)
            .await?
            .is_none()
        {
            return Err(CommandError::not_found("Session"));
        }
        let id = session_id.uuid();
        let org = ctx.org_id();
        // Turns and tool calls come from the trace index; bring it up to date.
        ctx.db.database().catch_up_session_trace_fully(id).await?;

        let generations = ctx.db.session_cost_generations(org, id).await?;
        let turns = ctx.db.session_cost_turns(id).await?;
        let origins = ctx.db.session_cost_turn_origins(id).await?;
        let schedules = ctx.db.session_cost_schedules(org, id).await?;
        let tools = ctx.db.session_cost_tools(id).await?;
        let sandboxes = ctx.db.session_cost_sandboxes(org, id).await?;
        let mut spans = spans_by_sandbox(ctx.db.session_cost_sandbox_transitions(org, id).await?);

        let mut cost = aggregate(
            session_id.to_string(),
            &generations,
            &turns,
            &origins,
            &schedules,
            tools,
            &|provider, model, usage| price_split(provider, model, usage),
        );
        cost.sandboxes = sandboxes
            .into_iter()
            .map(|row| {
                let spans = spans.remove(&row.id).unwrap_or_default();
                let paused_seconds = spans
                    .iter()
                    .filter(|span| span.state == "paused")
                    .map(|span| (span.end - span.start).num_seconds())
                    .sum();
                CostSandbox {
                    sandbox_id: SandboxId::from_uuid(row.id).to_string(),
                    provider: row.provider,
                    profile_name: row.profile_name,
                    created_at: row.created_at,
                    running_seconds: running_seconds(&spans),
                    paused_seconds,
                    spans,
                }
            })
            .collect();
        Ok(cost)
    }
}

/// Prices one call's tokens by (provider, model); `None` when the model has no
/// price table. Injected so `aggregate` stays a pure function for tests.
pub type PriceFn = dyn Fn(Option<&str>, Option<&str>, CallTokens) -> Option<CostBreakdown>;

/// Token counts of one model call, in the disjoint-bucket convention.
#[derive(Debug, Clone, Copy)]
pub struct CallTokens {
    pub uncached_input: u32,
    pub output: u32,
    pub cache_read: u32,
    pub cache_write: u32,
}

/// Price-table split of a call, by the provider type it was served through.
fn price_split(
    provider: Option<&str>,
    model: Option<&str>,
    t: CallTokens,
) -> Option<CostBreakdown> {
    let provider = DriverId::from_str(&provider?.to_ascii_lowercase()).ok()?;
    estimate_cost_breakdown_usd(
        &provider,
        model?,
        t.uncached_input,
        t.output,
        t.cache_read,
        t.cache_write,
    )
}

fn clamp_u32(tokens: i64) -> u32 {
    u32::try_from(tokens.max(0)).unwrap_or(u32::MAX)
}

/// The recorded cost of a call split into categories that add up to it.
/// Returns the split (all zero when unknown) and the unattributed remainder.
fn split_call(recorded: f64, priced: Option<CostBreakdown>) -> (CostBreakdown, f64) {
    match priced {
        Some(parts) if parts.total() > 0.0 => {
            let scale = recorded / parts.total();
            (
                CostBreakdown {
                    uncached_input: parts.uncached_input * scale,
                    cache_read: parts.cache_read * scale,
                    cache_write: parts.cache_write * scale,
                    output: parts.output * scale,
                },
                0.0,
            )
        }
        _ => (CostBreakdown::default(), recorded),
    }
}

/// Build the response from the rows. Pure, so it is tested without a database;
/// `price` is the price-table lookup.
pub fn aggregate(
    session_id: String,
    generations: &[CostGenerationRow],
    turns: &[CostTurnRow],
    origins: &[CostTurnOriginRow],
    schedules: &[CostScheduleRow],
    tools: Vec<CostToolRow>,
    price: &PriceFn,
) -> SessionCost {
    let mut total = CostBreakdown::default();
    let mut tokens = [0_i64; 4];
    let mut total_usd = 0.0;
    let mut unattributed_usd = 0.0;
    let mut unpriced_calls = 0;
    let mut models: HashMap<(String, Option<String>), (i64, f64)> = HashMap::new();
    let mut hours: BTreeMap<DateTime<Utc>, CostHour> = BTreeMap::new();
    let mut per_turn: HashMap<Uuid, (f64, i64)> = HashMap::new();

    for row in generations {
        let call = CallTokens {
            uncached_input: clamp_u32(row.input_tokens),
            output: clamp_u32(row.output_tokens),
            cache_read: clamp_u32(row.cache_read_tokens),
            cache_write: clamp_u32(row.cache_creation_tokens),
        };
        let priced = price(row.provider.as_deref(), row.model.as_deref(), call);
        let recorded = row
            .actual_cost_usd
            .or(row.estimated_cost_usd)
            .or_else(|| priced.map(|p| p.total()));
        if recorded.is_none() {
            unpriced_calls += 1;
        }
        let recorded = recorded.unwrap_or(0.0).max(0.0);
        let (parts, rest) = split_call(recorded, priced);

        total_usd += recorded;
        unattributed_usd += rest;
        total.uncached_input += parts.uncached_input;
        total.cache_read += parts.cache_read;
        total.cache_write += parts.cache_write;
        total.output += parts.output;
        tokens[0] += row.input_tokens.max(0);
        tokens[1] += row.cache_read_tokens.max(0);
        tokens[2] += row.cache_creation_tokens.max(0);
        tokens[3] += row.output_tokens.max(0);

        let model = models
            .entry((
                row.model.clone().unwrap_or_else(|| "unknown".to_string()),
                row.provider.clone(),
            ))
            .or_default();
        model.0 += 1;
        model.1 += recorded;

        let hour = row
            .created_at
            .duration_trunc(TimeDelta::hours(1))
            .unwrap_or(row.created_at);
        let bucket = hours.entry(hour).or_insert_with(|| CostHour {
            hour,
            ..CostHour::default()
        });
        bucket.uncached_input_usd += parts.uncached_input;
        bucket.cache_read_usd += parts.cache_read;
        bucket.cache_write_usd += parts.cache_write;
        bucket.output_usd += parts.output;
        bucket.unattributed_usd += rest;
        bucket.model_calls += 1;

        if let Some(turn_id) = row.turn_id {
            let turn = per_turn.entry(turn_id).or_default();
            turn.0 += recorded;
            turn.1 += row.input_tokens.max(0)
                + row.cache_read_tokens.max(0)
                + row.cache_creation_tokens.max(0);
        }
    }

    let categories = [
        (UNCACHED_INPUT, tokens[0], total.uncached_input),
        (CACHE_READ, tokens[1], total.cache_read),
        (CACHE_WRITE, tokens[2], total.cache_write),
        (OUTPUT, tokens[3], total.output),
    ]
    .into_iter()
    .map(|(key, tokens, usd)| CostCategory {
        key: key.to_string(),
        tokens,
        usd,
    })
    .collect();

    let mut models: Vec<CostModel> = models
        .into_iter()
        .map(|((model, provider), (calls, usd))| CostModel {
            model,
            provider,
            calls,
            usd,
        })
        .collect();
    models.sort_by(|a, b| b.usd.total_cmp(&a.usd).then_with(|| a.model.cmp(&b.model)));

    // What started each turn, keyed by the turn's uuid.
    let origin_by_turn: HashMap<Uuid, &CostTurnOriginRow> = origins
        .iter()
        .filter_map(|o| Some((TurnId::from_str(&o.turn_id).ok()?.uuid(), o)))
        .collect();
    let schedule_by_id: HashMap<&str, &CostScheduleRow> = schedules
        .iter()
        .map(|s| (s.public_id.as_str(), s))
        .collect();

    let mut sources: HashMap<(String, Option<String>), (i64, f64)> = HashMap::new();
    let mut cost_turns = Vec::with_capacity(turns.len());
    for turn in turns {
        let origin = origin_by_turn.get(&turn.turn_id);
        let kind = origin
            .and_then(|o| o.kind.clone())
            .unwrap_or_else(|| USER_SOURCE.to_string());
        let source_id = origin.and_then(|o| o.source_id.clone());
        let (usd, prompt_tokens) = per_turn.get(&turn.turn_id).copied().unwrap_or_default();
        let source = sources.entry((kind.clone(), source_id)).or_default();
        source.0 += 1;
        source.1 += usd;
        cost_turns.push(CostTurn {
            turn: turn.turn_no,
            turn_id: TurnId::from_uuid(turn.turn_id).to_string(),
            started_at: turn.started_at,
            status: turn.status.clone(),
            usd,
            prompt_tokens,
            source_kind: kind,
        });
    }
    let turns_truncated = cost_turns.len() > MAX_TURNS;
    if turns_truncated {
        cost_turns.drain(..cost_turns.len() - MAX_TURNS);
    }

    let mut sources: Vec<CostSource> = sources
        .into_iter()
        .map(|((kind, source_id), (turns, usd))| {
            let schedule = source_id
                .as_deref()
                .and_then(|id| schedule_by_id.get(id).copied());
            CostSource {
                kind,
                name: schedule.map(|s| s.description.clone()),
                cron: schedule.and_then(|s| s.cron_expression.clone()),
                active: schedule.map(|s| s.enabled && !s.deleted),
                source_id,
                turns,
                usd,
            }
        })
        .collect();
    sources.sort_by(|a, b| {
        b.usd
            .total_cmp(&a.usd)
            .then_with(|| b.turns.cmp(&a.turns))
            .then_with(|| a.kind.cmp(&b.kind))
    });

    let tools = CostTools {
        calls: tools.iter().map(|t| t.calls).sum(),
        failed: tools.iter().map(|t| t.failed).sum(),
        duration_ms: tools.iter().map(|t| t.duration_ms).sum(),
        items: tools
            .into_iter()
            .map(|t| CostTool {
                name: t.name,
                calls: t.calls,
                failed: t.failed,
                duration_ms: t.duration_ms,
            })
            .collect(),
    };

    SessionCost {
        session_id,
        total_usd,
        model_calls: generations.len() as i64,
        unpriced_calls,
        categories,
        unattributed_usd,
        models,
        hours: hours.into_values().collect(),
        turns: cost_turns,
        turns_truncated,
        sources,
        tools,
        sandboxes: Vec::new(),
    }
}
