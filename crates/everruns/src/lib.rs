//! The application-facing crate for the [Everruns Framework](https://docs.everruns.com/framework/).
//!
//! Build agents, attach provider configuration, select model ids, add typed
//! tools, run isolated multi-turn sessions, read bounded history, resume typed
//! session identities, observe events, cancel work, and inspect the next model context
//! without constructing an execution host. Default features stay offline; the
//! deterministic simulator needs no credentials or network.
//! The default registry advertises only portable capabilities; hosted
//! knowledge, delegation, task, hook, and platform-management implementations
//! require an explicit Everruns Platform host.
//!
//! `everruns` is the primary Rust library in the
//! [Everruns](https://everruns.com) ecosystem. Ordinary applications begin
//! here; advanced execution hosts use `everruns` plus
//! [`everruns-host`](https://docs.rs/everruns-host) and focused sibling crates.
//!
//! # Example
//!
//! Run one simulated turn, importing only `everruns`:
//!
//! ```
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use everruns::{Agent, Model};
//!
//! let agent = Agent::builder()
//!     .instructions("You are a helpful assistant.")
//!     .model(Model::simulated("4"))
//!     .build()
//!     .expect("valid agent");
//! let result = agent.session().send_and_wait("What is 2 + 2?").await?;
//! assert!(result.success);
//! assert_eq!(result.response, "4");
//! # Ok(())
//! # }
//! ```

// Let code emitted by `#[everruns::tool]` resolve `::everruns::…` paths even
// inside this crate's own tests and doctests.
extern crate self as everruns;

// --- Value-first agent description and execution -------------------------
mod agent;
#[cfg(feature = "capabilities")]
pub mod capability;
mod capability_config;
mod context;
mod events;
mod history;
mod hooks;
mod mcp;
mod plugin;
mod session;
mod tool;
/// Session-owned background work, scheduling, cancellation, and wakes.
pub mod work;
pub use agent::{Agent, AgentBuilder, BuildError, Model};
pub use capability_config::{CapabilityRef, CapabilitySpec, IntoCapability};
pub use context::{ContextMessage, SessionContext, ToolInfo};
pub use events::{
    CancellationToken, EVENT_STREAM_CAPACITY, EventStream, EventStreamError, RunOptions,
    SessionEvent, SessionEventKind,
};
#[cfg(feature = "builtins")]
pub use everruns_builtins::{CompactionConfig, CompactionStrategy, ToolSearch};
pub use history::{
    HistoryCursor, HistoryCursorParseError, HistoryError, HistoryPage, HistoryPages, HistoryQuery,
    ResumeError, SessionMessage,
};
pub use hooks::{
    AgentStartContext, CompletionContext, HookFailure, HookPoint, IntoHookResult, ToolEndContext,
    ToolStartContext, TurnStartContext,
};
pub use mcp::McpServer;
pub use plugin::PluginError;
pub use session::{CancelError, RunError, SendDisposition, SentMessage, Session, Turn, TurnHandle};
pub use tool::{FunctionTool, IntoTool, IntoToolResult, Tool, ToolResponse};

#[cfg(feature = "local")]
mod local;
#[cfg(feature = "local")]
pub use local::LocalConfig;

#[cfg(all(test, feature = "macros"))]
mod tool_macro_tests;

// --- Function-tool procedural macro (feature-gated) ---------------------
/// Turn a typed async function into an agent tool.
///
/// `#[everruns::tool]` generates the argument JSON Schema and adapter for a
/// plain async function so it can be handed to
/// [`AgentBuilder::tool`](crate::AgentBuilder::tool) without writing either by
/// hand. See the [`macro@tool`] documentation for supported signatures and
/// options. Requires the default-enabled `macros` feature.
#[cfg(feature = "macros")]
pub use everruns_macros::tool;

/// Runtime support for code emitted by [`macro@tool`]. Not a stable API — the
/// expansion references these items by path so the calling crate needs no
/// direct dependency on `serde`, `schemars`, or `serde_json`.
#[cfg(feature = "macros")]
#[doc(hidden)]
pub mod __macro_support {
    pub use schemars;
    pub use serde;
    pub use serde_json::{self, Value};

    /// Serialize a type's JSON Schema to a `Value` for `FunctionTool::new`.
    pub fn schema_for<T: schemars::JsonSchema>() -> Value {
        serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null)
    }

    /// Deserialize the model's call arguments into the generated struct.
    pub fn from_value<T: serde::de::DeserializeOwned>(
        value: Value,
    ) -> Result<T, serde_json::Error> {
        serde_json::from_value(value)
    }

    /// Serialize a handler's success value to a `Value`.
    pub fn to_value<T: serde::Serialize + ?Sized>(value: &T) -> Result<Value, serde_json::Error> {
        serde_json::to_value(value)
    }
}

// --- Real LLM provider configuration (feature-gated) --------------------
// The default facade build stays offline; provider modules compile only when
// their feature is enabled. `openai` adds `providers::openai::OpenAI`.
pub mod providers;
#[cfg(feature = "openai")]
pub use providers::openai::{OpenAI, OpenAIError};

// --- Runtime construction and execution ---------------------------------
// Note: the value-first `AgentBuilder` above intentionally replaces the
// low-level host `AgentBuilder` at the facade root. Advanced hosts that need
// the low-level builders depend on `everruns-host` directly.
pub use everruns_host::{
    HarnessBuilder, HostComposition, InProcessRuntime, InProcessRuntimeBuilder, SessionBuilder,
    SingleSessionBuilder, TurnResult,
};

// --- Portable message, model, and platform types ------------------------
#[doc(hidden)]
pub use everruns_core::AgentCapabilityConfig;
pub use everruns_core::turn::TurnStopReason;
pub use everruns_core::{
    AgentLoopError, BearerAuth, CapabilityRegistry, ChatDriver, ContentPart, Controls,
    ImageContentPart, InitialFile, InputMessage, LlmCallConfig, LlmCompletionMetadata, LlmMessage,
    LlmResponseStream, LlmStreamEvent, MessageRole, ModelSpec, Provider, ProviderAuth,
    ProviderAuthRequest, ProviderEndpoint, ProviderKey, ProviderRegistry, ReasoningConfig,
    SessionId, StaticHeaderAuth, ToolCall, WorkspacePolicy, WorkspacePolicyBuilder,
    WorkspacePolicyError,
};

// --- Deterministic in-process LLM simulator -----------------------------
pub use everruns_test_support::LlmSimConfig;

/// Escape hatch onto the underlying `everruns-core` crate for APIs not yet
/// promoted onto the facade. Prefer the re-exports above; reach here only for
/// types the facade does not yet surface directly.
pub use everruns_core as core;

/// The common path: everything needed to describe an agent and run turns.
///
/// ```
/// use everruns::prelude::*;
///
/// let agent = Agent::builder()
///     .instructions("You are concise.")
///     .model(Model::simulated("Sure."))
///     .build();
/// assert!(agent.is_ok());
/// ```
pub mod prelude {
    #[cfg(feature = "local")]
    pub use crate::LocalConfig;
    #[cfg(feature = "capabilities")]
    pub use crate::capability;
    #[cfg(feature = "macros")]
    pub use crate::tool;
    pub use crate::work::{
        TaskOutcome, TaskRequest, WakePolicy, WakeRequest, WorkQueue, WorkSchedule,
    };
    pub use crate::{
        Agent, AgentBuilder, AgentStartContext, BuildError, CancelError, CancellationToken,
        CapabilityRef, CapabilitySpec, CompletionContext, EventStream, EventStreamError,
        FunctionTool, HistoryCursor, HistoryCursorParseError, HistoryError, HistoryPage,
        HistoryPages, HistoryQuery, HookFailure, HookPoint, InitialFile, IntoCapability,
        IntoHookResult, IntoTool, IntoToolResult, LlmSimConfig, McpServer, Model, PluginError,
        ResumeError, RunError, RunOptions, SendDisposition, SentMessage, Session, SessionContext,
        SessionEvent, SessionEventKind, SessionId, SessionMessage, Tool, ToolEndContext, ToolInfo,
        ToolResponse, ToolStartContext, Turn, TurnHandle, TurnStartContext, WorkspacePolicy,
        WorkspacePolicyBuilder, WorkspacePolicyError,
    };
    #[cfg(feature = "builtins")]
    pub use crate::{CompactionConfig, CompactionStrategy, ToolSearch};
    #[cfg(feature = "openai")]
    pub use crate::{OpenAI, OpenAIError};
    pub use everruns_core::turn::TurnStopReason;
    pub use everruns_core::{ContentPart, InputMessage, MessageRole};
}
