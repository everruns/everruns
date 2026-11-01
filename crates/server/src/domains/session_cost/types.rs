// Response shape of a session's Cost tab. See knowledge/ui/session-cost.md.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domains::sandboxes::types::SandboxStateSpan;

/// What a session spent, split by cost category, over time, by what started
/// each turn, by tool, and by sandbox.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionCost {
    /// Session's prefixed public identifier.
    #[schema(example = "session_01a1510c99c8714eabb22a463e7c1fa1")]
    pub session_id: String,
    /// Total spend in USD: the provider-reported cost of each model call where
    /// reported, otherwise the price-table estimate.
    #[schema(example = 106.26)]
    pub total_usd: f64,
    /// Model calls recorded for the session.
    #[schema(example = 3811)]
    pub model_calls: i64,
    /// Model calls with no reported cost and no price-table estimate. They count
    /// tokens but add nothing to `total_usd`.
    #[schema(example = 0)]
    pub unpriced_calls: i64,
    /// Spend and tokens per cost category, in a fixed order: `uncached_input`,
    /// `cache_read`, `cache_write`, `output`.
    pub categories: Vec<CostCategory>,
    /// Spend of model calls whose split by category is unknown: the provider
    /// reported a cost but the model has no price table.
    #[schema(example = 0.0)]
    pub unattributed_usd: f64,
    /// Spend per model, largest first.
    pub models: Vec<CostModel>,
    /// Spend per UTC hour that had model calls, oldest first.
    pub hours: Vec<CostHour>,
    /// Spend per turn, oldest first; the newest turns when `turns_truncated`.
    pub turns: Vec<CostTurn>,
    /// Older turns were left out of `turns`.
    pub turns_truncated: bool,
    /// Turns and spend by what started them, largest spend first.
    pub sources: Vec<CostSource>,
    /// Tool calls across the session.
    pub tools: CostTools,
    /// Sandboxes the session ran, oldest first.
    pub sandboxes: Vec<CostSandbox>,
}

/// Spend and tokens of one cost category.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CostCategory {
    /// `uncached_input`, `cache_read`, `cache_write` or `output`.
    #[schema(example = "uncached_input")]
    pub key: String,
    #[schema(example = 1_060_877_013_i64)]
    pub tokens: i64,
    /// USD, from each call's cost split by the model's price table.
    #[schema(example = 106.09)]
    pub usd: f64,
}

/// Spend of one model.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CostModel {
    #[schema(example = "meta/muse-spark-1.3-contributor")]
    pub model: String,
    #[schema(example = "openrouter")]
    pub provider: Option<String>,
    #[schema(example = 3811)]
    pub calls: i64,
    #[schema(example = 106.26)]
    pub usd: f64,
}

/// Spend in one UTC hour, by category.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct CostHour {
    /// Start of the hour (UTC).
    pub hour: DateTime<Utc>,
    pub uncached_input_usd: f64,
    pub cache_read_usd: f64,
    pub cache_write_usd: f64,
    pub output_usd: f64,
    /// Spend with no category split (see `SessionCost.unattributed_usd`).
    pub unattributed_usd: f64,
    pub model_calls: i64,
}

/// Spend of one turn.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CostTurn {
    /// Per-session turn number, 1-based.
    #[schema(example = 1162)]
    pub turn: i32,
    #[schema(example = "turn_01a124cd7e9571ac95c312b3c6d0cd82")]
    pub turn_id: String,
    pub started_at: DateTime<Utc>,
    /// `running`, `completed`, `failed`, `cancelled` or `sealed`.
    #[schema(example = "completed")]
    pub status: String,
    #[schema(example = 0.2)]
    pub usd: f64,
    /// Prompt tokens of the turn, cached and uncached.
    #[schema(example = 2_034_790_i64)]
    pub prompt_tokens: i64,
    /// What started the turn: `user`, `schedule`, `agent_trigger`, `channel`, ...
    #[schema(example = "schedule")]
    pub source_kind: String,
}

/// Turns and spend of one origin.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CostSource {
    /// `user`, `schedule`, `agent_trigger`, `channel`, ...
    #[schema(example = "schedule")]
    pub kind: String,
    /// The schedule, trigger or channel, when there is one.
    #[schema(example = "sched_01a1510ebffc7370a495ed68cc7f9ab3")]
    pub source_id: Option<String>,
    /// The schedule's description, when it is a session schedule.
    #[schema(example = "Shipmail claim poll")]
    pub name: Option<String>,
    /// Cron expression of a recurring schedule.
    #[schema(example = "*/5 * * * *")]
    pub cron: Option<String>,
    /// The schedule still fires: enabled and not deleted.
    pub active: Option<bool>,
    #[schema(example = 1845)]
    pub turns: i64,
    #[schema(example = 106.11)]
    pub usd: f64,
}

/// Tool calls across the session, from the trace index.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct CostTools {
    #[schema(example = 1977)]
    pub calls: i64,
    #[schema(example = 14)]
    pub failed: i64,
    /// Summed running time of every call.
    #[schema(example = 645000)]
    pub duration_ms: i64,
    /// Per tool, most calls first.
    pub items: Vec<CostTool>,
}

/// Calls, failures and running time of one tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CostTool {
    #[schema(example = "web_fetch")]
    pub name: String,
    #[schema(example = 1180)]
    pub calls: i64,
    #[schema(example = 13)]
    pub failed: i64,
    #[schema(example = 627000)]
    pub duration_ms: i64,
}

/// A sandbox the session ran and how long it ran.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CostSandbox {
    #[schema(example = "sandbox_01a1510c99c8714eabb22a463e7c1fa1")]
    pub sandbox_id: String,
    #[schema(example = "daytona")]
    pub provider: String,
    pub profile_name: Option<String>,
    pub created_at: DateTime<Utc>,
    #[schema(example = 4320)]
    pub running_seconds: i64,
    #[schema(example = 1860)]
    pub paused_seconds: i64,
    /// Lifecycle spans, oldest first.
    pub spans: Vec<SandboxStateSpan>,
}
