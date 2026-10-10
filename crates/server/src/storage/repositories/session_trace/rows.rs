use chrono::{DateTime, Utc};
use uuid::Uuid;

/// One turn of a session's trace index.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TraceTurnRow {
    pub turn_no: i32,
    pub turn_id: Uuid,
    pub start_sequence: i32,
    pub end_sequence: Option<i32>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub status: String,
    pub prompt_preview: Option<String>,
    pub error: Option<String>,
    pub step_count: i32,
    pub model_calls: i32,
    pub tool_calls: i32,
    pub subagent_calls: i32,
    pub error_count: i32,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

/// One step (model call, tool call, approval, sub-agent, message) of a turn.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TraceStepRow {
    pub turn_no: i32,
    pub step_no: i32,
    pub kind: String,
    pub status: String,
    pub start_sequence: i32,
    pub end_sequence: Option<i32>,
    pub started_at: DateTime<Utc>,
    pub duration_ms: Option<i64>,
    pub name: Option<String>,
    pub target: Option<String>,
    pub result: Option<String>,
    pub narration: Option<String>,
    pub tool_call_id: Option<String>,
    pub model: Option<String>,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub message_count: Option<i32>,
    pub requested_tool_call_ids: Vec<String>,
}

/// Outcome of one catch-up pass over a session's events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceCatchUp {
    /// The index covers every event up to this sequence.
    UpToDate { projected_sequence: i32 },
    /// The pass hit its event budget; more events remain.
    Partial { projected_sequence: i32 },
    /// Another projector holds the session; it will apply the new events.
    Busy,
}
