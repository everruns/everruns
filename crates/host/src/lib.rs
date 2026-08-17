//! Shared effectful host orchestration for Everruns execution adapters.
//!
//! `everruns-host` composes the shared `everruns-engine` execution kernel for
//! application, worker, and local adapters. It resolves deployment services,
//! credentials, and stores, then applies the engine planner's effects. It is a
//! transitive implementation boundary, not the ordinary application
//! entrypoint; applications in the [Everruns](https://everruns.com) ecosystem
//! should normally depend on `everruns`.
//!
//! Advanced hosts — servers, evaluation harnesses, research runtimes, and
//! specialized embedders — depend on `everruns` plus this crate and the
//! focused sibling crates they actually need.
//!
//! [`runtime_capability_registry`] owns the Framework preset: it starts from an
//! empty core registry, adds the runtime-safe portable catalog selected by
//! `builtins`, and then adds only the integrations selected by this crate's
//! `filesystem`, `bashkit`, `web-fetch`, and `lua` features. MCP transport
//! wiring is separately enabled by `mcp`;
//! local-process MCP additionally requires `mcp-stdio`.
//! [`compose_runtime_capability_registry`] applies the selected integrations to
//! a caller-supplied registry when a broader preset is required.
//! [`runtime_egress_service`] supplies the matching direct transport only when
//! a network-capable integration is selected.
//! Advanced hosts can select `direct-egress` to construct
//! [`DirectEgressService`] without enabling an integration bundle.
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
mod command_host;
mod composition;
#[cfg(feature = "direct-egress")]
mod egress;
pub mod events;
pub mod execution_snapshot;
mod extensions;
mod file_store_decorators;
mod grep_limits;
mod host;
mod in_memory;
mod in_process_execution;
#[cfg(feature = "mcp")]
mod mcp;
#[cfg(feature = "mcp")]
mod mcp_cache;
#[cfg(feature = "process")]
mod process_command;
mod real_disk;
mod runtime;
mod runtime_context;
mod session_file_system_factory;
mod turn_strategy;
#[cfg(feature = "utility-openai")]
mod utility_llm;
mod workspace;

pub use backends::{
    HostBackends, RuntimeAgentStore, RuntimeHarnessStore, RuntimeProviderStore,
    RuntimeSessionStore, ScheduleStoreFactory,
};
pub use builders::{
    AgentBuilder, HarnessBuilder, SeededHarness, SessionBuilder, SingleSessionBuilder,
};
pub use command_host::StoreCommandHost;
pub use composition::{HostComposition, HostCompositionBuilder};
#[cfg(feature = "direct-egress")]
pub use egress::DirectEgressService;
pub use events::{
    DEFAULT_EVENT_READ_LIMIT, EventCursor, EventDeliveryStats, EventDurability, EventHistory,
    EventHistoryPage, EventHistoryReadLimit, EventHistoryReadRequest, EventLog, EventLogError,
    EventPage, EventReadLimit, EventReadRequest, EventReader, EventSink, EventSinkError,
    HostEventEmitter, InMemoryEventLog, JsonlEventLog, MAX_EVENT_HISTORY_PAGE_SIZE,
    MAX_EVENT_HISTORY_REPLAY, MAX_EVENT_PAGE_SIZE, MAX_JSONL_RECOVERY_BYTES,
    MAX_JSONL_RECOVERY_EVENTS, NoopEventSink,
};
pub use everruns_core::AssembledTurnContext;
pub use everruns_core::task_observer::{TaskTransition, TaskTransitionObserver};
pub use everruns_core::turn::TurnStopReason;
pub use everruns_provider::typed_id::WorkspaceId;
pub use execution_snapshot::{load_execution_snapshot, load_execution_snapshot_for_session};
pub use extensions::{HostToolAugmentor, SubagentDelegateFactory, ToolContextExtensionsFactory};
#[allow(deprecated)]
pub use file_store_decorators::{
    ApprovalGatingFileStore, FileApprovalGate, PolicyFileStore, WriteBlocklistFileStore,
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
    InMemoryAgentStore, InMemoryCompactionCheckpointStore, InMemoryHarnessStore,
    InMemoryProviderStore, InMemorySessionFileStore, InMemorySessionFileSystemFactory,
    InMemorySessionStorageStore, InMemorySessionStore,
};
pub use in_process_execution::InProcessExecution;
#[cfg(feature = "process")]
pub use process_command::ProcessCommandExecutor;
pub use real_disk::{RealDiskFileStore, RealDiskSessionFileSystemFactory, multi_root_file_system};
pub use runtime::{
    AcceptedTurnInput, CapabilityDelta, InProcessRuntime, InProcessRuntimeBuilder, TurnResult,
    TurnSteering, TurnSteeringPushError, in_process_internal_org_id,
};
pub use runtime_context::{
    StoreTurnContextResolver, assemble_turn_context, assemble_turn_context_from_snapshot,
    inspect_turn_context,
};
pub use session_file_system_factory::{
    DisabledSessionFileSystemFactory, FixedSessionFileSystemFactory, SessionFileSystemFactory,
    SessionFileSystemFactoryContext,
};
pub use turn_strategy::advance_host_execution;
#[cfg(feature = "utility-openai")]
pub use utility_llm::{
    OpenAiUtilityLlmService, SystemUtilityLlmConfig, UTILITY_OPENAI_API_KEY_ENV,
};
pub use workspace::{
    Environment, EnvironmentBindingError, EnvironmentBindingStore, EnvironmentBuilder,
    InMemoryEnvironmentBindingStore, Workspace, WorkspaceBinding, WorkspaceCheckpoint,
    WorkspaceDescriptor, WorkspaceDiff, WorkspaceError, WorkspaceHead, WorkspaceHeadAccess,
    WorkspaceHeadBuilder, WorkspaceHeadDescriptor, WorkspaceHeadId, WorkspaceHeadRequest,
    WorkspaceHeadResource, WorkspaceHeadStatus, WorkspaceProvider, WorkspaceProviderId,
};
