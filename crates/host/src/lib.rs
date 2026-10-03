//! Deprecated compatibility shim for the canonical core module.
//!
//! This is the final forwarding release. Enable the matching `everruns-core`
//! feature and migrate imports to its module before the next platform release.
//!
//! ```
//! use everruns_core::host::HostComposition;
//! let composition = HostComposition::default();
//! assert!(composition.driver_registry().registered_providers().is_empty());
//! ```

#![allow(deprecated)]

#[deprecated(note = "use everruns::batteries::compose_runtime_capability_registry")]
pub use everruns::batteries::compose_runtime_capability_registry;
#[deprecated(note = "use everruns::batteries::runtime_capability_registry")]
pub use everruns::batteries::runtime_capability_registry;
#[deprecated(note = "use everruns::batteries::runtime_egress_service")]
pub use everruns::batteries::runtime_egress_service;
#[cfg(feature = "utility-llm")]
#[deprecated(note = "use everruns::utility_llm::ProviderUtilityLlmService")]
pub use everruns::utility_llm::ProviderUtilityLlmService;
#[cfg(feature = "utility-llm")]
#[deprecated(note = "use everruns::utility_llm::SystemUtilityLlmConfig")]
pub use everruns::utility_llm::SystemUtilityLlmConfig;
#[cfg(feature = "utility-llm")]
#[deprecated(note = "use everruns::utility_llm::UTILITY_LLM_MODEL_ENV")]
pub use everruns::utility_llm::UTILITY_LLM_MODEL_ENV;
#[cfg(feature = "utility-llm")]
#[deprecated(note = "use everruns::utility_llm::UTILITY_OPENAI_API_KEY_ENV")]
pub use everruns::utility_llm::UTILITY_OPENAI_API_KEY_ENV;
#[cfg(feature = "utility-llm")]
#[deprecated(note = "use everruns::utility_llm::UTILITY_OPENROUTER_API_KEY_ENV")]
pub use everruns::utility_llm::UTILITY_OPENROUTER_API_KEY_ENV;
#[cfg(feature = "utility-llm")]
#[deprecated(note = "use everruns::utility_llm::UTILITY_OPENROUTER_LLM_MODEL")]
pub use everruns::utility_llm::UTILITY_OPENROUTER_LLM_MODEL;
#[cfg(feature = "utility-llm")]
#[deprecated(note = "use everruns::utility_llm::UtilityLlmBackend")]
pub use everruns::utility_llm::UtilityLlmBackend;
#[deprecated(note = "use everruns_core::host::AcceptedTurnInput")]
pub use everruns_core::host::AcceptedTurnInput;
#[deprecated(note = "use everruns_core::host::AgentBuilder")]
pub use everruns_core::host::AgentBuilder;
#[deprecated(note = "use everruns_core::host::AgentLoopError")]
pub use everruns_core::host::AgentLoopError;
#[deprecated(note = "use everruns_core::host::ApprovalGatingFileStore")]
pub use everruns_core::host::ApprovalGatingFileStore;
#[deprecated(note = "use everruns_core::host::AssembledTurnContext")]
pub use everruns_core::host::AssembledTurnContext;
#[deprecated(note = "use everruns_core::host::BillingPressureReason")]
pub use everruns_core::host::BillingPressureReason;
#[deprecated(note = "use everruns_core::host::CapabilityDelta")]
pub use everruns_core::host::CapabilityDelta;
#[deprecated(note = "use everruns_core::host::Compute")]
pub use everruns_core::host::Compute;
#[deprecated(note = "use everruns_core::host::ComputeCapabilities")]
pub use everruns_core::host::ComputeCapabilities;
#[deprecated(note = "use everruns_core::host::ComputeError")]
pub use everruns_core::host::ComputeError;
#[deprecated(note = "use everruns_core::host::ComputeKind")]
pub use everruns_core::host::ComputeKind;
#[deprecated(note = "use everruns_core::host::ComputeSession")]
pub use everruns_core::host::ComputeSession;
#[deprecated(note = "use everruns_core::host::Containment")]
pub use everruns_core::host::Containment;
#[deprecated(note = "use everruns_core::host::ContainmentLevel")]
pub use everruns_core::host::ContainmentLevel;
#[deprecated(note = "use everruns_core::host::DEFAULT_EVENT_READ_LIMIT")]
pub use everruns_core::host::DEFAULT_EVENT_READ_LIMIT;
#[deprecated(note = "use everruns_core::host::DecisionDriverRegistry")]
pub use everruns_core::host::DecisionDriverRegistry;
#[deprecated(note = "use everruns_core::host::DecisionRouter")]
pub use everruns_core::host::DecisionRouter;
#[deprecated(note = "use everruns_core::host::DecisionRoutingError")]
pub use everruns_core::host::DecisionRoutingError;
#[cfg(feature = "direct-egress")]
#[deprecated(note = "use everruns_core::host::DirectEgressService")]
pub use everruns_core::host::DirectEgressService;
#[deprecated(note = "use everruns_core::host::DisabledSessionFileSystemFactory")]
pub use everruns_core::host::DisabledSessionFileSystemFactory;
#[deprecated(note = "use everruns_core::host::Durability")]
pub use everruns_core::host::Durability;
#[deprecated(note = "use everruns_core::host::Environment")]
pub use everruns_core::host::Environment;
#[deprecated(note = "use everruns_core::host::EnvironmentBindingError")]
pub use everruns_core::host::EnvironmentBindingError;
#[deprecated(note = "use everruns_core::host::EnvironmentBindingStore")]
pub use everruns_core::host::EnvironmentBindingStore;
#[deprecated(note = "use everruns_core::host::EnvironmentBuilder")]
pub use everruns_core::host::EnvironmentBuilder;
#[deprecated(note = "use everruns_core::host::EnvironmentError")]
pub use everruns_core::host::EnvironmentError;
#[deprecated(note = "use everruns_core::host::EventCursor")]
pub use everruns_core::host::EventCursor;
#[deprecated(note = "use everruns_core::host::EventDeliveryStats")]
pub use everruns_core::host::EventDeliveryStats;
#[deprecated(note = "use everruns_core::host::EventDurability")]
pub use everruns_core::host::EventDurability;
#[deprecated(note = "use everruns_core::host::EventHistory")]
pub use everruns_core::host::EventHistory;
#[deprecated(note = "use everruns_core::host::EventHistoryPage")]
pub use everruns_core::host::EventHistoryPage;
#[deprecated(note = "use everruns_core::host::EventHistoryReadLimit")]
pub use everruns_core::host::EventHistoryReadLimit;
#[deprecated(note = "use everruns_core::host::EventHistoryReadRequest")]
pub use everruns_core::host::EventHistoryReadRequest;
#[deprecated(note = "use everruns_core::host::EventLog")]
pub use everruns_core::host::EventLog;
#[deprecated(note = "use everruns_core::host::EventLogError")]
pub use everruns_core::host::EventLogError;
#[deprecated(note = "use everruns_core::host::EventPage")]
pub use everruns_core::host::EventPage;
#[deprecated(note = "use everruns_core::host::EventReadLimit")]
pub use everruns_core::host::EventReadLimit;
#[deprecated(note = "use everruns_core::host::EventReadRequest")]
pub use everruns_core::host::EventReadRequest;
#[deprecated(note = "use everruns_core::host::EventReader")]
pub use everruns_core::host::EventReader;
#[deprecated(note = "use everruns_core::host::EventSink")]
pub use everruns_core::host::EventSink;
#[deprecated(note = "use everruns_core::host::EventSinkError")]
pub use everruns_core::host::EventSinkError;
#[deprecated(note = "use everruns_core::host::ExecRequest")]
pub use everruns_core::host::ExecRequest;
#[deprecated(note = "use everruns_core::host::ExecResult")]
pub use everruns_core::host::ExecResult;
#[deprecated(note = "use everruns_core::host::FileApprovalGate")]
pub use everruns_core::host::FileApprovalGate;
#[deprecated(note = "use everruns_core::host::FixedSessionFileSystemFactory")]
pub use everruns_core::host::FixedSessionFileSystemFactory;
#[deprecated(note = "use everruns_core::host::GetSessionInfoTool")]
pub use everruns_core::host::GetSessionInfoTool;
#[deprecated(note = "use everruns_core::host::HarnessBuilder")]
pub use everruns_core::host::HarnessBuilder;
#[deprecated(note = "use everruns_core::host::HostBackends")]
pub use everruns_core::host::HostBackends;
#[deprecated(note = "use everruns_core::host::HostComposition")]
pub use everruns_core::host::HostComposition;
#[deprecated(note = "use everruns_core::host::HostCompositionBuilder")]
pub use everruns_core::host::HostCompositionBuilder;
#[cfg(feature = "process")]
#[deprecated(note = "use everruns_core::host::HostCompute")]
pub use everruns_core::host::HostCompute;
#[cfg(feature = "process")]
#[deprecated(note = "use everruns_core::host::HostComputeSession")]
pub use everruns_core::host::HostComputeSession;
#[deprecated(note = "use everruns_core::host::HostEventEmitter")]
pub use everruns_core::host::HostEventEmitter;
#[deprecated(note = "use everruns_core::host::HostToolAugmentor")]
pub use everruns_core::host::HostToolAugmentor;
#[deprecated(note = "use everruns_core::host::InMemoryAgentStore")]
pub use everruns_core::host::InMemoryAgentStore;
#[deprecated(note = "use everruns_core::host::InMemoryCompactionCheckpointStore")]
pub use everruns_core::host::InMemoryCompactionCheckpointStore;
#[deprecated(note = "use everruns_core::host::InMemoryEnvironmentBindingStore")]
pub use everruns_core::host::InMemoryEnvironmentBindingStore;
#[deprecated(note = "use everruns_core::host::InMemoryEventLog")]
pub use everruns_core::host::InMemoryEventLog;
#[deprecated(note = "use everruns_core::host::InMemoryHarnessStore")]
pub use everruns_core::host::InMemoryHarnessStore;
#[deprecated(note = "use everruns_core::host::InMemoryProviderStore")]
pub use everruns_core::host::InMemoryProviderStore;
#[deprecated(note = "use everruns_core::host::InMemorySessionFileStore")]
pub use everruns_core::host::InMemorySessionFileStore;
#[deprecated(note = "use everruns_core::host::InMemorySessionFileSystemFactory")]
pub use everruns_core::host::InMemorySessionFileSystemFactory;
#[deprecated(note = "use everruns_core::host::InMemorySessionStorageStore")]
pub use everruns_core::host::InMemorySessionStorageStore;
#[deprecated(note = "use everruns_core::host::InMemorySessionStore")]
pub use everruns_core::host::InMemorySessionStore;
#[deprecated(note = "use everruns_core::host::InProcessExecution")]
pub use everruns_core::host::InProcessExecution;
#[deprecated(note = "use everruns_core::host::InProcessRuntime")]
pub use everruns_core::host::InProcessRuntime;
#[deprecated(note = "use everruns_core::host::InProcessRuntimeBuilder")]
pub use everruns_core::host::InProcessRuntimeBuilder;
#[deprecated(note = "use everruns_core::host::InterruptedToolCalls")]
pub use everruns_core::host::InterruptedToolCalls;
#[deprecated(note = "use everruns_core::host::JsonlEventLog")]
pub use everruns_core::host::JsonlEventLog;
#[deprecated(note = "use everruns_core::host::KvStoreTool")]
pub use everruns_core::host::KvStoreTool;
#[deprecated(note = "use everruns_core::host::LLM_DECISION_DRIVER_ID")]
pub use everruns_core::host::LLM_DECISION_DRIVER_ID;
#[deprecated(note = "use everruns_core::host::LlmDecisionDriver")]
pub use everruns_core::host::LlmDecisionDriver;
#[deprecated(note = "use everruns_core::host::LlmError")]
pub use everruns_core::host::LlmError;
#[deprecated(note = "use everruns_core::host::LlmErrorKind")]
pub use everruns_core::host::LlmErrorKind;
#[deprecated(note = "use everruns_core::host::MAX_EVENT_HISTORY_PAGE_SIZE")]
pub use everruns_core::host::MAX_EVENT_HISTORY_PAGE_SIZE;
#[deprecated(note = "use everruns_core::host::MAX_EVENT_HISTORY_REPLAY")]
pub use everruns_core::host::MAX_EVENT_HISTORY_REPLAY;
#[deprecated(note = "use everruns_core::host::MAX_EVENT_PAGE_SIZE")]
pub use everruns_core::host::MAX_EVENT_PAGE_SIZE;
#[deprecated(note = "use everruns_core::host::MAX_JSONL_RECOVERY_BYTES")]
pub use everruns_core::host::MAX_JSONL_RECOVERY_BYTES;
#[deprecated(note = "use everruns_core::host::MAX_JSONL_RECOVERY_EVENTS")]
pub use everruns_core::host::MAX_JSONL_RECOVERY_EVENTS;
#[deprecated(note = "use everruns_core::host::NetworkPolicy")]
pub use everruns_core::host::NetworkPolicy;
#[deprecated(note = "use everruns_core::host::NoopEventSink")]
pub use everruns_core::host::NoopEventSink;
#[deprecated(note = "use everruns_core::host::ParkedToolCalls")]
pub use everruns_core::host::ParkedToolCalls;
#[deprecated(note = "use everruns_core::host::PolicyFileStore")]
pub use everruns_core::host::PolicyFileStore;
#[cfg(feature = "process")]
#[deprecated(note = "use everruns_core::host::ProcessCommandExecutor")]
pub use everruns_core::host::ProcessCommandExecutor;
#[deprecated(note = "use everruns_core::host::RealDiskFileStore")]
pub use everruns_core::host::RealDiskFileStore;
#[deprecated(note = "use everruns_core::host::RealDiskSessionFileSystemFactory")]
pub use everruns_core::host::RealDiskSessionFileSystemFactory;
#[deprecated(note = "use everruns_core::host::ResolvedTurnInputs")]
pub use everruns_core::host::ResolvedTurnInputs;
#[deprecated(note = "use everruns_core::host::RuntimeAgentStore")]
pub use everruns_core::host::RuntimeAgentStore;
#[deprecated(note = "use everruns_core::host::RuntimeHarnessStore")]
pub use everruns_core::host::RuntimeHarnessStore;
#[deprecated(note = "use everruns_core::host::RuntimeHostAdapter")]
pub use everruns_core::host::RuntimeHostAdapter;
#[deprecated(note = "use everruns_core::host::RuntimeProviderStore")]
pub use everruns_core::host::RuntimeProviderStore;
#[deprecated(note = "use everruns_core::host::RuntimeSessionLifecycle")]
pub use everruns_core::host::RuntimeSessionLifecycle;
#[deprecated(note = "use everruns_core::host::RuntimeSessionStore")]
pub use everruns_core::host::RuntimeSessionStore;
#[deprecated(note = "use everruns_core::host::SESSION_CAPABILITY_ID")]
pub use everruns_core::host::SESSION_CAPABILITY_ID;
#[deprecated(note = "use everruns_core::host::SESSION_STORAGE_CAPABILITY_ID")]
pub use everruns_core::host::SESSION_STORAGE_CAPABILITY_ID;
#[deprecated(note = "use everruns_core::host::ScheduleStoreFactory")]
pub use everruns_core::host::ScheduleStoreFactory;
#[deprecated(note = "use everruns_core::host::SecretStoreTool")]
pub use everruns_core::host::SecretStoreTool;
#[deprecated(note = "use everruns_core::host::SeededHarness")]
pub use everruns_core::host::SeededHarness;
#[deprecated(note = "use everruns_core::host::SessionBuilder")]
pub use everruns_core::host::SessionBuilder;
#[deprecated(note = "use everruns_core::host::SessionCapability")]
pub use everruns_core::host::SessionCapability;
#[deprecated(note = "use everruns_core::host::SessionCapabilityConfig")]
pub use everruns_core::host::SessionCapabilityConfig;
#[deprecated(note = "use everruns_core::host::SessionFileSystemFactory")]
pub use everruns_core::host::SessionFileSystemFactory;
#[deprecated(note = "use everruns_core::host::SessionFileSystemFactoryContext")]
pub use everruns_core::host::SessionFileSystemFactoryContext;
#[deprecated(note = "use everruns_core::host::SessionMutator")]
pub use everruns_core::host::SessionMutator;
#[deprecated(note = "use everruns_core::host::SessionMutatorExt")]
pub use everruns_core::host::SessionMutatorExt;
#[deprecated(note = "use everruns_core::host::SessionStorageCapability")]
pub use everruns_core::host::SessionStorageCapability;
#[deprecated(note = "use everruns_core::host::SessionTitleMutation")]
pub use everruns_core::host::SessionTitleMutation;
#[deprecated(note = "use everruns_core::host::SingleSessionBuilder")]
pub use everruns_core::host::SingleSessionBuilder;
#[deprecated(note = "use everruns_core::host::StoreCommandHost")]
pub use everruns_core::host::StoreCommandHost;
#[deprecated(note = "use everruns_core::host::StoreTurnContextResolver")]
pub use everruns_core::host::StoreTurnContextResolver;
#[deprecated(note = "use everruns_core::host::SubagentDelegateFactory")]
pub use everruns_core::host::SubagentDelegateFactory;
#[deprecated(note = "use everruns_core::host::TaskTransition")]
pub use everruns_core::host::TaskTransition;
#[deprecated(note = "use everruns_core::host::TaskTransitionObserver")]
pub use everruns_core::host::TaskTransitionObserver;
#[deprecated(note = "use everruns_core::host::ToolContextExtensionsFactory")]
pub use everruns_core::host::ToolContextExtensionsFactory;
#[deprecated(note = "use everruns_core::host::ToolContextRequest")]
pub use everruns_core::host::ToolContextRequest;
#[deprecated(note = "use everruns_core::host::TurnResult")]
pub use everruns_core::host::TurnResult;
#[deprecated(note = "use everruns_core::host::TurnSteering")]
pub use everruns_core::host::TurnSteering;
#[deprecated(note = "use everruns_core::host::TurnSteeringPushError")]
pub use everruns_core::host::TurnSteeringPushError;
#[deprecated(note = "use everruns_core::host::TurnStopReason")]
pub use everruns_core::host::TurnStopReason;
#[deprecated(note = "use everruns_core::host::Workspace")]
pub use everruns_core::host::Workspace;
#[deprecated(note = "use everruns_core::host::WorkspaceBackend")]
pub use everruns_core::host::WorkspaceBackend;
#[deprecated(note = "use everruns_core::host::WorkspaceBackendId")]
pub use everruns_core::host::WorkspaceBackendId;
#[deprecated(note = "use everruns_core::host::WorkspaceBinding")]
pub use everruns_core::host::WorkspaceBinding;
#[deprecated(note = "use everruns_core::host::WorkspaceCheckpoint")]
pub use everruns_core::host::WorkspaceCheckpoint;
#[deprecated(note = "use everruns_core::host::WorkspaceDescriptor")]
pub use everruns_core::host::WorkspaceDescriptor;
#[deprecated(note = "use everruns_core::host::WorkspaceDiff")]
pub use everruns_core::host::WorkspaceDiff;
#[deprecated(note = "use everruns_core::host::WorkspaceError")]
pub use everruns_core::host::WorkspaceError;
#[deprecated(note = "use everruns_core::host::WorkspaceHead")]
pub use everruns_core::host::WorkspaceHead;
#[deprecated(note = "use everruns_core::host::WorkspaceHeadAccess")]
pub use everruns_core::host::WorkspaceHeadAccess;
#[deprecated(note = "use everruns_core::host::WorkspaceHeadBuilder")]
pub use everruns_core::host::WorkspaceHeadBuilder;
#[deprecated(note = "use everruns_core::host::WorkspaceHeadDescriptor")]
pub use everruns_core::host::WorkspaceHeadDescriptor;
#[deprecated(note = "use everruns_core::host::WorkspaceHeadId")]
pub use everruns_core::host::WorkspaceHeadId;
#[deprecated(note = "use everruns_core::host::WorkspaceHeadRequest")]
pub use everruns_core::host::WorkspaceHeadRequest;
#[deprecated(note = "use everruns_core::host::WorkspaceHeadResource")]
pub use everruns_core::host::WorkspaceHeadResource;
#[deprecated(note = "use everruns_core::host::WorkspaceHeadStatus")]
pub use everruns_core::host::WorkspaceHeadStatus;
#[deprecated(note = "use everruns_core::host::WorkspaceId")]
pub use everruns_core::host::WorkspaceId;
#[deprecated(note = "use everruns_core::host::WorkspaceProvider")]
pub use everruns_core::host::WorkspaceProvider;
#[deprecated(note = "use everruns_core::host::WorkspaceProviderId")]
pub use everruns_core::host::WorkspaceProviderId;
#[deprecated(note = "use everruns_core::host::WriteBlocklistFileStore")]
pub use everruns_core::host::WriteBlocklistFileStore;
#[deprecated(note = "use everruns_core::host::WriteSessionTitleTool")]
pub use everruns_core::host::WriteSessionTitleTool;
#[deprecated(note = "use everruns_core::host::advance_host_execution")]
pub use everruns_core::host::advance_host_execution;
#[deprecated(note = "use everruns_core::host::assemble_turn_context")]
pub use everruns_core::host::assemble_turn_context;
#[deprecated(note = "use everruns_core::host::assemble_turn_context_from_snapshot")]
pub use everruns_core::host::assemble_turn_context_from_snapshot;
#[deprecated(note = "use everruns_core::host::detect_dependency_blocker")]
pub use everruns_core::host::detect_dependency_blocker;
#[deprecated(note = "use everruns_core::host::execute_act_activity")]
pub use everruns_core::host::execute_act_activity;
#[deprecated(note = "use everruns_core::host::execute_input_activity")]
pub use everruns_core::host::execute_input_activity;
#[deprecated(note = "use everruns_core::host::execute_reason_activity")]
pub use everruns_core::host::execute_reason_activity;
#[deprecated(note = "use everruns_core::host::execute_reason_activity_with_prompt_messages")]
pub use everruns_core::host::execute_reason_activity_with_prompt_messages;
#[deprecated(note = "use everruns_core::host::in_process_internal_org_id")]
pub use everruns_core::host::in_process_internal_org_id;
#[deprecated(note = "use everruns_core::host::inspect_turn_context")]
pub use everruns_core::host::inspect_turn_context;
#[deprecated(note = "use everruns_core::host::is_internal_session_kv_key")]
pub use everruns_core::host::is_internal_session_kv_key;
#[deprecated(note = "use everruns_core::host::is_internal_session_secret_name")]
pub use everruns_core::host::is_internal_session_secret_name;
#[deprecated(note = "use everruns_core::host::load_execution_snapshot")]
pub use everruns_core::host::load_execution_snapshot;
#[deprecated(note = "use everruns_core::host::load_execution_snapshot_for_session")]
pub use everruns_core::host::load_execution_snapshot_for_session;
#[deprecated(note = "use everruns_core::host::multi_root_file_system")]
pub use everruns_core::host::multi_root_file_system;
#[deprecated(note = "use everruns_core::host::session_title_updated_event")]
pub use everruns_core::host::session_title_updated_event;
#[deprecated(note = "use everruns_core::host::update_session_title_with_event")]
pub use everruns_core::host::update_session_title_with_event;
#[deprecated(note = "use everruns_core::host")]
pub use everruns_core::host::*;

/// Compatibility composition helpers; new hosts use the facade batteries.
#[deprecated(note = "use everruns_core::host::capabilities and everruns::batteries")]
pub mod capabilities {
    pub use everruns::batteries::{
        compose_runtime_capability_registry, runtime_capability_registry, runtime_egress_service,
    };
    pub use everruns_core::host::capabilities::*;
}
