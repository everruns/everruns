//! Atom Event Declarations
//!
//! This module defines typed events emitted by atoms during execution.
//! Events provide observability into the atom execution lifecycle.
//!
//! Event naming convention: `{atom}.{state}` where:
//! - atom: input, reason, act, tool
//! - state: started, completed, failed
//!
//! All events include the AtomContext for correlation and tracing.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::AtomContext;
use crate::message::Message;
use crate::tool_types::ToolCall;

// ============================================================================
// Event Type Constants
// ============================================================================

/// Event type for InputAtom started
pub const INPUT_STARTED: &str = "input.started";
/// Event type for InputAtom completed
pub const INPUT_COMPLETED: &str = "input.completed";

/// Event type for ReasonAtom started
pub const REASON_STARTED: &str = "reason.started";
/// Event type for ReasonAtom completed
pub const REASON_COMPLETED: &str = "reason.completed";

/// Event type for ActAtom started
pub const ACT_STARTED: &str = "act.started";
/// Event type for ActAtom completed
pub const ACT_COMPLETED: &str = "act.completed";

/// Event type for individual tool call started
pub const TOOL_CALL_STARTED: &str = "tool.call_started";
/// Event type for individual tool call completed
pub const TOOL_CALL_COMPLETED: &str = "tool.call_completed";

// ============================================================================
// Atom Event Enum
// ============================================================================

/// Typed events emitted by atoms during execution
///
/// Each variant contains the relevant payload for that event type.
/// Events are serializable for storage and SSE streaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AtomEvent {
    // ========================================================================
    // InputAtom Events
    // ========================================================================
    /// Emitted when InputAtom starts processing
    #[serde(rename = "input.started")]
    InputStarted(InputStartedEvent),

    /// Emitted when InputAtom completes successfully
    #[serde(rename = "input.completed")]
    InputCompleted(InputCompletedEvent),

    // ========================================================================
    // ReasonAtom Events
    // ========================================================================
    /// Emitted when ReasonAtom starts LLM call
    #[serde(rename = "reason.started")]
    ReasonStarted(ReasonStartedEvent),

    /// Emitted when ReasonAtom completes (success or failure)
    #[serde(rename = "reason.completed")]
    ReasonCompleted(ReasonCompletedEvent),

    // ========================================================================
    // ActAtom Events
    // ========================================================================
    /// Emitted when ActAtom starts parallel tool execution
    #[serde(rename = "act.started")]
    ActStarted(ActStartedEvent),

    /// Emitted when ActAtom completes all tool calls
    #[serde(rename = "act.completed")]
    ActCompleted(ActCompletedEvent),

    // ========================================================================
    // Tool Call Events
    // ========================================================================
    /// Emitted when a single tool call starts
    #[serde(rename = "tool.call_started")]
    ToolCallStarted(ToolCallStartedEvent),

    /// Emitted when a single tool call completes
    #[serde(rename = "tool.call_completed")]
    ToolCallCompleted(ToolCallCompletedEvent),
}

impl AtomEvent {
    /// Get the event type string for this event
    pub fn event_type(&self) -> &'static str {
        match self {
            AtomEvent::InputStarted(_) => INPUT_STARTED,
            AtomEvent::InputCompleted(_) => INPUT_COMPLETED,
            AtomEvent::ReasonStarted(_) => REASON_STARTED,
            AtomEvent::ReasonCompleted(_) => REASON_COMPLETED,
            AtomEvent::ActStarted(_) => ACT_STARTED,
            AtomEvent::ActCompleted(_) => ACT_COMPLETED,
            AtomEvent::ToolCallStarted(_) => TOOL_CALL_STARTED,
            AtomEvent::ToolCallCompleted(_) => TOOL_CALL_COMPLETED,
        }
    }

    /// Get the session ID from any event
    pub fn session_id(&self) -> Uuid {
        match self {
            AtomEvent::InputStarted(e) => e.context.session_id,
            AtomEvent::InputCompleted(e) => e.context.session_id,
            AtomEvent::ReasonStarted(e) => e.context.session_id,
            AtomEvent::ReasonCompleted(e) => e.context.session_id,
            AtomEvent::ActStarted(e) => e.context.session_id,
            AtomEvent::ActCompleted(e) => e.context.session_id,
            AtomEvent::ToolCallStarted(e) => e.context.session_id,
            AtomEvent::ToolCallCompleted(e) => e.context.session_id,
        }
    }

    /// Convert event to JSON value for storage
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

// ============================================================================
// InputAtom Event Payloads
// ============================================================================

/// Payload for input.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputStartedEvent {
    /// Atom execution context
    pub context: AtomContext,
}

impl InputStartedEvent {
    pub fn new(context: AtomContext) -> Self {
        Self { context }
    }
}

/// Payload for input.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputCompletedEvent {
    /// Atom execution context
    pub context: AtomContext,
    /// The user message that was retrieved
    pub message: Message,
}

impl InputCompletedEvent {
    pub fn new(context: AtomContext, message: Message) -> Self {
        Self { context, message }
    }
}

// ============================================================================
// ReasonAtom Event Payloads
// ============================================================================

/// Payload for reason.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonStartedEvent {
    /// Atom execution context
    pub context: AtomContext,
    /// Agent ID being used
    pub agent_id: Uuid,
    /// Model being used (if known at start)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl ReasonStartedEvent {
    pub fn new(context: AtomContext, agent_id: Uuid) -> Self {
        Self {
            context,
            agent_id,
            model: None,
        }
    }

    pub fn with_model(mut self, model: String) -> Self {
        self.model = Some(model);
        self
    }
}

/// Payload for reason.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonCompletedEvent {
    /// Atom execution context
    pub context: AtomContext,
    /// Whether the LLM call succeeded
    pub success: bool,
    /// Text response from the model (truncated for event)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_preview: Option<String>,
    /// Whether tool calls were requested
    pub has_tool_calls: bool,
    /// Number of tool calls requested
    pub tool_call_count: usize,
    /// Error message if the call failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ReasonCompletedEvent {
    pub fn success(
        context: AtomContext,
        text: &str,
        has_tool_calls: bool,
        tool_call_count: usize,
    ) -> Self {
        // Truncate text for event payload (first 200 chars)
        let text_preview = if text.is_empty() {
            None
        } else {
            Some(text.chars().take(200).collect())
        };

        Self {
            context,
            success: true,
            text_preview,
            has_tool_calls,
            tool_call_count,
            error: None,
        }
    }

    pub fn failure(context: AtomContext, error: String) -> Self {
        Self {
            context,
            success: false,
            text_preview: None,
            has_tool_calls: false,
            tool_call_count: 0,
            error: Some(error),
        }
    }
}

// ============================================================================
// ActAtom Event Payloads
// ============================================================================

/// Payload for act.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActStartedEvent {
    /// Atom execution context
    pub context: AtomContext,
    /// Tool calls to be executed
    pub tool_calls: Vec<ToolCallSummary>,
}

impl ActStartedEvent {
    pub fn new(context: AtomContext, tool_calls: &[ToolCall]) -> Self {
        Self {
            context,
            tool_calls: tool_calls.iter().map(ToolCallSummary::from).collect(),
        }
    }
}

/// Payload for act.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActCompletedEvent {
    /// Atom execution context
    pub context: AtomContext,
    /// Whether all tool calls completed
    pub completed: bool,
    /// Number of successful tool calls
    pub success_count: usize,
    /// Number of failed tool calls
    pub error_count: usize,
}

impl ActCompletedEvent {
    pub fn new(
        context: AtomContext,
        completed: bool,
        success_count: usize,
        error_count: usize,
    ) -> Self {
        Self {
            context,
            completed,
            success_count,
            error_count,
        }
    }
}

// ============================================================================
// Tool Call Event Payloads
// ============================================================================

/// Summary of a tool call (without full arguments for compactness)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallSummary {
    /// Tool call ID
    pub id: String,
    /// Tool name
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

/// Payload for tool.call_started event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallStartedEvent {
    /// Atom execution context
    pub context: AtomContext,
    /// The tool call being executed
    pub tool_call: ToolCall,
}

impl ToolCallStartedEvent {
    pub fn new(context: AtomContext, tool_call: ToolCall) -> Self {
        Self { context, tool_call }
    }
}

/// Payload for tool.call_completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallCompletedEvent {
    /// Atom execution context
    pub context: AtomContext,
    /// Tool call ID
    pub tool_call_id: String,
    /// Tool name
    pub tool_name: String,
    /// Whether the tool call succeeded
    pub success: bool,
    /// Status: "success", "error", "timeout", or "cancelled"
    pub status: String,
    /// Error message if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ToolCallCompletedEvent {
    pub fn success(context: AtomContext, tool_call_id: String, tool_name: String) -> Self {
        Self {
            context,
            tool_call_id,
            tool_name,
            success: true,
            status: "success".to_string(),
            error: None,
        }
    }

    pub fn failure(
        context: AtomContext,
        tool_call_id: String,
        tool_name: String,
        status: String,
        error: String,
    ) -> Self {
        Self {
            context,
            tool_call_id,
            tool_name,
            success: false,
            status,
            error: Some(error),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> AtomContext {
        AtomContext::new(Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7())
    }

    #[test]
    fn test_event_type_constants() {
        assert_eq!(INPUT_STARTED, "input.started");
        assert_eq!(INPUT_COMPLETED, "input.completed");
        assert_eq!(REASON_STARTED, "reason.started");
        assert_eq!(REASON_COMPLETED, "reason.completed");
        assert_eq!(ACT_STARTED, "act.started");
        assert_eq!(ACT_COMPLETED, "act.completed");
        assert_eq!(TOOL_CALL_STARTED, "tool.call_started");
        assert_eq!(TOOL_CALL_COMPLETED, "tool.call_completed");
    }

    #[test]
    fn test_atom_event_type() {
        let ctx = test_context();

        let event = AtomEvent::InputStarted(InputStartedEvent::new(ctx.clone()));
        assert_eq!(event.event_type(), INPUT_STARTED);

        let event = AtomEvent::ReasonStarted(ReasonStartedEvent::new(ctx.clone(), Uuid::now_v7()));
        assert_eq!(event.event_type(), REASON_STARTED);
    }

    #[test]
    fn test_atom_event_session_id() {
        let ctx = test_context();
        let session_id = ctx.session_id;

        let event = AtomEvent::InputStarted(InputStartedEvent::new(ctx));
        assert_eq!(event.session_id(), session_id);
    }

    #[test]
    fn test_reason_completed_text_truncation() {
        let ctx = test_context();
        let long_text = "a".repeat(500);

        let event = ReasonCompletedEvent::success(ctx, &long_text, false, 0);

        assert!(event.text_preview.is_some());
        assert_eq!(event.text_preview.as_ref().unwrap().len(), 200);
    }

    #[test]
    fn test_atom_event_serialization() {
        let ctx = test_context();
        let event = AtomEvent::InputStarted(InputStartedEvent::new(ctx));

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"input.started\""));

        let parsed: AtomEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, AtomEvent::InputStarted(_)));
    }

    #[test]
    fn test_tool_call_summary() {
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "Tokyo"}),
        };

        let summary = ToolCallSummary::from(&tool_call);
        assert_eq!(summary.id, "call_123");
        assert_eq!(summary.name, "get_weather");
    }
}
