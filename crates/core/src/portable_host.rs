//! Portable host orchestration over injected services, without native filesystem or process backends.
//!
//! The implementation files are shared with the native host. Isolate hosts
//! implement the same adapter and use the same actor recovery and turn loop.

#[path = "host/actor.rs"]
mod actor;
#[path = "host/backends.rs"]
mod backends;
#[path = "host/builders.rs"]
mod builders;
#[path = "host/capabilities/mod.rs"]
pub mod capabilities;
#[path = "host/command_host.rs"]
mod command_host;
#[path = "host/composition.rs"]
mod composition;
#[path = "host/events.rs"]
pub mod events;
#[path = "host/file_store_decorators.rs"]
mod file_store_decorators;
#[path = "host/grep_limits.rs"]
mod grep_limits;
#[path = "host/in_memory.rs"]
mod in_memory;
#[path = "host/partial_stream.rs"]
mod partial_stream;
#[path = "host/runtime/mod.rs"]
mod runtime;
#[path = "host/session_file_system_factory.rs"]
mod session_file_system_factory;
#[path = "host/session_services/mod.rs"]
pub mod session_services;
#[path = "host/turn_backend.rs"]
mod turn_backend;

#[path = "host/ask_user_lifecycle.rs"]
mod ask_user_lifecycle;
#[path = "host/attachment_files.rs"]
mod attachment_files;
#[path = "host/attachment_zip.rs"]
pub mod attachment_zip;
#[path = "host/background_call.rs"]
mod background_call;
#[path = "host/budget_gate.rs"]
mod budget_gate;
#[path = "host/execution_snapshot.rs"]
pub mod execution_snapshot;
#[path = "host/extensions.rs"]
mod extensions;
#[path = "host/in_process_execution.rs"]
mod in_process_execution;
#[path = "host/native_async.rs"]
pub mod native_async;
#[path = "host/reason_backend.rs"]
mod reason_backend;
#[path = "host/runtime_context.rs"]
mod runtime_context;
#[path = "host/host.rs"]
mod runtime_host;
#[path = "host/script_run.rs"]
mod script_run;
#[cfg(feature = "actor-runtime")]
#[path = "host/session_actor/mod.rs"]
pub mod session_actor;
#[path = "host/turn_loop.rs"]
mod turn_loop;
#[path = "host/turn_resume.rs"]
mod turn_resume;
#[path = "host/turn_strategy.rs"]
mod turn_strategy;
#[path = "host/turn_tool_context.rs"]
mod turn_tool_context;

pub use actor::{
    ActorRunner, DEFAULT_SESSION_LEASE_TTL, InMemorySessionLeases, SessionLease, SessionLeases,
};
pub use attachment_files::{SessionAttachmentFiles, stage_attachment};
pub use backends::{
    HostBackends, RuntimeAgentStore, RuntimeHarnessStore, RuntimeProviderStore, RuntimeSessionStore,
};
pub use builders::{
    AgentBuilder, HarnessBuilder, SeededHarness, SessionBuilder, SingleSessionBuilder,
};
pub use capabilities::{runtime_capability_registry, runtime_egress_service};
pub use command_host::StoreCommandHost;
pub use composition::{HostComposition, HostCompositionBuilder};
pub use events::*;
pub use execution_snapshot::{load_execution_snapshot, load_execution_snapshot_for_session};
pub use extensions::{
    BashHookDispatcherFactory, SubagentDelegateFactory, ToolContextExtensionsFactory,
};
pub use extensions::{DisabledBashHookDispatcher, HostToolAugmentor};
pub(crate) use file_store_decorators::apply_workspace_policy;
#[allow(deprecated)]
pub use file_store_decorators::{
    ApprovalGatingFileStore, FileApprovalGate, PolicyFileStore, WriteBlocklistFileStore,
};
pub use in_memory::{
    InMemoryAgentStore, InMemoryCompactionCheckpointStore, InMemoryHarnessStore,
    InMemoryProviderStore, InMemorySessionFileStore, InMemorySessionStorageStore,
    InMemorySessionStore,
};
pub use in_process_execution::InProcessExecution;
pub use runtime::{
    AcceptedTurnInput, InProcessRuntime, InProcessRuntimeBuilder, ParkedToolCalls, TurnSteering,
    TurnSteeringPushError,
};
pub use runtime_context::{
    assemble_turn_context, assemble_turn_context_from_snapshot, inspect_turn_context,
};
pub use runtime_host::{
    ResolvedTurnInputs, RuntimeHostAdapter, RuntimeSessionLifecycle, ToolContextRequest,
    execute_act_activity, execute_input_activity, execute_reason_activity,
    execute_reason_activity_with_prompt_messages,
};
pub use session_file_system_factory::{
    DisabledSessionFileSystemFactory, FixedSessionFileSystemFactory, SessionFileSystemFactory,
    SessionFileSystemFactoryContext,
};
pub use session_services::{
    SESSION_CAPABILITY_ID, SESSION_STORAGE_CAPABILITY_ID, SessionCapability, SessionMutator,
    SessionMutatorExt, SessionStorageCapability,
};
pub use turn_backend::{
    InProcessBackend, TurnBackend, TurnBackendFactory, TurnInput, TurnRequest, TurnScope,
    TurnTicket,
};
pub use turn_loop::{PendingInput, TurnDrive, TurnInbox, TurnResult, drive_turn, turn_entry_plan};
pub use turn_resume::{
    InterruptedToolCalls, OpenTurn, TurnLogFold, WaitsOnPerson, WaitsOnPersonFn,
    interrupted_tool_calls, resume_entry_plan, resume_open_turn, resume_plan_from_log,
};
pub use turn_strategy::advance_host_execution;
