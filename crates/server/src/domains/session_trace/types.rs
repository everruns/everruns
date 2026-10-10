// Response shapes of the session Trace view. See knowledge/ui/session-trace.md.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Session-wide trace summary and minimap.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceOverview {
    pub turn_count: i64,
    pub step_count: i64,
    /// Steps and turns that failed, across the session.
    pub error_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub first_started_at: Option<DateTime<Utc>>,
    pub last_activity_at: Option<DateTime<Utc>>,
    /// Turns per minimap bucket.
    pub bucket_size: i32,
    /// Minimap buckets in turn order.
    pub buckets: Vec<TraceBucket>,
    /// Turns with errors, newest first, at most 500.
    pub error_turns: Vec<i32>,
}

/// A range of turns on the minimap.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceBucket {
    pub from_turn: i32,
    pub to_turn: i32,
    pub steps: i64,
    pub duration_ms: i64,
    pub errors: i64,
}

/// A page of turns with their steps.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceTurnsPage {
    pub turns: Vec<TraceTurn>,
    /// Turns exist before the first one returned.
    pub has_earlier: bool,
    /// Turns exist after the last one returned.
    pub has_later: bool,
    pub turn_count: i64,
}

/// One turn: what the user asked, how it went, and what happened.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceTurn {
    /// Per-session turn number, 1-based.
    pub turn: i32,
    /// Public turn identifier.
    pub turn_id: String,
    /// `running`, `completed`, `failed`, `cancelled` or `sealed`.
    pub status: String,
    pub prompt: Option<String>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    pub step_count: i32,
    pub model_calls: i32,
    pub tool_calls: i32,
    pub subagent_calls: i32,
    pub error_count: i32,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// First and last event sequence of the turn, for links into Events.
    pub start_sequence: i32,
    pub end_sequence: Option<i32>,
    /// Steps, with repeated calls folded into batches and the middle of very
    /// long turns folded into a gap.
    pub items: Vec<TraceItem>,
}

/// One row of a turn.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TraceItem {
    Step(TraceStep),
    Batch(TraceBatch),
    Gap(TraceGap),
}

/// A model call, tool call, approval, sub-agent or message. Empty fields are
/// left out: a page carries hundreds of these.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceStep {
    pub turn: i32,
    /// Per-turn step number, 1-based.
    pub step: i32,
    /// `model`, `answer`, `tool`, `approval`, `agent` or `send`.
    pub kind: String,
    /// `running`, `success`, `error` or `cancelled`.
    pub status: String,
    pub started_at: DateTime<Utc>,
    /// Start relative to the turn's start.
    pub offset_ms: i64,
    pub duration_ms: Option<i64>,
    /// Tool name, or model for model calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What the call acted on, from the tool's narration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Short result or error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// What the model said alongside this call (model calls and answers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narration: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i32>,
    /// Tool calls this model call asked for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_tool_call_ids: Vec<String>,
    /// Session a `spawn_agent` call started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    pub start_sequence: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_sequence: Option<i32>,
}

/// Consecutive calls of one tool, folded.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceBatch {
    pub turn: i32,
    pub name: String,
    pub first_step: i32,
    pub last_step: i32,
    pub count: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub running: i64,
    pub p50_ms: Option<i64>,
    pub p95_ms: Option<i64>,
    pub started_at: DateTime<Utc>,
    pub offset_ms: i64,
    /// From the first start to the last end.
    pub wall_ms: Option<i64>,
    /// The first failed calls, at most 20; page the rest with the steps list.
    pub failures: Vec<TraceStep>,
}

/// Steps of a long turn not returned inline.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceGap {
    pub turn: i32,
    pub first_step: i32,
    pub last_step: i32,
    pub count: i64,
    pub errors: i64,
}

/// A page of plain steps of one turn.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceStepsPage {
    pub steps: Vec<TraceStep>,
    /// Step number to continue from, when more remain in the range.
    pub next_step: Option<i32>,
}

/// Everything about one step, for the inspector.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceStepDetail {
    pub step: TraceStep,
    /// Tool arguments.
    pub input: Option<TracePayload>,
    /// Tool result, or error.
    pub output: Option<TracePayload>,
    /// The request of a model call: what changed since the previous call.
    pub request: Option<TraceRequestSummary>,
    /// Raw events of the step, in sequence order.
    pub events: Vec<TraceEventRef>,
}

/// A JSON payload, cut when over the inline limit.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TracePayload {
    /// The value, when it fits or `full` was asked for.
    pub value: Option<serde_json::Value>,
    /// The first part of the serialized value, when it was cut.
    pub preview: Option<String>,
    pub size_bytes: usize,
    pub truncated: bool,
}

/// What one model call was sent.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceRequestSummary {
    pub message_count: i32,
    /// Messages before this index were already in the previous call.
    pub new_from: i32,
    pub tool_count: i32,
    pub system_preview: Option<String>,
    /// The new messages, at most 50.
    pub new_messages: Vec<TraceRequestMessage>,
}

/// One message of a model request.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceRequestMessage {
    pub index: i32,
    pub role: String,
    pub preview: String,
    pub size_bytes: usize,
    /// The whole message, when asked for and within the limit.
    pub content: Option<serde_json::Value>,
}

/// A page of a model request's messages.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceRequestPage {
    pub message_count: i32,
    pub new_from: i32,
    pub model: Option<String>,
    pub messages: Vec<TraceRequestMessage>,
}

/// An event in the session log.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceEventRef {
    pub id: String,
    pub sequence: i32,
    #[serde(rename = "type")]
    pub event_type: String,
    pub ts: DateTime<Utc>,
    /// Payload, when small enough to inline.
    pub data: Option<serde_json::Value>,
    pub size_bytes: i32,
}

/// Events of a turn, for the lifecycle rows.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceEventsPage {
    pub events: Vec<TraceEventRef>,
    /// Sequence to continue after, when more remain.
    pub next_after_sequence: Option<i32>,
}
