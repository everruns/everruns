//! Public in-process runtime for embedding Everruns.
//!
//! The runtime crate exposes an in-memory execution surface that runs the same
//! core atoms (`input`, `reason`, `act`) used elsewhere in the system, but
//! without the durable engine, gRPC worker boundary, or control-plane server.
//! It is part of the [Everruns](https://everruns.com) ecosystem.
//!
//! This is the intended public entrypoint for embedders who want to:
//!
//! - run sessions in their own process
//! - provide their own platform definition (capabilities, drivers, harnesses)
//! - seed harnesses, agents, sessions, and workspace files directly in code
//! - replace the default in-memory stores with custom runtime backends
//! - inspect the assembled turn context before or after executing a turn
//! - reuse runtime-owned host phase execution from durable or server-backed hosts
//! - map `plan_next_host_turn(...)` onto their own queue, retry, or in-memory host
//!
//! For a runnable example, see:
//!
//! ```text
//! cargo run -p everruns-runtime --example in_process_runtime
//! cargo run -p everruns-runtime --example inspect_context
//! ```
//!
//! # Example
//!
//! ```ignore
//! use everruns_core::{
//!     CapabilityRegistry, DriverRegistry, InputMessage, DriverId, ResolvedModel,
//!     PlatformDefinition,
//! };
//! use everruns_core::capabilities::TestMathCapability;
//! use everruns_runtime::InProcessRuntimeBuilder;
//!
//! let mut capabilities = CapabilityRegistry::new();
//! capabilities.register(TestMathCapability);
//!
//! let platform = PlatformDefinition::new(capabilities, DriverRegistry::new());
//!
//! let runtime = InProcessRuntimeBuilder::new()
//!     .platform_definition(platform)
//!     .single_session(|s| {
//!         s.harness("math", "You are a calculator.")
//!             .harness_display_name("Math")
//!             .with_capability("test_math")
//!             .agent("math-agent", "Use tools when needed.")
//!             .agent_display_name("Math Agent")
//!             .agent_max_iterations(8)
//!             .session_title("Math Session")
//!     })
//!     .llm_sim(everruns_core::llmsim_driver::LlmSimConfig::fixed("4"))
//!     .default_model(ResolvedModel {
//!         model: "llmsim-model".into(),
//!         provider_type: DriverId::LlmSim,
//!         api_key: Some("fake-key".into()),
//!         base_url: None,
//!     })
//!     .build()
//!     .await?;
//!
//! let session_id = runtime.default_session_id().expect("single_session id");
//! let result = runtime
//!     .run_turn(
//!         session_id,
//!         InputMessage::user("What is 2 + 2?"),
//!     )
//!     .await?;
//! assert!(result.success);
//! # Ok::<(), everruns_core::AgentLoopError>(())
//! ```
//!
//! # Real-disk workspace
//!
//! Embedders who want built-in capabilities (`file_system`,
//! `agent_instructions`, `skills`, ...) to read and write a real directory
//! on disk can configure [`RealDiskSessionFileSystemFactory`] on their
//! [`PlatformDefinition`](everruns_core::PlatformDefinition). Every capability that goes through
//! `ToolContext.file_store` or `SystemPromptContext.file_store` picks it up
//! automatically.
//!
//! See the runnable examples for the full wiring:
//!
//! ```text
//! cargo run -p everruns-runtime --example real_disk_agent_instructions
//! cargo run -p everruns-runtime --example real_disk_file_system_tools
//! ```
//!
//! And `specs/file-store.md` for the trait contract.

mod backends;
mod builders;
mod file_store_decorators;
mod host;
mod in_memory;
mod mcp;
mod mcp_cache;
mod real_disk;
mod runtime;
mod turn_strategy;

pub use backends::{
    EventBus, PlatformStoreFactory, RuntimeAgentStore, RuntimeBackends, RuntimeHarnessStore,
    RuntimeMessageStore, RuntimeProviderStore, RuntimeSessionStore, ScheduleStoreFactory,
};
pub use builders::{AgentBuilder, HarnessBuilder, SessionBuilder, SingleSessionBuilder};
pub use everruns_core::AssembledTurnContext;
pub use file_store_decorators::{
    ApprovalGatingFileStore, DEFAULT_WRITE_BLOCKLIST, FileApprovalGate, WriteBlocklistFileStore,
};
pub use host::{
    RuntimeHostAdapter, RuntimeHostTurnContext, RuntimeSessionLifecycle, detect_dependency_blocker,
    execute_act_activity, execute_input_activity, execute_reason_activity,
};
pub use in_memory::{
    InMemorySessionFileStore, InMemorySessionFileSystemFactory, InMemorySessionStorageStore,
    InMemorySessionStore,
};
pub use real_disk::{RealDiskFileStore, RealDiskSessionFileSystemFactory};
pub use runtime::{
    InProcessRuntime, InProcessRuntimeBuilder, TurnResult, in_process_internal_org_id,
};
pub use turn_strategy::{RuntimeActPlan, RuntimeTurnPlan, RuntimeTurnState, plan_next_host_turn};
