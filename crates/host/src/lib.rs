//! Shared effectful host orchestration for Everruns execution adapters.
//!
//! `everruns-host` sits between the pure `everruns-engine` planner and the
//! application, worker, and local adapters that apply its effects. It is a
//! transitive implementation boundary, not the ordinary application
//! entrypoint; applications in the [Everruns](https://everruns.com) ecosystem
//! should normally depend on `everruns`.
//!
//! Advanced hosts — servers, evaluation harnesses, research runtimes, and
//! specialized embedders — depend on `everruns` plus this crate and the
//! focused sibling crates they actually need.
//!
//! [`runtime_capability_registry`] composes the effect-neutral core registry
//! with the backend-neutral policy catalog selected by `builtins` and only the
//! integrations selected by this crate's `filesystem`, `bashkit`, `web-fetch`,
//! and `lua` features. MCP transport wiring is separately enabled by `mcp`;
//! local-process MCP additionally requires `mcp-stdio`.
//! [`compose_runtime_capability_registry`] applies the selected integrations to
//! a caller-supplied core registry when a broader preset is required.
//! [`runtime_egress_service`] supplies the matching direct transport only when
//! a network-capable integration is selected.
//!
//! # Example
//!
//! ```
//! use everruns_host::{ResolvedTurnInputs, RuntimeHostAdapter};
//!
//! fn accepts_host<A: RuntimeHostAdapter>() {}
//! fn accepts_inputs(_: ResolvedTurnInputs) {}
//! # let _ = accepts_inputs;
//! ```

mod backends;
mod builders;
mod capabilities;
mod composition;
pub mod events;
mod file_store_decorators;
mod grep_limits;
mod host;
mod in_memory;
#[cfg(feature = "mcp")]
mod mcp;
#[cfg(feature = "mcp")]
mod mcp_cache;
#[cfg(feature = "process")]
mod process_command;
mod real_disk;
mod runtime;
mod turn_strategy;

pub use backends::{
    HostBackends, PlatformStoreFactory, RuntimeAgentStore, RuntimeHarnessStore,
    RuntimeProviderStore, RuntimeSessionStore, ScheduleStoreFactory,
};
pub use builders::{
    AgentBuilder, HarnessBuilder, SeededHarness, SessionBuilder, SingleSessionBuilder,
};
pub use composition::{HostComposition, HostCompositionBuilder};
pub use events::{
    DEFAULT_EVENT_READ_LIMIT, EventCursor, EventDeliveryStats, EventDurability, EventHistory,
    EventHistoryPage, EventHistoryReadLimit, EventHistoryReadRequest, EventLog, EventLogError,
    EventPage, EventReadLimit, EventReadRequest, EventReader, EventSink, EventSinkError,
    HostEventEmitter, InMemoryEventLog, JsonlEventLog, MAX_EVENT_HISTORY_PAGE_SIZE,
    MAX_EVENT_HISTORY_REPLAY, MAX_EVENT_PAGE_SIZE, NoopEventSink,
};
pub use everruns_core::AssembledTurnContext;
pub use everruns_core::task_observer::{TaskTransition, TaskTransitionObserver};
pub use everruns_core::turn::TurnStopReason;
#[allow(deprecated)]
pub use file_store_decorators::{
    ApprovalGatingFileStore, DEFAULT_WRITE_BLOCKLIST, FileApprovalGate, PolicyFileStore,
    WriteBlocklistFileStore,
};

pub use capabilities::{
    compose_runtime_capability_registry, runtime_capability_registry, runtime_egress_service,
};
pub use host::{
    ResolvedTurnInputs, RuntimeHostAdapter, RuntimeSessionLifecycle, detect_dependency_blocker,
    execute_act_activity, execute_input_activity, execute_reason_activity,
    execute_reason_activity_with_prompt_messages,
};
pub use in_memory::{
    InMemorySessionFileStore, InMemorySessionFileSystemFactory, InMemorySessionStorageStore,
    InMemorySessionStore,
};
#[cfg(feature = "process")]
pub use process_command::ProcessCommandExecutor;
pub use real_disk::{RealDiskFileStore, RealDiskSessionFileSystemFactory, multi_root_file_system};
pub use runtime::{
    AcceptedTurnInput, CapabilityDelta, InProcessRuntime, InProcessRuntimeBuilder, TurnResult,
    TurnSteering, TurnSteeringPushError, in_process_internal_org_id,
};
pub use turn_strategy::{RuntimeActPlan, RuntimeTurnPlan, RuntimeTurnState, plan_next_host_turn};
