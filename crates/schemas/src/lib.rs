// Everruns Schemas
//
// Decision: This crate is the source of truth for all shared data structures
// Decision: Minimal dependencies - only serde, uuid, chrono, thiserror, base64
// Decision: Optional OpenAPI support via "openapi" feature flag
// Decision: No runtime logic - only type definitions and serialization

// Core type modules
pub mod agent;
pub mod capability_dto;
pub mod capability_types;
pub mod events;
pub mod llm_models;
pub mod message;
pub mod session;
pub mod session_file;
pub mod tool_types;

// Re-exports for convenience
// Agent types
pub use agent::{Agent, AgentStatus};

// Capability types
pub use capability_dto::{AgentCapability, CapabilityInfo};
pub use capability_types::{CapabilityId, CapabilityStatus};

// Event types
pub use events::{
    ActCompletedData, ActStartedData, Event, EventBuilder, EventContext, EventData,
    InputReceivedData, LlmGenerationData, LlmGenerationMetadata, LlmGenerationOutput,
    MessageAgentData, MessageUserData, ModelMetadata, ReasonCompletedData, ReasonStartedData,
    SessionStartedData, TokenUsage, ToolCallCompletedData, ToolCallStartedData, ToolCallSummary,
    TurnCompletedData, TurnFailedData, TurnStartedData, ACT_COMPLETED, ACT_STARTED, INPUT_RECEIVED,
    LLM_GENERATION, MESSAGE_AGENT, MESSAGE_USER, REASON_COMPLETED, REASON_STARTED, SESSION_STARTED,
    TOOL_CALL_COMPLETED, TOOL_CALL_STARTED, TURN_COMPLETED, TURN_FAILED, TURN_STARTED, UNKNOWN,
};

// LLM model types
pub use llm_models::{
    LlmModel, LlmModelCost, LlmModelLimits, LlmModelModalities, LlmModelProfile, LlmModelStatus,
    LlmModelWithProvider, LlmProvider, LlmProviderStatus, LlmProviderType, Modality,
    ReasoningEffort, ReasoningEffortConfig, ReasoningEffortValue,
};

// Message types
pub use message::{
    ContentPart, ContentType, Controls, ImageContentPart, InputContentPart, Message, MessageRole,
    ReasoningConfig, TextContentPart, ToolCallContentPart, ToolResultContentPart,
};

// Session types
pub use session::{Session, SessionStatus};

// Session file types (virtual filesystem)
pub use session_file::{FileInfo, FileStat, GrepMatch, GrepResult, SessionFile};

// Tool types
pub use tool_types::{BuiltinTool, ToolCall, ToolDefinition, ToolPolicy, ToolResult};
