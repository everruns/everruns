// Event Protocol
//
// This module defines the standard event schema used throughout Everruns.
// All events follow a consistent structure: id, type, ts, context, data.
// Events are the source of truth for conversation data and provide
// observability into session execution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

// ============================================================================
// Event Type Constants
// ============================================================================

// Message events
pub const MESSAGE_USER: &str = "message.user";
pub const MESSAGE_AGENT: &str = "message.agent";

// Turn lifecycle events
pub const TURN_STARTED: &str = "turn.started";
pub const TURN_COMPLETED: &str = "turn.completed";
pub const TURN_FAILED: &str = "turn.failed";

// Atom lifecycle events
pub const INPUT_RECEIVED: &str = "input.received";
pub const REASON_STARTED: &str = "reason.started";
pub const REASON_COMPLETED: &str = "reason.completed";
pub const ACT_STARTED: &str = "act.started";
pub const ACT_COMPLETED: &str = "act.completed";
pub const TOOL_CALL_STARTED: &str = "tool.call_started";
pub const TOOL_CALL_COMPLETED: &str = "tool.call_completed";

// Session events
pub const SESSION_STARTED: &str = "session.started";

// ============================================================================
// Event Context
// ============================================================================

use crate::atoms::AtomContext;

/// Context for event correlation and tracing
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct EventContext {
    /// Turn identifier (for turn-scoped events)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<Uuid>,

    /// User message that triggered this turn
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_message_id: Option<Uuid>,

    /// Atom execution identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exec_id: Option<Uuid>,
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
        }
    }

    /// Create a context for turn-scoped events (without exec_id)
    pub fn turn(turn_id: Uuid, input_message_id: Uuid) -> Self {
        Self {
            turn_id: Some(turn_id),
            input_message_id: Some(input_message_id),
            exec_id: None,
        }
    }
}

// ============================================================================
// Standard Event Schema
// ============================================================================

/// Standard event following the Everruns event protocol.
///
/// All events have a consistent structure:
/// - `id`: Unique UUID v7 identifier (monotonically increasing)
/// - `type`: Event type in dot notation (e.g., "message.user", "reason.started")
/// - `ts`: ISO 8601 timestamp with millisecond precision
/// - `session_id`: Session this event belongs to
/// - `context`: Correlation context for tracing
/// - `data`: Event-specific payload
/// - `metadata`: Optional arbitrary metadata
/// - `tags`: Optional list of tags for filtering
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Event {
    /// Unique event identifier (UUID v7, monotonically increasing)
    pub id: Uuid,

    /// Event type in dot notation
    #[serde(rename = "type")]
    pub event_type: String,

    /// Event timestamp
    pub ts: DateTime<Utc>,

    /// Session this event belongs to
    pub session_id: Uuid,

    /// Correlation context
    pub context: EventContext,

    /// Event-specific payload
    pub data: serde_json::Value,

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
    /// Create a new event with the given type, session_id, context, and data
    pub fn new(
        event_type: impl Into<String>,
        session_id: Uuid,
        context: EventContext,
        data: impl Serialize,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            event_type: event_type.into(),
            ts: Utc::now(),
            session_id,
            context,
            data: serde_json::to_value(data).unwrap_or_default(),
            metadata: None,
            tags: None,
            sequence: None,
        }
    }

    /// Create an event with a specific ID (for testing or replay)
    pub fn with_id(
        id: Uuid,
        event_type: impl Into<String>,
        session_id: Uuid,
        context: EventContext,
        data: impl Serialize,
    ) -> Self {
        Self {
            id,
            event_type: event_type.into(),
            ts: Utc::now(),
            session_id,
            context,
            data: serde_json::to_value(data).unwrap_or_default(),
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

    /// Get the session_id
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// Check if this is a message event
    pub fn is_message_event(&self) -> bool {
        self.event_type.starts_with("message.")
    }

    /// Check if this is an atom lifecycle event
    pub fn is_atom_event(&self) -> bool {
        matches!(
            self.event_type.as_str(),
            INPUT_RECEIVED
                | REASON_STARTED
                | REASON_COMPLETED
                | ACT_STARTED
                | ACT_COMPLETED
                | TOOL_CALL_STARTED
                | TOOL_CALL_COMPLETED
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
}

// ============================================================================
// Message Event Data Types
// ============================================================================

use crate::message::{ContentPart, Message};
use crate::tool_types::ToolCall;

/// Metadata about the model used for generation
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Data for message.user event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageUserData {
    /// The user message
    pub message: Message,
}

impl MessageUserData {
    pub fn new(message: Message) -> Self {
        Self { message }
    }
}

/// Data for message.agent event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageAgentData {
    /// The agent message
    pub message: Message,

    /// Metadata about the model used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ModelMetadata>,

    /// Token usage
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

impl MessageAgentData {
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

/// Data for input.received event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputReceivedData {
    /// The user message that was received
    pub message: Message,
}

impl InputReceivedData {
    /// Create a new InputReceivedData from a message
    pub fn new(message: Message) -> Self {
        Self { message }
    }
}

/// Data for reason.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonStartedData {
    /// Agent ID being used
    pub agent_id: Uuid,

    /// Metadata about the model being used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ModelMetadata>,
}

/// Data for reason.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonCompletedData {
    /// Whether the LLM call succeeded
    pub success: bool,

    /// Text response preview (first 200 chars)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_preview: Option<String>,

    /// Whether tool calls were requested
    pub has_tool_calls: bool,

    /// Number of tool calls requested
    pub tool_call_count: usize,

    /// Error message if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ReasonCompletedData {
    pub fn success(text: &str, has_tool_calls: bool, tool_call_count: usize) -> Self {
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
        }
    }

    pub fn failure(error: String) -> Self {
        Self {
            success: false,
            text_preview: None,
            has_tool_calls: false,
            tool_call_count: 0,
            error: Some(error),
        }
    }
}

/// Summary of a tool call (compact form without arguments)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallSummary {
    pub id: String,
    pub name: String,
}

impl From<&ToolCall> for ToolCallSummary {
    fn from(tc: &ToolCall) -> Self {
        Self {
            id: tc.id.clone(),
            name: tc.name.clone(),
        }
    }
}

/// Data for act.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActStartedData {
    /// Tool calls to be executed
    pub tool_calls: Vec<ToolCallSummary>,
}

impl ActStartedData {
    pub fn new(tool_calls: &[ToolCall]) -> Self {
        Self {
            tool_calls: tool_calls.iter().map(ToolCallSummary::from).collect(),
        }
    }
}

/// Data for act.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActCompletedData {
    /// Whether all tool calls completed
    pub completed: bool,

    /// Number of successful tool calls
    pub success_count: usize,

    /// Number of failed tool calls
    pub error_count: usize,
}

/// Data for tool.call_started event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallStartedData {
    /// The tool call being executed
    pub tool_call: ToolCall,
}

/// Data for tool.call_completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallCompletedData {
    /// Tool call ID
    pub tool_call_id: String,

    /// Tool name
    pub tool_name: String,

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
}

impl ToolCallCompletedData {
    pub fn success(tool_call_id: String, tool_name: String, result: Vec<ContentPart>) -> Self {
        Self {
            tool_call_id,
            tool_name,
            success: true,
            status: "success".to_string(),
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(tool_call_id: String, tool_name: String, status: String, error: String) -> Self {
        Self {
            tool_call_id,
            tool_name,
            success: false,
            status,
            result: None,
            error: Some(error),
        }
    }
}

// ============================================================================
// Turn Event Data Types
// ============================================================================

/// Data for turn.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnStartedData {
    /// Turn identifier
    pub turn_id: Uuid,

    /// Input message ID that triggered this turn
    pub input_message_id: Uuid,
}

/// Data for turn.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCompletedData {
    /// Turn identifier
    pub turn_id: Uuid,

    /// Number of iterations in this turn
    pub iterations: usize,

    /// Duration in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Data for turn.failed event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnFailedData {
    /// Turn identifier
    pub turn_id: Uuid,

    /// Error message
    pub error: String,

    /// Error code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

// ============================================================================
// Session Event Data Types
// ============================================================================

/// Data for session.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartedData {
    /// Agent ID
    pub agent_id: Uuid,

    /// Model ID if specified
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Uuid>,
}

// ============================================================================
// Event Builder
// ============================================================================

/// Builder for creating events with fluent API
pub struct EventBuilder {
    event_type: String,
    session_id: Uuid,
    context: EventContext,
}

impl EventBuilder {
    pub fn new(event_type: impl Into<String>, session_id: Uuid) -> Self {
        Self {
            event_type: event_type.into(),
            session_id,
            context: EventContext::empty(),
        }
    }

    pub fn with_turn(mut self, turn_id: Uuid, input_message_id: Uuid) -> Self {
        self.context.turn_id = Some(turn_id);
        self.context.input_message_id = Some(input_message_id);
        self
    }

    pub fn with_exec(mut self, exec_id: Uuid) -> Self {
        self.context.exec_id = Some(exec_id);
        self
    }

    pub fn build<T: Serialize>(self, data: T) -> Event {
        Event::new(self.event_type, self.session_id, self.context, data)
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
        let session_id = Uuid::now_v7();
        let context = EventContext::empty();
        let data = InputReceivedData::new(Message::user("test"));

        let event = Event::new(INPUT_RECEIVED, session_id, context, data);

        assert_eq!(event.event_type, "input.received");
        assert_eq!(event.session_id(), session_id);
        assert!(event.is_atom_event());
        assert!(!event.is_message_event());
    }

    #[test]
    fn test_event_context_from_atom_context() {
        let session_id = Uuid::now_v7();
        let turn_id = Uuid::now_v7();
        let input_message_id = Uuid::now_v7();

        let atom_ctx = AtomContext::new(session_id, turn_id, input_message_id);
        let context = EventContext::from_atom_context(&atom_ctx);

        assert_eq!(context.turn_id, Some(turn_id));
        assert_eq!(context.input_message_id, Some(input_message_id));
        assert_eq!(context.exec_id, Some(atom_ctx.exec_id));
    }

    #[test]
    fn test_event_serialization() {
        let session_id = Uuid::now_v7();
        let context = EventContext::empty();
        let event = Event::new(MESSAGE_USER, session_id, context, serde_json::json!({"test": true}));

        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"type\":\"message.user\""));
        assert!(json.contains("\"session_id\""));
        assert!(json.contains("\"context\""));
        assert!(json.contains("\"data\""));
    }

    #[test]
    fn test_event_builder() {
        let session_id = Uuid::now_v7();
        let turn_id = Uuid::now_v7();
        let input_message_id = Uuid::now_v7();
        let exec_id = Uuid::now_v7();

        let event = EventBuilder::new(REASON_STARTED, session_id)
            .with_turn(turn_id, input_message_id)
            .with_exec(exec_id)
            .build(ReasonStartedData {
                agent_id: Uuid::now_v7(),
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
        let data = ReasonCompletedData::success("Hello world", true, 2);
        assert!(data.success);
        assert_eq!(data.text_preview, Some("Hello world".to_string()));
        assert!(data.has_tool_calls);
        assert_eq!(data.tool_call_count, 2);

        let data = ReasonCompletedData::failure("Network error".to_string());
        assert!(!data.success);
        assert_eq!(data.error, Some("Network error".to_string()));
    }

    #[test]
    fn test_message_event_types() {
        assert_eq!(MESSAGE_USER, "message.user");
        assert_eq!(MESSAGE_AGENT, "message.agent");
    }

    #[test]
    fn test_turn_event_types() {
        assert_eq!(TURN_STARTED, "turn.started");
        assert_eq!(TURN_COMPLETED, "turn.completed");
        assert_eq!(TURN_FAILED, "turn.failed");
    }

    #[test]
    fn test_input_received_event() {
        assert_eq!(INPUT_RECEIVED, "input.received");
    }
}
