use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::typed_id::{EventId, ExecId, MessageId, SessionId, TurnId};

// Split out of one 5300-line file; every item keeps its visibility, so the module's public surface is unchanged.
mod compaction_data;
mod file_voice_data;
mod llm_data;
mod message_data;
mod reason_data;
mod tool_data;
mod turn_data;
mod usage;

pub use compaction_data::*;
pub use file_voice_data::*;
pub use llm_data::*;
pub use message_data::*;
pub use reason_data::*;
pub use tool_data::*;
pub use turn_data::*;
pub use usage::*;

// Input events
pub const INPUT_MESSAGE: &str = "input.message";

// Output events (lifecycle: started → delta* → completed)
pub const OUTPUT_MESSAGE_STARTED: &str = "output.message.started";
pub const OUTPUT_MESSAGE_DELTA: &str = "output.message.delta";
pub const OUTPUT_MESSAGE_COMPLETED: &str = "output.message.completed";
/// Streaming output was withheld by an output guardrail. Clients should
/// discard everything they accumulated for `turn_id` and show `replacement`
/// instead. The subsequent `output.message.completed` event carries the
/// replacement as the persisted assistant message.
pub const OUTPUT_MESSAGE_REPLACED: &str = "output.message.replaced";

// Turn lifecycle events
pub const TURN_STARTED: &str = "turn.started";
pub const TURN_COMPLETED: &str = "turn.completed";
pub const TURN_FAILED: &str = "turn.failed";
/// Turn was deliberately sealed (stopped to prevent waste): no forward progress
/// across repeated crash-reclaims, or work budget exhausted. Distinct from
/// `turn.completed` (success) and `turn.failed` (error). Carries a `reason`.
/// See EVE-534 and `knowledge/operations/durable-execution-engine.md`.
pub const TURN_SEALED: &str = "turn.sealed";
pub const TURN_CANCELLED: &str = "turn.cancelled";

// Atom lifecycle events
pub const REASON_STARTED: &str = "reason.started";
pub const REASON_COMPLETED: &str = "reason.completed";
pub const REASON_RECOVERED: &str = "reason.recovered";
pub const CAPABILITY_USAGE: &str = "capability.usage";
pub const ACT_STARTED: &str = "act.started";
pub const ACT_COMPLETED: &str = "act.completed";
pub const TOOL_STARTED: &str = "tool.started";
pub const TOOL_COMPLETED: &str = "tool.completed";
pub const TOOL_PROGRESS: &str = "tool.progress";
pub const TOOL_OUTPUT_DELTA: &str = "tool.output.delta";
pub const TOOL_CALL_REQUESTED: &str = "tool.call_requested";
pub const TRANSCRIPT_REPAIRED: &str = "transcript.repaired";
/// A malformed tool call was repaired (or repair was attempted) by the opt-in
/// `tool_call_repair` capability (EVE-600). Carries an outcome label
/// (`local-salvage` | `re-prompt` | `gave-up`).
pub const TOOL_CALL_REPAIRED: &str = "tool.call_repaired";

// LLM events
pub const LLM_GENERATION: &str = "llm.generation";

/// Single source of truth for which event types are ephemeral. Both
/// `EventRequest::is_ephemeral` and `Event::is_ephemeral` delegate here so the
/// match set cannot drift between the input and persisted forms.
fn is_ephemeral_event_type(event_type: &str) -> bool {
    matches!(
        event_type,
        OUTPUT_MESSAGE_DELTA
            | REASON_THINKING_DELTA
            | TOOL_OUTPUT_DELTA
            | VOICE_INPUT_TRANSCRIPT_DELTA
            | VOICE_OUTPUT_TRANSCRIPT_DELTA
    )
}

// Reasoning/thinking events (extended thinking from models like Claude)
pub const REASON_THINKING_STARTED: &str = "reason.thinking.started";
pub const REASON_THINKING_DELTA: &str = "reason.thinking.delta";
pub const REASON_THINKING_COMPLETED: &str = "reason.thinking.completed";

/// Durable record of an opaque assistant reasoning response item.
///
/// Distinct from `reason.thinking.*` (user-visible thinking streams). This event
/// captures provider-supplied opaque/encrypted reasoning artifacts plus safe
/// summary text and per-item metadata, without persisting plaintext hidden
/// chain-of-thought.
pub const REASON_ITEM: &str = "reason.item";

// Session events
pub const SESSION_STARTED: &str = "session.started";
pub const SESSION_ACTIVATED: &str = "session.activated";
pub const SESSION_IDLED: &str = "session.idled";
/// Session title changed through a mutation path that participates in the
/// semantic event protocol.
pub const SESSION_TITLE_UPDATED: &str = "session.title.updated";
/// The model answering the session changed between turns. Emitted at the start
/// of a turn whose input carries a model override differing from the previous
/// turn's, so the transcript records where the switch happened instead of
/// leaving readers to infer it from `llm.generation`.
pub const SESSION_MODEL_CHANGED: &str = "session.model.changed";

// Schedule events
pub const SCHEDULE_TRIGGERED: &str = "schedule.triggered";

// Subagent lifecycle events (`subagent.*`) were retired (EVE-585): the subagent
// flow became Session Tasks and now emits `task.*` events. The legacy types are
// no longer emitted or parsed; historical `subagent.*` rows in old session logs
// deserialize via the generic unsupported-type fallback. See knowledge/execution/events.md.

// Session task lifecycle events (knowledge/runtime-resources/session-tasks.md)
pub const TASK_CREATED: &str = "task.created";
pub const TASK_UPDATED: &str = "task.updated";
pub const TASK_MESSAGE_SENT: &str = "task.message.sent";
pub const TASK_MESSAGE_RECEIVED: &str = "task.message.received";

// Context compaction events
pub const CONTEXT_COMPACTING: &str = "context.compacting";
pub const CONTEXT_COMPACTED: &str = "context.compacted";
/// Event type for context.compaction.skipped: an evaluation ran but installed nothing.
pub const CONTEXT_COMPACTION_SKIPPED: &str = "context.compaction.skipped";
/// Event type for context.compaction.failed: an attempt errored before installing.
pub const CONTEXT_COMPACTION_FAILED: &str = "context.compaction.failed";

// File events
pub const FILE_WRITTEN: &str = "file.written";

// Budget events
pub const BUDGET_WARNING: &str = "budget.warning";
pub const BUDGET_PAUSED: &str = "budget.paused";
pub const BUDGET_EXHAUSTED: &str = "budget.exhausted";
pub const BUDGET_RESUMED: &str = "budget.resumed";

// Voice events
pub const VOICE_SESSION_STARTED: &str = "voice.session.started";
pub const VOICE_INPUT_TRANSCRIPT_DELTA: &str = "voice.input_transcript.delta";
pub const VOICE_INPUT_TRANSCRIPT_COMPLETED: &str = "voice.input_transcript.completed";
pub const VOICE_OUTPUT_TRANSCRIPT_DELTA: &str = "voice.output_transcript.delta";
pub const VOICE_OUTPUT_TRANSCRIPT_COMPLETED: &str = "voice.output_transcript.completed";
pub const VOICE_SESSION_ENDED: &str = "voice.session.ended";
pub const VOICE_SESSION_FAILED: &str = "voice.session.failed";

/// All valid event types for API filtering validation.
/// Used by `types` and `exclude` query parameter validation to reject unknown types
/// and prevent unbounded arrays from reaching the database.
pub const VALID_EVENT_TYPES: &[&str] = &[
    INPUT_MESSAGE,
    OUTPUT_MESSAGE_STARTED,
    OUTPUT_MESSAGE_DELTA,
    OUTPUT_MESSAGE_COMPLETED,
    OUTPUT_MESSAGE_REPLACED,
    TURN_STARTED,
    TURN_COMPLETED,
    TURN_FAILED,
    TURN_SEALED,
    TURN_CANCELLED,
    REASON_STARTED,
    REASON_COMPLETED,
    REASON_RECOVERED,
    ACT_STARTED,
    ACT_COMPLETED,
    TOOL_STARTED,
    TOOL_COMPLETED,
    TOOL_PROGRESS,
    TOOL_OUTPUT_DELTA,
    TOOL_CALL_REQUESTED,
    TRANSCRIPT_REPAIRED,
    TOOL_CALL_REPAIRED,
    LLM_GENERATION,
    REASON_THINKING_STARTED,
    REASON_THINKING_DELTA,
    REASON_THINKING_COMPLETED,
    REASON_ITEM,
    SESSION_STARTED,
    SESSION_ACTIVATED,
    SESSION_IDLED,
    SESSION_TITLE_UPDATED,
    SESSION_MODEL_CHANGED,
    SCHEDULE_TRIGGERED,
    CONTEXT_COMPACTING,
    CONTEXT_COMPACTED,
    CONTEXT_COMPACTION_SKIPPED,
    CONTEXT_COMPACTION_FAILED,
    BUDGET_WARNING,
    BUDGET_PAUSED,
    BUDGET_EXHAUSTED,
    BUDGET_RESUMED,
    VOICE_SESSION_STARTED,
    VOICE_INPUT_TRANSCRIPT_DELTA,
    VOICE_INPUT_TRANSCRIPT_COMPLETED,
    VOICE_OUTPUT_TRANSCRIPT_DELTA,
    VOICE_OUTPUT_TRANSCRIPT_COMPLETED,
    VOICE_SESSION_ENDED,
    VOICE_SESSION_FAILED,
    FILE_WRITTEN,
    CAPABILITY_USAGE,
];

// ============================================================================
// Event Context
// ============================================================================

use crate::execution_context::ExecutionContext;

/// Context for event correlation and tracing
///
/// Uses OpenTelemetry-style trace/span IDs for observability correlation:
/// - `trace_id`: Root of the trace (typically the turn_id string)
/// - `span_id`: This event's unique span identifier
/// - `parent_span_id`: The parent span's identifier for hierarchical linking
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct EventContext {
    /// Turn identifier (for turn-scoped events)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: Option<TurnId>,

    /// User message that triggered this turn
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "message_01933b5a00007000800000000000001"))]
    pub input_message_id: Option<MessageId>,

    /// Atom execution identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "exec_01933b5a00007000800000000000001"))]
    pub exec_id: Option<ExecId>,

    /// Trace ID for observability (OTel-style). Groups related spans into a single trace.
    /// For agent turns, this is typically the turn_id string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,

    /// This event's span ID for observability (OTel-style).
    /// Uniquely identifies this span within the trace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,

    /// Parent span ID for hierarchical linking (OTel-style).
    /// Links this span to its parent in the trace hierarchy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
}

impl EventContext {
    /// Create an empty context (for session-level events)
    pub fn empty() -> Self {
        Self::default()
    }

    /// Create a full event context from an execution context.
    pub fn from_execution_context(ctx: &ExecutionContext) -> Self {
        Self {
            turn_id: Some(ctx.turn_id),
            input_message_id: Some(ctx.input_message_id),
            exec_id: Some(ctx.exec_id),
            trace_id: None,
            span_id: None,
            parent_span_id: None,
        }
    }

    /// Create a context for turn-scoped events (without exec_id)
    pub fn turn(turn_id: TurnId, input_message_id: MessageId) -> Self {
        Self {
            turn_id: Some(turn_id),
            input_message_id: Some(input_message_id),
            exec_id: None,
            trace_id: None,
            span_id: None,
            parent_span_id: None,
        }
    }

    /// Set OTel-style span context for hierarchical tracing
    pub fn with_span(
        mut self,
        trace_id: String,
        span_id: String,
        parent_span_id: Option<String>,
    ) -> Self {
        self.trace_id = Some(trace_id);
        self.span_id = Some(span_id);
        self.parent_span_id = parent_span_id;
        self
    }
}

// ============================================================================
// Standard Event Schema
// ============================================================================

/// Standard event following the Everruns event protocol.
///
/// All events have a consistent structure:
/// - `id`: Unique event identifier (format: event_{32-hex})
/// - `type`: Event type in dot notation (e.g., "input.message", "reason.started")
/// - `ts`: ISO 8601 timestamp with millisecond precision
/// - `session_id`: Session this event belongs to (format: session_{32-hex})
/// - `context`: Correlation context for tracing
/// - `data`: Event-specific payload (typed via EventData enum)
/// - `metadata`: Optional arbitrary metadata
/// - `tags`: Optional list of tags for filtering
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Event {
    /// Unique event identifier (format: event_{32-hex})
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "event_01933b5a00007000800000000000001"))]
    pub id: EventId,

    /// Event type in dot notation
    #[serde(rename = "type")]
    pub event_type: String,

    /// Event timestamp
    pub ts: DateTime<Utc>,

    /// Session this event belongs to (format: session_{32-hex})
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "session_01933b5a00007000800000000000001"))]
    pub session_id: SessionId,

    /// Correlation context
    pub context: EventContext,

    /// Event-specific payload. The schema depends on the event type.
    /// See EventData documentation for the mapping of type to data schema.
    pub data: EventData,

    /// Arbitrary metadata for the event
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    /// Tags for filtering and categorization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,

    /// Sequence number within session (for ordering)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct RawEvent {
    id: EventId,
    #[serde(rename = "type")]
    event_type: String,
    ts: DateTime<Utc>,
    session_id: SessionId,
    context: EventContext,
    data: serde_json::Value,
    metadata: Option<serde_json::Value>,
    tags: Option<Vec<String>>,
    sequence: Option<i32>,
}

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawEvent::deserialize(deserializer)?;
        let data = deserialize_event_data(&raw.event_type, raw.data);
        Ok(Self {
            id: raw.id,
            event_type: raw.event_type,
            ts: raw.ts,
            session_id: raw.session_id,
            context: raw.context,
            data,
            metadata: raw.metadata,
            tags: raw.tags,
            sequence: raw.sequence,
        })
    }
}

impl Event {
    /// Projection safe to publish on an API surface: see
    /// [`EventData::into_public`]. Apply at every event read boundary.
    pub fn into_public(mut self) -> Self {
        self.data = self.data.into_public();
        self
    }

    /// Create a new event with the given session_id, context, and typed data
    ///
    /// The event type is automatically inferred from the data type.
    pub fn new(session_id: SessionId, context: EventContext, data: impl Into<EventData>) -> Self {
        let data = data.into();
        let event_type = data.event_type().to_string();
        Self {
            id: EventId::new(),
            event_type,
            ts: Utc::now(),
            session_id,
            context,
            data,
            metadata: None,
            tags: None,
            sequence: None,
        }
    }

    /// Create an event with a specific ID (for testing or replay)
    pub fn with_id(
        id: EventId,
        session_id: SessionId,
        context: EventContext,
        data: impl Into<EventData>,
    ) -> Self {
        let data = data.into();
        let event_type = data.event_type().to_string();
        Self {
            id,
            event_type,
            ts: Utc::now(),
            session_id,
            context,
            data,
            metadata: None,
            tags: None,
            sequence: None,
        }
    }

    /// Set the sequence number
    pub fn with_sequence(mut self, sequence: i32) -> Self {
        self.sequence = Some(sequence);
        self
    }

    /// Set metadata
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Set tags
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = Some(tags);
        self
    }

    /// Get the session_id as raw UUID
    pub fn session_uuid(&self) -> Uuid {
        self.session_id.uuid()
    }

    /// Check if this is an input or output message event
    pub fn is_message_event(&self) -> bool {
        self.event_type == INPUT_MESSAGE || self.event_type == OUTPUT_MESSAGE_COMPLETED
    }

    /// Whether this event is ephemeral. Delta events may be delivered to
    /// listeners without ever being inserted into the `events` table, so any
    /// downstream storage that holds an FK to `events.id` must treat the
    /// reference as best-effort and skip it for ephemeral sources.
    ///
    /// Mirror of [`EventRequest::is_ephemeral`]; both delegate to
    /// `is_ephemeral_event_type` so the match set stays in lockstep.
    pub fn is_ephemeral(&self) -> bool {
        is_ephemeral_event_type(&self.event_type)
    }

    /// Check if this is an input event
    pub fn is_input_event(&self) -> bool {
        self.event_type.starts_with("input.")
    }

    /// Check if this is an output event
    pub fn is_output_event(&self) -> bool {
        self.event_type.starts_with("output.")
    }

    /// Check if this is an atom lifecycle event
    pub fn is_atom_event(&self) -> bool {
        matches!(
            self.event_type.as_str(),
            REASON_STARTED
                | REASON_COMPLETED
                | REASON_RECOVERED
                | ACT_STARTED
                | ACT_COMPLETED
                | TOOL_STARTED
                | TOOL_COMPLETED
                | TOOL_PROGRESS
                | TOOL_CALL_REQUESTED
                | TRANSCRIPT_REPAIRED
        )
    }

    /// Check if this is a turn lifecycle event
    pub fn is_turn_event(&self) -> bool {
        self.event_type.starts_with("turn.")
    }

    /// Check if this is a session lifecycle event
    pub fn is_session_event(&self) -> bool {
        self.event_type.starts_with("session.")
    }

    /// Check if this event has unsupported data.
    /// Unsupported events should be filtered before API responses.
    pub fn is_unsupported(&self) -> bool {
        self.data.is_unsupported()
    }
}

// ============================================================================
// Input/Output Event Data Types
// ============================================================================

use crate::message::{ContentPart, Message};
use crate::tool_narration::{
    ToolNarrationPhase, render_group_headline_with_locale, render_tool_narration_with_locale,
};
use crate::tool_types::ToolCall;
use everruns_provider::execution_phase::ExecutionPhase;

/// File operation constants for `FileWrittenData.operation`.
pub const FILE_OP_CREATE: &str = "create";
/// Typed event data enum for all event payloads
///
/// This enum provides type safety for event data. Each variant corresponds
/// to a specific event type and contains the appropriate data structure.
/// The `Raw` variant is used for backward compatibility with legacy events
/// or unknown event types.
///
/// The data type depends on the event `type` field:
/// - `input.message` → InputMessageData
/// - `output.message.started` → OutputMessageStartedData
/// - `output.message.delta` → OutputMessageDeltaData
/// - `output.message.completed` → OutputMessageCompletedData
/// - `turn.started` → TurnStartedData
/// - `turn.completed` → TurnCompletedData
/// - `turn.failed` → TurnFailedData
/// - `turn.cancelled` → TurnCancelledData
/// - `reason.started` → ReasonStartedData
/// - `reason.completed` → ReasonCompletedData
/// - `capability.usage` → CapabilityUsageData
/// - `act.started` → ActStartedData
/// - `act.completed` → ActCompletedData
/// - `tool.started` → ToolStartedData
/// - `tool.completed` → ToolCompletedData
/// - `tool.output.delta` → ToolOutputDeltaData
/// - `tool.call_requested` → ToolCallRequestedData
/// - `llm.generation` → LlmGenerationData
/// - `reason.thinking.started` → ReasonThinkingStartedData
/// - `reason.thinking.delta` → ReasonThinkingDeltaData
/// - `reason.thinking.completed` → ReasonThinkingCompletedData
/// - `reason.item` → ReasonItemData
/// - `session.started` → SessionStartedData
/// - `session.activated` → SessionActivatedData
/// - `session.idled` → SessionIdledData
/// - `session.title.updated` → SessionTitleUpdatedData
/// - `session.model.changed` → SessionModelChangedData
/// - `file.written` → FileWrittenData
// `untagged` is retained ONLY for encoding and schema, not decoding:
//   - `Serialize` emits the payload inline (the event `type` lives as a sibling
//     field on `Event`/`EventRequest`, never inside `data`), and
//   - the OpenAPI schema renders as a `oneOf` of the payload schemas.
// Decoding never goes through serde's untagged matching. The single source of
// truth for the `type` -> variant mapping is `event_data_kinds!` below, used by
// `deserialize_event_data`. `EventData` deliberately does NOT derive
// `Deserialize`, so the declaration order of variants is irrelevant.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(
    title = "EventData",
    description = "Event-specific payload. The schema depends on the event type field.",
    example = json!({"message": {"id": "...", "role": "user", "content": []}})
))]
pub enum EventData {
    // Input events
    InputMessage(InputMessageData),

    // Output events (lifecycle: started → delta* → completed)
    OutputMessageDelta(OutputMessageDeltaData),
    OutputMessageStarted(OutputMessageStartedData),
    OutputMessageReplaced(OutputMessageReplacedData),
    OutputMessageCompleted(OutputMessageCompletedData),

    // Turn lifecycle events
    TurnStarted(TurnStartedData),
    TurnCompleted(TurnCompletedData),
    TurnFailed(TurnFailedData),

    // Atom lifecycle events
    ReasonStarted(ReasonStartedData),
    ReasonCompleted(ReasonCompletedData),
    ReasonRecovered(ReasonRecoveredData),
    CapabilityUsage(CapabilityUsageData),
    ActStarted(ActStartedData),
    ActCompleted(ActCompletedData),
    ToolStarted(ToolStartedData),
    ToolCompleted(ToolCompletedData),
    ToolProgress(ToolProgressData),
    ToolOutputDelta(ToolOutputDeltaData),
    ToolCallRequested(ToolCallRequestedData),

    // Recovery / repair events
    TranscriptRepaired(TranscriptRepairedData),
    ToolCallRepaired(ToolCallRepairedData),

    // LLM events
    LlmGeneration(LlmGenerationData),

    // Extended thinking events (for models with reasoning like Claude)
    ReasonThinkingDelta(ReasonThinkingDeltaData),
    ReasonItem(ReasonItemData),
    ReasonThinkingStarted(ReasonThinkingStartedData),
    ReasonThinkingCompleted(ReasonThinkingCompletedData),

    TurnSealed(TurnSealedData),
    TurnCancelled(TurnCancelledData),

    // Session events
    SessionStarted(SessionStartedData),
    SessionActivated(SessionActivatedData),
    SessionIdled(SessionIdledData),
    SessionTitleUpdated(SessionTitleUpdatedData),
    SessionModelChanged(SessionModelChangedData),

    // Session task lifecycle events (full snapshots)
    TaskCreated(SessionTaskEventData),
    TaskUpdated(SessionTaskEventData),
    TaskMessageSent(TaskMessageEventData),
    TaskMessageReceived(TaskMessageEventData),

    // Context compaction events
    ContextCompacting(ContextCompactingData),
    ContextCompacted(ContextCompactedData),
    /// Evaluation ran but installed nothing (context.compaction.skipped).
    ContextCompactionSkipped(ContextCompactionSkippedData),
    /// Attempt errored before installing (context.compaction.failed).
    ContextCompactionFailed(ContextCompactionFailedData),

    // File events
    FileWritten(FileWrittenData),

    // Budget events
    BudgetWarning(BudgetEventData),
    BudgetPaused(BudgetEventData),
    BudgetExhausted(BudgetEventData),
    BudgetResumed(BudgetEventData),

    // Voice events
    VoiceSessionStarted(VoiceSessionStartedData),
    VoiceInputTranscriptDelta(VoiceTranscriptData),
    VoiceInputTranscriptCompleted(VoiceTranscriptData),
    VoiceOutputTranscriptDelta(VoiceTranscriptData),
    VoiceOutputTranscriptCompleted(VoiceTranscriptData),
    VoiceSessionEnded(VoiceSessionEndedData),
    VoiceSessionFailed(VoiceSessionFailedData),

    /// Internal-only variant for unknown event types.
    /// Never serialized to API responses - filtered out before transmission.
    /// Logs a warning when created to alert developers of unknown types.
    #[serde(skip)]
    Unsupported {
        /// The unknown event type string
        event_type: String,
        /// The raw JSON data
        data: serde_json::Value,
    },
}

impl EventData {
    /// Replace every carried message with its publishable projection, dropping
    /// the opaque provider replay state on reasoning parts.
    ///
    /// The message read path already does this via [`Message::into_public`], so
    /// without it here `GET /v1/sessions/{id}/events` would hand a client
    /// exactly the `signature` / `encrypted` material that
    /// `GET /v1/sessions/{id}/messages` withholds (EVE-933).
    ///
    /// This is a *read* projection. The stored event must keep the replay state,
    /// because replaying a turn reconstructs the message from the event log.
    /// Whether [`Self::into_public`] would change this payload.
    ///
    /// Lets a hot read path (the SSE stream) skip cloning for the many variants
    /// that carry no message. Keep in step with `into_public`.
    pub fn needs_public_projection(&self) -> bool {
        matches!(
            self,
            EventData::InputMessage(_)
                | EventData::OutputMessageCompleted(_)
                | EventData::LlmGeneration(_)
        )
    }

    pub fn into_public(self) -> Self {
        match self {
            EventData::InputMessage(mut data) => {
                data.message = data.message.into_public();
                EventData::InputMessage(data)
            }
            EventData::OutputMessageCompleted(mut data) => {
                data.message = data.message.into_public();
                EventData::OutputMessageCompleted(data)
            }
            // The full prompt sent to the provider, so it carries the same
            // reasoning parts the assistant message does.
            EventData::LlmGeneration(mut data) => {
                data.messages = data
                    .messages
                    .into_iter()
                    .map(Message::into_public)
                    .collect();
                EventData::LlmGeneration(data)
            }
            other => other,
        }
    }

    /// Check if this is an unsupported event type.
    /// Unsupported events should be filtered before API responses.
    pub fn is_unsupported(&self) -> bool {
        matches!(self, EventData::Unsupported { .. })
    }

    /// Create an unsupported event data with warning log.
    /// This is used when deserializing unknown event types.
    pub fn unsupported(event_type: String, data: serde_json::Value) -> Self {
        tracing::warn!(
            event_type = %event_type,
            "Encountered unsupported event type - will be filtered from API responses"
        );
        EventData::Unsupported { event_type, data }
    }
}

/// Single source of truth for event identity.
///
/// Each entry maps an event `type` string to its [`EventData`] variant and the
/// payload struct that variant carries. The macro generates both directions from
/// this one list:
///   - [`EventData::event_type`] (variant -> `type` string), and
///   - [`deserialize_event_data`] (`type` string -> variant).
///
/// Keeping them in one place means the two can never drift out of sync, which is
/// why there is no separate hand-written dispatcher. Encoding and the OpenAPI
/// schema are handled by the `#[serde(untagged)]` `Serialize` derive on the enum;
/// declaration order there is irrelevant because decoding is driven entirely by
/// the `type` string below.
macro_rules! event_data_kinds {
    ($( $variant:ident($data:ty) = $type_const:path ),+ $(,)?) => {
        impl EventData {
            /// Get the event type constant for this data.
            /// For Unsupported events, returns "unsupported" (internal use only).
            pub fn event_type(&self) -> &'static str {
                match self {
                    $( EventData::$variant(_) => $type_const, )+
                    EventData::Unsupported { .. } => "unsupported",
                }
            }
        }

        /// Deserialize event data from JSON based on `event_type`.
        ///
        /// The outer `type` string selects the variant, avoiding serde's untagged
        /// matching where a payload with fewer required fields could shadow a more
        /// specific one.
        ///
        /// # Returns
        /// The deserialized [`EventData`] variant. Unknown event types, and known
        /// types whose payload fails to decode, fall back to
        /// [`EventData::Unsupported`] (logged) rather than erroring or panicking.
        /// Unsupported events should be filtered before API responses.
        pub fn deserialize_event_data(event_type: &str, data: serde_json::Value) -> EventData {
            let result = match event_type {
                $(
                    $type_const => serde_json::from_value::<$data>(data.clone())
                        .map(EventData::$variant),
                )+
                _ => return EventData::unsupported(event_type.to_string(), data),
            };

            result.unwrap_or_else(|e| {
                tracing::warn!(
                    event_type = %event_type,
                    error = %e,
                    "Failed to deserialize known event type - treating as unsupported"
                );
                EventData::Unsupported {
                    event_type: event_type.to_string(),
                    data,
                }
            })
        }
    };
}

event_data_kinds! {
    // Input events
    InputMessage(InputMessageData) = INPUT_MESSAGE,

    // Output events
    OutputMessageStarted(OutputMessageStartedData) = OUTPUT_MESSAGE_STARTED,
    OutputMessageDelta(OutputMessageDeltaData) = OUTPUT_MESSAGE_DELTA,
    OutputMessageReplaced(OutputMessageReplacedData) = OUTPUT_MESSAGE_REPLACED,
    OutputMessageCompleted(OutputMessageCompletedData) = OUTPUT_MESSAGE_COMPLETED,

    // Turn lifecycle events
    TurnStarted(TurnStartedData) = TURN_STARTED,
    TurnCompleted(TurnCompletedData) = TURN_COMPLETED,
    TurnFailed(TurnFailedData) = TURN_FAILED,
    TurnSealed(TurnSealedData) = TURN_SEALED,
    TurnCancelled(TurnCancelledData) = TURN_CANCELLED,

    // Atom lifecycle events
    ReasonStarted(ReasonStartedData) = REASON_STARTED,
    ReasonCompleted(ReasonCompletedData) = REASON_COMPLETED,
    ReasonRecovered(ReasonRecoveredData) = REASON_RECOVERED,
    CapabilityUsage(CapabilityUsageData) = CAPABILITY_USAGE,
    ActStarted(ActStartedData) = ACT_STARTED,
    ActCompleted(ActCompletedData) = ACT_COMPLETED,
    ToolStarted(ToolStartedData) = TOOL_STARTED,
    ToolCompleted(ToolCompletedData) = TOOL_COMPLETED,
    ToolProgress(ToolProgressData) = TOOL_PROGRESS,
    ToolOutputDelta(ToolOutputDeltaData) = TOOL_OUTPUT_DELTA,
    ToolCallRequested(ToolCallRequestedData) = TOOL_CALL_REQUESTED,

    // Recovery / repair events
    TranscriptRepaired(TranscriptRepairedData) = TRANSCRIPT_REPAIRED,
    ToolCallRepaired(ToolCallRepairedData) = TOOL_CALL_REPAIRED,

    // LLM events
    LlmGeneration(LlmGenerationData) = LLM_GENERATION,

    // Extended thinking events
    ReasonThinkingStarted(ReasonThinkingStartedData) = REASON_THINKING_STARTED,
    ReasonThinkingDelta(ReasonThinkingDeltaData) = REASON_THINKING_DELTA,
    ReasonThinkingCompleted(ReasonThinkingCompletedData) = REASON_THINKING_COMPLETED,
    ReasonItem(ReasonItemData) = REASON_ITEM,

    // Session events
    SessionStarted(SessionStartedData) = SESSION_STARTED,
    SessionActivated(SessionActivatedData) = SESSION_ACTIVATED,
    SessionIdled(SessionIdledData) = SESSION_IDLED,
    SessionTitleUpdated(SessionTitleUpdatedData) = SESSION_TITLE_UPDATED,
    SessionModelChanged(SessionModelChangedData) = SESSION_MODEL_CHANGED,

    // Context compaction events
    ContextCompacting(ContextCompactingData) = CONTEXT_COMPACTING,
    ContextCompacted(ContextCompactedData) = CONTEXT_COMPACTED,
    ContextCompactionSkipped(ContextCompactionSkippedData) = CONTEXT_COMPACTION_SKIPPED,
    ContextCompactionFailed(ContextCompactionFailedData) = CONTEXT_COMPACTION_FAILED,

    // File events
    FileWritten(FileWrittenData) = FILE_WRITTEN,

    // Budget events (all four share BudgetEventData)
    BudgetWarning(BudgetEventData) = BUDGET_WARNING,
    BudgetPaused(BudgetEventData) = BUDGET_PAUSED,
    BudgetExhausted(BudgetEventData) = BUDGET_EXHAUSTED,
    BudgetResumed(BudgetEventData) = BUDGET_RESUMED,

    // Voice events
    VoiceSessionStarted(VoiceSessionStartedData) = VOICE_SESSION_STARTED,
    VoiceInputTranscriptDelta(VoiceTranscriptData) = VOICE_INPUT_TRANSCRIPT_DELTA,
    VoiceInputTranscriptCompleted(VoiceTranscriptData) = VOICE_INPUT_TRANSCRIPT_COMPLETED,
    VoiceOutputTranscriptDelta(VoiceTranscriptData) = VOICE_OUTPUT_TRANSCRIPT_DELTA,
    VoiceOutputTranscriptCompleted(VoiceTranscriptData) = VOICE_OUTPUT_TRANSCRIPT_COMPLETED,
    VoiceSessionEnded(VoiceSessionEndedData) = VOICE_SESSION_ENDED,
    VoiceSessionFailed(VoiceSessionFailedData) = VOICE_SESSION_FAILED,

    // Session task lifecycle events
    TaskCreated(SessionTaskEventData) = TASK_CREATED,
    TaskUpdated(SessionTaskEventData) = TASK_UPDATED,
    TaskMessageSent(TaskMessageEventData) = TASK_MESSAGE_SENT,
    TaskMessageReceived(TaskMessageEventData) = TASK_MESSAGE_RECEIVED,
}

/// Macro to generate From implementations for EventData variants.
///
/// Reduces boilerplate from 5 lines to 1 line per variant.
macro_rules! impl_from_event_data {
    ($($data_type:ty => $variant:ident),* $(,)?) => {
        $(
            impl From<$data_type> for EventData {
                fn from(data: $data_type) -> Self {
                    EventData::$variant(data)
                }
            }
        )*
    };
}

// Generate From implementations for all typed event data
impl_from_event_data! {
    InputMessageData => InputMessage,
    OutputMessageStartedData => OutputMessageStarted,
    OutputMessageDeltaData => OutputMessageDelta,
    OutputMessageReplacedData => OutputMessageReplaced,
    OutputMessageCompletedData => OutputMessageCompleted,
    TurnStartedData => TurnStarted,
    TurnCompletedData => TurnCompleted,
    TurnFailedData => TurnFailed,
    TurnSealedData => TurnSealed,
    TurnCancelledData => TurnCancelled,
    ReasonStartedData => ReasonStarted,
    ReasonCompletedData => ReasonCompleted,
    ReasonRecoveredData => ReasonRecovered,
    CapabilityUsageData => CapabilityUsage,
    ActStartedData => ActStarted,
    ActCompletedData => ActCompleted,
    ToolStartedData => ToolStarted,
    ToolCompletedData => ToolCompleted,
    ToolProgressData => ToolProgress,
    ToolOutputDeltaData => ToolOutputDelta,
    ToolCallRequestedData => ToolCallRequested,
    TranscriptRepairedData => TranscriptRepaired,
    ToolCallRepairedData => ToolCallRepaired,
    LlmGenerationData => LlmGeneration,
    ReasonThinkingStartedData => ReasonThinkingStarted,
    ReasonThinkingDeltaData => ReasonThinkingDelta,
    ReasonThinkingCompletedData => ReasonThinkingCompleted,
    ReasonItemData => ReasonItem,
    SessionStartedData => SessionStarted,
    SessionActivatedData => SessionActivated,
    SessionIdledData => SessionIdled,
    SessionTitleUpdatedData => SessionTitleUpdated,
    SessionModelChangedData => SessionModelChanged,
    ContextCompactingData => ContextCompacting,
    ContextCompactedData => ContextCompacted,
    ContextCompactionSkippedData => ContextCompactionSkipped,
    ContextCompactionFailedData => ContextCompactionFailed,
    FileWrittenData => FileWritten,
    VoiceSessionStartedData => VoiceSessionStarted,
    VoiceSessionEndedData => VoiceSessionEnded,
    VoiceSessionFailedData => VoiceSessionFailed,
}

impl EventData {
    pub fn voice_transcript_event(data: VoiceTranscriptData, event_type: &str) -> Self {
        match event_type {
            VOICE_INPUT_TRANSCRIPT_DELTA => EventData::VoiceInputTranscriptDelta(data),
            VOICE_INPUT_TRANSCRIPT_COMPLETED => EventData::VoiceInputTranscriptCompleted(data),
            VOICE_OUTPUT_TRANSCRIPT_DELTA => EventData::VoiceOutputTranscriptDelta(data),
            VOICE_OUTPUT_TRANSCRIPT_COMPLETED => EventData::VoiceOutputTranscriptCompleted(data),
            _ => EventData::unsupported(
                event_type.to_string(),
                serde_json::to_value(&data).unwrap_or(serde_json::Value::Null),
            ),
        }
    }
}

// Budget events reuse BudgetEventData for all four variants,
// so we can't use the macro (it would conflict). Named constructor instead.
impl EventData {
    pub fn budget_event(data: BudgetEventData, event_type: &str) -> Self {
        match event_type {
            BUDGET_WARNING => EventData::BudgetWarning(data),
            BUDGET_PAUSED => EventData::BudgetPaused(data),
            BUDGET_EXHAUSTED => EventData::BudgetExhausted(data),
            BUDGET_RESUMED => EventData::BudgetResumed(data),
            _ => EventData::unsupported(
                event_type.to_string(),
                serde_json::to_value(&data).unwrap_or(serde_json::Value::Null),
            ),
        }
    }
}

// ============================================================================
// Event Request (input type without id/sequence)
// ============================================================================

/// Request to create a new event.
///
/// This is the input type for event ingestion. It contains all the data
/// needed to create an event, but without the `id` and `sequence` fields
/// which are assigned by the storage layer.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct EventRequest {
    /// Event type in dot notation
    #[serde(rename = "type")]
    pub event_type: String,

    /// Event timestamp
    pub ts: DateTime<Utc>,

    /// Session this event belongs to
    pub session_id: SessionId,

    /// Correlation context
    pub context: EventContext,

    /// Event-specific payload
    pub data: EventData,

    /// Arbitrary metadata for the event
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    /// Tags for filtering and categorization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct RawEventRequest {
    #[serde(rename = "type")]
    event_type: String,
    ts: DateTime<Utc>,
    session_id: SessionId,
    context: EventContext,
    data: serde_json::Value,
    metadata: Option<serde_json::Value>,
    tags: Option<Vec<String>>,
}

impl<'de> Deserialize<'de> for EventRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawEventRequest::deserialize(deserializer)?;
        let data = deserialize_event_data(&raw.event_type, raw.data);
        Ok(Self {
            event_type: raw.event_type,
            ts: raw.ts,
            session_id: raw.session_id,
            context: raw.context,
            data,
            metadata: raw.metadata,
            tags: raw.tags,
        })
    }
}

impl EventRequest {
    /// Create a new event request with the given session_id, context, and typed data
    ///
    /// The event type is automatically inferred from the data type.
    pub fn new(session_id: SessionId, context: EventContext, data: impl Into<EventData>) -> Self {
        let data = data.into();
        let event_type = data.event_type().to_string();
        Self {
            event_type,
            ts: Utc::now(),
            session_id,
            context,
            data,
            metadata: None,
            tags: None,
        }
    }

    /// Set metadata
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Set tags
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = Some(tags);
        self
    }

    /// Whether this event is ephemeral (high-frequency streaming deltas that
    /// don't need durable storage). Delivery backends that support ephemeral
    /// routing can publish these events without inserting them into PostgreSQL.
    ///
    /// The authoritative content lives in the corresponding "completed" event
    /// (e.g. `output.message.completed` has the full text), so missing a delta
    /// on reconnect is acceptable.
    pub fn is_ephemeral(&self) -> bool {
        is_ephemeral_event_type(&self.event_type)
    }

    /// Convert to an Event with the given id and sequence
    pub fn into_event(self, id: EventId, sequence: i32) -> Event {
        Event {
            id,
            event_type: self.event_type,
            ts: self.ts,
            session_id: self.session_id,
            context: self.context,
            data: self.data,
            metadata: self.metadata,
            tags: self.tags,
            sequence: Some(sequence),
        }
    }
}

// ============================================================================
// Event Builder
// ============================================================================

/// Builder for creating events with fluent API
pub struct EventBuilder {
    session_id: SessionId,
    context: EventContext,
}

impl EventBuilder {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            context: EventContext::empty(),
        }
    }

    pub fn with_turn(mut self, turn_id: TurnId, input_message_id: MessageId) -> Self {
        self.context.turn_id = Some(turn_id);
        self.context.input_message_id = Some(input_message_id);
        self
    }

    pub fn with_exec(mut self, exec_id: ExecId) -> Self {
        self.context.exec_id = Some(exec_id);
        self
    }

    pub fn build(self, data: impl Into<EventData>) -> Event {
        Event::new(self.session_id, self.context, data)
    }
}

// ============================================================================
// Tests
// ============================================================================

// ============================================================================
// Contract Tests
// ============================================================================
//
// These tests validate the event protocol contract defined in knowledge/execution/events.md.
// Snapshot tests ensure JSON structure doesn't change accidentally.
// Forward compatibility tests verify unknown fields are handled correctly.

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod token_usage_tests;
