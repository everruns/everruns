// Agent Loop Abstraction
//
// This crate provides a DB-agnostic, streamable, and decomposable implementation
// of an agentic loop (LLM call → tool execution → repeat).
//
// Key design decisions:
// - Uses traits (MessageStore, ToolExecutor) for pluggable backends
// - Can be decomposed into steps for Temporal activity execution
// - Configuration via RuntimeAgent (can be built from Agent entity or created directly)
// - Tools are defined via a Tool trait for flexibility (function-style tools)
// - ToolRegistry implements ToolExecutor for easy tool management
// - Error handling distinguishes between user-visible and internal errors
// - Capabilities provide modular functionality units for composing agent behavior
// - Domain entity types (Agent, Session, LlmProvider, etc.) are defined here
// - Tool types are defined here as runtime types

// Runtime types (tool definitions, capability types)
pub mod capability_types;
pub mod tool_types;

// Telemetry (OpenTelemetry with gen-ai semantic conventions)
pub mod telemetry;

// Domain entity types
// These are DB-agnostic entity types used by both API and worker
pub mod agent;
pub mod capability_dto;
pub mod events;
pub mod llm_model_profiles;
pub mod llm_models;
pub mod session;
pub mod session_file;

pub mod atoms;
pub mod capabilities;
pub mod error;
pub mod llm_driver_registry;
pub mod message;
pub mod openai_protocol;
pub mod runtime_agent;
pub mod tools;
pub mod traits;

// In-memory implementations for examples and testing
pub mod memory;

// LLM Simulator driver for testing
pub mod llmsim_driver;

// Note: LLM Driver implementations (AnthropicLlmDriver, OpenAILlmDriver) are now in
// separate crates (everruns-anthropic, everruns-openai) that depend on everruns-core.
// This enables dependency inversion - provider crates register their drivers at startup.

// Re-exports for convenience
pub use error::{AgentLoopError, Result};
pub use message::{
    ContentPart, ContentType, Controls, ImageContentPart, InputContentPart, Message, MessageRole,
    ReasoningConfig, TextContentPart, ToolCallContentPart, ToolResultContentPart,
};
pub use runtime_agent::{RuntimeAgent, RuntimeAgentBuilder};
pub use traits::{
    EventEmitter, InputMessage, LlmProviderStore, MessageStore, ModelWithProvider,
    NoopEventEmitter, SessionFileStore, SessionStore, ToolContext, ToolExecutor,
};

// LLM driver types re-exports
pub use llm_driver_registry::{
    BoxedLlmDriver, DriverFactory, DriverRegistry, LlmCallConfig, LlmCallConfigBuilder,
    LlmCompletionMetadata, LlmContentPart, LlmDriver, LlmMessage, LlmMessageContent,
    LlmMessageRole, LlmResponse, LlmResponseStream, LlmStreamEvent, ProviderConfig, ProviderType,
};

// OpenAI Protocol driver (base implementation for OpenAI-compatible APIs)
pub use openai_protocol::OpenAIProtocolLlmDriver;

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
    ActAtom, ActInput, ActResult, Atom, AtomContext, InputAtom, InputAtomInput, InputAtomResult,
    ReasonAtom, ReasonInput, ReasonResult, ToolCallResult,
};

// Tool types (runtime types defined in this crate)
pub use tool_types::{BuiltinTool, ToolCall, ToolDefinition, ToolPolicy, ToolResult};

// Note: CapabilityId and CapabilityStatus are re-exported via capabilities module

// Domain entity re-exports
// Note: LlmProvider entity is in llm_models module. Import as: everruns_core::llm_models::LlmProvider
pub use agent::{Agent, AgentStatus};
pub use capability_dto::{AgentCapability, CapabilityInfo};
pub use events::{
    ActCompletedData, ActStartedData, Event, EventBuilder, EventContext, EventData, EventRequest,
    InputReceivedData, LlmGenerationData, LlmGenerationMetadata, LlmGenerationOutput,
    MessageAgentData, MessageUserData, ModelMetadata, ReasonCompletedData, ReasonStartedData,
    SessionStartedData, TokenUsage, ToolCallCompletedData, ToolCallStartedData, ToolCallSummary,
    TurnCompletedData, TurnFailedData, TurnStartedData, ACT_COMPLETED, ACT_STARTED, INPUT_RECEIVED,
    LLM_GENERATION, MESSAGE_AGENT, MESSAGE_USER, REASON_COMPLETED, REASON_STARTED, SESSION_STARTED,
    TOOL_CALL_COMPLETED, TOOL_CALL_STARTED, TURN_COMPLETED, TURN_FAILED, TURN_STARTED, UNKNOWN,
};
pub use llm_model_profiles::get_model_profile;
pub use llm_models::{
    LlmModel, LlmModelCost, LlmModelLimits, LlmModelModalities, LlmModelProfile, LlmModelStatus,
    LlmModelWithProvider, LlmProviderStatus, LlmProviderType, Modality, ReasoningEffort,
    ReasoningEffortConfig, ReasoningEffortValue,
};
pub use session::{Session, SessionStatus};
pub use session_file::{FileInfo, FileStat, GrepMatch, GrepResult, SessionFile};
