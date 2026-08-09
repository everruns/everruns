//! The application-facing entrypoint to the [Everruns](https://everruns.com)
//! agentic framework.
//!
//! `everruns` is a thin, publishable facade over the existing in-process
//! runtime. It re-exports the minimum needed to construct and run a session
//! without depending on `everruns-core` or `everruns-runtime` directly, so an
//! application can add a single dependency and run an agent turn.
//!
//! This first release is a **compatibility facade**: it moves no engine code.
//! Default features stay offline — no provider, MCP, filesystem, SQLx, server,
//! or worker integrations are activated. Anything not yet promoted onto the
//! facade is reachable through the escape-hatch [`core`] and [`runtime`]
//! modules.
//!
//! # Example
//!
//! Build one in-process session and run one simulated turn, importing only
//! `everruns`:
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
mod events;
mod session;
mod tool;
pub use agent::{Agent, AgentBuilder, BuildError, Model};
pub use events::{CancellationToken, EventStream, RunOptions, SessionEvent, SessionEventKind};
pub use session::{RunError, Session, Turn};
pub use tool::{FunctionTool, IntoTool, IntoToolResult, Tool, ToolResponse};

#[cfg(all(test, feature = "macros"))]
mod tool_macro_tests;

// --- File-backed session persistence (feature-gated) --------------------
/// Persist and resume a [`Session`]'s conversation as an append-only JSONL file
/// with only `everruns` and the `jsonl` feature. Off by default; adds no
/// filesystem dependencies to the standard build.
#[cfg(feature = "jsonl")]
pub mod persistence;
#[cfg(feature = "jsonl")]
pub use persistence::{JsonlError, JsonlSessionStore};

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
pub use everruns_runtime::{
    HarnessBuilder, InProcessRuntime, InProcessRuntimeBuilder, SessionBuilder,
    SingleSessionBuilder, TurnResult,
};

// --- Portable message, model, and platform types ------------------------
pub use everruns_core::turn::TurnStopReason;
pub use everruns_core::{
    AgentLoopError, BearerAuth, CapabilityRegistry, ChatDriver, ContentPart, InputMessage,
    LlmCallConfig, LlmCompletionMetadata, LlmMessage, LlmResponseStream, LlmStreamEvent,
    MessageRole, ModelSpec, PlatformDefinition, Provider, ProviderAuth, ProviderAuthRequest,
    ProviderEndpoint, ProviderKey, ProviderRegistry, StaticHeaderAuth,
};

// --- Deterministic in-process LLM simulator -----------------------------
pub use everruns_core::llmsim_driver::LlmSimConfig;

/// Escape hatch onto the underlying `everruns-core` crate for APIs not yet
/// promoted onto the facade. Prefer the re-exports above; reach here only for
/// types the facade does not yet surface directly.
pub use everruns_core as core;

/// Escape hatch onto the underlying `everruns-runtime` crate for APIs not yet
/// promoted onto the facade. Prefer the re-exports above; reach here only for
/// types the facade does not yet surface directly.
pub use everruns_runtime as runtime;

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
    #[cfg(feature = "macros")]
    pub use crate::tool;
    pub use crate::{
        Agent, AgentBuilder, BuildError, CancellationToken, EventStream, FunctionTool, IntoTool,
        IntoToolResult, Model, RunError, RunOptions, Session, SessionEvent, SessionEventKind, Tool,
        ToolResponse, Turn,
    };
    #[cfg(feature = "openai")]
    pub use crate::{ModelError, OpenAI};
    pub use everruns_core::turn::TurnStopReason;
    pub use everruns_core::{ContentPart, InputMessage, MessageRole};
}
