// Rows the observers repository reads and writes.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Observer row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct ObserverRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub match_config: serde_json::Value,
    pub sampling_rate: f64,
    pub scorers: serde_json::Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    /// Pass-rate alert (`ObserverAlert`), when set.
    pub alert: Option<serde_json::Value>,
}

/// Input for creating an observer
#[derive(Debug, Clone)]
pub struct CreateObserverRow {
    pub public_id: String,
    pub name: String,
    pub description: Option<String>,
    pub match_config: serde_json::Value,
    pub sampling_rate: f64,
    pub scorers: serde_json::Value,
    pub alert: Option<serde_json::Value>,
}

/// Input for updating an observer
#[derive(Debug, Clone, Default)]
pub struct UpdateObserverRow {
    pub name: Option<String>,
    pub description: Option<String>,
    pub match_config: Option<serde_json::Value>,
    pub sampling_rate: Option<f64>,
    pub scorers: Option<serde_json::Value>,
    pub status: Option<String>,
    /// `Some(None)` clears the alert.
    pub alert: Option<Option<serde_json::Value>>,
}

/// Trace score row from database. Pending rows double as the scoring queue.
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct TraceScoreRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub observer_id: Uuid,
    pub scorer_key: String,
    pub session_id: Uuid,
    pub turn_id: String,
    pub agent_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
    pub status: String,
    pub attempts: i32,
    pub pass: Option<bool>,
    pub value: Option<f64>,
    pub label: Option<String>,
    pub reason: Option<String>,
    pub judge_input_tokens: Option<i64>,
    pub judge_output_tokens: Option<i64>,
    pub judge_cost_usd: Option<f64>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// `turn`, `session` or `tool`.
    pub scope: String,
    /// A person's review of this score (`ScoreReview`).
    pub review: Option<serde_json::Value>,
}

/// Input for enqueueing a trace score (one per matched scorer).
#[derive(Debug, Clone)]
pub struct CreateTraceScoreRow {
    pub public_id: String,
    pub observer_id: Uuid,
    pub scorer_key: String,
    pub session_id: Uuid,
    pub turn_id: String,
    pub agent_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
    /// `turn`, `session` or `tool`.
    pub scope: String,
    /// Not claimed before this time (a session score waits for the
    /// conversation to settle).
    pub not_before: Option<DateTime<Utc>>,
}

/// Terminal result written back by the scoring worker.
#[derive(Debug, Clone, Default)]
pub struct CompleteTraceScoreRow {
    /// `completed`, `errored`, or `skipped`.
    pub status: String,
    pub pass: Option<bool>,
    pub value: Option<f64>,
    pub label: Option<String>,
    pub reason: Option<String>,
    pub judge_input_tokens: Option<i64>,
    pub judge_output_tokens: Option<i64>,
    pub judge_cost_usd: Option<f64>,
    pub error_message: Option<String>,
}

/// Filters for listing trace scores.
#[derive(Debug, Clone, Default)]
pub struct ListTraceScoresParams {
    pub observer_id: Uuid,
    pub session_id: Option<Uuid>,
    /// Only completed scores no person has reviewed, failing ones first.
    pub review_queue: bool,
    pub limit: i64,
    pub offset: i64,
}

/// Completed-score counts for live-quality summaries.
#[derive(Debug, Clone, FromRow)]
pub struct ScoreSummaryRow {
    /// Grouping key: an agent id, an observer id or a day, by query.
    pub key: Option<String>,
    pub scored: i64,
    pub passed: i64,
    pub sessions: i64,
}

/// One observer's pass rate over a window, for its alert.
#[derive(Debug, Clone, FromRow)]
pub struct ObserverWindowRow {
    pub scored: i64,
    pub passed: i64,
}

/// How `summarize_trace_scores` groups.
#[derive(Debug, Clone, Copy)]
pub enum ScoreGroup {
    Total,
    Agent,
    Observer,
    Day,
    /// `<agent uuid>|<YYYY-MM-DD>`, for per-agent trends.
    AgentDay,
}

/// One past event a backfill can score, with its session's attributes.
#[derive(Debug, Clone, FromRow)]
pub struct BackfillEventRow {
    pub session_id: Uuid,
    pub event_type: String,
    pub data: serde_json::Value,
    pub agent_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
    pub tags: Option<Vec<String>>,
}
