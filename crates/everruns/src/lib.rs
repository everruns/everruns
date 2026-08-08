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

// --- Runtime construction and execution ---------------------------------
pub use everruns_runtime::{
    AgentBuilder, HarnessBuilder, InProcessRuntime, InProcessRuntimeBuilder, SessionBuilder,
    SingleSessionBuilder, TurnResult,
};

// --- Portable message, model, and platform types ------------------------
pub use everruns_core::{
    AgentLoopError, CapabilityRegistry, DriverId, DriverRegistry, InputMessage, PlatformDefinition,
    ResolvedModel,
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
