// Agent Loop Abstraction
//
// This crate provides a DB-agnostic, streamable, and decomposable implementation
// of an agentic loop (LLM call → tool execution → repeat).
//
// Key design decisions:
// - Uses traits (EventEmitter, MessageStore, ToolExecutor) for pluggable backends
// - Can run fully in-process or decomposed into steps (for Temporal integration)
// - Emits AG-UI compatible events for SSE streaming
// - Configuration via AgentConfig (can be built from Agent entity or created directly)

pub mod config;
pub mod error;
pub mod events;
pub mod executor;
pub mod message;
pub mod step;
pub mod traits;

// In-memory implementations for examples and testing
pub mod memory;

// Re-exports for convenience
pub use config::AgentConfig;
pub use error::{AgentLoopError, Result};
pub use events::LoopEvent;
pub use executor::AgentLoop;
pub use message::{ConversationMessage, MessageRole};
pub use step::{LoopStep, StepKind, StepResult};
pub use traits::{EventEmitter, LlmProvider, MessageStore, ToolExecutor};

// Re-export AG-UI events for compatibility
pub use everruns_contracts::events::AgUiEvent;
pub use everruns_contracts::tools::{ToolCall, ToolDefinition, ToolResult};
