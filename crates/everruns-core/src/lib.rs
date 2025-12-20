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

pub mod capabilities;
pub mod config;
pub mod error;
pub mod events;
pub mod executor;
pub mod llm;
pub mod message;
pub mod protocol;
pub mod step;
pub mod tools;
pub mod traits;

// In-memory implementations for examples and testing
pub mod memory;

// OpenAI Protocol provider (requires "openai" feature)
#[cfg(feature = "openai")]
pub mod openai;

// Re-exports for convenience
pub use config::AgentConfig;
pub use error::{AgentLoopError, Result};
pub use events::LoopEvent;
pub use executor::AgentLoop;
pub use message::{ConversationMessage, MessageRole};
pub use step::{LoopStep, StepKind, StepResult};
pub use traits::{EventEmitter, MessageStore, ToolExecutor};

// LLM types re-exports
pub use llm::{
    LlmCallConfig, LlmCompletionMetadata, LlmContentPart, LlmMessage, LlmMessageContent,
    LlmMessageRole, LlmProvider, LlmResponse, LlmResponseStream, LlmStreamEvent,
};

// Tool abstraction re-exports
pub use tools::{
    EchoTool, FailingTool, GetCurrentTime, Tool, ToolExecutionResult, ToolInternalError,
    ToolRegistry, ToolRegistryBuilder,
};

// Capability re-exports
pub use capabilities::{
    apply_capabilities, AppliedCapabilities, Capability, CapabilityId, CapabilityRegistry,
    CapabilityRegistryBuilder, CapabilityStatus, CurrentTimeCapability, FileSystemCapability,
    GetCurrentTimeTool, NoopCapability, ResearchCapability, SandboxCapability,
};

// Protocol re-exports (stateless atomic operations)
pub use protocol::{
    AgentProtocol, Atom, CallModelAtom, CallModelInput, CallModelResult, ExecuteToolAtom,
    ExecuteToolInput, ExecuteToolResult, ExecuteToolsAtom, ExecuteToolsInput, ExecuteToolsResult,
    LoadMessagesResult, NextAction,
};

// Re-export AG-UI events for compatibility
pub use everruns_contracts::events::AgUiEvent;
pub use everruns_contracts::tools::{ToolCall, ToolDefinition, ToolResult};
