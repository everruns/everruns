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

use crate::message::{ContentPart, Message};
use crate::tool_types::ToolCall;

// ============================================================================
// Event Type Constants
// ============================================================================

pub const MESSAGE_USER: &str = "message.user";
pub const MESSAGE_AGENT: &str = "message.agent";
pub const TURN_STARTED: &str = "turn.started";
pub const TURN_COMPLETED: &str = "turn.completed";
pub const TURN_FAILED: &str = "turn.failed";
pub const INPUT_RECEIVED: &str = "input.received";
pub const REASON_STARTED: &str = "reason.started";
pub const REASON_COMPLETED: &str = "reason.completed";
pub const ACT_STARTED: &str = "act.started";
pub const ACT_COMPLETED: &str = "act.completed";
pub const TOOL_CALL_STARTED: &str = "tool.call_started";
pub const TOOL_CALL_COMPLETED: &str = "tool.call_completed";
pub const LLM_GENERATION: &str = "llm.generation";
pub const SESSION_STARTED: &str = "session.started";
pub const UNKNOWN: &str = "unknown";

// ============================================================================
// Event Context
// ============================================================================

/// Context for event correlation and tracing
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct EventContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_message_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exec_id: Option<Uuid>,
}

impl EventContext {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn turn(turn_id: Uuid, input_message_id: Uuid) -> Self {
        Self {
            turn_id: Some(turn_id),
            input_message_id: Some(input_message_id),
            exec_id: None,
        }
    }

    pub fn with_exec(mut self, exec_id: Uuid) -> Self {
        self.exec_id = Some(exec_id);
        self
    }
}

// ============================================================================
// Standard Event Schema
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Event {
    pub id: Uuid,
    #[serde(rename = "type")]
    pub event_type: String,
    pub ts: DateTime<Utc>,
    pub session_id: Uuid,
    pub context: EventContext,
    pub data: EventData,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<i32>,
}

impl Event {
    pub fn new(session_id: Uuid, context: EventContext, data: impl Into<EventData>) -> Self {
        let data = data.into();
        let event_type = data.event_type().to_string();
        Self {
            id: Uuid::now_v7(),
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

    pub fn with_id(
        id: Uuid,
        session_id: Uuid,
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

    pub fn with_sequence(mut self, sequence: i32) -> Self {
        self.sequence = Some(sequence);
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = Some(tags);
        self
    }

    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    pub fn is_message_event(&self) -> bool {
        self.event_type.starts_with("message.")
    }

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

    pub fn is_turn_event(&self) -> bool {
        self.event_type.starts_with("turn.")
    }

    pub fn is_session_event(&self) -> bool {
        self.event_type.starts_with("session.")
    }
}

// ============================================================================
// Message Event Data Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ModelMetadata {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MessageUserData {
    pub message: Message,
}

impl MessageUserData {
    pub fn new(message: Message) -> Self {
        Self { message }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MessageAgentData {
    pub message: Message,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ModelMetadata>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct InputReceivedData {
    pub message: Message,
}

impl InputReceivedData {
    pub fn new(message: Message) -> Self {
        Self { message }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonStartedData {
    pub agent_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ModelMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonCompletedData {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_preview: Option<String>,
    pub has_tool_calls: bool,
    pub tool_call_count: usize,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ActStartedData {
    pub tool_calls: Vec<ToolCallSummary>,
}

impl ActStartedData {
    pub fn new(tool_calls: &[ToolCall]) -> Self {
        Self {
            tool_calls: tool_calls.iter().map(ToolCallSummary::from).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ActCompletedData {
    pub completed: bool,
    pub success_count: usize,
    pub error_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolCallStartedData {
    pub tool_call: ToolCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolCallCompletedData {
    pub tool_call_id: String,
    pub tool_name: String,
    pub success: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Vec<ContentPart>>,
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
// LLM Event Data Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmGenerationOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmGenerationMetadata {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmGenerationData {
    pub messages: Vec<Message>,
    pub output: LlmGenerationOutput,
    pub metadata: LlmGenerationMetadata,
}

impl LlmGenerationData {
    pub fn success(
        messages: Vec<Message>,
        text: Option<String>,
        tool_calls: Vec<ToolCall>,
        model: String,
        provider: Option<String>,
        usage: Option<TokenUsage>,
        duration_ms: Option<u64>,
    ) -> Self {
        Self {
            messages,
            output: LlmGenerationOutput { text, tool_calls },
            metadata: LlmGenerationMetadata {
                model,
                provider,
                usage,
                duration_ms,
                success: true,
                error: None,
            },
        }
    }

    pub fn failure(
        messages: Vec<Message>,
        model: String,
        provider: Option<String>,
        error: String,
        duration_ms: Option<u64>,
    ) -> Self {
        Self {
            messages,
            output: LlmGenerationOutput {
                text: None,
                tool_calls: vec![],
            },
            metadata: LlmGenerationMetadata {
                model,
                provider,
                usage: None,
                duration_ms,
                success: false,
                error: Some(error),
            },
        }
    }
}

// ============================================================================
// Turn Event Data Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TurnStartedData {
    pub turn_id: Uuid,
    pub input_message_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TurnCompletedData {
    pub turn_id: Uuid,
    pub iterations: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TurnFailedData {
    pub turn_id: Uuid,
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

// ============================================================================
// Session Event Data Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionStartedData {
    pub agent_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Uuid>,
}

// ============================================================================
// EventData Enum
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(
    title = "EventData",
    description = "Event-specific payload. The schema depends on the event type field.",
    example = json!({"message": {"id": "...", "role": "user", "content": []}})
))]
pub enum EventData {
    MessageUser(MessageUserData),
    MessageAgent(MessageAgentData),
    TurnStarted(TurnStartedData),
    TurnCompleted(TurnCompletedData),
    TurnFailed(TurnFailedData),
    InputReceived(InputReceivedData),
    ReasonStarted(ReasonStartedData),
    ReasonCompleted(ReasonCompletedData),
    ActStarted(ActStartedData),
    ActCompleted(ActCompletedData),
    ToolCallStarted(ToolCallStartedData),
    ToolCallCompleted(ToolCallCompletedData),
    LlmGeneration(LlmGenerationData),
    SessionStarted(SessionStartedData),
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    Raw(serde_json::Value),
}

impl EventData {
    pub fn event_type(&self) -> &'static str {
        match self {
            EventData::MessageUser(_) => MESSAGE_USER,
            EventData::MessageAgent(_) => MESSAGE_AGENT,
            EventData::TurnStarted(_) => TURN_STARTED,
            EventData::TurnCompleted(_) => TURN_COMPLETED,
            EventData::TurnFailed(_) => TURN_FAILED,
            EventData::InputReceived(_) => INPUT_RECEIVED,
            EventData::ReasonStarted(_) => REASON_STARTED,
            EventData::ReasonCompleted(_) => REASON_COMPLETED,
            EventData::ActStarted(_) => ACT_STARTED,
            EventData::ActCompleted(_) => ACT_COMPLETED,
            EventData::ToolCallStarted(_) => TOOL_CALL_STARTED,
            EventData::ToolCallCompleted(_) => TOOL_CALL_COMPLETED,
            EventData::LlmGeneration(_) => LLM_GENERATION,
            EventData::SessionStarted(_) => SESSION_STARTED,
            EventData::Raw(_) => UNKNOWN,
        }
    }

    pub fn raw(value: serde_json::Value) -> Self {
        EventData::Raw(value)
    }
}

// From implementations
impl From<MessageUserData> for EventData {
    fn from(data: MessageUserData) -> Self {
        EventData::MessageUser(data)
    }
}

impl From<MessageAgentData> for EventData {
    fn from(data: MessageAgentData) -> Self {
        EventData::MessageAgent(data)
    }
}

impl From<TurnStartedData> for EventData {
    fn from(data: TurnStartedData) -> Self {
        EventData::TurnStarted(data)
    }
}

impl From<TurnCompletedData> for EventData {
    fn from(data: TurnCompletedData) -> Self {
        EventData::TurnCompleted(data)
    }
}

impl From<TurnFailedData> for EventData {
    fn from(data: TurnFailedData) -> Self {
        EventData::TurnFailed(data)
    }
}

impl From<InputReceivedData> for EventData {
    fn from(data: InputReceivedData) -> Self {
        EventData::InputReceived(data)
    }
}

impl From<ReasonStartedData> for EventData {
    fn from(data: ReasonStartedData) -> Self {
        EventData::ReasonStarted(data)
    }
}

impl From<ReasonCompletedData> for EventData {
    fn from(data: ReasonCompletedData) -> Self {
        EventData::ReasonCompleted(data)
    }
}

impl From<ActStartedData> for EventData {
    fn from(data: ActStartedData) -> Self {
        EventData::ActStarted(data)
    }
}

impl From<ActCompletedData> for EventData {
    fn from(data: ActCompletedData) -> Self {
        EventData::ActCompleted(data)
    }
}

impl From<ToolCallStartedData> for EventData {
    fn from(data: ToolCallStartedData) -> Self {
        EventData::ToolCallStarted(data)
    }
}

impl From<ToolCallCompletedData> for EventData {
    fn from(data: ToolCallCompletedData) -> Self {
        EventData::ToolCallCompleted(data)
    }
}

impl From<LlmGenerationData> for EventData {
    fn from(data: LlmGenerationData) -> Self {
        EventData::LlmGeneration(data)
    }
}

impl From<SessionStartedData> for EventData {
    fn from(data: SessionStartedData) -> Self {
        EventData::SessionStarted(data)
    }
}

impl From<serde_json::Value> for EventData {
    fn from(data: serde_json::Value) -> Self {
        EventData::Raw(data)
    }
}

// ============================================================================
// Event Builder
// ============================================================================

pub struct EventBuilder {
    session_id: Uuid,
    context: EventContext,
}

impl EventBuilder {
    pub fn new(session_id: Uuid) -> Self {
        Self {
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

    pub fn build(self, data: impl Into<EventData>) -> Event {
        Event::new(self.session_id, self.context, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_creation() {
        let session_id = Uuid::now_v7();
        let context = EventContext::empty();
        let data = InputReceivedData::new(Message::user("test"));

        let event = Event::new(session_id, context, data);

        assert_eq!(event.event_type, "input.received");
        assert_eq!(event.session_id(), session_id);
        assert!(event.is_atom_event());
    }

    #[test]
    fn test_event_builder() {
        let session_id = Uuid::now_v7();
        let turn_id = Uuid::now_v7();
        let input_message_id = Uuid::now_v7();
        let exec_id = Uuid::now_v7();

        let event = EventBuilder::new(session_id)
            .with_turn(turn_id, input_message_id)
            .with_exec(exec_id)
            .build(ReasonStartedData {
                agent_id: Uuid::now_v7(),
                metadata: None,
            });

        assert_eq!(event.event_type, "reason.started");
        assert_eq!(event.context.turn_id, Some(turn_id));
        assert_eq!(event.context.exec_id, Some(exec_id));
    }
}
