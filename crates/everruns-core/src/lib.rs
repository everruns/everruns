// Agent Loop Abstraction
//
// This crate provides a DB-agnostic, streamable, and decomposable implementation
// of an agentic loop (LLM call → tool execution → repeat).
//
// Key design decisions:
// - Uses traits (EventEmitter, MessageStore, ToolExecutor) for pluggable backends
// - Can be decomposed into steps for Temporal activity execution
// - Emits events for SSE streaming via LoopEvent
// - Configuration via AgentConfig (can be built from Agent entity or created directly)
// - Tools are defined via a Tool trait for flexibility (function-style tools)
// - ToolRegistry implements ToolExecutor for easy tool management
// - Error handling distinguishes between user-visible and internal errors
// - Capabilities provide modular functionality units for composing agent behavior
// - Domain entity types (Agent, Session, LlmProvider, etc.) are defined here
// - Tool types are defined here as runtime types

// Runtime types (tool definitions, capability types)
pub mod capability_types;
pub mod tool_types;

// Domain entity types
// These are DB-agnostic entity types used by both API and worker
pub mod agent;
pub mod capability_dto;
pub mod event;
pub mod llm_models;
pub mod model_profiles;
pub mod session;
pub mod session_file;

pub mod atoms;
pub mod capabilities;
pub mod config;
pub mod error;
pub mod events;
pub mod llm_drivers;
pub mod r#loop;
pub mod message;
pub mod step;
pub mod tools;
pub mod traits;

// In-memory implementations for examples and testing
pub mod memory;

// LLM Driver implementations
pub mod anthropic;
pub mod openai;

// Re-exports for convenience
pub use config::{AgentConfig, AgentConfigBuilder};
pub use error::{AgentLoopError, Result};
pub use events::LoopEvent;
pub use message::{
    ContentPart, ContentType, Controls, ImageContentPart, InputContentPart, Message, MessageRole,
    ReasoningConfig, TextContentPart, ToolCallContentPart, ToolResultContentPart,
};
pub use r#loop::{AgentLoop, LoadMessagesResult};
pub use step::{LoopStep, StepKind, StepResult};
pub use traits::{
    EventEmitter, LlmProviderStore, MessageStore, ModelWithProvider, SessionFileStore,
    SessionStore, ToolContext, ToolExecutor,
};

// LLM driver types re-exports
pub use llm_drivers::{
    create_driver, BoxedLlmDriver, LlmCallConfig, LlmCallConfigBuilder, LlmCompletionMetadata,
    LlmContentPart, LlmDriver, LlmMessage, LlmMessageContent, LlmMessageRole, LlmResponse,
    LlmResponseStream, LlmStreamEvent, ProviderConfig, ProviderType,
};

// Tool abstraction re-exports
pub use tools::{
    EchoTool, FailingTool, Tool, ToolExecutionResult, ToolInternalError, ToolRegistry,
    ToolRegistryBuilder,
};

// Capability re-exports
pub use capabilities::{
    apply_capabilities, AddTool, AppliedCapabilities, Capability, CapabilityId, CapabilityRegistry,
    CapabilityRegistryBuilder, CapabilityStatus, CurrentTimeCapability, DeleteFileTool, DivideTool,
    FileSystemCapability, GetCurrentTimeTool, GetForecastTool, GetWeatherTool, GrepFilesTool,
    ListDirectoryTool, MultiplyTool, NoopCapability, ReadFileTool, ResearchCapability,
    SandboxCapability, StatFileTool, StatelessTodoListCapability, SubtractTool, TestMathCapability,
    TestWeatherCapability, WriteFileTool, WriteTodosTool,
};

// Atoms re-exports (stateless atomic operations)
pub use atoms::{
    AddUserMessageAtom, AddUserMessageInput, AddUserMessageResult, Atom, CallModelAtom,
    CallModelInput, CallModelResult, ExecuteToolAtom, ExecuteToolInput, ExecuteToolResult,
};

// Tool types (runtime types defined in this crate)
pub use tool_types::{BuiltinTool, ToolCall, ToolDefinition, ToolPolicy, ToolResult};

// Note: CapabilityId and CapabilityStatus are re-exported via capabilities module

// Domain entity re-exports
// Note: LlmProvider entity is in llm_models module. Import as: everruns_core::llm_models::LlmProvider
pub use agent::{Agent, AgentStatus};
pub use capability_dto::{AgentCapability, CapabilityInfo};
pub use event::Event;
pub use llm_models::{
    LlmModel, LlmModelCost, LlmModelLimits, LlmModelModalities, LlmModelProfile, LlmModelStatus,
    LlmModelWithProvider, LlmProviderStatus, LlmProviderType, Modality, ReasoningEffort,
    ReasoningEffortConfig, ReasoningEffortValue,
};
pub use model_profiles::get_model_profile;
pub use session::{Session, SessionStatus};
pub use session_file::{FileInfo, FileStat, GrepMatch, GrepResult, SessionFile};
