//! Deprecated compatibility shim for the canonical core module.
//!
//! This is the final forwarding release. Enable the matching `everruns-core`
//! feature and migrate imports to its module before the next platform release.
//!
//! ```
//! use everruns_core::ag_ui::{Event, RunStartedEvent};
//! let event = Event::RunStarted(RunStartedEvent::new("thread", "run"));
//! let _ = event;
//! ```

#![allow(deprecated)]

#[deprecated(note = "use everruns_core::ag_ui::ActivityDeltaEvent")]
pub use everruns_core::ag_ui::ActivityDeltaEvent;
#[deprecated(note = "use everruns_core::ag_ui::ActivityMessage")]
pub use everruns_core::ag_ui::ActivityMessage;
#[deprecated(note = "use everruns_core::ag_ui::ActivitySnapshotEvent")]
pub use everruns_core::ag_ui::ActivitySnapshotEvent;
#[deprecated(note = "use everruns_core::ag_ui::AgentCapabilities")]
pub use everruns_core::ag_ui::AgentCapabilities;
#[deprecated(note = "use everruns_core::ag_ui::AssistantMessage")]
pub use everruns_core::ag_ui::AssistantMessage;
#[deprecated(note = "use everruns_core::ag_ui::BaseEvent")]
pub use everruns_core::ag_ui::BaseEvent;
#[deprecated(note = "use everruns_core::ag_ui::Content")]
pub use everruns_core::ag_ui::Content;
#[deprecated(note = "use everruns_core::ag_ui::ContentPart")]
pub use everruns_core::ag_ui::ContentPart;
#[deprecated(note = "use everruns_core::ag_ui::Context")]
pub use everruns_core::ag_ui::Context;
#[deprecated(note = "use everruns_core::ag_ui::CustomEvent")]
pub use everruns_core::ag_ui::CustomEvent;
#[deprecated(note = "use everruns_core::ag_ui::Event")]
pub use everruns_core::ag_ui::Event;
#[deprecated(note = "use everruns_core::ag_ui::ExecutionCapabilities")]
pub use everruns_core::ag_ui::ExecutionCapabilities;
#[deprecated(note = "use everruns_core::ag_ui::FunctionCall")]
pub use everruns_core::ag_ui::FunctionCall;
#[deprecated(note = "use everruns_core::ag_ui::HumanInTheLoopCapabilities")]
pub use everruns_core::ag_ui::HumanInTheLoopCapabilities;
#[deprecated(note = "use everruns_core::ag_ui::IdentityCapabilities")]
pub use everruns_core::ag_ui::IdentityCapabilities;
#[deprecated(note = "use everruns_core::ag_ui::Interrupt")]
pub use everruns_core::ag_ui::Interrupt;
#[deprecated(note = "use everruns_core::ag_ui::JsonPatch")]
pub use everruns_core::ag_ui::JsonPatch;
#[deprecated(note = "use everruns_core::ag_ui::JsonPatchOperation")]
pub use everruns_core::ag_ui::JsonPatchOperation;
#[deprecated(note = "use everruns_core::ag_ui::MediaPart")]
pub use everruns_core::ag_ui::MediaPart;
#[deprecated(note = "use everruns_core::ag_ui::Message")]
pub use everruns_core::ag_ui::Message;
#[deprecated(note = "use everruns_core::ag_ui::MessagesSnapshotEvent")]
pub use everruns_core::ag_ui::MessagesSnapshotEvent;
#[deprecated(note = "use everruns_core::ag_ui::Metadata")]
pub use everruns_core::ag_ui::Metadata;
#[deprecated(note = "use everruns_core::ag_ui::MultiAgentCapabilities")]
pub use everruns_core::ag_ui::MultiAgentCapabilities;
#[deprecated(note = "use everruns_core::ag_ui::MultimodalCapabilities")]
pub use everruns_core::ag_ui::MultimodalCapabilities;
#[deprecated(note = "use everruns_core::ag_ui::MultimodalInputCapabilities")]
pub use everruns_core::ag_ui::MultimodalInputCapabilities;
#[deprecated(note = "use everruns_core::ag_ui::MultimodalOutputCapabilities")]
pub use everruns_core::ag_ui::MultimodalOutputCapabilities;
#[deprecated(note = "use everruns_core::ag_ui::OutputCapabilities")]
pub use everruns_core::ag_ui::OutputCapabilities;
#[deprecated(note = "use everruns_core::ag_ui::PROTOCOL_VERSION")]
pub use everruns_core::ag_ui::PROTOCOL_VERSION;
#[deprecated(note = "use everruns_core::ag_ui::PartSource")]
pub use everruns_core::ag_ui::PartSource;
#[deprecated(note = "use everruns_core::ag_ui::RawEvent")]
pub use everruns_core::ag_ui::RawEvent;
#[deprecated(note = "use everruns_core::ag_ui::ReasoningCapabilities")]
pub use everruns_core::ag_ui::ReasoningCapabilities;
#[deprecated(note = "use everruns_core::ag_ui::ReasoningEncryptedValueEvent")]
pub use everruns_core::ag_ui::ReasoningEncryptedValueEvent;
#[deprecated(note = "use everruns_core::ag_ui::ReasoningEncryptedValueSubtype")]
pub use everruns_core::ag_ui::ReasoningEncryptedValueSubtype;
#[deprecated(note = "use everruns_core::ag_ui::ReasoningMessage")]
pub use everruns_core::ag_ui::ReasoningMessage;
#[deprecated(note = "use everruns_core::ag_ui::ReasoningMessageChunkEvent")]
pub use everruns_core::ag_ui::ReasoningMessageChunkEvent;
#[deprecated(note = "use everruns_core::ag_ui::ReasoningMessageContentEvent")]
pub use everruns_core::ag_ui::ReasoningMessageContentEvent;
#[deprecated(note = "use everruns_core::ag_ui::ReasoningMessageEndEvent")]
pub use everruns_core::ag_ui::ReasoningMessageEndEvent;
#[deprecated(note = "use everruns_core::ag_ui::ReasoningMessageStartEvent")]
pub use everruns_core::ag_ui::ReasoningMessageStartEvent;
#[deprecated(note = "use everruns_core::ag_ui::ReasoningRole")]
pub use everruns_core::ag_ui::ReasoningRole;
#[deprecated(note = "use everruns_core::ag_ui::ReasoningSpanEvent")]
pub use everruns_core::ag_ui::ReasoningSpanEvent;
#[deprecated(note = "use everruns_core::ag_ui::ResumeBuilder")]
pub use everruns_core::ag_ui::ResumeBuilder;
#[deprecated(note = "use everruns_core::ag_ui::ResumeEntry")]
pub use everruns_core::ag_ui::ResumeEntry;
#[deprecated(note = "use everruns_core::ag_ui::ResumeError")]
pub use everruns_core::ag_ui::ResumeError;
#[deprecated(note = "use everruns_core::ag_ui::ResumeStatus")]
pub use everruns_core::ag_ui::ResumeStatus;
#[deprecated(note = "use everruns_core::ag_ui::RunAgentInput")]
pub use everruns_core::ag_ui::RunAgentInput;
#[deprecated(note = "use everruns_core::ag_ui::RunErrorEvent")]
pub use everruns_core::ag_ui::RunErrorEvent;
#[deprecated(note = "use everruns_core::ag_ui::RunFinishedEvent")]
pub use everruns_core::ag_ui::RunFinishedEvent;
#[deprecated(note = "use everruns_core::ag_ui::RunFinishedOutcome")]
pub use everruns_core::ag_ui::RunFinishedOutcome;
#[deprecated(note = "use everruns_core::ag_ui::RunStartedEvent")]
pub use everruns_core::ag_ui::RunStartedEvent;
#[deprecated(note = "use everruns_core::ag_ui::SCHEMA_JSON")]
pub use everruns_core::ag_ui::SCHEMA_JSON;
#[deprecated(note = "use everruns_core::ag_ui::StateCapabilities")]
pub use everruns_core::ag_ui::StateCapabilities;
#[deprecated(note = "use everruns_core::ag_ui::StateDeltaEvent")]
pub use everruns_core::ag_ui::StateDeltaEvent;
#[deprecated(note = "use everruns_core::ag_ui::StateSnapshotEvent")]
pub use everruns_core::ag_ui::StateSnapshotEvent;
#[deprecated(note = "use everruns_core::ag_ui::StepEvent")]
pub use everruns_core::ag_ui::StepEvent;
#[deprecated(note = "use everruns_core::ag_ui::SubagentErrorEvent")]
pub use everruns_core::ag_ui::SubagentErrorEvent;
#[deprecated(note = "use everruns_core::ag_ui::SubagentFinishedEvent")]
pub use everruns_core::ag_ui::SubagentFinishedEvent;
#[deprecated(note = "use everruns_core::ag_ui::SubagentFinishedOutcome")]
pub use everruns_core::ag_ui::SubagentFinishedOutcome;
#[deprecated(note = "use everruns_core::ag_ui::SubagentInfo")]
pub use everruns_core::ag_ui::SubagentInfo;
#[deprecated(note = "use everruns_core::ag_ui::SubagentStartedEvent")]
pub use everruns_core::ag_ui::SubagentStartedEvent;
#[deprecated(note = "use everruns_core::ag_ui::TextMessageChunkEvent")]
pub use everruns_core::ag_ui::TextMessageChunkEvent;
#[deprecated(note = "use everruns_core::ag_ui::TextMessageContentEvent")]
pub use everruns_core::ag_ui::TextMessageContentEvent;
#[deprecated(note = "use everruns_core::ag_ui::TextMessageEndEvent")]
pub use everruns_core::ag_ui::TextMessageEndEvent;
#[deprecated(note = "use everruns_core::ag_ui::TextMessageRole")]
pub use everruns_core::ag_ui::TextMessageRole;
#[deprecated(note = "use everruns_core::ag_ui::TextMessageStartEvent")]
pub use everruns_core::ag_ui::TextMessageStartEvent;
#[deprecated(note = "use everruns_core::ag_ui::TextOnlyMessage")]
pub use everruns_core::ag_ui::TextOnlyMessage;
#[deprecated(note = "use everruns_core::ag_ui::TextPart")]
pub use everruns_core::ag_ui::TextPart;
#[deprecated(note = "use everruns_core::ag_ui::TokenUsage")]
pub use everruns_core::ag_ui::TokenUsage;
#[deprecated(note = "use everruns_core::ag_ui::Tool")]
pub use everruns_core::ag_ui::Tool;
#[deprecated(note = "use everruns_core::ag_ui::ToolCall")]
pub use everruns_core::ag_ui::ToolCall;
#[deprecated(note = "use everruns_core::ag_ui::ToolCallArgsEvent")]
pub use everruns_core::ag_ui::ToolCallArgsEvent;
#[deprecated(note = "use everruns_core::ag_ui::ToolCallChunkEvent")]
pub use everruns_core::ag_ui::ToolCallChunkEvent;
#[deprecated(note = "use everruns_core::ag_ui::ToolCallEndEvent")]
pub use everruns_core::ag_ui::ToolCallEndEvent;
#[deprecated(note = "use everruns_core::ag_ui::ToolCallResultEvent")]
pub use everruns_core::ag_ui::ToolCallResultEvent;
#[deprecated(note = "use everruns_core::ag_ui::ToolCallStartEvent")]
pub use everruns_core::ag_ui::ToolCallStartEvent;
#[deprecated(note = "use everruns_core::ag_ui::ToolCallType")]
pub use everruns_core::ag_ui::ToolCallType;
#[deprecated(note = "use everruns_core::ag_ui::ToolMessage")]
pub use everruns_core::ag_ui::ToolMessage;
#[deprecated(note = "use everruns_core::ag_ui::ToolRole")]
pub use everruns_core::ag_ui::ToolRole;
#[deprecated(note = "use everruns_core::ag_ui::ToolsCapabilities")]
pub use everruns_core::ag_ui::ToolsCapabilities;
#[deprecated(note = "use everruns_core::ag_ui::TransportCapabilities")]
pub use everruns_core::ag_ui::TransportCapabilities;
#[deprecated(note = "use everruns_core::ag_ui::UserMessage")]
pub use everruns_core::ag_ui::UserMessage;
#[deprecated(note = "use everruns_core::ag_ui::check_resume_coverage")]
pub use everruns_core::ag_ui::check_resume_coverage;
#[deprecated(note = "use everruns_core::ag_ui")]
pub use everruns_core::ag_ui::*;
