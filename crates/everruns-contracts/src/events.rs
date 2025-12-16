// AG-UI protocol event types for SSE streaming
// Spec: https://docs.ag-ui.com/concepts/events

use chrono::Utc;
use serde::{Deserialize, Serialize};

/// AG-UI Protocol Events
/// These match the official AG-UI specification for CopilotKit compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgUiEvent {
    // Lifecycle Events
    RunStarted(RunStartedEvent),
    RunFinished(RunFinishedEvent),
    RunError(RunErrorEvent),
    StepStarted(StepStartedEvent),
    StepFinished(StepFinishedEvent),

    // Text Message Events (Start-Content-End pattern)
    TextMessageStart(TextMessageStartEvent),
    TextMessageContent(TextMessageContentEvent),
    TextMessageEnd(TextMessageEndEvent),

    // Tool Call Events
    ToolCallStart(ToolCallStartEvent),
    ToolCallArgs(ToolCallArgsEvent),
    ToolCallEnd(ToolCallEndEvent),
    ToolCallResult(ToolCallResultEvent),

    // State Events
    StateSnapshot(StateSnapshotEvent),
    StateDelta(StateDeltaEvent),
    MessagesSnapshot(MessagesSnapshotEvent),

    // Custom Events
    Custom(CustomEvent),
}

// Lifecycle Events

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStartedEvent {
    pub thread_id: String,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFinishedEvent {
    pub thread_id: String,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunErrorEvent {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepStartedEvent {
    pub step_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepFinishedEvent {
    pub step_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

// Text Message Events

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMessageStartEvent {
    pub message_id: String,
    pub role: MessageRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMessageContentEvent {
    pub message_id: String,
    pub delta: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMessageEndEvent {
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    Developer,
    System,
    Assistant,
    User,
    Tool,
}

// Tool Call Events

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallStartEvent {
    pub tool_call_id: String,
    pub tool_call_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallArgsEvent {
    pub tool_call_id: String,
    pub delta: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallEndEvent {
    pub tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallResultEvent {
    pub message_id: String,
    pub tool_call_id: String,
    pub content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<MessageRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

// State Events

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshotEvent {
    pub snapshot: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateDeltaEvent {
    pub delta: Vec<JsonPatchOp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonPatchOp {
    pub op: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesSnapshotEvent {
    pub messages: Vec<SnapshotMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
}

// Custom Events

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomEvent {
    pub name: String,
    pub value: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

// Helper functions to create events with current timestamp
impl AgUiEvent {
    pub fn run_started(thread_id: impl Into<String>, run_id: impl Into<String>) -> Self {
        AgUiEvent::RunStarted(RunStartedEvent {
            thread_id: thread_id.into(),
            run_id: run_id.into(),
            timestamp: Some(Utc::now().timestamp_millis()),
        })
    }

    pub fn run_finished(thread_id: impl Into<String>, run_id: impl Into<String>) -> Self {
        AgUiEvent::RunFinished(RunFinishedEvent {
            thread_id: thread_id.into(),
            run_id: run_id.into(),
            result: None,
            timestamp: Some(Utc::now().timestamp_millis()),
        })
    }

    pub fn run_error(message: impl Into<String>) -> Self {
        AgUiEvent::RunError(RunErrorEvent {
            message: message.into(),
            code: None,
            timestamp: Some(Utc::now().timestamp_millis()),
        })
    }

    pub fn text_message_start(message_id: impl Into<String>, role: MessageRole) -> Self {
        AgUiEvent::TextMessageStart(TextMessageStartEvent {
            message_id: message_id.into(),
            role,
            timestamp: Some(Utc::now().timestamp_millis()),
        })
    }

    pub fn text_message_content(message_id: impl Into<String>, delta: impl Into<String>) -> Self {
        AgUiEvent::TextMessageContent(TextMessageContentEvent {
            message_id: message_id.into(),
            delta: delta.into(),
            timestamp: Some(Utc::now().timestamp_millis()),
        })
    }

    pub fn text_message_end(message_id: impl Into<String>) -> Self {
        AgUiEvent::TextMessageEnd(TextMessageEndEvent {
            message_id: message_id.into(),
            timestamp: Some(Utc::now().timestamp_millis()),
        })
    }

    pub fn tool_call_start(
        tool_call_id: impl Into<String>,
        tool_call_name: impl Into<String>,
    ) -> Self {
        AgUiEvent::ToolCallStart(ToolCallStartEvent {
            tool_call_id: tool_call_id.into(),
            tool_call_name: tool_call_name.into(),
            parent_message_id: None,
            timestamp: Some(Utc::now().timestamp_millis()),
        })
    }

    pub fn tool_call_args(tool_call_id: impl Into<String>, delta: impl Into<String>) -> Self {
        AgUiEvent::ToolCallArgs(ToolCallArgsEvent {
            tool_call_id: tool_call_id.into(),
            delta: delta.into(),
            timestamp: Some(Utc::now().timestamp_millis()),
        })
    }

    pub fn tool_call_end(tool_call_id: impl Into<String>) -> Self {
        AgUiEvent::ToolCallEnd(ToolCallEndEvent {
            tool_call_id: tool_call_id.into(),
            timestamp: Some(Utc::now().timestamp_millis()),
        })
    }

    pub fn tool_call_result(
        message_id: impl Into<String>,
        tool_call_id: impl Into<String>,
        content: serde_json::Value,
    ) -> Self {
        AgUiEvent::ToolCallResult(ToolCallResultEvent {
            message_id: message_id.into(),
            tool_call_id: tool_call_id.into(),
            content,
            role: Some(MessageRole::Tool),
            timestamp: Some(Utc::now().timestamp_millis()),
        })
    }

    pub fn custom(name: impl Into<String>, value: serde_json::Value) -> Self {
        AgUiEvent::Custom(CustomEvent {
            name: name.into(),
            value,
            timestamp: Some(Utc::now().timestamp_millis()),
        })
    }
}

// HITL-specific custom event kinds
pub const HITL_REQUEST: &str = "hitl.request";
pub const HITL_DECISION: &str = "hitl.decision";

// M2 Session lifecycle event types (stored in database)
pub const EVENT_SESSION_STARTED: &str = "session.started";
pub const EVENT_SESSION_FINISHED: &str = "session.finished";
pub const EVENT_SESSION_ERROR: &str = "session.error";
pub const EVENT_MESSAGE_USER: &str = "message.user";
pub const EVENT_MESSAGE_ASSISTANT: &str = "message.assistant";
pub const EVENT_MESSAGE_SYSTEM: &str = "message.system";
pub const EVENT_TEXT_START: &str = "text.start";
pub const EVENT_TEXT_DELTA: &str = "text.delta";
pub const EVENT_TEXT_END: &str = "text.end";
pub const EVENT_TOOL_CALL_START: &str = "tool.call.start";
pub const EVENT_TOOL_CALL_ARGS: &str = "tool.call.args";
pub const EVENT_TOOL_CALL_END: &str = "tool.call.end";
pub const EVENT_TOOL_RESULT: &str = "tool.result";
pub const EVENT_STATE_SNAPSHOT: &str = "state.snapshot";
pub const EVENT_STATE_DELTA: &str = "state.delta";

// M2 Session-based helper functions
impl AgUiEvent {
    /// Create a session started event (maps to RunStarted for AG-UI compatibility)
    pub fn session_started(session_id: impl Into<String>) -> Self {
        let session_id = session_id.into();
        AgUiEvent::RunStarted(RunStartedEvent {
            thread_id: session_id.clone(),
            run_id: session_id,
            timestamp: Some(Utc::now().timestamp_millis()),
        })
    }

    /// Create a session finished event (maps to RunFinished for AG-UI compatibility)
    pub fn session_finished(session_id: impl Into<String>) -> Self {
        let session_id = session_id.into();
        AgUiEvent::RunFinished(RunFinishedEvent {
            thread_id: session_id.clone(),
            run_id: session_id,
            result: None,
            timestamp: Some(Utc::now().timestamp_millis()),
        })
    }

    /// Create a session error event (maps to RunError for AG-UI compatibility)
    pub fn session_error(message: impl Into<String>) -> Self {
        AgUiEvent::RunError(RunErrorEvent {
            message: message.into(),
            code: None,
            timestamp: Some(Utc::now().timestamp_millis()),
        })
    }
}
