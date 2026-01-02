// Everruns Runtime - Agent Execution Engine
//
// Decision: This crate provides the agent execution engine (atoms, capabilities, tools, LLM drivers)
// Decision: Initially re-exports from everruns-core during migration, will be fully migrated later
// Decision: New code should depend on everruns-schemas for types

// Re-export core modules for now during migration
// These will be moved here incrementally

pub mod error;
pub mod traits;

// Re-export everything from error module
pub use error::{AgentLoopError, Result};

// Re-export traits
pub use traits::{
    AgentStore, EventEmitter, InputMessage, LlmProviderStore, MessageStore, ModelWithProvider,
    NoopEventEmitter, SessionFileStore, SessionStore, ToolContext, ToolExecutor,
};

// Re-export from schemas for convenience
pub use everruns_schemas::{
    // Event types
    ActCompletedData,
    ActStartedData,
    // Agent types
    Agent,
    // Capability types
    AgentCapability,
    AgentStatus,
    // Tool types
    BuiltinTool,
    CapabilityId,
    CapabilityInfo,
    CapabilityStatus,
    // Message types
    ContentPart,
    ContentType,
    Controls,
    Event,
    EventBuilder,
    EventContext,
    EventData,
    // Session file types
    FileInfo,
    FileStat,
    GrepMatch,
    GrepResult,
    ImageContentPart,
    InputContentPart,
    InputReceivedData,
    LlmGenerationData,
    LlmGenerationMetadata,
    LlmGenerationOutput,
    // LLM model types
    LlmModel,
    LlmModelCost,
    LlmModelLimits,
    LlmModelModalities,
    LlmModelProfile,
    LlmModelStatus,
    LlmModelWithProvider,
    LlmProvider,
    LlmProviderStatus,
    LlmProviderType,
    Message,
    MessageAgentData,
    MessageRole,
    MessageUserData,
    Modality,
    ModelMetadata,
    ReasonCompletedData,
    ReasonStartedData,
    ReasoningConfig,
    ReasoningEffort,
    ReasoningEffortConfig,
    ReasoningEffortValue,
    // Session types
    Session,
    SessionFile,
    SessionStartedData,
    SessionStatus,
    TextContentPart,
    TokenUsage,
    ToolCall,
    ToolCallCompletedData,
    ToolCallContentPart,
    ToolCallStartedData,
    ToolCallSummary,
    ToolDefinition,
    ToolPolicy,
    ToolResult,
    ToolResultContentPart,
    TurnCompletedData,
    TurnFailedData,
    TurnStartedData,
};
