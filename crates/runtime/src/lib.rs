//! Public in-process runtime for embedding Everruns.
//!
//! The runtime crate exposes an in-memory execution surface that runs the same
//! core atoms (`input`, `reason`, `act`) used elsewhere in the system, but
//! without the durable engine, gRPC worker boundary, or control-plane server.
//!
//! This is the intended public entrypoint for embedders who want to:
//!
//! - run sessions in their own process
//! - provide their own platform definition (capabilities, drivers, harnesses)
//! - seed harnesses, agents, sessions, and workspace files directly in code
//! - replace the default in-memory stores with custom runtime backends
//! - inspect the assembled turn context before or after executing a turn
//! - reuse runtime-owned host phase execution from durable or server-backed hosts
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
//!     Agent, AgentCapabilityConfig, AgentStatus, CapabilityRegistry, DriverRegistry, Harness,
//!     HarnessStatus, InputMessage, LlmProviderType, ModelWithProvider, PlatformDefinition,
//!     Session, SessionStatus,
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
//!     .harness(Harness {
//!         id: "harness_00000000000000000000000000000010".parse().unwrap(),
//!         name: "math".into(),
//!         display_name: Some("Math".into()),
//!         description: Some("Minimal in-process harness".into()),
//!         system_prompt: "You are a calculator.".into(),
//!         parent_harness_id: None,
//!         default_model_id: None,
//!         tags: vec![],
//!         capabilities: vec![AgentCapabilityConfig::new("test_math")],
//!         initial_files: vec![],
//!         network_access: None,
//!         is_built_in: false,
//!         status: HarnessStatus::Active,
//!         created_at: chrono::Utc::now(),
//!         updated_at: chrono::Utc::now(),
//!         archived_at: None,
//!         deleted_at: None,
//!     })
//!     .agent(Agent {
//!         public_id: "agent_00000000000000000000000000000010".parse().unwrap(),
//!         internal_id: uuid::Uuid::nil(),
//!         name: "math-agent".into(),
//!         display_name: Some("Math Agent".into()),
//!         description: None,
//!         system_prompt: "Use tools when needed.".into(),
//!         default_model_id: None,
//!         tags: vec![],
//!         capabilities: vec![],
//!         initial_files: vec![],
//!         network_access: None,
//!         max_iterations: Some(8),
//!         tools: vec![],
//!         status: AgentStatus::Active,
//!         created_at: chrono::Utc::now(),
//!         updated_at: chrono::Utc::now(),
//!         archived_at: None,
//!         deleted_at: None,
//!         usage: None,
//!     })
//!     .session(Session {
//!         id: "session_00000000000000000000000000000010".parse().unwrap(),
//!         organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
//!         harness_id: "harness_00000000000000000000000000000010".parse().unwrap(),
//!         agent_id: Some("agent_00000000000000000000000000000010".parse().unwrap()),
//!         agent_identity_id: None,
//!         title: Some("Math Session".into()),
//!         locale: None,
//!         preview: None,
//!         output_preview: None,
//!         tags: vec![],
//!         model_id: None,
//!         capabilities: vec![],
//!         tools: vec![],
//!         system_prompt: None,
//!         initial_files: vec![],
//!         hints: None,
//!         network_access: None,
//!         max_iterations: None,
//!         status: SessionStatus::Started,
//!         created_at: chrono::Utc::now(),
//!         updated_at: chrono::Utc::now(),
//!         started_at: None,
//!         finished_at: None,
//!         usage: None,
//!         is_pinned: None,
//!         active_schedule_count: None,
//!         features: vec![],
//!         parent_session_id: None,
//!         subagent_name: None,
//!         subagent_task: None,
//!         subagent_status: None,
//!         blueprint_id: None,
//!         blueprint_config: None,
//!     })
//!     .llm_sim(everruns_core::llmsim_driver::LlmSimConfig::fixed("4"))
//!     .default_model(ModelWithProvider {
//!         model: "llmsim-model".into(),
//!         provider_type: LlmProviderType::LlmSim,
//!         api_key: Some("fake-key".into()),
//!         base_url: None,
//!     })
//!     .build()
//!     .await?;
//!
//! let result = runtime
//!     .run_turn(
//!         "session_00000000000000000000000000000010".parse().unwrap(),
//!         InputMessage::user("What is 2 + 2?"),
//!     )
//!     .await?;
//! assert!(result.success);
//! # Ok::<(), everruns_core::AgentLoopError>(())
//! ```

mod backends;
mod host;
mod in_memory;
mod runtime;

pub use backends::{
    RuntimeAgentStore, RuntimeBackends, RuntimeEventCollector, RuntimeFileStore,
    RuntimeHarnessStore, RuntimeMessageStore, RuntimeProviderStore, RuntimeSessionStore,
};
pub use everruns_core::AssembledTurnContext;
pub use host::{
    RuntimeHostAdapter, RuntimeHostTurnContext, RuntimeSessionLifecycle, detect_dependency_blocker,
    execute_act_activity, execute_input_activity, execute_reason_activity,
};
pub use in_memory::{InMemorySessionFileStore, InMemorySessionStorageStore, InMemorySessionStore};
pub use runtime::{InProcessRuntime, InProcessRuntimeBuilder, TurnResult};
