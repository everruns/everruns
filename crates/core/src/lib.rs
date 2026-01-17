// Agent Loop Abstraction
//
// This crate provides a DB-agnostic, streamable, and decomposable implementation
// of an agentic loop (LLM call → tool execution → repeat).
//
// Key design decisions:
// - Uses traits (MessageRetriever, ToolExecutor) for pluggable backends
// - MessageRetriever is retrieval-only; messages are stored via EventEmitter
// - Can be decomposed into steps for durable activity execution
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

// Event listeners (pluggable observability backends)
pub mod event_listeners;

// Observation backends (OTel, etc.)
pub mod observation;

// Domain entity types
// These are DB-agnostic entity types used by both API and worker
pub mod agent;
pub mod capability_dto;
pub mod events;
pub mod llm_model_profiles;
pub mod llm_models;
pub mod mcp_server;
pub mod organization;
pub mod session;
pub mod session_file;

pub mod atoms;
pub mod capabilities;
pub mod error;
pub mod llm_driver_registry;
pub mod message;
pub mod message_retriever;
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
    ContentPart, ContentType, Controls, ImageContentPart, ImageFileContentPart, InputContentPart,
    Message, MessageRole, ReasoningConfig, TextContentPart, ToolCallContentPart,
    ToolResultContentPart,
};
pub use message_retriever::{InputMessage, MessageRetriever};
pub use runtime_agent::{RuntimeAgent, RuntimeAgentBuilder};
pub use traits::{
    EventEmitter, ImageResolver, LlmProviderStore, ModelWithProvider, NoopEventEmitter,
    ResolvedImage, SessionFileStore, SessionStore, ToolContext, ToolExecutor,
};

// Event listener re-exports
pub use event_listeners::{CompositeEventListener, EventListener, NoopEventListener};

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
    AddTool, AgentCapabilityConfig, AppliedCapabilities, Capability, CapabilityId,
    CapabilityRegistry, CapabilityRegistryBuilder, CapabilityStatus, CurrentTimeCapability,
    DeleteFileTool, DivideTool, FileSystemCapability, GetCurrentTimeTool, GetForecastTool,
    GetWeatherTool, GrepFilesTool, ListDirectoryTool, MCP_CAPABILITY_PREFIX, McpCapability,
    MountAccess, MountDirectoryBuilder, MountEntry, MountPoint, MountSource, MultiplyTool,
    NoopCapability, ReadFileTool, ResearchCapability, SampleDataCapability, SandboxCapability,
    StatFileTool, StatelessTodoListCapability, SubtractTool, TestMathCapability,
    TestWeatherCapability, WriteFileTool, WriteTodosTool, apply_capabilities, is_mcp_capability,
    mcp_capability_id, parse_mcp_capability_id,
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
    ACT_COMPLETED, ACT_STARTED, ActCompletedData, ActStartedData, Event, EventBuilder,
    EventContext, EventData, EventRequest, INPUT_RECEIVED, InputReceivedData, LLM_GENERATION,
    LlmGenerationData, LlmGenerationMetadata, LlmGenerationOutput, MESSAGE_AGENT, MESSAGE_USER,
    MessageAgentData, MessageUserData, ModelMetadata, REASON_COMPLETED, REASON_STARTED,
    ReasonCompletedData, ReasonStartedData, SESSION_STARTED, SessionStartedData,
    TOOL_CALL_COMPLETED, TOOL_CALL_STARTED, TURN_COMPLETED, TURN_FAILED, TURN_STARTED, TokenUsage,
    ToolCallCompletedData, ToolCallStartedData, ToolCallSummary, TurnCompletedData, TurnFailedData,
    TurnStartedData, UNKNOWN,
};
pub use llm_model_profiles::get_model_profile;
pub use llm_models::{
    LlmModel, LlmModelCost, LlmModelLimits, LlmModelModalities, LlmModelProfile, LlmModelStatus,
    LlmModelWithProvider, LlmProviderStatus, LlmProviderType, Modality, ReasoningEffort,
    ReasoningEffortConfig, ReasoningEffortValue,
};
pub use mcp_server::{
    McpContent, McpError, McpServer, McpServerStatus, McpServerTransportType, McpToolCallParams,
    McpToolCallRequest, McpToolCallResponse, McpToolCallResult, McpToolDefinition,
    McpToolsListRequest, McpToolsListResponse, McpToolsListResult, is_mcp_tool, mcp_tool_name,
    parse_mcp_tool_name,
};
pub use organization::{
    DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgMembership, Organization, generate_org_public_id,
    validate_org_public_id,
};
pub use session::{Session, SessionStatus};
pub use session_file::{FileInfo, FileStat, GrepMatch, GrepResult, SessionFile};

// OTel event listener (observation backend)
pub use observation::OtelEventListener;
