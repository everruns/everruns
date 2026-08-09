//! The application-facing crate for the [Everruns Framework](https://docs.everruns.com/framework/).
//!
//! Build agents, attach open model/provider configuration and typed tools, run
//! isolated multi-turn sessions, observe events, cancel work, and inspect the
//! next model context without constructing an execution host. Default features
//! stay offline; the deterministic simulator needs no credentials or network.
//!
//! `everruns` is the primary Rust library in the
//! [Everruns](https://everruns.com) ecosystem. Existing low-level and 0.17.x
//! applications may continue using `everruns-runtime`; new ordinary
//! applications should begin here.
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
//! let result = agent.session().run("What is 2 + 2?").await?;
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
mod compaction;
mod context;
mod events;
mod hooks;
mod mcp;
mod plugin;
mod session;
mod tool;
/// Session-owned background work, scheduling, cancellation, and wakes.
pub mod work;
pub use agent::{Agent, AgentBuilder, BuildError, Model};
pub use compaction::{CompactionConfig, CompactionStrategy};
pub use context::{ContextMessage, SessionContext, ToolInfo};
pub use events::{CancellationToken, EventStream, RunOptions, SessionEvent, SessionEventKind};
pub use hooks::{
    AgentStartContext, CompletionContext, HookFailure, HookPoint, IntoHookResult, ToolEndContext,
    ToolStartContext, TurnStartContext,
};
pub use mcp::McpServer;
pub use plugin::PluginError;
pub use session::{RunError, Session, Turn};
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
pub use providers::openai::{ModelError, OpenAI};

// --- Runtime construction and execution ---------------------------------
// Note: the value-first `AgentBuilder` above intentionally replaces the runtime
// crate's low-level `AgentBuilder` at the facade root. The runtime builder
// remains reachable as `everruns::runtime::AgentBuilder`.
pub use everruns_host::{
    HarnessBuilder, InProcessRuntime, InProcessRuntimeBuilder, SessionBuilder,
    SingleSessionBuilder, TurnResult,
};

// --- Portable message, model, and platform types ------------------------
pub use everruns_core::turn::TurnStopReason;
pub use everruns_core::{
    AgentLoopError, BearerAuth, CapabilityRegistry, ChatDriver, ContentPart, InitialFile,
    InputMessage, LlmCallConfig, LlmCompletionMetadata, LlmMessage, LlmResponseStream,
    LlmStreamEvent, MessageRole, ModelSpec, PlatformDefinition, Provider, ProviderAuth,
    ProviderAuthRequest, ProviderEndpoint, ProviderKey, ProviderRegistry, SessionId,
    StaticHeaderAuth,
};

// --- Deterministic in-process LLM simulator -----------------------------
pub use everruns_core::llmsim_driver::LlmSimConfig;

/// Escape hatch onto the underlying `everruns-core` crate for APIs not yet
/// promoted onto the facade. Prefer the re-exports above; reach here only for
/// types the facade does not yet surface directly.
pub use everruns_core as core;

/// 0.17 facade-module compatibility alias for the canonical host implementation.
#[deprecated(
    since = "0.17.25",
    note = "advanced hosts should depend on everruns-host directly"
)]
pub use everruns_host as runtime;

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
        Agent, AgentBuilder, AgentStartContext, BuildError, CancellationToken, CompactionConfig,
        CompactionStrategy, CompletionContext, EventStream, FunctionTool, HookFailure, HookPoint,
        InitialFile, IntoHookResult, IntoTool, IntoToolResult, McpServer, Model, PluginError,
        RunError, RunOptions, Session, SessionContext, SessionEvent, SessionEventKind, SessionId,
        Tool, ToolEndContext, ToolInfo, ToolResponse, ToolStartContext, Turn, TurnStartContext,
    };
    #[cfg(feature = "openai")]
    pub use crate::{ModelError, OpenAI};
    pub use everruns_core::turn::TurnStopReason;
    pub use everruns_core::{ContentPart, InputMessage, MessageRole};
}
