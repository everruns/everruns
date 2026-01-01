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
pub const MESSAGE_ASSISTANT: &str = "message.assistant";
pub const MESSAGE_TOOL_CALL: &str = "message.tool_call";
pub const MESSAGE_TOOL_RESULT: &str = "message.tool_result";

// Atom lifecycle events
pub const INPUT_STARTED: &str = "input.started";
pub const INPUT_COMPLETED: &str = "input.completed";
pub const REASON_STARTED: &str = "reason.started";
pub const REASON_COMPLETED: &str = "reason.completed";
pub const ACT_STARTED: &str = "act.started";
pub const ACT_COMPLETED: &str = "act.completed";
pub const TOOL_CALL_STARTED: &str = "tool.call_started";
pub const TOOL_CALL_COMPLETED: &str = "tool.call_completed";

// Session events
pub const SESSION_STARTED: &str = "session.started";
pub const SESSION_COMPLETED: &str = "session.completed";
pub const SESSION_FAILED: &str = "session.failed";

// ============================================================================
// Event Context
// ============================================================================

/// Context for event correlation and tracing
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct EventContext {
    /// Session this event belongs to
    pub session_id: Uuid,

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
    /// Create a minimal context with just session_id
    pub fn session(session_id: Uuid) -> Self {
        Self {
            session_id,
            ..Default::default()
        }
    }

    /// Create a full context for atom execution
    pub fn atom(session_id: Uuid, turn_id: Uuid, input_message_id: Uuid, exec_id: Uuid) -> Self {
        Self {
            session_id,
            turn_id: Some(turn_id),
            input_message_id: Some(input_message_id),
            exec_id: Some(exec_id),
        }
    }

    /// Create a context for turn-scoped events (without exec_id)
    pub fn turn(session_id: Uuid, turn_id: Uuid, input_message_id: Uuid) -> Self {
        Self {
            session_id,
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
/// - `context`: Correlation context for tracing
/// - `data`: Event-specific payload
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

    /// Correlation context
    pub context: EventContext,

    /// Event-specific payload
    pub data: serde_json::Value,

    /// Sequence number within session (for ordering)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<i32>,
}

impl Event {
    /// Create a new event with the given type, context, and data
    pub fn new(event_type: impl Into<String>, context: EventContext, data: impl Serialize) -> Self {
        Self {
            id: Uuid::now_v7(),
            event_type: event_type.into(),
            ts: Utc::now(),
            context,
            data: serde_json::to_value(data).unwrap_or_default(),
            sequence: None,
        }
    }

    /// Create an event with a specific ID (for testing or replay)
    pub fn with_id(
        id: Uuid,
        event_type: impl Into<String>,
        context: EventContext,
        data: impl Serialize,
    ) -> Self {
        Self {
            id,
            event_type: event_type.into(),
            ts: Utc::now(),
            context,
            data: serde_json::to_value(data).unwrap_or_default(),
            sequence: None,
        }
    }

    /// Set the sequence number
    pub fn with_sequence(mut self, sequence: i32) -> Self {
        self.sequence = Some(sequence);
        self
    }

    /// Get the session_id from context
    pub fn session_id(&self) -> Uuid {
        self.context.session_id
    }

    /// Check if this is a message event
    pub fn is_message_event(&self) -> bool {
        self.event_type.starts_with("message.")
    }

    /// Check if this is an atom lifecycle event
    pub fn is_atom_event(&self) -> bool {
        matches!(
            self.event_type.as_str(),
            INPUT_STARTED
                | INPUT_COMPLETED
                | REASON_STARTED
                | REASON_COMPLETED
                | ACT_STARTED
                | ACT_COMPLETED
                | TOOL_CALL_STARTED
                | TOOL_CALL_COMPLETED
        )
    }

    /// Check if this is a session lifecycle event
    pub fn is_session_event(&self) -> bool {
        self.event_type.starts_with("session.")
    }
}

// ============================================================================
// Message Event Data Types
// ============================================================================

use crate::message::ContentPart;
use crate::tool_types::ToolCall;
use crate::Controls;
use std::collections::HashMap;

/// Data for message.user event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageUserData {
    /// Unique message identifier
    pub message_id: Uuid,

    /// Message content
    pub content: Vec<ContentPart>,

    /// Optional execution controls
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controls: Option<Controls>,

    /// Optional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,

    /// Optional tags
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Data for message.assistant event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageAssistantData {
    /// Unique message identifier
    pub message_id: Uuid,

    /// Message content
    pub content: Vec<ContentPart>,

    /// Model used for generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Token usage
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// Token usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Data for message.tool_call event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageToolCallData {
    /// Unique message identifier
    pub message_id: Uuid,

    /// Tool calls requested by the assistant
    pub tool_calls: Vec<ToolCall>,
}

/// Data for message.tool_result event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageToolResultData {
    /// Unique message identifier
    pub message_id: Uuid,

    /// ID of the tool call this result is for
    pub tool_call_id: String,

    /// Name of the tool
    pub tool_name: String,

    /// Result content
    pub content: Vec<ContentPart>,

    /// Whether this is an error result
    #[serde(default)]
    pub is_error: bool,
}

// ============================================================================
// Atom Event Data Types
// ============================================================================

use crate::message::Message;

/// Data for input.started event (empty - just signals start)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InputStartedData {}

/// Data for input.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputCompletedData {
    /// The user message that was retrieved
    pub message: Message,
}

/// Data for reason.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonStartedData {
    /// Agent ID being used
    pub agent_id: Uuid,

    /// Model being used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
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

    /// Error message if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ToolCallCompletedData {
    pub fn success(tool_call_id: String, tool_name: String) -> Self {
        Self {
            tool_call_id,
            tool_name,
            success: true,
            status: "success".to_string(),
            error: None,
        }
    }

    pub fn failure(tool_call_id: String, tool_name: String, status: String, error: String) -> Self {
        Self {
            tool_call_id,
            tool_name,
            success: false,
            status,
            error: Some(error),
        }
    }
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

/// Data for session.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCompletedData {
    /// Duration in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Data for session.failed event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFailedData {
    /// Error message
    pub error: String,

    /// Error code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

// ============================================================================
// Event Builder
// ============================================================================

/// Builder for creating events with fluent API
pub struct EventBuilder {
    event_type: String,
    context: EventContext,
}

impl EventBuilder {
    pub fn new(event_type: impl Into<String>, session_id: Uuid) -> Self {
        Self {
            event_type: event_type.into(),
            context: EventContext::session(session_id),
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
        Event::new(self.event_type, self.context, data)
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
        let context = EventContext::session(session_id);
        let data = InputStartedData::default();

        let event = Event::new(INPUT_STARTED, context, data);

        assert_eq!(event.event_type, "input.started");
        assert_eq!(event.session_id(), session_id);
        assert!(event.is_atom_event());
        assert!(!event.is_message_event());
    }

    #[test]
    fn test_event_context_atom() {
        let session_id = Uuid::now_v7();
        let turn_id = Uuid::now_v7();
        let input_message_id = Uuid::now_v7();
        let exec_id = Uuid::now_v7();

        let context = EventContext::atom(session_id, turn_id, input_message_id, exec_id);

        assert_eq!(context.session_id, session_id);
        assert_eq!(context.turn_id, Some(turn_id));
        assert_eq!(context.input_message_id, Some(input_message_id));
        assert_eq!(context.exec_id, Some(exec_id));
    }

    #[test]
    fn test_event_serialization() {
        let session_id = Uuid::now_v7();
        let context = EventContext::session(session_id);
        let event = Event::new(MESSAGE_USER, context, serde_json::json!({"test": true}));

        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"type\":\"message.user\""));
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
                model: Some("gpt-4o".to_string()),
            });

        assert_eq!(event.event_type, "reason.started");
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
        assert_eq!(MESSAGE_ASSISTANT, "message.assistant");
        assert_eq!(MESSAGE_TOOL_CALL, "message.tool_call");
        assert_eq!(MESSAGE_TOOL_RESULT, "message.tool_result");
    }
}
