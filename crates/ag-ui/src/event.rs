use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Content, Interrupt, JsonPatch, Message, Metadata, PROTOCOL_VERSION, RunAgentInput,
    TextMessageRole,
};

/// Fields every event may carry.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseEvent {
    /// Milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
    /// The upstream event this one was translated from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_event: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

/// One AG-UI event, tagged by `type`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Event {
    TextMessageStart(TextMessageStartEvent),
    TextMessageContent(TextMessageContentEvent),
    TextMessageEnd(TextMessageEndEvent),
    TextMessageChunk(TextMessageChunkEvent),
    ToolCallStart(ToolCallStartEvent),
    ToolCallArgs(ToolCallArgsEvent),
    ToolCallEnd(ToolCallEndEvent),
    ToolCallChunk(ToolCallChunkEvent),
    ToolCallResult(ToolCallResultEvent),
    StateSnapshot(StateSnapshotEvent),
    StateDelta(StateDeltaEvent),
    MessagesSnapshot(MessagesSnapshotEvent),
    ActivitySnapshot(ActivitySnapshotEvent),
    ActivityDelta(ActivityDeltaEvent),
    Raw(RawEvent),
    Custom(CustomEvent),
    RunStarted(RunStartedEvent),
    RunFinished(RunFinishedEvent),
    RunError(RunErrorEvent),
    StepStarted(StepEvent),
    StepFinished(StepEvent),
    ReasoningStart(ReasoningSpanEvent),
    ReasoningMessageStart(ReasoningMessageStartEvent),
    ReasoningMessageContent(ReasoningMessageContentEvent),
    ReasoningMessageEnd(ReasoningMessageEndEvent),
    ReasoningMessageChunk(ReasoningMessageChunkEvent),
    ReasoningEnd(ReasoningSpanEvent),
    ReasoningEncryptedValue(ReasoningEncryptedValueEvent),
    SubagentStarted(SubagentStartedEvent),
    SubagentFinished(SubagentFinishedEvent),
    SubagentError(SubagentErrorEvent),
}

impl Event {
    /// The wire `type` of this event.
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::TextMessageStart(_) => "TEXT_MESSAGE_START",
            Self::TextMessageContent(_) => "TEXT_MESSAGE_CONTENT",
            Self::TextMessageEnd(_) => "TEXT_MESSAGE_END",
            Self::TextMessageChunk(_) => "TEXT_MESSAGE_CHUNK",
            Self::ToolCallStart(_) => "TOOL_CALL_START",
            Self::ToolCallArgs(_) => "TOOL_CALL_ARGS",
            Self::ToolCallEnd(_) => "TOOL_CALL_END",
            Self::ToolCallChunk(_) => "TOOL_CALL_CHUNK",
            Self::ToolCallResult(_) => "TOOL_CALL_RESULT",
            Self::StateSnapshot(_) => "STATE_SNAPSHOT",
            Self::StateDelta(_) => "STATE_DELTA",
            Self::MessagesSnapshot(_) => "MESSAGES_SNAPSHOT",
            Self::ActivitySnapshot(_) => "ACTIVITY_SNAPSHOT",
            Self::ActivityDelta(_) => "ACTIVITY_DELTA",
            Self::Raw(_) => "RAW",
            Self::Custom(_) => "CUSTOM",
            Self::RunStarted(_) => "RUN_STARTED",
            Self::RunFinished(_) => "RUN_FINISHED",
            Self::RunError(_) => "RUN_ERROR",
            Self::StepStarted(_) => "STEP_STARTED",
            Self::StepFinished(_) => "STEP_FINISHED",
            Self::ReasoningStart(_) => "REASONING_START",
            Self::ReasoningMessageStart(_) => "REASONING_MESSAGE_START",
            Self::ReasoningMessageContent(_) => "REASONING_MESSAGE_CONTENT",
            Self::ReasoningMessageEnd(_) => "REASONING_MESSAGE_END",
            Self::ReasoningMessageChunk(_) => "REASONING_MESSAGE_CHUNK",
            Self::ReasoningEnd(_) => "REASONING_END",
            Self::ReasoningEncryptedValue(_) => "REASONING_ENCRYPTED_VALUE",
            Self::SubagentStarted(_) => "SUBAGENT_STARTED",
            Self::SubagentFinished(_) => "SUBAGENT_FINISHED",
            Self::SubagentError(_) => "SUBAGENT_ERROR",
        }
    }

    /// Whether this event closes a run (`RUN_FINISHED` or `RUN_ERROR`).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::RunFinished(_) | Self::RunError(_))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMessageStartEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub message_id: String,
    #[serde(default)]
    pub role: TextMessageRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

impl TextMessageStartEvent {
    /// Opens an assistant text message.
    pub fn assistant(message_id: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMessageContentEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub message_id: String,
    pub delta: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

impl TextMessageContentEvent {
    pub fn new(message_id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
            delta: delta.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMessageEndEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

impl TextMessageEndEvent {
    pub fn new(message_id: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
            ..Self::default()
        }
    }
}

/// The chunked form: the first chunk must carry `messageId`; later chunks may
/// omit it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMessageChunkEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<TextMessageRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallStartEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub tool_call_id: String,
    pub tool_call_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

impl ToolCallStartEvent {
    pub fn new(tool_call_id: impl Into<String>, tool_call_name: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            tool_call_name: tool_call_name.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallArgsEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub tool_call_id: String,
    pub delta: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

impl ToolCallArgsEvent {
    pub fn new(tool_call_id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            delta: delta.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallEndEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

impl ToolCallEndEvent {
    pub fn new(tool_call_id: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            ..Self::default()
        }
    }
}

/// The chunked form: the first chunk must carry `toolCallId` and
/// `toolCallName`; later chunks may omit them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallChunkEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

/// A tool result. It is a message in its own right (`messageId`) and does not
/// reopen the call it answers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallResultEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub message_id: String,
    pub tool_call_id: String,
    pub content: Content,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<ToolRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

/// The only role a tool result may carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolRole {
    Tool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshotEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub snapshot: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateDeltaEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub delta: JsonPatch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesSnapshotEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub messages: Vec<Message>,
}

/// Structured progress: an activity message whose content is an object.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySnapshotEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub message_id: String,
    pub activity_type: String,
    pub content: serde_json::Map<String, Value>,
    /// Whether this snapshot replaces an existing activity message with the
    /// same id. Absent means `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDeltaEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub message_id: String,
    pub activity_type: String,
    pub patch: JsonPatch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub event: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub name: String,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStartedEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub thread_id: String,
    pub run_id: String,
    /// The producer's protocol version, answering the consumer's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Box<RunAgentInput>>,
}

impl RunStartedEvent {
    pub fn new(thread_id: impl Into<String>, run_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            run_id: run_id.into(),
            ..Self::default()
        }
    }

    /// Declares [`PROTOCOL_VERSION`].
    pub fn with_protocol_version(mut self) -> Self {
        self.protocol_version = Some(PROTOCOL_VERSION.to_owned());
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFinishedEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub thread_id: String,
    pub run_id: String,
    /// The run's return value. Never `Some(Value::Null)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// How the run ended. Absent means success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<RunFinishedOutcome>,
    /// Token usage, one entry per provider and model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Vec<TokenUsage>>,
}

impl RunFinishedEvent {
    pub fn new(thread_id: impl Into<String>, run_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            run_id: run_id.into(),
            ..Self::default()
        }
    }

    pub fn with_outcome(mut self, outcome: RunFinishedOutcome) -> Self {
        self.outcome = Some(outcome);
        self
    }
}

/// How a run ended, tagged by `type`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RunFinishedOutcome {
    /// The run completed. A run that stopped on frontend tool calls is a
    /// success that names the calls it left unanswered.
    Success {
        #[serde(
            rename = "pendingToolCallIds",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        pending_tool_call_ids: Option<Vec<String>>,
    },
    /// The run stopped to ask for something; a resuming run answers it.
    Interrupt {
        /// At least one.
        interrupts: Vec<Interrupt>,
    },
    /// The run was stopped on purpose before it completed.
    Cancelled,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunErrorEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Vec<TokenUsage>>,
}

impl RunErrorEvent {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ..Self::default()
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}

/// Token usage of one provider and model. Input and output counts are
/// totals; cached and reasoning tokens are parts of them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_input_tokens: Option<u64>,
}

/// `STEP_STARTED` and `STEP_FINISHED`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub step_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

/// `REASONING_START` and `REASONING_END`: a reasoning span. Spans and
/// reasoning messages are separate namespaces.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningSpanEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

impl ReasoningSpanEvent {
    pub fn new(message_id: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningMessageStartEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub message_id: String,
    pub role: ReasoningRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

impl ReasoningMessageStartEvent {
    pub fn new(message_id: impl Into<String>) -> Self {
        Self {
            base: BaseEvent::default(),
            message_id: message_id.into(),
            role: ReasoningRole::Reasoning,
            subagent_run_id: None,
        }
    }
}

/// The only role a reasoning message may carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningRole {
    Reasoning,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningMessageContentEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub message_id: String,
    pub delta: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

impl ReasoningMessageContentEvent {
    pub fn new(message_id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
            delta: delta.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningMessageEndEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

impl ReasoningMessageEndEvent {
    pub fn new(message_id: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningMessageChunkEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

/// A provider reasoning artefact a consumer stores and returns without
/// reading.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningEncryptedValueEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub subtype: ReasoningEncryptedValueSubtype,
    pub entity_id: String,
    pub encrypted_value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReasoningEncryptedValueSubtype {
    ToolCall,
    Message,
}

/// Announces a subagent invocation before any event attributed to it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentStartedEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub subagent_run_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_subagent_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentFinishedEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub subagent_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<SubagentFinishedOutcome>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SubagentFinishedOutcome {
    Success,
    /// The subagent is waiting on interrupts carried by the run's outcome.
    Suspended {
        #[serde(
            rename = "interruptIds",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        interrupt_ids: Option<Vec<String>>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentErrorEvent {
    #[serde(flatten)]
    pub base: BaseEvent,
    pub subagent_run_id: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}
