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
//! # async fn main() -> Result<(), everruns::AgentLoopError> {
//! use everruns::{
//!     DriverId, InProcessRuntimeBuilder, InputMessage, LlmSimConfig, ResolvedModel,
//! };
//!
//! let runtime = InProcessRuntimeBuilder::new()
//!     .single_session(|s| {
//!         s.harness("assistant", "You are a helpful assistant.")
//!             .harness_display_name("Assistant")
//!             .agent("assistant-agent", "Answer the user.")
//!             .agent_display_name("Assistant Agent")
//!             .agent_max_iterations(4)
//!             .session_title("Facade Smoke")
//!     })
//!     .llm_sim(LlmSimConfig::fixed("4"))
//!     .default_model(ResolvedModel {
//!         model: "llmsim-model".into(),
//!         provider_type: DriverId::LlmSim,
//!         api_key: Some("fake-key".into()),
//!         base_url: None,
//!         provider_metadata: None,
//!     })
//!     .build()
//!     .await?;
//!
//! let session_id = runtime.default_session_id().expect("single_session id");
//! let result = runtime
//!     .run_turn(session_id, InputMessage::user("What is 2 + 2?"))
//!     .await?;
//!
//! assert!(result.success);
//! assert_eq!(result.response, "4");
//! # Ok(())
//! # }
//! ```

// --- Value-first agent description and execution -------------------------
mod agent;
mod events;
mod session;
mod tool;
pub use agent::{Agent, AgentBuilder, BuildError, Model};
pub use events::{CancellationToken, EventStream, RunOptions, SessionEvent, SessionEventKind};
pub use session::{RunError, Session, Turn};
pub use tool::{FunctionTool, IntoTool, IntoToolResult, Tool, ToolResponse};

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
    AgentLoopError, CapabilityRegistry, ContentPart, DriverId, DriverRegistry, InputMessage,
    MessageRole, PlatformDefinition, ResolvedModel,
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
