// Agent Loop Abstraction
//
// This crate provides a DB-agnostic, streamable, and decomposable implementation
// of an agentic loop (LLM call → tool execution → repeat).
//
// Key design decisions:
// - Uses traits (EventEmitter, MessageStore, ToolExecutor) for pluggable backends
// - Can be decomposed into steps for Temporal activity execution
// - Emits AG-UI compatible events for SSE streaming
// - Configuration via AgentConfig (can be built from Agent entity or created directly)
// - Tools are defined via a Tool trait for flexibility (function-style tools)
// - ToolRegistry implements ToolExecutor for easy tool management
// - Error handling distinguishes between user-visible and internal errors
// - Capabilities provide modular functionality units for composing agent behavior
// - Domain entity types (Agent, Session, LlmProvider, etc.) are defined here
// - AG-UI events and tool types are defined here as runtime types

// Runtime types (AG-UI protocol events, tool definitions, capability types)
pub mod ag_ui;
pub mod capability_types;
pub mod tool_types;

// Domain entity types (moved from everruns-contracts)
// These are DB-agnostic entity types used by both API and worker
pub mod agent;
pub mod capability_dto;
pub mod event;
pub mod llm_entities;
pub mod session;

pub mod atoms;
pub mod capabilities;
pub mod config;
pub mod error;
pub mod events;
pub mod executor;
pub mod llm;
pub mod r#loop;
pub mod message;
pub mod step;
pub mod tools;
pub mod traits;

// In-memory implementations for examples and testing
pub mod memory;

// LLM Protocol providers
pub mod anthropic;
pub mod openai;
pub mod provider_factory;

// Re-exports for convenience
pub use config::AgentConfig;
pub use error::{AgentLoopError, Result};
pub use events::LoopEvent;
pub use executor::AgentLoop;
pub use message::{
    ContentPart, ContentType, Controls, ImageContentPart, InputContentPart, Message, MessageRole,
    ReasoningConfig, TextContentPart, ToolCallContentPart, ToolResultContentPart,
};
pub use step::{LoopStep, StepKind, StepResult};
pub use traits::{EventEmitter, MessageStore, ToolExecutor};

// LLM types re-exports
pub use llm::{
    LlmCallConfig, LlmCompletionMetadata, LlmContentPart, LlmMessage, LlmMessageContent,
    LlmMessageRole, LlmProvider, LlmResponse, LlmResponseStream, LlmStreamEvent,
};

// Tool abstraction re-exports
pub use tools::{
    EchoTool, FailingTool, Tool, ToolExecutionResult, ToolInternalError, ToolRegistry,
    ToolRegistryBuilder,
};

// Capability re-exports
pub use capabilities::{
    apply_capabilities, AddTool, AppliedCapabilities, Capability, CapabilityId, CapabilityRegistry,
    CapabilityRegistryBuilder, CapabilityStatus, CurrentTimeCapability, DivideTool,
    FileSystemCapability, GetCurrentTimeTool, GetForecastTool, GetWeatherTool, MultiplyTool,
    NoopCapability, ResearchCapability, SandboxCapability, StatelessTodoListCapability,
    SubtractTool, TestMathCapability, TestWeatherCapability, WriteTodosTool,
};

// Atoms re-exports (stateless atomic operations)
pub use atoms::{
    AddUserMessageAtom, AddUserMessageInput, AddUserMessageResult, Atom, CallModelAtom,
    CallModelInput, CallModelResult, ExecuteToolAtom, ExecuteToolInput, ExecuteToolResult,
};

// Loop re-exports
pub use r#loop::{AgentLoop2, LoadMessagesResult};

// AG-UI events (runtime types defined in this crate)
pub use ag_ui::{
    AgUiEvent, AgUiMessageRole, CustomEvent, JsonPatchOp, MessagesSnapshotEvent, RunErrorEvent,
    RunFinishedEvent, RunStartedEvent, SnapshotMessage, StateDeltaEvent, StateSnapshotEvent,
    StepFinishedEvent, StepStartedEvent, TextMessageContentEvent, TextMessageEndEvent,
    TextMessageStartEvent, ToolCallArgsEvent, ToolCallEndEvent, ToolCallResultEvent,
    ToolCallStartEvent, EVENT_MESSAGE_ASSISTANT, EVENT_MESSAGE_SYSTEM, EVENT_MESSAGE_USER,
    EVENT_SESSION_ERROR, EVENT_SESSION_FINISHED, EVENT_SESSION_STARTED, EVENT_STATE_DELTA,
    EVENT_STATE_SNAPSHOT, EVENT_TEXT_DELTA, EVENT_TEXT_END, EVENT_TEXT_START, EVENT_TOOL_CALL_ARGS,
    EVENT_TOOL_CALL_END, EVENT_TOOL_CALL_START, EVENT_TOOL_RESULT, HITL_DECISION, HITL_REQUEST,
};

// Tool types (runtime types defined in this crate)
pub use tool_types::{BuiltinTool, ToolCall, ToolDefinition, ToolPolicy, ToolResult};

// Provider factory re-exports
pub use provider_factory::{create_provider, BoxedLlmProvider, ProviderConfig, ProviderType};

// Note: CapabilityId and CapabilityStatus are re-exported via capabilities module

// Domain entity re-exports (from everruns-contracts migration)
// Note: LlmProvider entity is in llm_entities module (not re-exported at root to avoid
// conflict with LlmProvider trait). Import as: everruns_core::llm_entities::LlmProvider
pub use agent::{Agent, AgentStatus};
pub use capability_dto::{AgentCapability, CapabilityInfo};
pub use event::Event;
pub use llm_entities::{
    LlmModel, LlmModelStatus, LlmModelWithProvider, LlmProviderStatus, LlmProviderType,
};
pub use session::{Session, SessionStatus};
