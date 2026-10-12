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
    pub limit: i64,
    pub offset: i64,
}
