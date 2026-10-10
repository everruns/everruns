// Response shapes of the session Trace view. See knowledge/ui/session-trace.md.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Session-wide trace summary and minimap.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceOverview {
    /// Turns in the session.
    #[schema(example = 1240)]
    pub turn_count: i64,
    /// Steps across all turns.
    #[schema(example = 18311)]
    pub step_count: i64,
    /// Steps and turns that failed, across the session.
    #[schema(example = 27)]
    pub error_count: i64,
    /// Prompt tokens across all model calls.
    #[schema(example = 4812330)]
    pub input_tokens: i64,
    /// Completion tokens across all model calls.
    #[schema(example = 91204)]
    pub output_tokens: i64,
    /// Start of the first turn.
    pub first_started_at: Option<DateTime<Utc>>,
    /// Latest start or end of any turn.
    pub last_activity_at: Option<DateTime<Utc>>,
    /// Turns per minimap bucket.
    #[schema(example = 10)]
    pub bucket_size: i32,
    /// Minimap buckets in turn order.
    pub buckets: Vec<TraceBucket>,
    /// Turns with errors, newest first, at most 500.
    pub error_turns: Vec<i32>,
}

/// A range of turns on the minimap.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceBucket {
    /// First turn in the bucket.
    #[schema(example = 1)]
    pub from_turn: i32,
    /// Last turn in the bucket.
    #[schema(example = 10)]
    pub to_turn: i32,
    /// Steps across the bucket's turns.
    #[schema(example = 142)]
    pub steps: i64,
    /// Summed turn durations.
    #[schema(example = 421000)]
    pub duration_ms: i64,
    /// Failed steps and turns in the bucket.
    #[schema(example = 2)]
    pub errors: i64,
}

/// A page of turns with their steps.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceTurnsPage {
    /// Turns in turn order.
    pub turns: Vec<TraceTurn>,
    /// Turns exist before the first one returned.
    #[schema(example = true)]
    pub has_earlier: bool,
    /// Turns exist after the last one returned.
    #[schema(example = false)]
    pub has_later: bool,
    /// Turns in the session.
    #[schema(example = 1240)]
    pub turn_count: i64,
}

/// One turn: what the user asked, how it went, and what happened.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceTurn {
    /// Per-session turn number, 1-based.
    #[schema(example = 1240)]
    pub turn: i32,
    /// Public turn identifier.
    #[schema(example = "turn_01a124cd7e9571ac95c312b3c6d0cd82")]
    pub turn_id: String,
    /// `running`, `completed`, `failed`, `cancelled` or `sealed`.
    #[schema(example = "completed")]
    pub status: String,
    /// The user message that started the turn.
    #[schema(example = "Check the links on the docs site")]
    pub prompt: Option<String>,
    /// Why the turn failed, when it did.
    pub error: Option<String>,
    /// When the turn started.
    pub started_at: DateTime<Utc>,
    /// When the turn ended; absent while running.
    pub ended_at: Option<DateTime<Utc>>,
    /// Wall time of the turn.
    #[schema(example = 4998)]
    pub duration_ms: Option<i64>,
    /// Steps in the turn.
    #[schema(example = 3)]
    pub step_count: i32,
    /// Model calls in the turn.
    #[schema(example = 2)]
    pub model_calls: i32,
    /// Tool calls in the turn.
    #[schema(example = 1)]
    pub tool_calls: i32,
    /// Sub-agent calls in the turn.
    #[schema(example = 0)]
    pub subagent_calls: i32,
    /// Failed steps in the turn.
    #[schema(example = 0)]
    pub error_count: i32,
    /// Prompt tokens across the turn's model calls.
    #[schema(example = 4210)]
    pub input_tokens: i64,
    /// Completion tokens across the turn's model calls.
    #[schema(example = 129)]
    pub output_tokens: i64,
    /// First and last event sequence of the turn, for links into Events.
    #[schema(example = 12)]
    pub start_sequence: i32,
    /// Last event sequence of the turn; absent while running.
    #[schema(example = 31)]
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
    /// Turn the step belongs to.
    #[schema(example = 1240)]
    pub turn: i32,
    /// Per-turn step number, 1-based.
    #[schema(example = 2)]
    pub step: i32,
    /// `model`, `answer`, `tool`, `approval`, `agent` or `send`.
    #[schema(example = "tool")]
    pub kind: String,
    /// `running`, `success`, `error` or `cancelled`.
    #[schema(example = "success")]
    pub status: String,
    /// When the step started.
    pub started_at: DateTime<Utc>,
    /// Start relative to the turn's start.
    #[schema(example = 3010)]
    pub offset_ms: i64,
    /// Wall time of the step; absent while running.
    #[schema(example = 269)]
    pub duration_ms: Option<i64>,
    /// Tool name, or model for model calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "bash")]
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
    /// Tool call identifier, for tool steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "call_PzDcvnFaUll8os3tPhWj")]
    pub tool_call_id: Option<String>,
    /// Model that served the call, for model steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "gpt-6.1-sol")]
    pub model: Option<String>,
    /// Prompt tokens of a model call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i32>,
    /// Completion tokens of a model call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i32>,
    /// Tool calls this model call asked for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_tool_call_ids: Vec<String>,
    /// Session a `spawn_agent` call started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    /// First event sequence of the step.
    #[schema(example = 14)]
    pub start_sequence: i32,
    /// Last event sequence of the step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 21)]
    pub end_sequence: Option<i32>,
}

/// Consecutive calls of one tool, folded.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceBatch {
    /// Turn the batch belongs to.
    pub turn: i32,
    /// Tool name shared by every call.
    #[schema(example = "web_fetch")]
    pub name: String,
    /// First step number in the batch.
    #[schema(example = 4)]
    pub first_step: i32,
    /// Last step number in the batch.
    #[schema(example = 63)]
    pub last_step: i32,
    /// Calls in the batch.
    #[schema(example = 60)]
    pub count: i64,
    /// Calls that succeeded.
    #[schema(example = 58)]
    pub succeeded: i64,
    /// Calls that failed.
    #[schema(example = 2)]
    pub failed: i64,
    /// Calls still running.
    #[schema(example = 0)]
    pub running: i64,
    /// Median call duration.
    #[schema(example = 410)]
    pub p50_ms: Option<i64>,
    /// 95th percentile call duration.
    #[schema(example = 1830)]
    pub p95_ms: Option<i64>,
    /// When the first call started.
    pub started_at: DateTime<Utc>,
    /// Start relative to the turn's start.
    pub offset_ms: i64,
    /// From the first start to the last end.
    pub wall_ms: Option<i64>,
    /// The first failed calls, at most 20; page the rest with the steps list.
    pub failures: Vec<TraceStep>,
}

/// Steps of a long turn not returned inline.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceGap {
    /// Turn the gap belongs to.
    pub turn: i32,
    /// First step number not returned.
    #[schema(example = 13)]
    pub first_step: i32,
    /// Last step number not returned.
    #[schema(example = 388)]
    pub last_step: i32,
    /// Steps not returned.
    #[schema(example = 376)]
    pub count: i64,
    /// Failed steps among them.
    #[schema(example = 4)]
    pub errors: i64,
}

/// A page of plain steps of one turn.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceStepsPage {
    /// Steps in step order.
    pub steps: Vec<TraceStep>,
    /// Step number to continue from, when more remain in the range.
    #[schema(example = 101)]
    pub next_step: Option<i32>,
}

/// Everything about one step, for the inspector.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceStepDetail {
    /// The step itself.
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
    /// Size of the serialized value.
    #[schema(example = 221)]
    pub size_bytes: usize,
    /// Whether the value was cut to `preview`.
    #[schema(example = false)]
    pub truncated: bool,
}

/// What one model call was sent.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceRequestSummary {
    /// Messages sent in the call.
    #[schema(example = 14)]
    pub message_count: i32,
    /// Messages before this index were already in the previous call.
    #[schema(example = 12)]
    pub new_from: i32,
    /// Tools offered in the call.
    #[schema(example = 9)]
    pub tool_count: i32,
    /// Start of the system prompt.
    pub system_preview: Option<String>,
    /// The new messages, at most 50.
    pub new_messages: Vec<TraceRequestMessage>,
}

/// One message of a model request.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceRequestMessage {
    /// Position in the request.
    #[schema(example = 12)]
    pub index: i32,
    /// `system`, `user`, `assistant` or `tool`.
    #[schema(example = "tool")]
    pub role: String,
    /// Start of the message text.
    pub preview: String,
    /// Size of the serialized message.
    #[schema(example = 244)]
    pub size_bytes: usize,
    /// The whole message, when asked for and within the limit.
    pub content: Option<serde_json::Value>,
}

/// A page of a model request's messages.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceRequestPage {
    /// Messages sent in the call.
    #[schema(example = 14)]
    pub message_count: i32,
    /// Messages before this index were already in the previous call.
    #[schema(example = 12)]
    pub new_from: i32,
    /// Model that served the call.
    pub model: Option<String>,
    /// Requested messages, in order.
    pub messages: Vec<TraceRequestMessage>,
}

/// An event in the session log.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceEventRef {
    /// Event identifier.
    #[schema(example = "event_01a124cd8a1b7f02a5c4e6d1b2f3a4c5")]
    pub id: String,
    /// Sequence in the session event log.
    #[schema(example = 14)]
    pub sequence: i32,
    /// Event type, such as `tool.call_completed`.
    #[serde(rename = "type")]
    #[schema(example = "tool.started")]
    pub event_type: String,
    /// When the event happened.
    pub ts: DateTime<Utc>,
    /// Payload, when small enough to inline.
    pub data: Option<serde_json::Value>,
    /// Size of the serialized payload.
    #[schema(example = 512)]
    pub size_bytes: i32,
}

/// Events of a turn, for the lifecycle rows.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceEventsPage {
    /// Events in sequence order.
    pub events: Vec<TraceEventRef>,
    /// Sequence to continue after, when more remain.
    #[schema(example = 31)]
    pub next_after_sequence: Option<i32>,
}
