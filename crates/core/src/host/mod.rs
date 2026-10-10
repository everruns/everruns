//! Effectful host orchestration over injected contracts, enabled by `host`.
//!
//! This module composes the portable [`crate::engine`] algorithms without
//! selecting vendor drivers or environment integrations. Custom hosts supply
//! [`HostComposition`](crate::host::HostComposition) and [`HostBackends`](crate::host::HostBackends). Application presets and optional
//! integration selection live in `everruns::batteries`.
//!
//! # Example
//!
//! ```
//! use everruns_core::host::{ResolvedTurnInputs, RuntimeHostAdapter};
//!
//! fn accepts_host<A: RuntimeHostAdapter>() {}
//! fn accepts_inputs(_: ResolvedTurnInputs) {}
//! # let _ = accepts_inputs;
//! ```

mod ask_user_lifecycle;
pub mod native_async;

mod backends;
mod background_call;
mod budget_gate;
mod builders;
pub mod capabilities;
mod command_host;
mod composition;
pub mod compute;
#[cfg(feature = "native-containment")]
pub mod containment;
pub mod decisions;
#[cfg(feature = "direct-egress")]
mod egress;
#[cfg(feature = "direct-egress")]
mod egress_reputation;
pub mod environment_preamble;
mod event_cursor;
pub mod events;
pub mod execution_snapshot;
mod extensions;
mod file_store_decorators;
mod grep_limits;
mod in_memory;
mod in_process_execution;
#[cfg(feature = "mcp")]
mod mcp;
#[cfg(feature = "mcp")]
mod mcp_cache;
#[cfg(any(feature = "otel", feature = "braintrust"))]
pub mod observability;
#[cfg(feature = "openai-agents-api")]
pub mod openai_agents_api;
#[cfg(feature = "direct-egress")]
mod org_egress_allowlist;
mod partial_stream;
#[cfg(feature = "process")]
mod process_command;
mod real_disk;
mod reason_backend;
mod runtime;
mod runtime_context;
#[path = "host.rs"]
mod runtime_host;
mod script_run;
mod session_file_system_factory;
pub mod session_services;
mod turn_backend;
mod turn_strategy;
mod turn_tool_context;
mod workspace;

pub use crate::AssembledTurnContext;
pub use crate::task_observer::{TaskTransition, TaskTransitionObserver};
pub use crate::turn::TurnStopReason;
pub use backends::{
    HostBackends, RuntimeAgentStore, RuntimeHarnessStore, RuntimeProviderStore,
    RuntimeSessionStore, ScheduleStoreFactory,
};
pub use builders::{
    AgentBuilder, HarnessBuilder, SeededHarness, SessionBuilder, SingleSessionBuilder,
};
pub use command_host::StoreCommandHost;
pub use composition::{HostComposition, HostCompositionBuilder};
pub use compute::{
    Compute, ComputeCapabilities, ComputeError, ComputeKind, ComputeSession, Containment,
    ContainmentLevel, Durability, ExecRequest, ExecResult, NetworkPolicy,
};
#[cfg(feature = "process")]
pub use compute::{HostCompute, HostComputeSession};
#[cfg(feature = "direct-egress")]
pub use egress::DirectEgressService;
#[cfg(feature = "direct-egress")]
pub use egress_reputation::{DomainReputation, EGRESS_REPUTATION_ENV};
pub use events::{
    DEFAULT_EVENT_READ_LIMIT, EventCursor, EventDeliveryStats, EventDurability, EventHistory,
    EventHistoryPage, EventHistoryReadLimit, EventHistoryReadRequest, EventLog, EventLogError,
    EventPage, EventReadLimit, EventReadRequest, EventReader, EventSink, EventSinkError,
    HostEventEmitter, InMemoryEventLog, JsonlEventLog, MAX_EVENT_HISTORY_PAGE_SIZE,
    MAX_EVENT_HISTORY_REPLAY, MAX_EVENT_PAGE_SIZE, MAX_JSONL_RECOVERY_BYTES,
    MAX_JSONL_RECOVERY_EVENTS, NoopEventSink,
};
pub use everruns_contracts::error::{
    AgentLoopError, BillingPressureReason, LlmError, LlmErrorKind,
};
pub use everruns_contracts::typed_id::WorkspaceId;
pub use execution_snapshot::{load_execution_snapshot, load_execution_snapshot_for_session};
pub use extensions::{
    BashHookDispatcherFactory, DisabledBashHookDispatcher, HostToolAugmentor,
    SubagentDelegateFactory, ToolContextExtensionsFactory,
};
pub(crate) use file_store_decorators::apply_workspace_policy;
#[allow(deprecated)]
pub use file_store_decorators::{
    ApprovalGatingFileStore, FileApprovalGate, PolicyFileStore, WriteBlocklistFileStore,
};
#[cfg(feature = "direct-egress")]
pub use org_egress_allowlist::{
    ORG_EGRESS_ALLOWLIST_CACHE_TTL, install_runtime_org_egress_allowlist,
};

pub use capabilities::{
    compose_runtime_capability_registry, runtime_capability_registry, runtime_egress_service,
};
pub use decisions::{DecisionRouter, LLM_DECISION_DRIVER_ID, LlmDecisionDriver};
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
    AcceptedTurnInput, CapabilityDelta, InProcessRuntime, InProcessRuntimeBuilder,
    InterruptedToolCalls, ParkedToolCalls, TurnResult, TurnSteering, TurnSteeringPushError,
    in_process_internal_org_id,
};
pub use runtime_context::{
    StoreTurnContextResolver, assemble_turn_context, assemble_turn_context_from_snapshot,
    inspect_turn_context,
};
pub use runtime_host::{
    ResolvedTurnInputs, RuntimeHostAdapter, RuntimeSessionLifecycle, ToolContextRequest,
    detect_dependency_blocker, execute_act_activity, execute_input_activity,
    execute_reason_activity, execute_reason_activity_with_prompt_messages,
};
pub use session_file_system_factory::{
    DisabledSessionFileSystemFactory, FixedSessionFileSystemFactory, SessionFileSystemFactory,
    SessionFileSystemFactoryContext,
};
pub use session_services::{
    GetSessionInfoTool, KvStoreTool, SESSION_CAPABILITY_ID, SESSION_STORAGE_CAPABILITY_ID,
    SecretStoreTool, SessionCapability, SessionCapabilityConfig, SessionMutator, SessionMutatorExt,
    SessionStorageCapability, SessionTitleMutation, WriteSessionTitleTool,
    is_internal_session_kv_key, is_internal_session_secret_name, session_title_updated_event,
    update_session_title_with_event,
};
pub use turn_backend::{
    InProcessBackend, TurnBackend, TurnInput, TurnRequest, TurnScope, TurnTicket,
};
pub use turn_strategy::advance_host_execution;
#[deprecated(note = "use WorkspaceBackend")]
pub use workspace::WorkspaceBackend as WorkspaceProvider;
#[deprecated(note = "use WorkspaceBackendId")]
pub use workspace::WorkspaceBackendId as WorkspaceProviderId;
pub use workspace::{
    Environment, EnvironmentBindingError, EnvironmentBindingStore, EnvironmentBuilder,
    EnvironmentError, InMemoryEnvironmentBindingStore, Workspace, WorkspaceBackend,
    WorkspaceBackendId, WorkspaceBinding, WorkspaceCheckpoint, WorkspaceDescriptor, WorkspaceDiff,
    WorkspaceError, WorkspaceHead, WorkspaceHeadAccess, WorkspaceHeadBuilder,
    WorkspaceHeadDescriptor, WorkspaceHeadId, WorkspaceHeadRequest, WorkspaceHeadResource,
    WorkspaceHeadStatus,
};
