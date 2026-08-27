// ==========================================================================
// PUBLIC CONTRACT - Event Protocol
// ==========================================================================
//
// This module defines the Everruns event protocol - a PUBLIC API CONTRACT.
// Changes must follow the compatibility guidelines in knowledge/execution/events.md.
//
// STABILITY: Stable (v1)
// - Event structure (id, type, ts, session_id, context, data) is frozen
// - New event types are additive (non-breaking)
// - New optional fields are non-breaking
// - Unsupported events are filtered before API responses
//
// See: knowledge/execution/events.md for full contract specification.
// ==========================================================================
//
// All events follow a consistent structure: id, type, ts, context, data.
// Events are the source of truth for conversation data and provide
// observability into session execution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::localization::localized_tool_display_name;
use crate::typed_id::{AgentId, EventId, ExecId, HarnessId, MessageId, ModelId, SessionId, TurnId};
use crate::user_facing_error::{UserFacingError, UserFacingErrorFields};

// ============================================================================
// Event Type Constants
// ============================================================================

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
    SCHEDULE_TRIGGERED,
    CONTEXT_COMPACTING,
    CONTEXT_COMPACTED,
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

/// Metadata about the model used for generation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ModelMetadata {
    /// Model name (e.g., "gpt-5.6-sol", "claude-sonnet-5")
    pub model: String,

    /// Model ID (internal identifier)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Uuid>,

    /// Provider ID (internal identifier)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<Uuid>,
}

/// Token usage statistics
///
/// Tracks token consumption per LLM call including cache tokens for cost
/// optimization.
///
/// # Disjoint bucket convention
///
/// Prompt token buckets are **disjoint** (non-overlapping). Drivers normalize
/// provider wire formats at the boundary so this holds for every provider:
///
/// ```text
/// total_prompt = input_tokens + cache_read_tokens + cache_creation_tokens
/// ```
///
/// - `input_tokens` — non-cached prompt tokens only.
/// - `cache_read_tokens` — tokens served from cache, never counted in
///   `input_tokens`.
/// - `cache_creation_tokens` — tokens written to cache, never counted in
///   `input_tokens`.
///
/// Inclusive providers (OpenAI Responses / Chat Completions, Gemini) report a
/// prompt count that *includes* cached reads; their drivers subtract the cached
/// subset so the value stored here is the non-cached remainder. Anthropic /
/// Bedrock already report disjoint buckets. Cost is therefore uniform across
/// providers (`input·in + cache_read·cr + cache_creation·cw + output·out`);
/// consumers must not re-derive a non-cached input by subtracting cache reads.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TokenUsage {
    /// Number of non-cached prompt tokens (cached reads/writes are tracked
    /// separately; see the disjoint bucket convention on the struct)
    pub input_tokens: u32,
    /// Number of output/completion tokens
    pub output_tokens: u32,
    /// Number of tokens read from cache (reduces cost), disjoint from `input_tokens`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
    /// Number of tokens written to cache, disjoint from `input_tokens`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_tokens: Option<u32>,

    /// Actual cost of this generation in USD, as reported by the provider inline
    /// (e.g. OpenRouter's `usage.cost`, which reflects real post-routing/BYOK/cache
    /// pricing). `None` for providers that do not return a cost.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_cost_usd: Option<f64>,

    /// Estimated cost of this generation in USD, derived from the model's static
    /// price-table profile. Computed whenever a profile with cost data exists,
    /// independently of `actual_cost_usd`, so estimate-vs-actual drift can be
    /// reconciled. `None` when there is no profile cost data for the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,

    /// Best-effort USD cost for already-aggregated usage. Per-generation usage
    /// leaves this unset and derives the effective cost from actual/estimated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_cost_usd: Option<f64>,
}

impl TokenUsage {
    /// Create a new TokenUsage with just input and output tokens
    pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            actual_cost_usd: None,
            estimated_cost_usd: None,
            effective_cost_usd: None,
        }
    }

    /// Create a TokenUsage with cache tokens
    pub fn with_cache(
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: Option<u32>,
        cache_creation_tokens: Option<u32>,
    ) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            actual_cost_usd: None,
            estimated_cost_usd: None,
            effective_cost_usd: None,
        }
    }

    /// Set the actual (provider-reported) and estimated (price-table) USD costs,
    /// returning `self` for chaining. The two are tracked independently so an
    /// authoritative charge stays distinguishable from an estimate.
    pub fn with_cost(
        mut self,
        actual_cost_usd: Option<f64>,
        estimated_cost_usd: Option<f64>,
    ) -> Self {
        self.actual_cost_usd = actual_cost_usd;
        self.estimated_cost_usd = estimated_cost_usd;
        self
    }

    /// Set the precomputed best-effort USD cost for aggregate usage, returning
    /// `self` for chaining. Generation usage should leave this unset so the
    /// best-effort cost remains actual-else-estimated for that generation.
    pub fn with_effective_cost(mut self, effective_cost_usd: Option<f64>) -> Self {
        self.effective_cost_usd = effective_cost_usd;
        self
    }

    /// Best-effort cost in USD. Aggregate usage can carry a precomputed total
    /// that sums actual-else-estimated per generation; otherwise this falls back
    /// to the generation-level actual-else-estimated behavior.
    pub fn effective_cost_usd(&self) -> Option<f64> {
        self.effective_cost_usd
            .or(self.actual_cost_usd.or(self.estimated_cost_usd))
    }

    /// Get total tokens (input + output)
    pub fn total_tokens(&self) -> u32 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    /// Add another TokenUsage to this one (for aggregation)
    pub fn add(&mut self, other: &TokenUsage) {
        // Capture the accumulator's effective cost BEFORE mutating the
        // actual/estimated slots below: `effective_cost_usd()` falls back to
        // those slots when no explicit total is set, so reading it afterward
        // would fold in `other`'s just-added actual/estimated and double-count.
        let current_cost = self.effective_cost_usd();
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        if let Some(cache) = other.cache_read_tokens {
            let total = self.cache_read_tokens.get_or_insert(0);
            *total = total.saturating_add(cache);
        }
        if let Some(cache) = other.cache_creation_tokens {
            let total = self.cache_creation_tokens.get_or_insert(0);
            *total = total.saturating_add(cache);
        }
        if let Some(cost) = other.actual_cost_usd {
            *self.actual_cost_usd.get_or_insert(0.0) += cost;
        }
        if let Some(cost) = other.estimated_cost_usd {
            *self.estimated_cost_usd.get_or_insert(0.0) += cost;
        }
        if let Some(cost) = other.effective_cost_usd() {
            *self
                .effective_cost_usd
                .get_or_insert(current_cost.unwrap_or(0.0)) += cost;
        }
    }
}

#[cfg(test)]
mod token_usage_tests {
    use super::TokenUsage;

    #[test]
    fn aggregate_effective_cost_preserves_mixed_actual_and_estimated_total() {
        let aggregate = TokenUsage::with_cache(30, 15, None, None)
            .with_cost(Some(1.0), Some(2.0))
            .with_effective_cost(Some(3.0));

        assert_eq!(aggregate.actual_cost_usd, Some(1.0));
        assert_eq!(aggregate.estimated_cost_usd, Some(2.0));
        assert_eq!(aggregate.effective_cost_usd(), Some(3.0));
    }

    #[test]
    fn generation_effective_cost_still_prefers_actual_over_estimated() {
        let generation = TokenUsage::new(10, 5).with_cost(Some(1.0), Some(2.0));

        assert_eq!(generation.effective_cost_usd(), Some(1.0));
    }

    #[test]
    fn add_seeds_effective_cost_from_accumulator_implicit_cost() {
        let mut aggregate = TokenUsage::new(10, 5).with_cost(Some(1.0), Some(10.0));
        let generation = TokenUsage::new(20, 10).with_cost(None, Some(2.0));

        aggregate.add(&generation);

        assert_eq!(aggregate.actual_cost_usd, Some(1.0));
        assert_eq!(aggregate.estimated_cost_usd, Some(12.0));
        assert_eq!(aggregate.effective_cost_usd(), Some(3.0));
    }

    #[test]
    fn add_effective_cost_does_not_double_count_when_actual_is_mutated() {
        // Neither side carries an explicit effective total, so each side's
        // effective cost falls back to `actual`. Adding must sum the two
        // actuals once (1.0 + 3.0), not fold the accumulator's post-add actual
        // back into the effective total.
        let mut aggregate = TokenUsage::new(10, 5).with_cost(Some(1.0), None);
        let generation = TokenUsage::new(20, 10).with_cost(Some(3.0), None);

        aggregate.add(&generation);

        assert_eq!(aggregate.actual_cost_usd, Some(4.0));
        assert_eq!(aggregate.effective_cost_usd(), Some(4.0));
    }

    #[test]
    fn aggregate_token_counters_saturate_at_their_bound() {
        let mut aggregate = TokenUsage::with_cache(
            u32::MAX - 1,
            u32::MAX - 1,
            Some(u32::MAX - 1),
            Some(u32::MAX - 1),
        );
        aggregate.add(&TokenUsage::with_cache(10, 10, Some(10), Some(10)));

        assert_eq!(aggregate.input_tokens, u32::MAX);
        assert_eq!(aggregate.output_tokens, u32::MAX);
        assert_eq!(aggregate.cache_read_tokens, Some(u32::MAX));
        assert_eq!(aggregate.cache_creation_tokens, Some(u32::MAX));
        assert_eq!(aggregate.total_tokens(), u32::MAX);
    }
}

/// Data for input.message event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct InputMessageData {
    /// The user message
    pub message: Message,
}

impl InputMessageData {
    pub fn new(message: Message) -> Self {
        Self { message }
    }
}

// ============================================================================
// Output Event Data Types
// ============================================================================

/// Data for output.message.started event
///
/// Emitted when the LLM starts generating a response. UI can show a
/// "thinking" indicator until output.message.delta or output.message.completed events arrive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct OutputMessageStartedData {
    /// Turn ID this output belongs to
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Stable public ID for this assistant message across its streaming lifecycle.
    /// This is the same identifier as `OutputMessageCompletedData.message.id`.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "message_550e8400e29b41d4a716446655440000"))]
    pub message_id: MessageId,

    /// Optional model name being used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Current iteration number within this turn (1-based).
    /// Useful for UI to show progress during multi-step tool-calling flows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,

    /// Best-effort streamed phase hint (EVE-774).
    ///
    /// `None` means "not yet classified — treat as ordinary assistant text",
    /// NEVER "thinking". A missing/unknown phase must fall back to the
    /// assistant-text channel, never the reasoning channel. Only populated when
    /// the provider stream reveals a native phase before this event is emitted;
    /// today `output.message.started` is emitted before the LLM call, so this is
    /// generally `None` at start. The authoritative classification remains the
    /// completed `Message.phase`. See `knowledge/execution/events.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<ExecutionPhase>,
}

/// Data for output.message.delta event
///
/// Incremental text update during LLM generation. Events are batched (~100ms)
/// to reduce volume while providing real-time feedback.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct OutputMessageDeltaData {
    /// Turn ID this delta belongs to
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Stable public ID for this assistant message across its streaming lifecycle.
    /// This is the same identifier as `OutputMessageCompletedData.message.id`.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "message_550e8400e29b41d4a716446655440000"))]
    pub message_id: MessageId,

    /// The new text chunk
    pub delta: String,

    /// Accumulated text so far
    pub accumulated: String,

    /// Best-effort streamed phase hint (EVE-774).
    ///
    /// `None` means "not yet classified — treat as ordinary assistant text",
    /// NEVER "thinking". Refinement is monotonic: once a stream reveals a native
    /// phase (`Commentary` or `FinalAnswer`) it stays fixed for the rest of the
    /// message — it never flip-flops and never reverts to `None`
    /// (see `ExecutionPhase::refine_streamed_hint`). Providers without native
    /// mid-stream phase (Anthropic, Gemini, …) leave this `None` until
    /// completion. The authoritative classification remains the completed
    /// `Message.phase`. See `knowledge/execution/events.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<ExecutionPhase>,
}

/// Data for output.message.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct OutputMessageCompletedData {
    /// The agent message
    pub message: Message,

    /// Metadata about the model used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ModelMetadata>,

    /// Token usage
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,

    /// Stable error code for user-facing failures surfaced as assistant text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,

    /// Structured interpolation fields for localized error rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub error_fields: Option<UserFacingErrorFields>,

    /// Error-disclosure mode applied to `error_code`/`error_fields`
    /// ("generic" | "standard" | "detailed"). Tracking metadata; absent for
    /// non-error messages and for paths that predate disclosure modes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_disclosure: Option<String>,
}

impl OutputMessageCompletedData {
    pub fn new(message: Message) -> Self {
        Self {
            message,
            metadata: None,
            usage: None,
            error_code: None,
            error_fields: None,
            error_disclosure: None,
        }
    }

    pub fn with_metadata(mut self, metadata: ModelMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn with_usage(mut self, usage: TokenUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn with_user_facing_error(mut self, error: &UserFacingError) -> Self {
        error.apply_to_event_fields(&mut self.error_code, &mut self.error_fields);
        self
    }

    pub fn with_error_disclosure(mut self, mode: crate::ErrorDisclosure) -> Self {
        self.error_disclosure = Some(mode.as_str().to_string());
        self
    }
}

/// Data for `output.message.replaced` event.
///
/// Emitted between the last (suppressed) `output.message.delta` and the final
/// `output.message.completed`. Tells the client to discard the text accumulated
/// for `message_id` and use `replacement` as that assistant message's text. The
/// original model output is never persisted or replayed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct OutputMessageReplacedData {
    /// Turn ID this replacement belongs to.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Stable public ID for the assistant message whose streamed text is replaced.
    /// This is the same identifier as the subsequent completed `Message.id`.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "message_550e8400e29b41d4a716446655440000"))]
    pub message_id: MessageId,

    /// Stable ID of the capability that contributed the guardrail
    /// (e.g. `"prompt_canary_guardrail"`).
    pub guardrail_capability_id: String,

    /// Stable ID of the guardrail itself (e.g. `"prompt_canary"`).
    pub guardrail_id: String,

    /// Stable machine-readable reason code (e.g. `"system_prompt_leak"`).
    /// Clients localize their copy from this rather than the human text.
    pub reason_code: String,

    /// Replacement text shown to the user and stored as the assistant message.
    pub replacement: String,
}

// ============================================================================
// Atom Event Data Types
// ============================================================================

/// Data for reason.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonStartedData {
    /// Harness ID being used
    pub harness_id: HarnessId,

    /// Agent ID being used (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,

    /// Metadata about the model being used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ModelMetadata>,
}

/// Data for reason.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonCompletedData {
    /// Whether the LLM call succeeded
    pub success: bool,

    /// Text response preview (first 200 chars)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_preview: Option<String>,

    /// Whether tool calls were requested
    pub has_tool_calls: bool,

    /// Number of tool calls requested
    pub tool_call_count: u32,

    /// Error message if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Duration of the reason phase in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,

    /// Token usage from the LLM call
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

impl ReasonCompletedData {
    pub fn success(
        text: &str,
        has_tool_calls: bool,
        tool_call_count: u32,
        duration_ms: Option<u64>,
        usage: Option<TokenUsage>,
    ) -> Self {
        let text_preview = if text.is_empty() {
            None
        } else {
            Some(text.chars().take(200).collect())
        };

        Self {
            success: true,
            text_preview,
            has_tool_calls,
            tool_call_count,
            error: None,
            duration_ms,
            usage,
        }
    }

    pub fn failure(error: String, duration_ms: Option<u64>) -> Self {
        Self {
            success: false,
            text_preview: None,
            has_tool_calls: false,
            tool_call_count: 0,
            error: Some(error),
            duration_ms,
            usage: None,
        }
    }
}

/// Recovery mode chosen by the ContinuePartial classifier (EVE-532).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RecoveryMode {
    /// Persisted accumulated text was finalised as the assistant message;
    /// no second provider call was made.
    Finalize,
    /// Partial was unusable (empty accumulated); re-issued the provider call.
    Restart,
}

/// Data for the `reason.recovered` event (EVE-532).
///
/// Emitted by `ReasonAtom` when it detects an in-flight partial assistant
/// message from a previous worker execution and applies the ContinuePartial
/// recovery policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonRecoveredData {
    /// Turn ID the partial belonged to.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Recovery action taken.
    pub mode: RecoveryMode,

    /// Character length of the persisted accumulated text.
    pub accumulated_len: usize,
}

/// Reporting-only capability usage kinds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub enum CapabilityUsageKind {
    Configured,
    Resolved,
    Exposed,
    Invoked,
    EffectRan,
}

/// Single capability usage record. This intentionally carries only stable IDs
/// and small snapshots; prompts, messages, tool arguments, and results are not
/// allowed in reporting facts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CapabilityUsageRecord {
    /// Capability id (prefixed or namespaced) attributing this usage.
    pub capability_id: String,
    /// Capability display name for UI. `None` when the capability is unnamed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_name: Option<String>,
    /// Discriminator for the kind of usage being recorded (e.g. `tool_call`, `subagent_spawn`).
    pub usage_kind: CapabilityUsageKind,
    /// Concrete tool name when `usage_kind` is `tool_call`. `None` for non-tool usage kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Number of distinct usages recorded in this record (count-style records). `None` for duration-style records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_count: Option<u64>,
    /// Total wall-clock duration of the usage in milliseconds (duration-style records). `None` for count-style records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Data for capability.usage events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CapabilityUsageData {
    pub records: Vec<CapabilityUsageRecord>,
}

/// Summary of a tool call (compact form without arguments)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolCallSummary {
    pub id: String,
    pub name: String,
    /// Human-readable display name for UI rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Human-readable narration for timeline rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narration: Option<String>,
    /// Human-readable narration after the call completes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_narration: Option<String>,
}

impl From<&ToolCall> for ToolCallSummary {
    fn from(tc: &ToolCall) -> Self {
        Self {
            id: tc.id.clone(),
            name: tc.name.clone(),
            display_name: None,
            narration: None,
            completed_narration: None,
        }
    }
}

/// Summary of a tool definition (compact form for events)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolDefinitionSummary {
    /// Tool name
    pub name: String,
    /// Human-readable display name for UI rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Tool category for namespace grouping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Capability that contributed the tool definition, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,
    /// Human-readable capability name snapshot, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_name: Option<String>,
    /// Tool description
    pub description: String,
}

impl From<&crate::tool_types::ToolDefinition> for ToolDefinitionSummary {
    fn from(tool: &crate::tool_types::ToolDefinition) -> Self {
        let capability_attribution = tool.capability_attribution();
        Self {
            name: tool.name().to_string(),
            display_name: tool.display_name().map(|s| s.to_string()),
            category: tool.category().map(|s| s.to_string()),
            capability_id: capability_attribution.map(|(id, _)| id.to_string()),
            capability_name: capability_attribution.and_then(|(_, name)| name.map(str::to_string)),
            description: tool.description().to_string(),
        }
    }
}

/// Data for act.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ActStartedData {
    /// Tool calls to be executed
    pub tool_calls: Vec<ToolCallSummary>,
    /// Human-readable headline for the batch
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
}

impl ActStartedData {
    pub fn new(tool_calls: &[ToolCall]) -> Self {
        Self::new_with_locale(tool_calls, None)
    }

    pub fn new_with_locale(tool_calls: &[ToolCall], locale: Option<&str>) -> Self {
        Self {
            tool_calls: tool_calls.iter().map(ToolCallSummary::from).collect(),
            headline: render_group_headline_with_locale(
                tool_calls,
                &[],
                ToolNarrationPhase::Started,
                locale,
            ),
        }
    }

    /// Create with display names resolved from tool definitions
    pub fn with_definitions(
        tool_calls: &[ToolCall],
        tool_defs: &[crate::tool_types::ToolDefinition],
    ) -> Self {
        Self::with_definitions_and_locale(tool_calls, tool_defs, None)
    }

    pub fn with_definitions_and_locale(
        tool_calls: &[ToolCall],
        tool_defs: &[crate::tool_types::ToolDefinition],
        locale: Option<&str>,
    ) -> Self {
        let def_map: std::collections::HashMap<&str, &crate::tool_types::ToolDefinition> =
            tool_defs.iter().map(|d| (d.name(), d)).collect();
        Self {
            tool_calls: tool_calls
                .iter()
                .map(|tc| {
                    let tool_def = def_map.get(tc.name.as_str()).copied();
                    let display_name = localized_tool_display_name(
                        &tc.name,
                        tool_def.and_then(|d| d.display_name()),
                        locale,
                    );
                    ToolCallSummary {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        display_name,
                        narration: Some(render_tool_narration_with_locale(
                            tool_def,
                            tc,
                            ToolNarrationPhase::Started,
                            locale,
                        )),
                        completed_narration: Some(render_tool_narration_with_locale(
                            tool_def,
                            tc,
                            ToolNarrationPhase::Completed,
                            locale,
                        )),
                    }
                })
                .collect(),
            headline: render_group_headline_with_locale(
                tool_calls,
                tool_defs,
                ToolNarrationPhase::Started,
                locale,
            ),
        }
    }
}

/// Data for act.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ActCompletedData {
    /// Whether all tool calls completed
    pub completed: bool,

    /// Number of successful tool calls
    pub success_count: u32,

    /// Number of failed tool calls
    pub error_count: u32,

    /// Duration of the act phase in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Human-readable headline for the completed batch
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
}

/// Data for tool.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolStartedData {
    /// The tool call being executed
    pub tool_call: ToolCall,
    /// Stable fingerprint of tool name + normalized arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_fingerprint: Option<String>,
    /// Human-readable display name for UI rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Human-readable narration for timeline rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narration: Option<String>,
}

/// Data for tool.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolCompletedData {
    /// Tool call ID
    pub tool_call_id: String,

    /// Tool name
    pub tool_name: String,

    /// Stable fingerprint of tool name + normalized arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_fingerprint: Option<String>,

    /// Stable fingerprint of tool name + normalized result/error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result_fingerprint: Option<String>,

    /// Human-readable display name for UI rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// Whether the tool call succeeded
    pub success: bool,

    /// Status: "success", "error", "timeout", "cancelled"
    pub status: String,

    /// Result content (for successful calls)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Vec<ContentPart>>,

    /// Error message if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Duration of the tool call in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,

    /// Capability that contributed the tool definition, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,

    /// Human-readable capability name snapshot, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_name: Option<String>,

    /// Human-readable narration for timeline rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narration: Option<String>,
}

impl ToolCompletedData {
    pub fn success(
        tool_call_id: String,
        tool_name: String,
        result: Vec<ContentPart>,
        duration_ms: Option<u64>,
    ) -> Self {
        Self {
            tool_call_id,
            tool_name,
            tool_call_fingerprint: None,
            tool_result_fingerprint: None,
            display_name: None,
            success: true,
            status: "success".to_string(),
            result: Some(result),
            error: None,
            duration_ms,
            capability_id: None,
            capability_name: None,
            narration: None,
        }
    }

    pub fn failure(
        tool_call_id: String,
        tool_name: String,
        status: String,
        error: String,
        duration_ms: Option<u64>,
    ) -> Self {
        Self {
            tool_call_id,
            tool_name,
            tool_call_fingerprint: None,
            tool_result_fingerprint: None,
            display_name: None,
            success: false,
            status,
            result: None,
            error: Some(error),
            duration_ms,
            capability_id: None,
            capability_name: None,
            narration: None,
        }
    }

    /// Set display name on this event data
    pub fn with_display_name(mut self, display_name: Option<String>) -> Self {
        self.display_name = display_name;
        self
    }

    pub fn with_fingerprints(
        mut self,
        tool_call_fingerprint: String,
        tool_result_fingerprint: String,
    ) -> Self {
        self.tool_call_fingerprint = Some(tool_call_fingerprint);
        self.tool_result_fingerprint = Some(tool_result_fingerprint);
        self
    }

    /// Set narration on this event data
    pub fn with_narration(mut self, narration: Option<String>) -> Self {
        self.narration = narration;
        self
    }

    /// Set reporting attribution on this event data.
    pub fn with_capability_attribution(
        mut self,
        capability_id: Option<String>,
        capability_name: Option<String>,
    ) -> Self {
        self.capability_id = capability_id;
        self.capability_name = capability_name;
        self
    }
}

/// Data for tool.progress event.
///
/// Emitted by tools during execution to report interim status updates.
/// This allows long-running tools (e.g., browser operations, sandbox setup)
/// to stream progress feedback between tool.started and tool.completed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolProgressData {
    /// Tool call ID this progress belongs to
    pub tool_call_id: String,

    /// Tool name
    pub tool_name: String,

    /// Human-readable status message (e.g., "Connecting to browser…")
    pub message: String,

    /// Human-readable display name for UI rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Data for tool.output.delta event.
///
/// Emitted by tools during execution to stream incremental output chunks.
/// This enables live output rendering (e.g., bash stdout/stderr, command output)
/// between tool.started and tool.completed. Generic — usable by any tool that
/// produces streamed output (bashkit, Daytona exec, subagent speech, etc.).
///
/// The consumer accumulates deltas by tool_call_id for display. The final
/// tool.completed result is authoritative — deltas are informational only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolOutputDeltaData {
    /// Tool call ID this output belongs to
    pub tool_call_id: String,

    /// Tool name
    pub tool_name: String,

    /// Incremental output chunk
    pub delta: String,

    /// Output stream identifier (e.g., "stdout", "stderr")
    pub stream: String,
}

/// Action taken during transcript repair for a dangling tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TranscriptRepairAction {
    /// A settled result was found in durable storage and replayed into the transcript.
    Replay,
    /// A synthetic interrupted result was synthesized to make the transcript well-formed.
    Synthesize,
}

/// Data for transcript.repaired event (EVE-533).
///
/// Emitted once per dangling tool call when transcript repair runs before a `reason` call.
/// A dangling call is an assistant `tool_call` with no matching `ToolResult` in the
/// message history. Repair makes the transcript well-formed so the next LLM call succeeds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TranscriptRepairedData {
    /// The tool call ID that was repaired.
    pub tool_call_id: String,

    /// The tool name, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,

    /// Action taken: `replay` (settled result reused) or `synthesize` (interrupted placeholder added).
    pub action: TranscriptRepairAction,
}

/// Data for the `tool.call_repaired` event (EVE-600).
///
/// Emitted once per malformed tool call handled by the opt-in
/// `tool_call_repair` capability. `outcome` is the stable label
/// (`local-salvage` | `re-prompt` | `gave-up`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolCallRepairedData {
    /// Turn this repair belongs to.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// The tool call ID that was inspected/repaired.
    pub tool_call_id: String,

    /// The tool name the malformed call targeted.
    pub tool_name: String,

    /// Stable outcome label: `local-salvage`, `re-prompt`, or `gave-up`.
    pub outcome: String,
}

/// Data for tool.call_requested event
///
/// Emitted when the agent needs client-side tool calls executed.
/// The workflow pauses until the client submits results via the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolCallRequestedData {
    /// Tool calls that need to be executed by the client
    pub tool_calls: Vec<ToolCall>,
    /// Optional summaries with display names and narration for UI rendering
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_summaries: Vec<ToolCallSummary>,
    /// Human-readable headline for the requested batch
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
    /// Human-readable headline after the requested batch completes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_headline: Option<String>,
}

impl ToolCallRequestedData {
    pub fn with_definitions(
        tool_calls: &[ToolCall],
        tool_defs: &[crate::tool_types::ToolDefinition],
    ) -> Self {
        Self::with_definitions_and_locale(tool_calls, tool_defs, None)
    }

    pub fn with_definitions_and_locale(
        tool_calls: &[ToolCall],
        tool_defs: &[crate::tool_types::ToolDefinition],
        locale: Option<&str>,
    ) -> Self {
        let def_map: std::collections::HashMap<&str, &crate::tool_types::ToolDefinition> =
            tool_defs.iter().map(|d| (d.name(), d)).collect();

        let tool_summaries = tool_calls
            .iter()
            .map(|tool_call| {
                let tool_def = def_map.get(tool_call.name.as_str()).copied();
                ToolCallSummary {
                    id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    display_name: localized_tool_display_name(
                        &tool_call.name,
                        tool_def.and_then(|def| def.display_name()),
                        locale,
                    ),
                    narration: Some(render_tool_narration_with_locale(
                        tool_def,
                        tool_call,
                        ToolNarrationPhase::Waiting,
                        locale,
                    )),
                    completed_narration: Some(render_tool_narration_with_locale(
                        tool_def,
                        tool_call,
                        ToolNarrationPhase::Completed,
                        locale,
                    )),
                }
            })
            .collect();

        Self {
            tool_calls: tool_calls.to_vec(),
            tool_summaries,
            headline: render_group_headline_with_locale(
                tool_calls,
                tool_defs,
                ToolNarrationPhase::Waiting,
                locale,
            ),
            completed_headline: render_group_headline_with_locale(
                tool_calls,
                tool_defs,
                ToolNarrationPhase::Completed,
                locale,
            ),
        }
    }
}

// ============================================================================
// LLM Event Data Types
// ============================================================================

/// LLM generation output
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmGenerationOutput {
    /// Text response from the model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    /// Tool calls requested by the model
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

/// Request options applied to an LLM generation.
///
/// These fields capture request-side intent such as prompt caching or deferred
/// tool loading. They complement `usage`, which captures what actually happened.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmRequestOptions {
    /// Prompt caching configuration for this request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache: Option<LlmPromptCacheInfo>,
    /// Deferred tool-loading configuration for this request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_search: Option<LlmToolSearchInfo>,
    /// Provider-specific request options that do not warrant dedicated fields.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub provider_options: HashMap<String, Value>,
    /// General request metadata passed to the LLM provider for tracking and observability.
    /// Includes embedder-supplied labels merged with system tracking keys (session_id, turn_id, etc.).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl LlmRequestOptions {
    pub fn is_empty(&self) -> bool {
        self.prompt_cache.is_none()
            && self.tool_search.is_none()
            && self.provider_options.is_empty()
            && self.metadata.is_empty()
    }
}

/// Request-side prompt cache settings for an LLM generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmPromptCacheInfo {
    /// Whether prompt caching was enabled on the request.
    pub enabled: bool,
    /// Strategy used to enable prompt caching.
    pub strategy: crate::driver_registry::PromptCacheStrategy,
    /// Provider-specific prompt-cache mode used by the driver.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_mode: Option<String>,
}

/// Request-side tool_search settings for an LLM generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmToolSearchInfo {
    /// Whether tool_search was enabled on the request.
    pub enabled: bool,
    /// Minimum number of tools before deferred loading activates.
    pub threshold: usize,
}

/// Metadata about an LLM generation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmGenerationMetadata {
    /// Model identifier used for generation
    #[cfg_attr(feature = "openapi", schema(example = "claude-sonnet-4-5"))]
    pub model: String,

    /// Provider type (openai, anthropic, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "anthropic"))]
    pub provider: Option<String>,

    /// Token usage statistics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,

    /// Duration of the generation in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 1_842u64))]
    pub duration_ms: Option<u64>,

    /// Time to first token in milliseconds (streaming latency)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 312u64))]
    pub time_to_first_token_ms: Option<u64>,

    /// Whether the generation was successful
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub success: bool,

    /// Error message if generation failed
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "provider returned 503"))]
    pub error: Option<String>,

    /// Finish reasons from the LLM (e.g., ["stop"], ["tool_calls"])
    /// Required for gen-ai semantic conventions
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = json!(["tool_calls"])))]
    pub finish_reasons: Option<Vec<String>>,

    /// Unique response identifier from the LLM provider
    /// Required for gen-ai semantic conventions
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "msg_01ABCDef0123456789"))]
    pub response_id: Option<String>,

    /// Retry information if rate limit retries occurred
    /// Contains number of retries and total wait time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<LlmRetryInfo>,

    /// Compaction information if context was compressed before generation
    /// Occurs when the conversation context exceeded the model's limit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compaction: Option<LlmCompactionInfo>,

    /// Request-side driver options that were enabled for this generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_options: Option<LlmRequestOptions>,
}

/// Information about rate limit retries during LLM generation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmRetryInfo {
    /// Number of retry attempts made (0 = succeeded on first try)
    pub attempts: u32,

    /// Total time spent waiting between retries in milliseconds
    pub total_wait_ms: u64,
}

/// Information about context compaction performed before LLM generation
///
/// When the conversation context exceeds the model's limit, compaction is
/// automatically triggered to compress the context before retrying.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmCompactionInfo {
    /// Whether compaction was performed
    pub compacted: bool,

    /// Number of input tokens before compaction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens_before: Option<u32>,

    /// Number of input tokens after compaction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens_after: Option<u32>,

    /// Duration of the compaction operation in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,

    /// Provider-reported cost of the compaction call itself, in USD.
    ///
    /// Compaction is a separate billable model call, so its cost is also folded
    /// into the generation's `usage.actual_cost_usd` — that is what budgets and
    /// `llm_generations` read. This field keeps the split visible, so an
    /// operator can see how much of a turn's spend was compaction (EVE-895).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

impl LlmCompactionInfo {
    /// Create info for a successful compaction
    pub fn new(
        input_tokens_before: Option<u32>,
        input_tokens_after: Option<u32>,
        duration_ms: Option<u64>,
        cost_usd: Option<f64>,
    ) -> Self {
        Self {
            compacted: true,
            input_tokens_before,
            input_tokens_after,
            duration_ms,
            cost_usd,
        }
    }
}

/// Data for llm.generation event
///
/// Emitted after each LLM API call to provide full visibility into
/// the messages sent to the model and the response received.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmGenerationData {
    /// Messages sent to the LLM (including system prompt)
    pub messages: Vec<Message>,

    /// Tools available to the LLM for this generation
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinitionSummary>,

    /// Output from the LLM
    pub output: LlmGenerationOutput,

    /// Metadata about the generation
    pub metadata: LlmGenerationMetadata,
}

impl LlmGenerationData {
    /// Create a successful generation event
    #[allow(clippy::too_many_arguments)]
    pub fn success(
        messages: Vec<Message>,
        tools: Vec<ToolDefinitionSummary>,
        text: Option<String>,
        tool_calls: Vec<ToolCall>,
        model: String,
        provider: Option<String>,
        usage: Option<TokenUsage>,
        duration_ms: Option<u64>,
        time_to_first_token_ms: Option<u64>,
    ) -> Self {
        // Infer finish reasons from content
        let finish_reasons = if !tool_calls.is_empty() {
            Some(vec!["tool_calls".to_string()])
        } else {
            Some(vec!["stop".to_string()])
        };

        Self {
            messages,
            tools,
            output: LlmGenerationOutput { text, tool_calls },
            metadata: LlmGenerationMetadata {
                model,
                provider,
                usage,
                duration_ms,
                time_to_first_token_ms,
                success: true,
                error: None,
                finish_reasons,
                response_id: None,
                retry: None,
                compaction: None,
                request_options: None,
            },
        }
    }

    /// Create a successful generation event with full metadata
    #[allow(clippy::too_many_arguments)]
    pub fn success_with_metadata(
        messages: Vec<Message>,
        tools: Vec<ToolDefinitionSummary>,
        text: Option<String>,
        tool_calls: Vec<ToolCall>,
        model: String,
        provider: Option<String>,
        usage: Option<TokenUsage>,
        duration_ms: Option<u64>,
        time_to_first_token_ms: Option<u64>,
        finish_reasons: Option<Vec<String>>,
        response_id: Option<String>,
    ) -> Self {
        Self {
            messages,
            tools,
            output: LlmGenerationOutput { text, tool_calls },
            metadata: LlmGenerationMetadata {
                model,
                provider,
                usage,
                duration_ms,
                time_to_first_token_ms,
                success: true,
                error: None,
                finish_reasons,
                response_id,
                retry: None,
                compaction: None,
                request_options: None,
            },
        }
    }

    /// Create a successful generation event with retry information
    #[allow(clippy::too_many_arguments)]
    pub fn success_with_retry(
        messages: Vec<Message>,
        tools: Vec<ToolDefinitionSummary>,
        text: Option<String>,
        tool_calls: Vec<ToolCall>,
        model: String,
        provider: Option<String>,
        usage: Option<TokenUsage>,
        duration_ms: Option<u64>,
        time_to_first_token_ms: Option<u64>,
        finish_reasons: Option<Vec<String>>,
        response_id: Option<String>,
        retry: Option<LlmRetryInfo>,
    ) -> Self {
        Self {
            messages,
            tools,
            output: LlmGenerationOutput { text, tool_calls },
            metadata: LlmGenerationMetadata {
                model,
                provider,
                usage,
                duration_ms,
                time_to_first_token_ms,
                success: true,
                error: None,
                finish_reasons,
                response_id,
                retry,
                compaction: None,
                request_options: None,
            },
        }
    }

    /// Create a failed generation event
    pub fn failure(
        messages: Vec<Message>,
        tools: Vec<ToolDefinitionSummary>,
        model: String,
        provider: Option<String>,
        error: String,
        duration_ms: Option<u64>,
        time_to_first_token_ms: Option<u64>,
    ) -> Self {
        Self {
            messages,
            tools,
            output: LlmGenerationOutput {
                text: None,
                tool_calls: vec![],
            },
            metadata: LlmGenerationMetadata {
                model,
                provider,
                usage: None,
                duration_ms,
                time_to_first_token_ms,
                success: false,
                error: Some(error),
                finish_reasons: Some(vec!["error".to_string()]),
                response_id: None,
                retry: None,
                compaction: None,
                request_options: None,
            },
        }
    }

    /// Set compaction info on this generation event
    ///
    /// Call this when context was compacted before a successful retry.
    pub fn with_compaction(mut self, compaction: LlmCompactionInfo) -> Self {
        self.metadata.compaction = Some(compaction);
        self
    }

    /// Set retry info on this generation event
    pub fn with_retry(mut self, retry: LlmRetryInfo) -> Self {
        self.metadata.retry = Some(retry);
        self
    }

    /// Set request-side options on this generation event.
    pub fn with_request_options(mut self, request_options: LlmRequestOptions) -> Self {
        if !request_options.is_empty() {
            self.metadata.request_options = Some(request_options);
        }
        self
    }
}

// ============================================================================
// Extended Thinking Event Data Types
// ============================================================================

/// Data for reason.thinking.started event
///
/// Emitted when extended thinking begins during reasoning phase.
/// This signals the model is using chain-of-thought reasoning.
/// UI can show a "thinking" indicator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonThinkingStartedData {
    /// Turn ID this thinking belongs to
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Optional model name being used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Data for reason.thinking.delta event (extended thinking content from models like Claude)
///
/// This event streams incremental thinking/reasoning content from models that support
/// extended thinking mode (e.g., Claude with thinking enabled). The thinking content
/// represents the model's chain-of-thought reasoning before producing the final response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonThinkingDeltaData {
    /// Turn ID this delta belongs to (for correlation)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// The thinking delta (new thinking text since last delta)
    pub delta: String,

    /// Accumulated thinking text so far (convenience for UI)
    pub accumulated: String,
}

/// Data for reason.thinking.completed event
///
/// Emitted when extended thinking completes and the model transitions
/// to producing the final response. Contains the complete thinking content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonThinkingCompletedData {
    /// Turn ID this thinking belongs to
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Complete thinking content
    pub thinking: String,
}

/// Data for `reason.item` event.
///
/// Durable record of one provider reasoning artifact.
///
/// Carries identity and curated summary text only. Opaque replay state
/// (signatures, encrypted reasoning context) is deliberately absent: it is
/// state the driver hands back to the provider, not content, and it must not
/// reach an event stream or any API surface. Plaintext chain-of-thought is
/// likewise never persisted here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonItemData {
    /// Turn ID this reasoning item belongs to.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Provider that produced the reasoning item (e.g., "openai").
    pub provider: String,

    /// Model identifier reported by the provider, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Provider-assigned identifier for the reasoning item.
    pub item_id: String,

    /// Safe summary text segments curated by the provider. Never includes
    /// plaintext reasoning content.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summary: Vec<String>,

    /// Per-item reasoning token count, when the provider reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_count: Option<u32>,
}

// ============================================================================
// Turn Event Data Types
// ============================================================================

/// Data for turn.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TurnStartedData {
    /// Turn identifier
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Input message ID that triggered this turn
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "message_01933b5a00007000800000000000001"))]
    pub input_message_id: MessageId,

    /// Input message content (for observability)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_content: Option<String>,
}

/// Data for turn.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TurnCompletedData {
    /// Turn identifier
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Number of iterations in this turn
    pub iterations: u32,

    /// Duration in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,

    /// Aggregated token usage for all LLM calls in this turn
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,

    /// Input message content (for observability, passed through from turn.started)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_content: Option<String>,

    /// Canonical assistant message emitted by `output.message.completed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "message_01933b5a00007000800000000000001"))]
    pub final_message_id: Option<MessageId>,

    /// Bounded preview of the final visible assistant answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_answer_preview: Option<String>,

    /// First-token latency for the turn, usually from the first LLM generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<u64>,

    /// Number of tool calls completed during the turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_count: Option<u32>,

    /// Number of LLM generation calls executed during the turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_call_count: Option<u32>,

    /// Optional explicit completion status for consumers that summarize turns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Data for turn.failed event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TurnFailedData {
    /// Turn identifier
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Error message
    pub error: String,

    /// Error code
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,

    /// Structured interpolation fields for localized error rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub error_fields: Option<UserFacingErrorFields>,

    /// Error-disclosure mode applied to `error_code`/`error_fields`
    /// ("generic" | "standard" | "detailed"). Full diagnostic detail remains
    /// available to operators via reason.completed failure events and tracing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_disclosure: Option<String>,
}

/// Data for turn.sealed event (EVE-534).
///
/// A sealed turn was deliberately stopped to prevent waste. It is observably
/// distinct from `turn.completed` (success) and `turn.failed` (error). The
/// `reason` is the durable engine's stable seal-reason wire value.
/// (`"no_progress"` or `"budget"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TurnSealedData {
    /// Turn identifier
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Why the turn was sealed: `"no_progress"` (crash-loop with no forward
    /// progress) or `"budget"` (work budget exhausted).
    pub reason: String,

    /// Human-readable detail for operators (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,

    /// Iterations completed before the turn was sealed (if known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,

    /// Aggregated token usage before sealing, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// Data for turn.cancelled event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TurnCancelledData {
    /// Turn identifier
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Reason for cancellation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Token usage before cancellation (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

// ============================================================================
// Session Event Data Types
// ============================================================================

/// Data for session.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionStartedData {
    /// Harness ID
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "harness_01933b5a00007000800000000000001"))]
    pub harness_id: HarnessId,

    /// Agent ID (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001"))]
    pub agent_id: Option<AgentId>,

    /// Model ID if specified
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001"))]
    pub model_id: Option<ModelId>,
}

/// Data for session.activated event (turn started, session now active)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionActivatedData {
    /// Turn ID that activated the session
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Input message ID that triggered the turn
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "message_01933b5a00007000800000000000001"))]
    pub input_message_id: MessageId,
}

/// Data for session.idled event (turn completed, session now idle)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionIdledData {
    /// Turn ID that just completed
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Number of iterations in the completed turn
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,

    /// Cumulative token usage for the session at this point
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// Data for `session.title.updated`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionTitleUpdatedData {
    /// Title before the mutation. `None` means the session was untitled.
    pub previous_title: Option<String>,

    /// New session title.
    pub title: String,
}

// ============================================================================
// Session task event data
// ============================================================================

/// Data for task lifecycle events (`task.created`, `task.updated`).
///
/// Carries the full task snapshot so consumers never need a follow-up read;
/// UIs reconcile by `task.id` (snapshot-then-delta).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionTaskEventData {
    pub task: crate::session_task::SessionTask,
}

/// Data for task message events (`task.message.sent`, `task.message.received`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TaskMessageEventData {
    pub task_id: String,
    pub message: crate::session_task::TaskMessage,
}

// ============================================================================
// Context compaction event data
// ============================================================================

/// Reason why compaction was triggered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CompactionReason {
    /// Triggered proactively at budget threshold.
    ProactiveBudget,
    /// Triggered reactively on RequestTooLarge error.
    RequestTooLarge,
    /// Triggered manually by user command.
    Manual,
}

impl std::fmt::Display for CompactionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProactiveBudget => write!(f, "proactive_budget"),
            Self::RequestTooLarge => write!(f, "request_too_large"),
            Self::Manual => write!(f, "manual"),
        }
    }
}

/// Data for context.compacting event (compaction starting).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ContextCompactingData {
    /// Why compaction was triggered.
    pub reason: CompactionReason,
    /// Strategy requested (may differ from strategy_used in the completed event).
    pub strategy: String,
    /// Number of messages before compaction.
    pub messages_before: usize,
    /// Estimated or provider-reported input tokens before compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_before: Option<u64>,
    /// Serialized request-context bytes before compaction, when measurable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_before: Option<u64>,
}

/// A single step in a compaction cascade.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CompactionStepData {
    /// Strategy used in this step.
    pub strategy: String,
    /// Number of messages after this step.
    pub messages_after: usize,
    /// Duration of this step in milliseconds.
    pub duration_ms: u64,
}

/// Data for context.compacted event (compaction completed).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ContextCompactedData {
    /// Durable checkpoint installed by this compaction, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<String>,
    /// Combined strategy description (e.g., "observation_masking+native").
    pub strategy_used: String,
    /// Number of messages before compaction.
    pub messages_before: usize,
    /// Number of messages after compaction.
    pub messages_after: usize,
    /// Estimated or provider-reported input tokens before compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_before: Option<u64>,
    /// Provider-reported output tokens after compaction, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_after: Option<u64>,
    /// Serialized request-context bytes before compaction, when measurable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_before: Option<u64>,
    /// Serialized compact output bytes, when measurable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_after: Option<u64>,
    /// Total duration of all compaction steps in milliseconds.
    pub duration_ms: u64,
    /// Individual steps in the cascade.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<CompactionStepData>,
}

// ============================================================================
// File event data
// ============================================================================

/// Data for file.written events emitted when files are written to the session filesystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FileWrittenData {
    /// File path within the session filesystem (normalized, e.g. "/reports/summary.md").
    pub path: String,
    /// Operation type (see `FILE_OP_*` constants).
    pub operation: String,
    /// File size in bytes after write.
    pub size_bytes: i64,
    /// Whether this is a new file (true) or an update to an existing file (false).
    pub created: bool,
}

/// File operation constants for `FileWrittenData.operation`.
pub const FILE_OP_CREATE: &str = "create";
pub const FILE_OP_UPDATE: &str = "update";

// ============================================================================
// Budget event data
// ============================================================================

/// Data for budget lifecycle events (warning, paused, exhausted, resumed).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct BudgetEventData {
    /// Budget that triggered this event.
    pub budget_id: String,
    /// Current remaining balance.
    pub balance: f64,
    /// Budget limit.
    pub limit: f64,
    /// Budget currency (e.g. "usd", "tokens").
    pub currency: String,
    /// Human-readable message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Soft limit threshold (present for warning/paused events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soft_limit: Option<f64>,
}

// ============================================================================
// Voice Event Data Types
// ============================================================================

/// Data for voice.session.started.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VoiceSessionStartedData {
    /// Prefixed voice connection identifier for the started session.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "voice_01933b5a00007000800000000000001")
    )]
    pub voice_connection_id: String,
    /// Provider-side realtime model identifier negotiated for this session.
    #[cfg_attr(feature = "openapi", schema(example = "gpt-realtime"))]
    pub model: String,
    /// Realtime voice preset selected for this session.
    #[cfg_attr(feature = "openapi", schema(example = "alloy"))]
    pub voice: String,
    /// Reasoning effort applied to the realtime model. One of `low`, `medium`, `high`.
    #[cfg_attr(feature = "openapi", schema(example = "medium"))]
    pub reasoning_effort: String,
    /// Transport carrying the audio stream. One of `webrtc`, `sip`, `websocket`.
    #[cfg_attr(feature = "openapi", schema(example = "webrtc"))]
    pub transport: String,
}

/// Data for voice transcript delta/completed events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VoiceTranscriptData {
    /// Prefixed voice connection identifier this transcript belongs to.
    pub voice_connection_id: String,
    /// Provider-specific identifier of the conversation item being transcribed. `None` when not yet assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    /// Provider-specific identifier of the response stream emitting this transcript. `None` for user-side transcripts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    /// Transcript phase: `user_partial`, `user_final`, `assistant_partial`, `assistant_final`. `None` when not yet classified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Newly transcribed text chunk delivered in this event. Empty for "final" events that only mark completion.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub delta: String,
    /// Full transcript accumulated for this item up to and including `delta`.
    pub accumulated: String,
}

/// Data for voice.session.ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VoiceSessionEndedData {
    /// Prefixed voice connection identifier for the ended session.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "voice_01933b5a00007000800000000000001")
    )]
    pub voice_connection_id: String,
    /// Free-text end reason captured from the client or server. `None` when no reason was supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(example = "User hung up after refund confirmed.")
    )]
    pub reason: Option<String>,
    /// Total wall-clock duration of the connection in milliseconds. `None` when the connection
    /// never completed an audio handshake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 184_500_u64))]
    pub duration_ms: Option<u64>,
}

/// Data for voice.session.failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VoiceSessionFailedData {
    /// Prefixed voice connection identifier for the failed session.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "voice_01933b5a00007000800000000000001")
    )]
    pub voice_connection_id: String,
    /// Error message captured at failure. Provider-formatted; not stable for parsing.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "realtime provider closed stream: 1011 internal_error")
    )]
    pub error: String,
}

// ============================================================================
// EventData Enum - Typed event payloads
// ============================================================================

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

    // Session task lifecycle events (full snapshots)
    TaskCreated(SessionTaskEventData),
    TaskUpdated(SessionTaskEventData),
    TaskMessageSent(TaskMessageEventData),
    TaskMessageReceived(TaskMessageEventData),

    // Context compaction events
    ContextCompacting(ContextCompactingData),
    ContextCompacted(ContextCompactedData),

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

    // Context compaction events
    ContextCompacting(ContextCompactingData) = CONTEXT_COMPACTING,
    ContextCompacted(ContextCompactedData) = CONTEXT_COMPACTED,

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
    ContextCompactingData => ContextCompacting,
    ContextCompactedData => ContextCompacted,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver_registry::PromptCacheStrategy;
    use serde_json::json;
    use std::collections::HashMap;

    /// Build an assistant message whose reasoning part carries every kind of
    /// opaque replay state, plus readable text that must survive.
    fn message_with_replay_state() -> Message {
        let mut message = Message::assistant("the answer");
        message.content.push(ContentPart::reasoning(
            everruns_provider::reasoning::ReasoningContentPart::opaque("anthropic")
                .with_item_id("rs_abc")
                .with_signature("sig-do-not-publish")
                .with_encrypted("enc-do-not-publish")
                .with_text(everruns_provider::reasoning::ReasoningText::Plain {
                    text: "visible reasoning".to_string(),
                }),
        ));
        message
    }

    fn generation_metadata() -> LlmGenerationMetadata {
        LlmGenerationMetadata {
            model: "test-model".to_string(),
            provider: Some("anthropic".to_string()),
            usage: None,
            duration_ms: None,
            time_to_first_token_ms: None,
            success: true,
            error: None,
            finish_reasons: None,
            response_id: None,
            retry: None,
            compaction: None,
            request_options: None,
        }
    }

    fn reasoning_part(message: &Message) -> &everruns_provider::reasoning::ReasoningContentPart {
        message
            .content
            .iter()
            .find_map(|part| match part {
                ContentPart::Reasoning(r) => Some(r),
                _ => None,
            })
            .expect("reasoning part")
    }

    /// The events read path must strip the same replay state the message read
    /// path strips, or `GET .../events` republishes what `GET .../messages`
    /// withholds (EVE-933).
    #[test]
    fn into_public_strips_replay_state_from_completed_messages() {
        let data = EventData::OutputMessageCompleted(OutputMessageCompletedData::new(
            message_with_replay_state(),
        ));

        let EventData::OutputMessageCompleted(public) = data.into_public() else {
            panic!("variant must be preserved");
        };
        let part = reasoning_part(&public.message);

        assert_eq!(part.signature, None, "signature must not be published");
        assert_eq!(
            part.encrypted, None,
            "encrypted payload must not be published"
        );
        assert_eq!(
            part.item_id.as_deref(),
            Some("rs_abc"),
            "the provider id is an identifier, not replay state"
        );
        assert_eq!(
            part.display_text().as_deref(),
            Some("visible reasoning"),
            "readable reasoning is content and must survive"
        );
    }

    #[test]
    fn into_public_strips_replay_state_from_input_and_generation_messages() {
        let EventData::InputMessage(input) =
            EventData::InputMessage(InputMessageData::new(message_with_replay_state()))
                .into_public()
        else {
            panic!("variant must be preserved");
        };
        assert_eq!(reasoning_part(&input.message).signature, None);

        // The prompt replayed to the provider carries the same artifacts.
        let generation = LlmGenerationData {
            messages: vec![message_with_replay_state()],
            tools: Vec::new(),
            output: LlmGenerationOutput {
                text: Some("the answer".to_string()),
                tool_calls: Vec::new(),
            },
            metadata: generation_metadata(),
        };
        let EventData::LlmGeneration(public) = EventData::LlmGeneration(generation).into_public()
        else {
            panic!("variant must be preserved");
        };
        assert_eq!(reasoning_part(&public.messages[0]).signature, None);
        assert_eq!(reasoning_part(&public.messages[0]).encrypted, None);
    }

    /// `needs_public_projection` gates the SSE clone, so a variant it misses is
    /// a variant that leaks.
    #[test]
    fn needs_public_projection_covers_every_message_bearing_variant() {
        assert!(
            EventData::InputMessage(InputMessageData::new(Message::user("hi")))
                .needs_public_projection()
        );
        assert!(
            EventData::OutputMessageCompleted(OutputMessageCompletedData::new(Message::assistant(
                "hi"
            )))
            .needs_public_projection()
        );
        assert!(
            EventData::LlmGeneration(LlmGenerationData {
                messages: Vec::new(),
                tools: Vec::new(),
                output: LlmGenerationOutput {
                    text: None,
                    tool_calls: Vec::new(),
                },
                metadata: generation_metadata(),
            })
            .needs_public_projection()
        );
        assert!(
            !EventData::ReasonItem(ReasonItemData {
                turn_id: TurnId::new(),
                provider: "openai".to_string(),
                model: None,
                item_id: "rs_abc".to_string(),
                summary: Vec::new(),
                token_count: None,
            })
            .needs_public_projection(),
            "a message-free variant must not pay for a clone"
        );
    }

    #[test]
    fn test_event_creation() {
        let session_id = SessionId::new();
        let context = EventContext::empty();
        let data = InputMessageData::new(Message::user("test"));

        let event = Event::new(session_id, context, data);

        assert_eq!(event.event_type, "input.message");
        assert_eq!(event.session_uuid(), session_id.uuid());
        assert!(event.is_input_event());
        assert!(event.is_message_event());
    }

    #[test]
    fn test_event_context_from_execution_context() {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();

        let atom_ctx = ExecutionContext::new(session_id, turn_id, input_message_id);
        let context = EventContext::from_execution_context(&atom_ctx);

        assert_eq!(context.turn_id, Some(turn_id));
        assert_eq!(context.input_message_id, Some(input_message_id));
        assert_eq!(context.exec_id, Some(atom_ctx.exec_id));
    }

    #[test]
    fn test_event_serialization() {
        let session_id = SessionId::new();
        let context = EventContext::empty();
        let event = Event::new(
            session_id,
            context,
            InputMessageData::new(Message::user("test")),
        );

        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"type\":\"input.message\""));
        assert!(json.contains("\"session_id\""));
        assert!(json.contains("\"context\""));
        assert!(json.contains("\"data\""));
    }

    #[test]
    fn transcript_repaired_is_valid_filter_event_type() {
        assert!(VALID_EVENT_TYPES.contains(&TRANSCRIPT_REPAIRED));
    }

    /// `capability.usage` is emitted (`EventData::CapabilityUsage`) and documented
    /// as streamable, so the public `types`/`exclude` filter allowlist must accept
    /// it instead of rejecting it as an unknown event type.
    #[test]
    fn capability_usage_is_valid_filter_event_type() {
        assert!(VALID_EVENT_TYPES.contains(&CAPABILITY_USAGE));
    }

    #[test]
    fn test_event_builder() {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();
        let exec_id = ExecId::new();

        let event = EventBuilder::new(session_id)
            .with_turn(turn_id, input_message_id)
            .with_exec(exec_id)
            .build(ReasonStartedData {
                harness_id: HarnessId::from_seed(1),
                agent_id: Some(AgentId::new()),
                metadata: Some(ModelMetadata {
                    model: "gpt-4o".to_string(),
                    model_id: None,
                    provider_id: None,
                }),
            });

        assert_eq!(event.event_type, "reason.started");
        assert_eq!(event.session_id, session_id);
        assert_eq!(event.context.turn_id, Some(turn_id));
        assert_eq!(event.context.exec_id, Some(exec_id));
    }

    #[test]
    fn test_reason_completed_data() {
        let data = ReasonCompletedData::success("Hello world", true, 2, Some(1000), None);
        assert!(data.success);
        assert_eq!(data.text_preview, Some("Hello world".to_string()));
        assert!(data.has_tool_calls);
        assert_eq!(data.tool_call_count, 2);
        assert_eq!(data.duration_ms, Some(1000));
        assert!(data.usage.is_none());

        let data = ReasonCompletedData::failure("Network error".to_string(), Some(500));
        assert!(!data.success);
        assert_eq!(data.error, Some("Network error".to_string()));
        assert_eq!(data.duration_ms, Some(500));
    }

    #[test]
    fn test_input_output_event_types() {
        assert_eq!(INPUT_MESSAGE, "input.message");
        assert_eq!(OUTPUT_MESSAGE_STARTED, "output.message.started");
        assert_eq!(OUTPUT_MESSAGE_DELTA, "output.message.delta");
        assert_eq!(OUTPUT_MESSAGE_COMPLETED, "output.message.completed");
    }

    #[test]
    fn test_turn_event_types() {
        assert_eq!(TURN_STARTED, "turn.started");
        assert_eq!(TURN_COMPLETED, "turn.completed");
        assert_eq!(TURN_FAILED, "turn.failed");
        assert_eq!(TURN_CANCELLED, "turn.cancelled");
    }

    #[test]
    fn test_turn_cancelled_data() {
        let data = TurnCancelledData {
            turn_id: TurnId::from_uuid(Uuid::now_v7()),
            reason: Some("User requested cancellation".to_string()),
            usage: Some(TokenUsage::new(100, 50)),
        };

        let event_data: EventData = data.into();
        assert_eq!(event_data.event_type(), TURN_CANCELLED);
    }

    #[test]
    fn test_tool_event_types() {
        assert_eq!(TOOL_STARTED, "tool.started");
        assert_eq!(TOOL_COMPLETED, "tool.completed");
    }

    #[test]
    fn test_llm_generation_event_type() {
        assert_eq!(LLM_GENERATION, "llm.generation");
    }

    #[test]
    fn test_llm_generation_data_success() {
        let messages = vec![Message::user("Hello"), Message::assistant("Hi there!")];
        let tools = vec![ToolDefinitionSummary {
            name: "get_weather".to_string(),
            display_name: None,
            category: None,
            capability_id: None,
            capability_name: None,
            description: "Get weather for a city".to_string(),
        }];
        let tool_calls = vec![];
        let data = LlmGenerationData::success(
            messages.clone(),
            tools,
            Some("Hi there!".to_string()),
            tool_calls,
            "gpt-4o".to_string(),
            Some("openai".to_string()),
            Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                actual_cost_usd: None,
                estimated_cost_usd: None,
                effective_cost_usd: None,
            }),
            Some(100),
            Some(25), // time_to_first_token_ms
        );

        assert_eq!(data.messages.len(), 2);
        assert_eq!(data.tools.len(), 1);
        assert_eq!(data.tools[0].name, "get_weather");
        assert_eq!(data.output.text, Some("Hi there!".to_string()));
        assert!(data.output.tool_calls.is_empty());
        assert!(data.metadata.success);
        assert_eq!(data.metadata.model, "gpt-4o");
        assert_eq!(data.metadata.provider, Some("openai".to_string()));
        assert!(data.metadata.error.is_none());
        // New fields for gen-ai semantic conventions
        assert_eq!(data.metadata.finish_reasons, Some(vec!["stop".to_string()]));
        assert!(data.metadata.response_id.is_none());
    }

    #[test]
    fn test_llm_generation_data_with_full_metadata() {
        let messages = vec![Message::user("Hello")];
        let data = LlmGenerationData::success_with_metadata(
            messages,
            vec![],
            Some("Hi!".to_string()),
            vec![],
            "claude-opus-5".to_string(),
            Some("anthropic".to_string()),
            Some(TokenUsage {
                input_tokens: 5,
                output_tokens: 3,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                actual_cost_usd: None,
                estimated_cost_usd: None,
                effective_cost_usd: None,
            }),
            Some(50),
            Some(25), // time_to_first_token_ms
            Some(vec!["end_turn".to_string()]),
            Some("msg_12345".to_string()),
        );

        assert!(data.metadata.success);
        assert_eq!(data.metadata.model, "claude-opus-5");
        assert_eq!(data.metadata.provider, Some("anthropic".to_string()));
        assert_eq!(data.metadata.time_to_first_token_ms, Some(25));
        assert_eq!(
            data.metadata.finish_reasons,
            Some(vec!["end_turn".to_string()])
        );
        assert_eq!(data.metadata.response_id, Some("msg_12345".to_string()));
    }

    #[test]
    fn test_llm_generation_data_failure() {
        let messages = vec![Message::user("Hello")];
        let data = LlmGenerationData::failure(
            messages,
            vec![],
            "gpt-4o".to_string(),
            Some("openai".to_string()),
            "Rate limit exceeded".to_string(),
            Some(50),
            None, // time_to_first_token_ms
        );

        assert!(!data.metadata.success);
        assert_eq!(data.metadata.error, Some("Rate limit exceeded".to_string()));
        assert!(data.output.text.is_none());
        assert!(data.output.tool_calls.is_empty());
    }

    #[test]
    fn test_llm_generation_event_data() {
        let data = LlmGenerationData::success(
            vec![Message::user("test")],
            vec![],
            Some("response".to_string()),
            vec![],
            "model".to_string(),
            None,
            None,
            None,
            None, // time_to_first_token_ms
        );

        let event_data: EventData = data.into();
        assert_eq!(event_data.event_type(), LLM_GENERATION);
    }

    #[test]
    fn test_llm_generation_is_durable_not_ephemeral() {
        let session_id = SessionId::new();
        let data = LlmGenerationData::success(
            vec![Message::user("test")],
            vec![],
            Some("response".to_string()),
            vec![],
            "model".to_string(),
            None,
            None,
            None,
            None,
        );

        let request = EventRequest::new(session_id, EventContext::empty(), data);
        assert!(!request.is_ephemeral());
    }

    #[test]
    fn test_delta_events_are_ephemeral() {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();

        let output_delta = EventRequest::new(
            session_id,
            EventContext::empty(),
            OutputMessageDeltaData {
                turn_id,
                message_id: MessageId::new(),
                delta: "hel".to_string(),
                accumulated: "hel".to_string(),
                phase: None,
            },
        );
        assert!(output_delta.is_ephemeral());

        let thinking_delta = EventRequest::new(
            session_id,
            EventContext::empty(),
            ReasonThinkingDeltaData {
                turn_id,
                delta: "step".to_string(),
                accumulated: "step".to_string(),
            },
        );
        assert!(thinking_delta.is_ephemeral());

        let tool_delta = EventRequest::new(
            session_id,
            EventContext::empty(),
            ToolOutputDeltaData {
                tool_call_id: "call_123".to_string(),
                tool_name: "bash".to_string(),
                delta: "line".to_string(),
                stream: "stdout".to_string(),
            },
        );
        assert!(tool_delta.is_ephemeral());
    }

    #[test]
    fn test_llm_generation_data_with_request_options() {
        let mut provider_options = HashMap::new();
        provider_options.insert(
            "openai".to_string(),
            json!({ "previous_response_id": true }),
        );

        let data = LlmGenerationData::success(
            vec![Message::user("Hello")],
            vec![],
            Some("Hi".to_string()),
            vec![],
            "gpt-5.4".to_string(),
            Some("openai".to_string()),
            None,
            Some(42),
            Some(12),
        )
        .with_request_options(LlmRequestOptions {
            prompt_cache: Some(LlmPromptCacheInfo {
                enabled: true,
                strategy: PromptCacheStrategy::Auto,
                provider_mode: Some("prompt_cache_key".to_string()),
            }),
            tool_search: Some(LlmToolSearchInfo {
                enabled: true,
                threshold: 8,
            }),
            provider_options,
            metadata: Default::default(),
        });

        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(
            json["metadata"]["request_options"]["prompt_cache"]["provider_mode"],
            "prompt_cache_key"
        );
        assert_eq!(
            json["metadata"]["request_options"]["tool_search"]["threshold"],
            8
        );
        assert_eq!(
            json["metadata"]["request_options"]["provider_options"]["openai"]["previous_response_id"],
            true
        );
    }

    #[test]
    fn test_extended_thinking_event_types() {
        assert_eq!(REASON_THINKING_STARTED, "reason.thinking.started");
        assert_eq!(REASON_THINKING_DELTA, "reason.thinking.delta");
        assert_eq!(REASON_THINKING_COMPLETED, "reason.thinking.completed");
    }

    #[test]
    fn test_output_message_started_data() {
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let data = OutputMessageStartedData {
            turn_id,
            message_id: MessageId::new(),
            model: Some("claude-4-opus".to_string()),
            iteration: None,
            phase: None,
        };

        let event_data: EventData = data.into();
        assert_eq!(event_data.event_type(), OUTPUT_MESSAGE_STARTED);

        // Test serialization
        let json = serde_json::to_string(&event_data).unwrap();
        assert!(json.contains("turn_id"));
        assert!(json.contains("claude-4-opus"));
    }

    #[test]
    fn test_output_message_started_data_without_model() {
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let data = OutputMessageStartedData {
            turn_id,
            message_id: MessageId::new(),
            model: None,
            iteration: None,
            phase: None,
        };

        // Model should be skipped when None
        let json = serde_json::to_string(&data).unwrap();
        assert!(!json.contains("model"));
    }

    #[test]
    fn test_reason_thinking_started_data() {
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let data = ReasonThinkingStartedData {
            turn_id,
            model: Some("claude-4-opus".to_string()),
        };

        let event_data: EventData = data.into();
        assert_eq!(event_data.event_type(), REASON_THINKING_STARTED);

        // Test serialization
        let json = serde_json::to_string(&event_data).unwrap();
        assert!(json.contains("turn_id"));
        assert!(json.contains("claude-4-opus"));
    }

    #[test]
    fn test_reason_thinking_delta_data() {
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let data = ReasonThinkingDeltaData {
            turn_id,
            delta: "thinking step 1".to_string(),
            accumulated: "thinking step 1".to_string(),
        };

        let event_data: EventData = data.into();
        assert_eq!(event_data.event_type(), REASON_THINKING_DELTA);

        // Test serialization
        let json = serde_json::to_string(&event_data).unwrap();
        assert!(json.contains("turn_id"));
        assert!(json.contains("delta"));
        assert!(json.contains("accumulated"));
    }

    #[test]
    fn test_reason_thinking_completed_data() {
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let data = ReasonThinkingCompletedData {
            turn_id,
            thinking: "Full thinking content here".to_string(),
        };

        let event_data: EventData = data.into();
        assert_eq!(event_data.event_type(), REASON_THINKING_COMPLETED);

        // Test serialization
        let json = serde_json::to_string(&event_data).unwrap();
        assert!(json.contains("turn_id"));
        assert!(json.contains("thinking"));
    }

    #[test]
    fn test_output_message_delta_data() {
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let data = OutputMessageDeltaData {
            turn_id,
            message_id: MessageId::new(),
            delta: "Hello".to_string(),
            accumulated: "Hello".to_string(),
            phase: None,
        };

        let event_data: EventData = data.into();
        assert_eq!(event_data.event_type(), OUTPUT_MESSAGE_DELTA);

        // Test serialization
        let json = serde_json::to_string(&event_data).unwrap();
        assert!(json.contains("turn_id"));
        assert!(json.contains("delta"));
        assert!(json.contains("accumulated"));
    }

    #[test]
    fn test_output_message_lifecycle_shares_message_id() {
        let turn_id = TurnId::new();
        let message_id = MessageId::new();
        let next_message_id = MessageId::new();

        let started = OutputMessageStartedData {
            turn_id,
            message_id,
            model: None,
            iteration: Some(1),
            phase: None,
        };
        let next_started = OutputMessageStartedData {
            turn_id,
            message_id: next_message_id,
            model: None,
            iteration: Some(2),
            phase: None,
        };
        let delta = OutputMessageDeltaData {
            turn_id,
            message_id,
            delta: "Hello".to_string(),
            accumulated: "Hello".to_string(),
            phase: None,
        };
        let replaced = OutputMessageReplacedData {
            turn_id,
            message_id,
            guardrail_capability_id: "guardrails".to_string(),
            guardrail_id: "example".to_string(),
            reason_code: "blocked".to_string(),
            replacement: "Safe response".to_string(),
        };
        let completed = OutputMessageCompletedData::new(
            Message::assistant("Safe response").with_id(message_id),
        );

        assert_eq!(started.message_id, message_id);
        assert_eq!(delta.message_id, message_id);
        assert_eq!(replaced.message_id, message_id);
        assert_eq!(completed.message.id, message_id);
        assert_ne!(started.message_id, next_started.message_id);

        for value in [
            serde_json::to_value(started).unwrap(),
            serde_json::to_value(delta).unwrap(),
            serde_json::to_value(replaced).unwrap(),
        ] {
            assert_eq!(value["message_id"], message_id.to_string());
        }
    }

    #[test]
    fn test_output_message_phase_hint_serde() {
        // EVE-774: the streamed phase hint is skipped when None (so existing
        // consumers see no new field) and serialized as the provider wire value
        // when present, on both started and delta events.
        let turn_id = TurnId::from_uuid(Uuid::now_v7());

        let started_none = OutputMessageStartedData {
            turn_id,
            message_id: MessageId::new(),
            model: None,
            iteration: None,
            phase: None,
        };
        assert!(
            !serde_json::to_string(&started_none)
                .unwrap()
                .contains("phase")
        );

        let delta_commentary = OutputMessageDeltaData {
            turn_id,
            message_id: MessageId::new(),
            delta: "one moment".to_string(),
            accumulated: "one moment".to_string(),
            phase: Some(everruns_provider::ExecutionPhase::Commentary),
        };
        let json = serde_json::to_value(&delta_commentary).unwrap();
        assert_eq!(json["phase"], "commentary");

        // Round-trips back to the same phase.
        let back: OutputMessageDeltaData = serde_json::from_value(json).unwrap();
        assert_eq!(
            back.phase,
            Some(everruns_provider::ExecutionPhase::Commentary)
        );

        let delta_final = OutputMessageDeltaData {
            turn_id,
            message_id: MessageId::new(),
            delta: "done".to_string(),
            accumulated: "done".to_string(),
            phase: Some(everruns_provider::ExecutionPhase::FinalAnswer),
        };
        assert_eq!(
            serde_json::to_value(&delta_final).unwrap()["phase"],
            "final_answer"
        );
    }

    #[test]
    fn test_output_message_delta_deserialization_preserves_fields() {
        // Verify OutputMessageDelta decodes with all fields preserved through the
        // type-driven dispatcher (regression guard for field-dropping decode bugs).
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let data = OutputMessageDeltaData {
            turn_id,
            message_id: MessageId::new(),
            delta: "Hello world".to_string(),
            accumulated: "Hello world".to_string(),
            phase: None,
        };

        // Serialize to JSON
        let json = serde_json::to_value(EventData::OutputMessageDelta(data.clone())).unwrap();

        // Deserialize back through the real (type-driven) decode path
        let deserialized = deserialize_event_data(OUTPUT_MESSAGE_DELTA, json);

        // Verify it's OutputMessageDelta and fields are preserved
        match deserialized {
            EventData::OutputMessageDelta(td) => {
                assert_eq!(td.turn_id, turn_id);
                assert_eq!(td.delta, "Hello world");
                assert_eq!(td.accumulated, "Hello world");
            }
            _ => panic!("Expected OutputMessageDelta, got different variant"),
        }
    }

    #[test]
    fn test_output_message_started_deserialization() {
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let data = OutputMessageStartedData {
            turn_id,
            message_id: MessageId::new(),
            model: Some("claude-opus-5".to_string()),
            iteration: None,
            phase: None,
        };

        // Serialize to JSON
        let json = serde_json::to_value(EventData::OutputMessageStarted(data.clone())).unwrap();

        // Deserialize back through the real (type-driven) decode path
        let deserialized = deserialize_event_data(OUTPUT_MESSAGE_STARTED, json);

        // Verify it's OutputMessageStarted and fields are preserved
        match deserialized {
            EventData::OutputMessageStarted(at) => {
                assert_eq!(at.turn_id, turn_id);
                assert_eq!(at.model, Some("claude-opus-5".to_string()));
            }
            _ => panic!("Expected OutputMessageStarted, got different variant"),
        }
    }

    #[test]
    fn test_reason_thinking_started_deserialization() {
        // NOTE: ReasonThinkingStartedData and OutputMessageStartedData have identical
        // structures (turn_id + model), so a payload alone can't distinguish them.
        // Decoding therefore goes through deserialize_event_data(), which selects the
        // correct variant from the event_type.
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let data = ReasonThinkingStartedData {
            turn_id,
            model: Some("claude-opus-5".to_string()),
        };

        // Serialize to JSON
        let json = serde_json::to_value(&data).unwrap();

        // Deserialize using typed function (not raw serde)
        let deserialized = deserialize_event_data(REASON_THINKING_STARTED, json);

        // Verify it's ReasonThinkingStarted and fields are preserved
        match deserialized {
            EventData::ReasonThinkingStarted(at) => {
                assert_eq!(at.turn_id, turn_id);
                assert_eq!(at.model, Some("claude-opus-5".to_string()));
            }
            other => panic!("Expected ReasonThinkingStarted, got {}", other.event_type()),
        }
    }

    #[test]
    fn test_llm_generation_with_ttft() {
        let messages = vec![Message::user("Hello")];
        let data = LlmGenerationData::success_with_metadata(
            messages,
            vec![],
            Some("Hi!".to_string()),
            vec![],
            "gpt-4o".to_string(),
            Some("openai".to_string()),
            Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                actual_cost_usd: None,
                estimated_cost_usd: None,
                effective_cost_usd: None,
            }),
            Some(500), // duration_ms
            Some(120), // time_to_first_token_ms
            Some(vec!["stop".to_string()]),
            None,
        );

        assert!(data.metadata.success);
        assert_eq!(data.metadata.duration_ms, Some(500));
        assert_eq!(data.metadata.time_to_first_token_ms, Some(120));
    }

    #[test]
    fn test_llm_generation_ttft_serialization() {
        let messages = vec![Message::user("test")];
        let data = LlmGenerationData::success_with_metadata(
            messages,
            vec![],
            Some("response".to_string()),
            vec![],
            "model".to_string(),
            None,
            None,
            Some(1000),
            Some(150), // TTFT
            None,
            None,
        );

        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("time_to_first_token_ms"));
        assert!(json.contains("150"));
    }

    #[test]
    fn test_reason_item_data_event_type_and_serialization() {
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let data = ReasonItemData {
            turn_id,
            provider: "openai".to_string(),
            model: Some("gpt-5.5".to_string()),
            item_id: "rs_abc".to_string(),
            summary: vec!["safe summary".to_string()],
            token_count: Some(123),
        };

        let event_data: EventData = data.into();
        assert_eq!(event_data.event_type(), REASON_ITEM);

        let json = serde_json::to_string(&event_data).unwrap();
        assert!(json.contains("turn_id"));
        assert!(json.contains("openai"));
        assert!(json.contains("rs_abc"));
        assert!(json.contains("safe summary"));
        // Opaque replay state is not event data. The event publishes identity
        // and curated summary only; signatures and encrypted context stay on
        // the message's reasoning parts, for the driver to hand back.
        assert!(!json.contains("encrypted_content"));
        assert!(!json.contains("signature"));
    }

    #[test]
    fn test_event_deserialize_reason_item_uses_event_type_dispatch() {
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let payload = serde_json::json!({
            "id": EventId::new().to_string(),
            "type": REASON_ITEM,
            "ts": Utc::now().to_rfc3339(),
            "session_id": SessionId::from_uuid(Uuid::now_v7()).to_string(),
            "context": {"trace_id": "t", "span_id": "s", "parent_span_id": null},
            "data": {
                "turn_id": turn_id.to_string(),
                "provider": "openai",
                "model": "gpt-5",
                "item_id": "rs_event",
                "summary": ["safe"],
                "token_count": 9
            }
        });

        let event: Event = serde_json::from_value(payload).expect("event deserializes");
        match event.data {
            EventData::ReasonItem(data) => {
                assert_eq!(data.turn_id, turn_id);
                assert_eq!(data.provider, "openai");
                assert_eq!(data.item_id, "rs_event");
                assert_eq!(data.token_count, Some(9));
            }
            other => panic!("expected reason.item data, got {}", other.event_type()),
        }
    }

    #[test]
    fn test_event_request_deserialize_reason_item_uses_event_type_dispatch() {
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let payload = serde_json::json!({
            "type": REASON_ITEM,
            "ts": Utc::now().to_rfc3339(),
            "session_id": SessionId::from_uuid(Uuid::now_v7()).to_string(),
            "context": {"trace_id": "t", "span_id": "s", "parent_span_id": null},
            "data": {
                "turn_id": turn_id.to_string(),
                "provider": "openai",
                "item_id": "rs_request",
                    "summary": ["safe"]
            }
        });

        let req: EventRequest = serde_json::from_value(payload).expect("request deserializes");
        match req.data {
            EventData::ReasonItem(data) => {
                assert_eq!(data.turn_id, turn_id);
                assert_eq!(data.provider, "openai");
                assert_eq!(data.item_id, "rs_request");
            }
            other => panic!("expected reason.item data, got {}", other.event_type()),
        }
    }

    #[test]
    fn test_reason_item_data_round_trip_uses_typed_dispatch() {
        // ReasonItemData carries (turn_id, item_id, provider...) which is
        // structurally close to other turn-scoped events. Verify the typed
        // dispatcher selects the correct variant.
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let data = ReasonItemData {
            turn_id,
            provider: "openai".to_string(),
            model: Some("gpt-5".to_string()),
            item_id: "rs_xyz".to_string(),
            summary: vec![],
            token_count: None,
        };

        let json = serde_json::to_value(&data).unwrap();
        let deserialized = deserialize_event_data(REASON_ITEM, json);

        match deserialized {
            EventData::ReasonItem(out) => {
                assert_eq!(out.turn_id, turn_id);
                assert_eq!(out.provider, "openai");
                assert_eq!(out.item_id, "rs_xyz");
            }
            other => panic!("Expected ReasonItem, got {}", other.event_type()),
        }
    }

    /// `ReasonThinkingStartedData` only requires `turn_id`, so a richer
    /// `reason.item` payload also satisfies it structurally. Decoding does not
    /// rely on serde to disambiguate (`EventData` has no `Deserialize` impl, so
    /// variant declaration order is irrelevant); `deserialize_event_data`
    /// selects the variant from the outer `type` string. Guard that the overlap
    /// exists and that type dispatch resolves `reason.item` to `ReasonItem`
    /// (keeping `provider`, `item_id`, …) rather than the looser
    /// `ReasonThinkingStarted`.
    #[test]
    fn test_reason_item_resolves_via_type_dispatch_despite_overlap() {
        // The two reasoning variants overlap structurally: ReasonThinkingStarted
        // (turn_id + optional model) accepts any superset, while ReasonItem
        // (turn_id + provider + item_id + …) is richer. Confirm both parse in
        // isolation, then that type dispatch picks ReasonItem.
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let json = serde_json::json!({
            "turn_id": turn_id.to_string(),
            "provider": "openai",
            "model": "gpt-5",
            "item_id": "rs_keep",
            "summary": ["s"],
            "token_count": 7,
        });

        // Both candidate structs accept the payload in isolation
        // (ReasonThinkingStarted ignores the extra fields), proving the overlap.
        // The canonical path disambiguates via the event_type, not via any
        // declaration order.
        let as_thinking: ReasonThinkingStartedData =
            serde_json::from_value(json.clone()).expect("thinking ignores extra fields");
        assert_eq!(as_thinking.turn_id, turn_id);
        assert_eq!(as_thinking.model.as_deref(), Some("gpt-5"));

        let as_item: ReasonItemData =
            serde_json::from_value(json.clone()).expect("ReasonItem accepts payload");
        assert_eq!(as_item.item_id, "rs_keep");
        assert_eq!(as_item.provider, "openai");

        // Canonical parse via type dispatch: this is the path used by Event
        // and EventRequest deserialization (see `deserialize_event_data`).
        let event_data = deserialize_event_data(REASON_ITEM, json);
        match event_data {
            EventData::ReasonItem(out) => {
                assert_eq!(out.item_id, "rs_keep");
                assert_eq!(out.provider, "openai");
            }
            other => panic!(
                "Typed dispatcher must select ReasonItem for {REASON_ITEM}, got {}",
                other.event_type()
            ),
        }
    }

    /// Regression guard for EVE-485: the persisted `reason.item` event must
    /// never carry plaintext hidden reasoning content, and must not carry the
    /// opaque replay state either — signatures and encrypted context are what
    /// the driver hands back to the provider, not event data.
    /// Assert structurally on parsed JSON keys rather than substrings so a
    /// payload value that happens to contain "content"/"thinking" cannot mask
    /// the guard.
    #[test]
    fn test_reason_item_data_excludes_plaintext_reasoning() {
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let data = ReasonItemData {
            turn_id,
            provider: "openai".to_string(),
            model: Some("gpt-5".to_string()),
            item_id: "rs_secret".to_string(),
            // Deliberately stuff the substrings the old guard checked into a
            // legitimate value to prove the structural check still rejects
            // them when present only as values.
            summary: vec!["safe summary mentioning content and thinking".to_string()],
            token_count: Some(1),
        };

        let value = serde_json::to_value(&data).expect("serializable");
        let object = value.as_object().expect("data serializes to JSON object");
        for forbidden in [
            "content",
            "reasoning_text",
            "thinking",
            "reasoning_content",
            "raw_reasoning",
            // Opaque replay state: carried on the message's reasoning parts and
            // handed back to the provider, never published as event data.
            "encrypted_content",
            "signature",
            "encrypted",
        ] {
            assert!(
                !object.contains_key(forbidden),
                "ReasonItemData JSON must not expose `{forbidden}` key, got: {object:?}",
            );
        }
        // The only sanctioned field that carries reasoning text.
        assert!(object.contains_key("summary"));
    }

    #[test]
    fn test_llm_generation_ttft_omitted_when_none() {
        let messages = vec![Message::user("test")];
        let data = LlmGenerationData::success(
            messages,
            vec![],
            Some("response".to_string()),
            vec![],
            "model".to_string(),
            None,
            None,
            None,
            None, // time_to_first_token_ms
        );

        // TTFT should be None when passed as None
        assert!(data.metadata.time_to_first_token_ms.is_none());

        // Should not appear in JSON when None
        let json = serde_json::to_string(&data).unwrap();
        assert!(!json.contains("time_to_first_token_ms"));
    }
}

// ============================================================================
// Contract Tests
// ============================================================================
//
// These tests validate the event protocol contract defined in knowledge/execution/events.md.
// Snapshot tests ensure JSON structure doesn't change accidentally.
// Forward compatibility tests verify unknown fields are handled correctly.

#[cfg(test)]
mod contract_tests {
    use super::*;
    use insta::{assert_json_snapshot, with_settings};

    /// Helper to create deterministic test IDs for snapshot stability
    fn test_session_id() -> SessionId {
        SessionId::from_uuid(uuid::Uuid::from_u128(
            0x0000_0000_0000_0000_0000_0000_0000_0001,
        ))
    }

    fn test_turn_id() -> TurnId {
        TurnId::from_uuid(uuid::Uuid::from_u128(
            0x0000_0000_0000_0000_0000_0000_0000_0002,
        ))
    }

    fn test_message_id() -> MessageId {
        MessageId::from_uuid(uuid::Uuid::from_u128(
            0x0000_0000_0000_0000_0000_0000_0000_0003,
        ))
    }

    fn test_agent_id() -> AgentId {
        AgentId::from_uuid(uuid::Uuid::from_u128(
            0x0000_0000_0000_0000_0000_0000_0000_0004,
        ))
    }

    fn test_harness_id() -> HarnessId {
        HarnessId::from_uuid(uuid::Uuid::from_u128(
            0x0000_0000_0000_0000_0000_0000_0000_0005,
        ))
    }

    // ========================================================================
    // Serialization Snapshot Tests
    // ========================================================================
    // These tests capture the canonical JSON representation of each event type.
    // Changes to these snapshots indicate a potential breaking change.

    #[test]
    fn snapshot_input_message() {
        let data = InputMessageData::new(Message::user("Hello, world!"));
        with_settings!({
            sort_maps => true,
        }, {
            // Redact volatile fields (id, created_at) to ensure snapshot stability
            assert_json_snapshot!("event_data_input_message", data, {
                ".message.id" => "[MESSAGE_ID]",
                ".message.created_at" => "[TIMESTAMP]"
            });
        });
    }

    #[test]
    fn snapshot_output_message_started() {
        let data = OutputMessageStartedData {
            turn_id: test_turn_id(),
            message_id: test_message_id(),
            model: Some("gpt-4o".to_string()),
            iteration: None,
            phase: None,
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_output_message_started", data);
        });
    }

    #[test]
    fn snapshot_output_message_delta() {
        let data = OutputMessageDeltaData {
            turn_id: test_turn_id(),
            message_id: test_message_id(),
            delta: "Hello".to_string(),
            accumulated: "Hello".to_string(),
            phase: None,
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_output_message_delta", data);
        });
    }

    #[test]
    fn snapshot_output_message_completed() {
        let data = OutputMessageCompletedData::new(Message::assistant("Hello!"));
        with_settings!({
            sort_maps => true,
        }, {
            // Redact volatile fields (id, created_at) to ensure snapshot stability
            assert_json_snapshot!("event_data_output_message_completed", data, {
                ".message.id" => "[MESSAGE_ID]",
                ".message.created_at" => "[TIMESTAMP]"
            });
        });
    }

    #[test]
    fn snapshot_turn_started() {
        let data = TurnStartedData {
            turn_id: test_turn_id(),
            input_message_id: test_message_id(),
            input_content: Some("Hello".to_string()),
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_turn_started", data);
        });
    }

    #[test]
    fn snapshot_turn_completed() {
        let data = TurnCompletedData {
            turn_id: test_turn_id(),
            iterations: 3,
            duration_ms: Some(1500),
            usage: Some(TokenUsage::new(100, 50)),
            input_content: None,
            final_message_id: Some(test_message_id()),
            final_answer_preview: Some("Done.".to_string()),
            time_to_first_token_ms: Some(120),
            tool_call_count: Some(2),
            llm_call_count: Some(3),
            status: Some("completed".to_string()),
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_turn_completed", data);
        });
    }

    #[test]
    fn snapshot_turn_failed() {
        let data = TurnFailedData {
            turn_id: test_turn_id(),
            error: "Rate limit exceeded".to_string(),
            error_code: Some("RATE_LIMIT".to_string()),
            error_fields: None,
            error_disclosure: None,
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_turn_failed", data);
        });
    }

    #[test]
    fn snapshot_turn_cancelled() {
        let data = TurnCancelledData {
            turn_id: test_turn_id(),
            reason: Some("User requested".to_string()),
            usage: Some(TokenUsage::new(50, 25)),
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_turn_cancelled", data);
        });
    }

    #[test]
    fn snapshot_reason_started() {
        let data = ReasonStartedData {
            harness_id: test_harness_id(),
            agent_id: Some(test_agent_id()),
            metadata: Some(ModelMetadata {
                model: "gpt-4o".to_string(),
                model_id: None,
                provider_id: None,
            }),
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_reason_started", data);
        });
    }

    #[test]
    fn snapshot_reason_completed() {
        let data = ReasonCompletedData::success(
            "Hello world",
            true,
            2,
            Some(1000),
            Some(TokenUsage::new(100, 50)),
        );
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_reason_completed", data);
        });
    }

    #[test]
    fn snapshot_act_started() {
        let data = ActStartedData {
            tool_calls: vec![ToolCallSummary {
                id: "tc_1".to_string(),
                name: "get_weather".to_string(),
                display_name: None,
                narration: None,
                completed_narration: None,
            }],
            headline: None,
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_act_started", data);
        });
    }

    #[test]
    fn snapshot_act_completed() {
        let data = ActCompletedData {
            completed: true,
            success_count: 2,
            error_count: 0,
            duration_ms: Some(500),
            headline: None,
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_act_completed", data);
        });
    }

    #[test]
    fn snapshot_tool_started() {
        let data = ToolStartedData {
            tool_call: ToolCall {
                id: "tc_1".to_string(),
                name: "get_weather".to_string(),
                arguments: serde_json::json!({"city": "London"}),
            },
            tool_call_fingerprint: None,
            display_name: None,
            narration: None,
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_tool_started", data);
        });
    }

    #[test]
    fn snapshot_tool_completed() {
        let data = ToolCompletedData::success(
            "tc_1".to_string(),
            "get_weather".to_string(),
            vec![crate::message::ContentPart::text("Sunny, 22°C")],
            Some(250),
        );
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_tool_completed", data);
        });
    }

    #[test]
    fn snapshot_llm_generation() {
        let data = LlmGenerationData::success(
            vec![Message::user("Hello")],
            vec![ToolDefinitionSummary {
                name: "tool1".to_string(),
                display_name: None,
                category: None,
                capability_id: None,
                capability_name: None,
                description: "A tool".to_string(),
            }],
            Some("Hi there!".to_string()),
            vec![],
            "gpt-4o".to_string(),
            Some("openai".to_string()),
            Some(TokenUsage::new(10, 5)),
            Some(100),
            Some(25),
        );
        with_settings!({
            sort_maps => true,
        }, {
            // Redact volatile fields (id, created_at) in messages array
            assert_json_snapshot!("event_data_llm_generation", data, {
                ".messages[].id" => "[MESSAGE_ID]",
                ".messages[].created_at" => "[TIMESTAMP]"
            });
        });
    }

    #[test]
    fn snapshot_reason_thinking_started() {
        let data = ReasonThinkingStartedData {
            turn_id: test_turn_id(),
            model: Some("claude-4-opus".to_string()),
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_reason_thinking_started", data);
        });
    }

    #[test]
    fn snapshot_reason_thinking_delta() {
        let data = ReasonThinkingDeltaData {
            turn_id: test_turn_id(),
            delta: "Let me think...".to_string(),
            accumulated: "Let me think...".to_string(),
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_reason_thinking_delta", data);
        });
    }

    #[test]
    fn snapshot_reason_thinking_completed() {
        let data = ReasonThinkingCompletedData {
            turn_id: test_turn_id(),
            thinking: "I need to consider...".to_string(),
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_reason_thinking_completed", data);
        });
    }

    #[test]
    fn snapshot_reason_item() {
        let data = ReasonItemData {
            turn_id: test_turn_id(),
            provider: "openai".to_string(),
            model: Some("gpt-5.5".to_string()),
            item_id: "rs_test".to_string(),
            summary: vec!["safe summary".to_string()],
            token_count: Some(42),
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_reason_item", data);
        });
    }

    #[test]
    fn snapshot_session_started() {
        let data = SessionStartedData {
            harness_id: test_harness_id(),
            agent_id: Some(test_agent_id()),
            model_id: None,
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_session_started", data);
        });
    }

    #[test]
    fn snapshot_session_activated() {
        let data = SessionActivatedData {
            turn_id: test_turn_id(),
            input_message_id: test_message_id(),
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_session_activated", data);
        });
    }

    #[test]
    fn snapshot_session_idled() {
        let data = SessionIdledData {
            turn_id: test_turn_id(),
            iterations: Some(3),
            usage: Some(TokenUsage::new(500, 200)),
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_session_idled", data);
        });
    }

    #[test]
    fn snapshot_session_title_updated() {
        let data = SessionTitleUpdatedData {
            previous_title: Some("Old title".to_string()),
            title: "Automatic Session Titles".to_string(),
        };
        with_settings!({
            sort_maps => true,
        }, {
            assert_json_snapshot!("event_data_session_title_updated", data);
        });

        let untitled = serde_json::to_value(SessionTitleUpdatedData {
            previous_title: None,
            title: "First title".to_string(),
        })
        .expect("serialize title event");
        assert!(untitled["previous_title"].is_null());
    }

    // ========================================================================
    // Display Name Tests
    // ========================================================================
    // Verify display_name propagation through event data types.

    #[test]
    fn tool_call_summary_with_display_name() {
        let summary = ToolCallSummary {
            id: "tc_1".to_string(),
            name: "get_weather".to_string(),
            display_name: Some("Get Weather".to_string()),
            narration: None,
            completed_narration: None,
        };
        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["display_name"], "Get Weather");

        // Round-trip
        let deserialized: ToolCallSummary = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.display_name.as_deref(), Some("Get Weather"));
    }

    #[test]
    fn tool_call_summary_without_display_name_omits_field() {
        let summary = ToolCallSummary {
            id: "tc_1".to_string(),
            name: "get_weather".to_string(),
            display_name: None,
            narration: None,
            completed_narration: None,
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(!json.contains("display_name"));

        // Deserialize without display_name field present
        let json_without = r#"{"id":"tc_1","name":"get_weather"}"#;
        let deserialized: ToolCallSummary = serde_json::from_str(json_without).unwrap();
        assert_eq!(deserialized.display_name, None);
    }

    #[test]
    fn act_started_with_definitions_populates_display_names() {
        use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolPolicy};

        let tool_calls = vec![
            ToolCall {
                id: "tc_1".to_string(),
                name: "get_weather".to_string(),
                arguments: serde_json::json!({}),
            },
            ToolCall {
                id: "tc_2".to_string(),
                name: "unknown_tool".to_string(),
                arguments: serde_json::json!({}),
            },
        ];
        let tool_defs = vec![crate::tool_types::ToolDefinition::Builtin(BuiltinTool {
            name: "get_weather".to_string(),
            display_name: Some("Get Weather".to_string()),
            description: "Gets weather".to_string(),
            parameters: serde_json::json!({}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: crate::tool_types::ToolHints::default(),
            full_parameters: None,
        })];

        let data = ActStartedData::with_definitions(&tool_calls, &tool_defs);
        assert_eq!(data.tool_calls.len(), 2);
        assert_eq!(
            data.tool_calls[0].display_name.as_deref(),
            Some("Get Weather")
        );
        assert_eq!(
            data.tool_calls[0].narration.as_deref(),
            Some("Running Get Weather")
        );
        assert_eq!(
            data.tool_calls[0].completed_narration.as_deref(),
            Some("Ran Get Weather")
        );
        assert_eq!(data.tool_calls[1].display_name, None);
    }

    #[test]
    fn tool_completed_with_display_name_roundtrip() {
        let data = ToolCompletedData::success(
            "tc_1".to_string(),
            "get_weather".to_string(),
            vec![crate::message::ContentPart::text("Sunny")],
            Some(100),
        )
        .with_display_name(Some("Get Weather".to_string()));

        assert_eq!(data.display_name.as_deref(), Some("Get Weather"));

        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["display_name"], "Get Weather");

        let deserialized: ToolCompletedData = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.display_name.as_deref(), Some("Get Weather"));
    }

    #[test]
    fn tool_started_display_name_serialization() {
        let data = ToolStartedData {
            tool_call: ToolCall {
                id: "tc_1".to_string(),
                name: "bash".to_string(),
                arguments: serde_json::json!({"command": "ls"}),
            },
            tool_call_fingerprint: None,
            display_name: Some("Bash".to_string()),
            narration: None,
        };

        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["display_name"], "Bash");
    }

    #[test]
    fn tool_definition_summary_display_name() {
        use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolPolicy};

        let def = crate::tool_types::ToolDefinition::Builtin(BuiltinTool {
            name: "read_file".to_string(),
            display_name: Some("Read File".to_string()),
            description: "Reads a file".to_string(),
            parameters: serde_json::json!({}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: crate::tool_types::ToolHints::default(),
            full_parameters: None,
        });

        let summary = ToolDefinitionSummary::from(&def);
        assert_eq!(summary.display_name.as_deref(), Some("Read File"));

        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["display_name"], "Read File");
    }

    // ========================================================================
    // Forward Compatibility Tests
    // ========================================================================
    // These tests verify that unknown fields and types are handled correctly
    // per the contract specification.

    #[test]
    fn forward_compat_unknown_fields_ignored() {
        // Unknown fields should be silently ignored during deserialization
        let json = r#"{
            "turn_id": "turn_00000000000000000000000000000002",
            "iterations": 3,
            "duration_ms": 1500,
            "usage": {"input_tokens": 100, "output_tokens": 50},
            "future_field": "should be ignored",
            "another_new_field": 42
        }"#;

        let data: TurnCompletedData = serde_json::from_str(json).unwrap();
        assert_eq!(data.iterations, 3);
        assert_eq!(data.duration_ms, Some(1500));
    }

    #[test]
    fn forward_compat_unknown_event_type_becomes_unsupported() {
        // Unknown event types should deserialize to Unsupported
        let json = serde_json::json!({"some_field": "value"});
        let data = deserialize_event_data("future.event.type", json);

        assert!(data.is_unsupported());
        assert_eq!(data.event_type(), "unsupported");
    }

    #[test]
    fn forward_compat_unsupported_preserves_data() {
        // Unsupported events should preserve the original data for debugging
        let original = serde_json::json!({"key": "value", "nested": {"a": 1}});
        let data = deserialize_event_data("unknown.event", original.clone());

        match data {
            EventData::Unsupported { event_type, data } => {
                assert_eq!(event_type, "unknown.event");
                assert_eq!(data, original);
            }
            _ => panic!("Expected Unsupported variant"),
        }
    }

    #[test]
    fn forward_compat_optional_fields_absent() {
        // Optional fields can be absent without causing errors
        let json = r#"{
            "turn_id": "turn_00000000000000000000000000000002",
            "iterations": 3
        }"#;

        let data: TurnCompletedData = serde_json::from_str(json).unwrap();
        assert_eq!(data.iterations, 3);
        assert!(data.duration_ms.is_none());
        assert!(data.usage.is_none());
        assert!(data.input_content.is_none());
        assert!(data.final_message_id.is_none());
        assert!(data.final_answer_preview.is_none());
        assert!(data.time_to_first_token_ms.is_none());
        assert!(data.tool_call_count.is_none());
        assert!(data.llm_call_count.is_none());
        assert!(data.status.is_none());
    }

    // ========================================================================
    // Round-Trip Serialization Tests
    // ========================================================================
    // These tests verify that events survive serialization/deserialization.

    #[test]
    fn round_trip_all_event_data_types() {
        // Test that all event data types can be serialized and deserialized
        let test_cases: Vec<(&str, EventData)> = vec![
            (
                INPUT_MESSAGE,
                InputMessageData::new(Message::user("test")).into(),
            ),
            (
                OUTPUT_MESSAGE_STARTED,
                OutputMessageStartedData {
                    turn_id: test_turn_id(),
                    message_id: test_message_id(),
                    model: None,
                    iteration: None,
                    phase: None,
                }
                .into(),
            ),
            (
                OUTPUT_MESSAGE_DELTA,
                OutputMessageDeltaData {
                    turn_id: test_turn_id(),
                    message_id: test_message_id(),
                    delta: "x".to_string(),
                    accumulated: "x".to_string(),
                    phase: None,
                }
                .into(),
            ),
            (
                OUTPUT_MESSAGE_COMPLETED,
                OutputMessageCompletedData::new(Message::assistant("hi")).into(),
            ),
            (
                TURN_STARTED,
                TurnStartedData {
                    turn_id: test_turn_id(),
                    input_message_id: test_message_id(),
                    input_content: None,
                }
                .into(),
            ),
            (
                TURN_COMPLETED,
                TurnCompletedData {
                    turn_id: test_turn_id(),
                    iterations: 1,
                    duration_ms: None,
                    usage: None,
                    input_content: None,
                    final_message_id: None,
                    final_answer_preview: None,
                    time_to_first_token_ms: None,
                    tool_call_count: None,
                    llm_call_count: None,
                    status: None,
                }
                .into(),
            ),
            (
                TURN_FAILED,
                TurnFailedData {
                    turn_id: test_turn_id(),
                    error: "err".to_string(),
                    error_code: None,
                    error_fields: None,
                    error_disclosure: None,
                }
                .into(),
            ),
            (
                TURN_CANCELLED,
                TurnCancelledData {
                    turn_id: test_turn_id(),
                    reason: None,
                    usage: None,
                }
                .into(),
            ),
            (
                TURN_SEALED,
                TurnSealedData {
                    turn_id: test_turn_id(),
                    reason: "no_progress".to_string(),
                    detail: Some("sealed".to_string()),
                    iterations: Some(3),
                    usage: None,
                }
                .into(),
            ),
            (
                REASON_STARTED,
                ReasonStartedData {
                    harness_id: test_harness_id(),
                    agent_id: Some(test_agent_id()),
                    metadata: None,
                }
                .into(),
            ),
            (
                REASON_COMPLETED,
                ReasonCompletedData::success("", false, 0, None, None).into(),
            ),
            (
                ACT_STARTED,
                ActStartedData {
                    tool_calls: vec![],
                    headline: None,
                }
                .into(),
            ),
            (
                ACT_COMPLETED,
                ActCompletedData {
                    completed: true,
                    success_count: 0,
                    error_count: 0,
                    duration_ms: None,
                    headline: None,
                }
                .into(),
            ),
            (
                SESSION_STARTED,
                SessionStartedData {
                    harness_id: test_harness_id(),
                    agent_id: Some(test_agent_id()),
                    model_id: None,
                }
                .into(),
            ),
            (
                SESSION_ACTIVATED,
                SessionActivatedData {
                    turn_id: test_turn_id(),
                    input_message_id: test_message_id(),
                }
                .into(),
            ),
            (
                SESSION_IDLED,
                SessionIdledData {
                    turn_id: test_turn_id(),
                    iterations: None,
                    usage: None,
                }
                .into(),
            ),
            (
                SESSION_TITLE_UPDATED,
                SessionTitleUpdatedData {
                    previous_title: Some("Old title".to_string()),
                    title: "New title".to_string(),
                }
                .into(),
            ),
        ];

        for (event_type, original) in test_cases {
            // Serialize
            let json = serde_json::to_value(&original).unwrap();
            // Deserialize using type-directed function
            let deserialized = deserialize_event_data(event_type, json);
            // Verify same event type
            assert_eq!(
                original.event_type(),
                deserialized.event_type(),
                "Event type mismatch for {}",
                event_type
            );
        }
    }

    // ========================================================================
    // Event Structure Tests
    // ========================================================================
    // Tests for the Event container structure

    #[test]
    fn event_structure_has_required_fields() {
        let session_id = test_session_id();
        let context = EventContext::turn(test_turn_id(), test_message_id());
        let event = Event::new(
            session_id,
            context,
            InputMessageData::new(Message::user("test")),
        );

        // Verify all required fields are present
        let json = serde_json::to_value(&event).unwrap();
        assert!(json.get("id").is_some(), "Missing id field");
        assert!(json.get("type").is_some(), "Missing type field");
        assert!(json.get("ts").is_some(), "Missing ts field");
        assert!(json.get("session_id").is_some(), "Missing session_id field");
        assert!(json.get("context").is_some(), "Missing context field");
        assert!(json.get("data").is_some(), "Missing data field");
    }

    #[test]
    fn event_context_span_fields() {
        let context = EventContext::empty().with_span(
            "trace123".to_string(),
            "span456".to_string(),
            Some("parent789".to_string()),
        );

        let json = serde_json::to_value(&context).unwrap();
        assert_eq!(
            json.get("trace_id").and_then(|v| v.as_str()),
            Some("trace123")
        );
        assert_eq!(
            json.get("span_id").and_then(|v| v.as_str()),
            Some("span456")
        );
        assert_eq!(
            json.get("parent_span_id").and_then(|v| v.as_str()),
            Some("parent789")
        );
    }

    #[test]
    fn is_unsupported_returns_false_for_known_types() {
        let data = InputMessageData::new(Message::user("test"));
        let event_data: EventData = data.into();
        assert!(!event_data.is_unsupported());
    }

    #[test]
    fn is_unsupported_returns_true_for_unsupported() {
        let data = deserialize_event_data("unknown.type", serde_json::json!({}));
        assert!(data.is_unsupported());
    }
}
