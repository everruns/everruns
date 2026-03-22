// ==========================================================================
// PUBLIC CONTRACT - Event Protocol
// ==========================================================================
//
// This module defines the Everruns event protocol - a PUBLIC API CONTRACT.
// Changes must follow the compatibility guidelines in specs/events-contract.md.
//
// STABILITY: Stable (v1)
// - Event structure (id, type, ts, session_id, context, data) is frozen
// - New event types are additive (non-breaking)
// - New optional fields are non-breaking
// - Unsupported events are filtered before API responses
//
// See: specs/events-contract.md for full contract specification.
// ==========================================================================
//
// All events follow a consistent structure: id, type, ts, context, data.
// Events are the source of truth for conversation data and provide
// observability into session execution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::localization::localized_tool_display_name;
use crate::typed_id::{AgentId, EventId, ExecId, HarnessId, MessageId, ModelId, SessionId, TurnId};

// ============================================================================
// Event Type Constants
// ============================================================================

// Input events
pub const INPUT_MESSAGE: &str = "input.message";

// Output events (lifecycle: started → delta* → completed)
pub const OUTPUT_MESSAGE_STARTED: &str = "output.message.started";
pub const OUTPUT_MESSAGE_DELTA: &str = "output.message.delta";
pub const OUTPUT_MESSAGE_COMPLETED: &str = "output.message.completed";

// Turn lifecycle events
pub const TURN_STARTED: &str = "turn.started";
pub const TURN_COMPLETED: &str = "turn.completed";
pub const TURN_FAILED: &str = "turn.failed";
pub const TURN_CANCELLED: &str = "turn.cancelled";

// Atom lifecycle events
pub const REASON_STARTED: &str = "reason.started";
pub const REASON_COMPLETED: &str = "reason.completed";
pub const ACT_STARTED: &str = "act.started";
pub const ACT_COMPLETED: &str = "act.completed";
pub const TOOL_STARTED: &str = "tool.started";
pub const TOOL_COMPLETED: &str = "tool.completed";
pub const TOOL_PROGRESS: &str = "tool.progress";
pub const TOOL_CALL_REQUESTED: &str = "tool.call_requested";

// LLM events
pub const LLM_GENERATION: &str = "llm.generation";

// Reasoning/thinking events (extended thinking from models like Claude)
pub const REASON_THINKING_STARTED: &str = "reason.thinking.started";
pub const REASON_THINKING_DELTA: &str = "reason.thinking.delta";
pub const REASON_THINKING_COMPLETED: &str = "reason.thinking.completed";

// Session events
pub const SESSION_STARTED: &str = "session.started";
pub const SESSION_ACTIVATED: &str = "session.activated";
pub const SESSION_IDLED: &str = "session.idled";

// Schedule events
pub const SCHEDULE_TRIGGERED: &str = "schedule.triggered";

// Subagent lifecycle events
pub const SUBAGENT_SPAWNED: &str = "subagent.spawned";
pub const SUBAGENT_COMPLETED: &str = "subagent.completed";
pub const SUBAGENT_FAILED: &str = "subagent.failed";
pub const SUBAGENT_CANCELLED: &str = "subagent.cancelled";

// Context compaction events
pub const CONTEXT_COMPACTING: &str = "context.compacting";
pub const CONTEXT_COMPACTED: &str = "context.compacted";

/// All valid event types for API filtering validation.
/// Used by `types` and `exclude` query parameter validation to reject unknown types
/// and prevent unbounded arrays from reaching the database.
pub const VALID_EVENT_TYPES: &[&str] = &[
    INPUT_MESSAGE,
    OUTPUT_MESSAGE_STARTED,
    OUTPUT_MESSAGE_DELTA,
    OUTPUT_MESSAGE_COMPLETED,
    TURN_STARTED,
    TURN_COMPLETED,
    TURN_FAILED,
    TURN_CANCELLED,
    REASON_STARTED,
    REASON_COMPLETED,
    ACT_STARTED,
    ACT_COMPLETED,
    TOOL_STARTED,
    TOOL_COMPLETED,
    TOOL_PROGRESS,
    TOOL_CALL_REQUESTED,
    LLM_GENERATION,
    REASON_THINKING_STARTED,
    REASON_THINKING_DELTA,
    REASON_THINKING_COMPLETED,
    SESSION_STARTED,
    SESSION_ACTIVATED,
    SESSION_IDLED,
    SCHEDULE_TRIGGERED,
    SUBAGENT_SPAWNED,
    SUBAGENT_COMPLETED,
    SUBAGENT_FAILED,
    SUBAGENT_CANCELLED,
    CONTEXT_COMPACTING,
    CONTEXT_COMPACTED,
];

// ============================================================================
// Event Context
// ============================================================================

use crate::atoms::AtomContext;

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

    /// Create a full context from an AtomContext
    pub fn from_atom_context(ctx: &AtomContext) -> Self {
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl Event {
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
                | ACT_STARTED
                | ACT_COMPLETED
                | TOOL_STARTED
                | TOOL_COMPLETED
                | TOOL_PROGRESS
                | TOOL_CALL_REQUESTED
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

/// Metadata about the model used for generation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ModelMetadata {
    /// Model name (e.g., "gpt-4o", "claude-3-sonnet")
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
/// Tracks token consumption per LLM call including cache tokens for cost optimization.
/// Cache tokens are provider-specific:
/// - OpenAI: `cache_read_tokens` from prompt_tokens_details.cached_tokens
/// - Anthropic: `cache_read_tokens` from cache_read_input_tokens,
///   `cache_creation_tokens` from cache_creation_input_tokens
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TokenUsage {
    /// Number of input/prompt tokens
    pub input_tokens: u32,
    /// Number of output/completion tokens
    pub output_tokens: u32,
    /// Number of tokens read from cache (reduces cost)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
    /// Number of tokens written to cache (Anthropic-specific)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_tokens: Option<u32>,
}

impl TokenUsage {
    /// Create a new TokenUsage with just input and output tokens
    pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_read_tokens: None,
            cache_creation_tokens: None,
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
        }
    }

    /// Get total tokens (input + output)
    pub fn total_tokens(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }

    /// Add another TokenUsage to this one (for aggregation)
    pub fn add(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        if let Some(cache) = other.cache_read_tokens {
            *self.cache_read_tokens.get_or_insert(0) += cache;
        }
        if let Some(cache) = other.cache_creation_tokens {
            *self.cache_creation_tokens.get_or_insert(0) += cache;
        }
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

    /// Optional model name being used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Current iteration number within this turn (1-based).
    /// Useful for UI to show progress during multi-step tool-calling flows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
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

    /// The new text chunk
    pub delta: String,

    /// Accumulated text so far
    pub accumulated: String,
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
}

impl OutputMessageCompletedData {
    pub fn new(message: Message) -> Self {
        Self {
            message,
            metadata: None,
            usage: None,
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
}

impl From<&ToolCall> for ToolCallSummary {
    fn from(tc: &ToolCall) -> Self {
        Self {
            id: tc.id.clone(),
            name: tc.name.clone(),
            display_name: None,
            narration: None,
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
    /// Tool description
    pub description: String,
}

impl From<&crate::tool_types::ToolDefinition> for ToolDefinitionSummary {
    fn from(tool: &crate::tool_types::ToolDefinition) -> Self {
        Self {
            name: tool.name().to_string(),
            display_name: tool.display_name().map(|s| s.to_string()),
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
            display_name: None,
            success: true,
            status: "success".to_string(),
            result: Some(result),
            error: None,
            duration_ms,
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
            display_name: None,
            success: false,
            status,
            result: None,
            error: Some(error),
            duration_ms,
            narration: None,
        }
    }

    /// Set display name on this event data
    pub fn with_display_name(mut self, display_name: Option<String>) -> Self {
        self.display_name = display_name;
        self
    }

    /// Set narration on this event data
    pub fn with_narration(mut self, narration: Option<String>) -> Self {
        self.narration = narration;
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

/// Metadata about an LLM generation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmGenerationMetadata {
    /// Model identifier used for generation
    pub model: String,

    /// Provider type (openai, anthropic, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,

    /// Token usage statistics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,

    /// Duration of the generation in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,

    /// Time to first token in milliseconds (streaming latency)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<u64>,

    /// Whether the generation was successful
    pub success: bool,

    /// Error message if generation failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Finish reasons from the LLM (e.g., ["stop"], ["tool_calls"])
    /// Required for gen-ai semantic conventions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reasons: Option<Vec<String>>,

    /// Unique response identifier from the LLM provider
    /// Required for gen-ai semantic conventions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,

    /// Retry information if rate limit retries occurred
    /// Contains number of retries and total wait time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<LlmRetryInfo>,

    /// Compaction information if context was compressed before generation
    /// Occurs when the conversation context exceeded the model's limit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compaction: Option<LlmCompactionInfo>,
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
}

impl LlmCompactionInfo {
    /// Create info for a successful compaction
    pub fn new(
        input_tokens_before: Option<u32>,
        input_tokens_after: Option<u32>,
        duration_ms: Option<u64>,
    ) -> Self {
        Self {
            compacted: true,
            input_tokens_before,
            input_tokens_after,
            duration_ms,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
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

// ============================================================================
// Subagent event data
// ============================================================================

/// Data for subagent lifecycle events (spawned, completed, failed, cancelled).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SubagentEventData {
    /// The subagent's child session ID
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub subagent_session_id: SessionId,
    /// Human-readable subagent name
    pub subagent_name: String,
    /// Task description
    pub task: String,
    /// Subagent status (spawning, running, completed, failed, cancelled)
    pub status: String,
    /// Result summary (only for completed/failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Error message (only for failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<SubagentEventData> for EventData {
    fn from(data: SubagentEventData) -> Self {
        match data.status.as_str() {
            "completed" => EventData::SubagentCompleted(data),
            "failed" => EventData::SubagentFailed(data),
            "cancelled" => EventData::SubagentCancelled(data),
            _ => EventData::SubagentSpawned(data),
        }
    }
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
    /// Combined strategy description (e.g., "observation_masking+native").
    pub strategy_used: String,
    /// Number of messages before compaction.
    pub messages_before: usize,
    /// Number of messages after compaction.
    pub messages_after: usize,
    /// Total duration of all compaction steps in milliseconds.
    pub duration_ms: u64,
    /// Individual steps in the cascade.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<CompactionStepData>,
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
/// - `act.started` → ActStartedData
/// - `act.completed` → ActCompletedData
/// - `tool.started` → ToolStartedData
/// - `tool.completed` → ToolCompletedData
/// - `tool.call_requested` → ToolCallRequestedData
/// - `llm.generation` → LlmGenerationData
/// - `reason.thinking.started` → ReasonThinkingStartedData
/// - `reason.thinking.delta` → ReasonThinkingDeltaData
/// - `reason.thinking.completed` → ReasonThinkingCompletedData
/// - `session.started` → SessionStartedData
/// - `session.activated` → SessionActivatedData
/// - `session.idled` → SessionIdledData
/// - `subagent.spawned` → SubagentEventData
/// - `subagent.completed` → SubagentEventData
/// - `subagent.failed` → SubagentEventData
/// - `subagent.cancelled` → SubagentEventData
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    // NOTE: OutputMessageDelta must come BEFORE OutputMessageStarted for untagged enum deserialization.
    // OutputMessageDelta has more required fields (turn_id, delta, accumulated) while
    // OutputMessageStarted only requires turn_id (model is optional). If OutputMessageStarted
    // comes first, it will match OutputMessageDelta JSON and discard delta/accumulated fields.
    OutputMessageDelta(OutputMessageDeltaData),
    OutputMessageStarted(OutputMessageStartedData),
    OutputMessageCompleted(OutputMessageCompletedData),

    // Turn lifecycle events
    TurnStarted(TurnStartedData),
    TurnCompleted(TurnCompletedData),
    TurnFailed(TurnFailedData),

    // Atom lifecycle events
    ReasonStarted(ReasonStartedData),
    ReasonCompleted(ReasonCompletedData),
    ActStarted(ActStartedData),
    ActCompleted(ActCompletedData),
    ToolStarted(ToolStartedData),
    ToolCompleted(ToolCompletedData),
    ToolProgress(ToolProgressData),
    ToolCallRequested(ToolCallRequestedData),

    // LLM events
    LlmGeneration(LlmGenerationData),

    // Extended thinking events (for models with reasoning like Claude)
    // NOTE: ReasonThinkingDelta must come BEFORE ReasonThinkingStarted/Completed for untagged enum deserialization.
    // ReasonThinkingDelta has more required fields (turn_id, delta, accumulated) while
    // ReasonThinkingStarted/Completed have fewer required fields. If simpler types come first,
    // they will match their JSON and discard delta/accumulated fields.
    ReasonThinkingDelta(ReasonThinkingDeltaData),
    ReasonThinkingStarted(ReasonThinkingStartedData),
    ReasonThinkingCompleted(ReasonThinkingCompletedData),

    // NOTE: TurnCancelled is placed at the end (before Raw/Session events) because it only
    // requires turn_id. If placed earlier, it would greedily match JSON for other turn_id-based
    // events (OutputMessageStarted, ReasonThinkingStarted, etc.) and discard their specific fields.
    TurnCancelled(TurnCancelledData),

    // Session events
    SessionStarted(SessionStartedData),
    SessionActivated(SessionActivatedData),
    SessionIdled(SessionIdledData),

    // Subagent lifecycle events
    SubagentSpawned(SubagentEventData),
    SubagentCompleted(SubagentEventData),
    SubagentFailed(SubagentEventData),
    SubagentCancelled(SubagentEventData),

    // Context compaction events
    ContextCompacting(ContextCompactingData),
    ContextCompacted(ContextCompactedData),

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
    /// Get the event type constant for this data.
    /// For Unsupported events, returns "unsupported" (internal use only).
    pub fn event_type(&self) -> &'static str {
        match self {
            EventData::InputMessage(_) => INPUT_MESSAGE,
            EventData::OutputMessageStarted(_) => OUTPUT_MESSAGE_STARTED,
            EventData::OutputMessageDelta(_) => OUTPUT_MESSAGE_DELTA,
            EventData::OutputMessageCompleted(_) => OUTPUT_MESSAGE_COMPLETED,
            EventData::TurnStarted(_) => TURN_STARTED,
            EventData::TurnCompleted(_) => TURN_COMPLETED,
            EventData::TurnFailed(_) => TURN_FAILED,
            EventData::TurnCancelled(_) => TURN_CANCELLED,
            EventData::ReasonStarted(_) => REASON_STARTED,
            EventData::ReasonCompleted(_) => REASON_COMPLETED,
            EventData::ActStarted(_) => ACT_STARTED,
            EventData::ActCompleted(_) => ACT_COMPLETED,
            EventData::ToolStarted(_) => TOOL_STARTED,
            EventData::ToolCompleted(_) => TOOL_COMPLETED,
            EventData::ToolProgress(_) => TOOL_PROGRESS,
            EventData::ToolCallRequested(_) => TOOL_CALL_REQUESTED,
            EventData::LlmGeneration(_) => LLM_GENERATION,
            EventData::ReasonThinkingDelta(_) => REASON_THINKING_DELTA,
            EventData::ReasonThinkingStarted(_) => REASON_THINKING_STARTED,
            EventData::ReasonThinkingCompleted(_) => REASON_THINKING_COMPLETED,
            EventData::SessionStarted(_) => SESSION_STARTED,
            EventData::SessionActivated(_) => SESSION_ACTIVATED,
            EventData::SessionIdled(_) => SESSION_IDLED,
            EventData::SubagentSpawned(_) => SUBAGENT_SPAWNED,
            EventData::SubagentCompleted(_) => SUBAGENT_COMPLETED,
            EventData::SubagentFailed(_) => SUBAGENT_FAILED,
            EventData::SubagentCancelled(_) => SUBAGENT_CANCELLED,
            EventData::ContextCompacting(_) => CONTEXT_COMPACTING,
            EventData::ContextCompacted(_) => CONTEXT_COMPACTED,
            EventData::Unsupported { .. } => "unsupported",
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

/// Deserialize event data from JSON based on event_type.
///
/// This function uses the event_type to select the correct EventData variant,
/// avoiding issues with serde's untagged enum deserialization where simpler
/// types (fewer required fields) might incorrectly match before more complex ones.
///
/// # Arguments
/// * `event_type` - The event type string (e.g., "reason.thinking.completed")
/// * `data` - The JSON value to deserialize
///
/// # Returns
/// The deserialized EventData variant, or EventData::Unsupported if the type is unknown.
/// Unsupported events log a warning and should be filtered before API responses.
pub fn deserialize_event_data(event_type: &str, data: serde_json::Value) -> EventData {
    let result =
        match event_type {
            INPUT_MESSAGE => serde_json::from_value::<InputMessageData>(data.clone())
                .map(EventData::InputMessage),
            OUTPUT_MESSAGE_STARTED => {
                serde_json::from_value::<OutputMessageStartedData>(data.clone())
                    .map(EventData::OutputMessageStarted)
            }
            OUTPUT_MESSAGE_DELTA => serde_json::from_value::<OutputMessageDeltaData>(data.clone())
                .map(EventData::OutputMessageDelta),
            OUTPUT_MESSAGE_COMPLETED => {
                serde_json::from_value::<OutputMessageCompletedData>(data.clone())
                    .map(EventData::OutputMessageCompleted)
            }
            TURN_STARTED => {
                serde_json::from_value::<TurnStartedData>(data.clone()).map(EventData::TurnStarted)
            }
            TURN_COMPLETED => serde_json::from_value::<TurnCompletedData>(data.clone())
                .map(EventData::TurnCompleted),
            TURN_FAILED => {
                serde_json::from_value::<TurnFailedData>(data.clone()).map(EventData::TurnFailed)
            }
            TURN_CANCELLED => serde_json::from_value::<TurnCancelledData>(data.clone())
                .map(EventData::TurnCancelled),
            REASON_STARTED => serde_json::from_value::<ReasonStartedData>(data.clone())
                .map(EventData::ReasonStarted),
            REASON_COMPLETED => serde_json::from_value::<ReasonCompletedData>(data.clone())
                .map(EventData::ReasonCompleted),
            ACT_STARTED => {
                serde_json::from_value::<ActStartedData>(data.clone()).map(EventData::ActStarted)
            }
            ACT_COMPLETED => serde_json::from_value::<ActCompletedData>(data.clone())
                .map(EventData::ActCompleted),
            TOOL_STARTED => {
                serde_json::from_value::<ToolStartedData>(data.clone()).map(EventData::ToolStarted)
            }
            TOOL_COMPLETED => serde_json::from_value::<ToolCompletedData>(data.clone())
                .map(EventData::ToolCompleted),
            TOOL_PROGRESS => serde_json::from_value::<ToolProgressData>(data.clone())
                .map(EventData::ToolProgress),
            TOOL_CALL_REQUESTED => serde_json::from_value::<ToolCallRequestedData>(data.clone())
                .map(EventData::ToolCallRequested),
            LLM_GENERATION => serde_json::from_value::<LlmGenerationData>(data.clone())
                .map(EventData::LlmGeneration),
            REASON_THINKING_STARTED => {
                serde_json::from_value::<ReasonThinkingStartedData>(data.clone())
                    .map(EventData::ReasonThinkingStarted)
            }
            REASON_THINKING_DELTA => {
                serde_json::from_value::<ReasonThinkingDeltaData>(data.clone())
                    .map(EventData::ReasonThinkingDelta)
            }
            REASON_THINKING_COMPLETED => {
                serde_json::from_value::<ReasonThinkingCompletedData>(data.clone())
                    .map(EventData::ReasonThinkingCompleted)
            }
            SESSION_STARTED => serde_json::from_value::<SessionStartedData>(data.clone())
                .map(EventData::SessionStarted),
            SESSION_ACTIVATED => serde_json::from_value::<SessionActivatedData>(data.clone())
                .map(EventData::SessionActivated),
            SESSION_IDLED => serde_json::from_value::<SessionIdledData>(data.clone())
                .map(EventData::SessionIdled),
            CONTEXT_COMPACTING => serde_json::from_value::<ContextCompactingData>(data.clone())
                .map(EventData::ContextCompacting),
            CONTEXT_COMPACTED => serde_json::from_value::<ContextCompactedData>(data.clone())
                .map(EventData::ContextCompacted),
            _ => {
                // Unknown event type - return as unsupported with warning
                return EventData::unsupported(event_type.to_string(), data);
            }
        };

    // If deserialization fails, return as unsupported
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
    OutputMessageCompletedData => OutputMessageCompleted,
    TurnStartedData => TurnStarted,
    TurnCompletedData => TurnCompleted,
    TurnFailedData => TurnFailed,
    TurnCancelledData => TurnCancelled,
    ReasonStartedData => ReasonStarted,
    ReasonCompletedData => ReasonCompleted,
    ActStartedData => ActStarted,
    ActCompletedData => ActCompleted,
    ToolStartedData => ToolStarted,
    ToolCompletedData => ToolCompleted,
    ToolProgressData => ToolProgress,
    ToolCallRequestedData => ToolCallRequested,
    LlmGenerationData => LlmGeneration,
    ReasonThinkingStartedData => ReasonThinkingStarted,
    ReasonThinkingDeltaData => ReasonThinkingDelta,
    ReasonThinkingCompletedData => ReasonThinkingCompleted,
    SessionStartedData => SessionStarted,
    SessionActivatedData => SessionActivated,
    SessionIdledData => SessionIdled,
    ContextCompactingData => ContextCompacting,
    ContextCompactedData => ContextCompacted,
}

// ============================================================================
// Event Request (input type without id/sequence)
// ============================================================================

/// Request to create a new event.
///
/// This is the input type for event ingestion. It contains all the data
/// needed to create an event, but without the `id` and `sequence` fields
/// which are assigned by the storage layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    fn test_event_context_from_atom_context() {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();

        let atom_ctx = AtomContext::new(session_id, turn_id, input_message_id);
        let context = EventContext::from_atom_context(&atom_ctx);

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
            "claude-3-opus".to_string(),
            Some("anthropic".to_string()),
            Some(TokenUsage {
                input_tokens: 5,
                output_tokens: 3,
                cache_read_tokens: None,
                cache_creation_tokens: None,
            }),
            Some(50),
            Some(25), // time_to_first_token_ms
            Some(vec!["end_turn".to_string()]),
            Some("msg_12345".to_string()),
        );

        assert!(data.metadata.success);
        assert_eq!(data.metadata.model, "claude-3-opus");
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
            model: Some("claude-4-opus".to_string()),
            iteration: None,
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
            model: None,
            iteration: None,
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
            delta: "Hello".to_string(),
            accumulated: "Hello".to_string(),
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
    fn test_output_message_delta_deserialization_preserves_fields() {
        // This test verifies that OutputMessageDelta deserializes correctly with all fields
        // (regression test for the untagged enum ordering fix)
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let data = OutputMessageDeltaData {
            turn_id,
            delta: "Hello world".to_string(),
            accumulated: "Hello world".to_string(),
        };

        // Serialize to JSON
        let json = serde_json::to_value(EventData::OutputMessageDelta(data.clone())).unwrap();

        // Deserialize back
        let deserialized: EventData = serde_json::from_value(json).unwrap();

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
            model: Some("claude-3".to_string()),
            iteration: None,
        };

        // Serialize to JSON
        let json = serde_json::to_value(EventData::OutputMessageStarted(data.clone())).unwrap();

        // Deserialize back
        let deserialized: EventData = serde_json::from_value(json).unwrap();

        // Verify it's OutputMessageStarted and fields are preserved
        match deserialized {
            EventData::OutputMessageStarted(at) => {
                assert_eq!(at.turn_id, turn_id);
                assert_eq!(at.model, Some("claude-3".to_string()));
            }
            _ => panic!("Expected OutputMessageStarted, got different variant"),
        }
    }

    #[test]
    fn test_reason_thinking_started_deserialization() {
        // NOTE: ReasonThinkingStartedData and OutputMessageStartedData have identical structures
        // (turn_id + model), so serde's untagged enum can't distinguish them.
        // This test uses deserialize_event_data() which uses the event_type to select the correct variant.
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let data = ReasonThinkingStartedData {
            turn_id,
            model: Some("claude-3".to_string()),
        };

        // Serialize to JSON
        let json = serde_json::to_value(&data).unwrap();

        // Deserialize using typed function (not raw serde)
        let deserialized = deserialize_event_data(REASON_THINKING_STARTED, json);

        // Verify it's ReasonThinkingStarted and fields are preserved
        match deserialized {
            EventData::ReasonThinkingStarted(at) => {
                assert_eq!(at.turn_id, turn_id);
                assert_eq!(at.model, Some("claude-3".to_string()));
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
// These tests validate the event protocol contract defined in specs/events-contract.md.
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
            model: Some("gpt-4o".to_string()),
            iteration: None,
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
            delta: "Hello".to_string(),
            accumulated: "Hello".to_string(),
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
        })];

        let data = ActStartedData::with_definitions(&tool_calls, &tool_defs);
        assert_eq!(data.tool_calls.len(), 2);
        assert_eq!(
            data.tool_calls[0].display_name.as_deref(),
            Some("Get Weather")
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
                    model: None,
                    iteration: None,
                }
                .into(),
            ),
            (
                OUTPUT_MESSAGE_DELTA,
                OutputMessageDeltaData {
                    turn_id: test_turn_id(),
                    delta: "x".to_string(),
                    accumulated: "x".to_string(),
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
                }
                .into(),
            ),
            (
                TURN_FAILED,
                TurnFailedData {
                    turn_id: test_turn_id(),
                    error: "err".to_string(),
                    error_code: None,
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
