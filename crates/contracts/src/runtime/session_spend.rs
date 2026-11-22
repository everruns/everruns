//! What a session spent, for the session itself to read (`get_session_info`)
//! and report.
//!
//! Decisions:
//! - One shape on every host. The numbers are those of the session's Cost
//!   tab (`GET /v1/sessions/{id}/cost`): each model call's provider-reported
//!   cost where there is one, else its price-table estimate, split into
//!   categories by [`split_call`]. The pure functions here build it from rows
//!   any host can produce (the control plane's tables, a cell's log), so a
//!   v1 session and a v2 session report alike.
//! - Only the session's own spend and its subagents'. Never org totals,
//!   other sessions, budget limits, credits, markup, the org's price list or
//!   which account paid: an agent may publish what it reads (a PR comment),
//!   so it is handed nothing it must not publish.
//! - Model calls outside a turn (automatic titles, dreaming) are
//!   `background`; calls in failed turns are `wasted`, a part of the total,
//!   not an addition to it.
//! - Each part says how it was priced ([`SpendBasis`]); the total takes the
//!   weakest, and `partial` says some usage carried no price at all.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::Result;
use crate::model_profiles::{CostBreakdown, estimate_cost_breakdown_usd, estimate_cost_usd};
use crate::provider::DriverId;
use crate::sandbox_prices::SandboxResources;
use crate::typed_id::{SessionId, TurnId};

/// Cost category keys, in display order.
pub const UNCACHED_INPUT: &str = "uncached_input";
pub const CACHE_READ: &str = "cache_read";
pub const CACHE_WRITE: &str = "cache_write";
pub const OUTPUT: &str = "output";

/// Tools whose results report image usage (`image_generation`).
pub const IMAGE_TOOLS: &[&str] = &["generate_image", "edit_image"];

/// How an amount was priced, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SpendBasis {
    /// The provider reported what it charged (OpenRouter per call; Modal's
    /// billing report).
    ProviderReported,
    /// The org's own price list.
    PriceList,
    /// The provider's published list price, times usage.
    ListPrice,
    /// Nothing priced it.
    Unpriced,
}

/// Spend and tokens of one cost category.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SpendCategory {
    /// `uncached_input`, `cache_read`, `cache_write` or `output`.
    #[cfg_attr(feature = "openapi", schema(example = "cache_read"))]
    pub key: String,
    #[cfg_attr(feature = "openapi", schema(example = 16_880_230))]
    pub tokens: i64,
    #[cfg_attr(feature = "openapi", schema(example = 0.0338))]
    pub usd: f64,
}

/// Spend of one model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SpendModel {
    #[cfg_attr(
        feature = "openapi",
        schema(example = "meta/muse-spark-1.3-contributor")
    )]
    pub model: String,
    #[cfg_attr(feature = "openapi", schema(example = 202))]
    pub calls: i64,
    #[cfg_attr(feature = "openapi", schema(example = 0.123))]
    pub usd: f64,
}

/// Model calls the agent made in its turns.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ModelSpend {
    #[cfg_attr(feature = "openapi", schema(example = 0.123))]
    pub usd: f64,
    #[cfg_attr(feature = "openapi", schema(example = 202))]
    pub calls: i64,
    /// Calls whose cost the provider reported.
    #[cfg_attr(feature = "openapi", schema(example = 202))]
    pub reported_calls: i64,
    /// Calls with no reported cost and no price table: they add nothing.
    #[cfg_attr(feature = "openapi", schema(example = 0))]
    pub unpriced_calls: i64,
    /// Spend by category, in a fixed order.
    pub categories: Vec<SpendCategory>,
    /// Spend whose split by category is unknown (a reported cost, no price
    /// table).
    #[cfg_attr(feature = "openapi", schema(example = 0.0))]
    pub unattributed_usd: f64,
    /// Spend per model, largest first.
    pub models: Vec<SpendModel>,
}

/// Model calls made for the session outside its turns.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BackgroundSpend {
    #[cfg_attr(feature = "openapi", schema(example = 0.002))]
    pub usd: f64,
    #[cfg_attr(feature = "openapi", schema(example = 3))]
    pub calls: i64,
}

/// Images the session generated or edited.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ImageSpend {
    #[cfg_attr(feature = "openapi", schema(example = 0.17))]
    pub usd: f64,
    /// Image tool calls that returned images.
    #[cfg_attr(feature = "openapi", schema(example = 2))]
    pub calls: i64,
    #[cfg_attr(feature = "openapi", schema(example = 2))]
    pub images: i64,
    /// Calls whose model has no price table or that reported no usage.
    #[cfg_attr(feature = "openapi", schema(example = 0))]
    pub unpriced_calls: i64,
}

/// One sandbox the session used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SandboxSpend {
    #[cfg_attr(feature = "openapi", schema(example = "daytona"))]
    pub provider: String,
    /// Size the profile named, when it named one.
    #[cfg_attr(feature = "openapi", schema(example = "small"))]
    pub size: Option<String>,
    /// Reserved resources the estimate used.
    pub resources: Option<SandboxResources>,
    #[cfg_attr(feature = "openapi", schema(example = 2914))]
    pub running_seconds: i64,
    /// Paused or stopped.
    #[cfg_attr(feature = "openapi", schema(example = 436_310))]
    pub stopped_seconds: i64,
    /// `None` when unpriced.
    #[cfg_attr(feature = "openapi", schema(example = 0.0565))]
    pub usd: Option<f64>,
    pub basis: SpendBasis,
}

/// Subagent sessions this session started.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SubagentSpend {
    #[cfg_attr(feature = "openapi", schema(example = 2))]
    pub sessions: i64,
    /// Their whole spend: models, images and sandboxes, theirs and their own
    /// subagents'.
    #[cfg_attr(feature = "openapi", schema(example = 0.04))]
    pub usd: f64,
}

/// What a session spent so far.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SessionSpend {
    #[cfg_attr(
        feature = "openapi",
        schema(example = "session_01a1e096dc55722795cb947ff66ddff9")
    )]
    pub session_id: String,
    /// Everything below, in USD.
    #[cfg_attr(feature = "openapi", schema(example = 0.1796))]
    pub total_usd: f64,
    /// The weakest basis among the priced parts.
    pub basis: SpendBasis,
    /// Some usage carried no price, so the total is low.
    pub partial: bool,
    pub models: ModelSpend,
    pub background: BackgroundSpend,
    pub images: ImageSpend,
    pub sandboxes: Vec<SandboxSpend>,
    pub subagents: SubagentSpend,
    /// Model spend of the turn that is asking, when a turn asks.
    #[cfg_attr(feature = "openapi", schema(example = 0.012))]
    pub current_turn_usd: Option<f64>,
    /// Model spend in turns that failed: part of the total.
    #[cfg_attr(feature = "openapi", schema(example = 0.0))]
    pub wasted_usd: f64,
    /// Spend this does not count: paid tools on the org's own keys (web
    /// search, browser), knowledge indexing, observers.
    pub not_included: Vec<String>,
}

/// The default `not_included` list.
pub fn not_included() -> Vec<String> {
    vec![
        "paid tool calls on the organization's own keys (web search, browser, page rendering)"
            .into(),
        "knowledge indexing".into(),
        "observers and evaluations".into(),
    ]
}

/// Reads a session's spend for the session itself.
#[async_trait]
pub trait SessionSpendReader: Send + Sync {
    /// `session`'s spend, with `turn`'s share when a turn asks; `None` when
    /// the host does not know the session.
    async fn session_spend(
        &self,
        session: SessionId,
        turn: Option<TurnId>,
    ) -> Result<Option<SessionSpend>>;
}

/// [`SessionSpendReader`] as a tool-context extension.
#[derive(Clone)]
pub struct SessionSpendReaderExt(pub std::sync::Arc<dyn SessionSpendReader>);

/// One model call, as a host records it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SpendCall {
    /// `None`: made outside a turn (background).
    pub turn_id: Option<Uuid>,
    pub provider: Option<String>,
    pub model: Option<String>,
    /// Prompt tokens not read from or written to cache.
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub actual_cost_usd: Option<f64>,
    pub estimated_cost_usd: Option<f64>,
}

/// What a host hands [`summarize`]: the session's model calls, turn outcomes
/// and image tool results; sandboxes and subagents already priced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpendInputs {
    pub calls: Vec<SpendCall>,
    /// `(turn, status)`; `failed` turns' calls are wasted.
    pub turns: Vec<(Uuid, String)>,
    /// Image tool results (`generate_image`, `edit_image`), as returned.
    pub image_results: Vec<Value>,
    /// Child sessions the session started, by public id.
    #[serde(default)]
    pub children: Vec<String>,
}

fn clamp_u32(tokens: i64) -> u32 {
    u32::try_from(tokens.max(0)).unwrap_or(u32::MAX)
}

/// Price-table split of one call, by the provider type it was served
/// through; `None` when the model has no price table.
pub fn price_call(
    provider: Option<&str>,
    model: Option<&str>,
    uncached_input: u32,
    output: u32,
    cache_read: u32,
    cache_write: u32,
) -> Option<CostBreakdown> {
    let provider = DriverId::from_str(&provider?.to_ascii_lowercase()).ok()?;
    estimate_cost_breakdown_usd(
        &provider,
        model?,
        uncached_input,
        output,
        cache_read,
        cache_write,
    )
}

/// The recorded cost of a call split into categories that add up to it, by
/// the price table's proportions. Returns the split (all zero when unknown)
/// and the unattributed remainder.
pub fn split_call(recorded: f64, priced: Option<CostBreakdown>) -> (CostBreakdown, f64) {
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

/// A call's cost (reported, else estimated, else the price table) and
/// whether the provider reported it; `None` when nothing prices it.
pub fn call_cost(call: &SpendCall, priced: Option<CostBreakdown>) -> Option<(f64, bool)> {
    match call.actual_cost_usd {
        Some(usd) => Some((usd.max(0.0), true)),
        None => call
            .estimated_cost_usd
            .or_else(|| priced.map(|p| p.total()))
            .map(|usd| (usd.max(0.0), false)),
    }
}

fn model_spend(calls: &[&SpendCall]) -> ModelSpend {
    let mut total = CostBreakdown::default();
    let mut tokens = [0_i64; 4];
    let mut spend = ModelSpend::default();
    let mut models: HashMap<String, (i64, f64)> = HashMap::new();
    for call in calls {
        let priced = price_call(
            call.provider.as_deref(),
            call.model.as_deref(),
            clamp_u32(call.input_tokens),
            clamp_u32(call.output_tokens),
            clamp_u32(call.cache_read_tokens),
            clamp_u32(call.cache_creation_tokens),
        );
        let (usd, reported) = match call_cost(call, priced) {
            Some(cost) => cost,
            None => {
                spend.unpriced_calls += 1;
                (0.0, false)
            }
        };
        if reported {
            spend.reported_calls += 1;
        }
        let (parts, rest) = split_call(usd, priced);
        spend.usd += usd;
        spend.unattributed_usd += rest;
        total.uncached_input += parts.uncached_input;
        total.cache_read += parts.cache_read;
        total.cache_write += parts.cache_write;
        total.output += parts.output;
        tokens[0] += call.input_tokens.max(0);
        tokens[1] += call.cache_read_tokens.max(0);
        tokens[2] += call.cache_creation_tokens.max(0);
        tokens[3] += call.output_tokens.max(0);
        let model = models
            .entry(call.model.clone().unwrap_or_else(|| "unknown".into()))
            .or_default();
        model.0 += 1;
        model.1 += usd;
    }
    spend.calls = calls.len() as i64;
    spend.categories = [
        (UNCACHED_INPUT, tokens[0], total.uncached_input),
        (CACHE_READ, tokens[1], total.cache_read),
        (CACHE_WRITE, tokens[2], total.cache_write),
        (OUTPUT, tokens[3], total.output),
    ]
    .into_iter()
    .map(|(key, tokens, usd)| SpendCategory {
        key: key.into(),
        tokens,
        usd,
    })
    .collect();
    spend.models = models
        .into_iter()
        .map(|(model, (calls, usd))| SpendModel { model, calls, usd })
        .collect();
    spend
        .models
        .sort_by(|a, b| b.usd.total_cmp(&a.usd).then_with(|| a.model.cmp(&b.model)));
    spend
}

/// List-price cost of one image tool result from the usage it reports;
/// `None` when it reports none or its model has no price table. The price
/// table has no image-input rate, so image prompt tokens are priced as text
/// input (a floor).
pub fn image_result_cost(result: &Value) -> Option<f64> {
    let provider = DriverId::from_str(&result["provider"].as_str()?.to_ascii_lowercase()).ok()?;
    let model = result["model"].as_str()?;
    let usage = result.get("usage").filter(|u| u.is_object())?;
    let tokens = |key: &str| {
        usage[key]
            .as_u64()
            .map_or(0, |t| t.min(u32::MAX as u64) as u32)
    };
    estimate_cost_usd(
        &provider,
        model,
        tokens("input_text_tokens").saturating_add(tokens("input_image_tokens")),
        tokens("output_tokens"),
        0,
        0,
    )
}

fn image_spend(results: &[Value]) -> ImageSpend {
    let mut spend = ImageSpend::default();
    for result in results {
        let images = result["count"].as_i64().unwrap_or(0);
        if images == 0 {
            continue;
        }
        spend.calls += 1;
        spend.images += images;
        match image_result_cost(result) {
            Some(usd) => spend.usd += usd,
            None => spend.unpriced_calls += 1,
        }
    }
    spend
}

/// Build a session's spend. `sandboxes` and `subagents` come priced; the
/// rest is priced here.
pub fn summarize(
    session_id: String,
    inputs: &SpendInputs,
    sandboxes: Vec<SandboxSpend>,
    subagents: SubagentSpend,
    current_turn: Option<Uuid>,
) -> SessionSpend {
    let (turn_calls, background_calls): (Vec<&SpendCall>, Vec<&SpendCall>) =
        inputs.calls.iter().partition(|c| c.turn_id.is_some());
    let models = model_spend(&turn_calls);
    let background = model_spend(&background_calls);
    let images = image_spend(&inputs.image_results);

    let failed: HashSet<Uuid> = inputs
        .turns
        .iter()
        .filter(|(_, status)| status == "failed")
        .map(|(turn, _)| *turn)
        .collect();
    let turn_usd = |pick: &dyn Fn(Uuid) -> bool| -> f64 {
        turn_calls
            .iter()
            .filter(|c| c.turn_id.is_some_and(pick))
            .map(|c| {
                let priced = price_call(
                    c.provider.as_deref(),
                    c.model.as_deref(),
                    clamp_u32(c.input_tokens),
                    clamp_u32(c.output_tokens),
                    clamp_u32(c.cache_read_tokens),
                    clamp_u32(c.cache_creation_tokens),
                );
                call_cost(c, priced).map_or(0.0, |(usd, _)| usd)
            })
            // An empty f64 sum is -0.0; report a plain 0.
            .sum::<f64>()
            + 0.0
    };
    let wasted_usd = turn_usd(&|t| failed.contains(&t));
    let current_turn_usd = current_turn.map(|current| turn_usd(&|t| t == current));

    let mut bases: Vec<SpendBasis> = Vec::new();
    let mut partial = false;
    for part in [&models, &background] {
        if part.calls > part.unpriced_calls {
            bases.push(if part.reported_calls == part.calls - part.unpriced_calls {
                SpendBasis::ProviderReported
            } else {
                SpendBasis::ListPrice
            });
        }
        partial |= part.unpriced_calls > 0;
    }
    if images.calls > images.unpriced_calls {
        bases.push(SpendBasis::ListPrice);
    }
    partial |= images.unpriced_calls > 0;
    for sandbox in &sandboxes {
        if sandbox.usd.is_some() {
            bases.push(sandbox.basis);
        } else if sandbox.running_seconds > 0 {
            partial = true;
        }
    }
    let total_usd = models.usd
        + background.usd
        + images.usd
        + sandboxes.iter().filter_map(|s| s.usd).sum::<f64>()
        + subagents.usd;
    SessionSpend {
        session_id,
        total_usd,
        basis: bases.into_iter().max().unwrap_or(SpendBasis::Unpriced),
        partial,
        models,
        background: BackgroundSpend {
            usd: background.usd,
            calls: background.calls,
        },
        images,
        sandboxes,
        subagents,
        current_turn_usd,
        wasted_usd,
        not_included: not_included(),
    }
}

/// One `llm.generation`'s call, as the control plane's usage tracking
/// records it in `llm_generations`; `None` when it reported no usage.
pub fn spend_call(
    turn: Option<Uuid>,
    metadata: &crate::runtime::events::LlmGenerationMetadata,
) -> Option<SpendCall> {
    let usage = metadata.usage.as_ref()?;
    Some(SpendCall {
        turn_id: turn,
        provider: metadata.provider.clone(),
        model: Some(metadata.model.clone()),
        input_tokens: i64::from(usage.input_tokens),
        output_tokens: i64::from(usage.output_tokens),
        cache_read_tokens: i64::from(usage.cache_read_tokens.unwrap_or(0)),
        cache_creation_tokens: i64::from(usage.cache_creation_tokens.unwrap_or(0)),
        actual_cost_usd: usage.actual_cost_usd,
        estimated_cost_usd: usage.estimated_cost_usd,
    })
}

/// Spend inputs from a session's committed events, for a host that keeps
/// the log itself (a cell, an embedded host). Children are left to the
/// caller.
pub fn spend_inputs(events: &[crate::runtime::events::Event]) -> SpendInputs {
    use crate::runtime::events::EventData;
    let mut inputs = SpendInputs::default();
    for event in events {
        let turn = event.context.turn_id.map(|t| t.uuid());
        match &event.data {
            EventData::LlmGeneration(data) => {
                inputs.calls.extend(spend_call(turn, &data.metadata));
            }
            EventData::TurnFailed(_) => {
                inputs.turns.extend(turn.map(|t| (t, "failed".to_owned())));
            }
            EventData::ToolCompleted(data) if IMAGE_TOOLS.contains(&data.tool_name.as_str()) => {
                if let Ok(value) = serde_json::to_value(data)
                    && let Some(result) = image_tool_result(&value)
                {
                    inputs.image_results.push(result);
                }
            }
            _ => {}
        }
    }
    inputs
}

/// The image tool result in a `tool.completed` event payload, if it is one:
/// the JSON object of its first text part that parses as one.
pub fn image_tool_result(data: &Value) -> Option<Value> {
    let name = data["tool_name"].as_str()?;
    if !IMAGE_TOOLS.contains(&name) || data["success"] == false {
        return None;
    }
    data["result"].as_array()?.iter().find_map(|part| {
        let text = part["text"].as_str()?;
        serde_json::from_str::<Value>(text)
            .ok()
            .filter(|v| v.is_object() && v.get("model").is_some())
    })
}

#[cfg(test)]
#[path = "session_spend_tests.rs"]
mod tests;
