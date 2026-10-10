//! Core agent abstractions for Everruns.
//!
//! `everruns-core` is the shared contract crate for the
//! [Everruns](https://everruns.com) ecosystem. It defines the runtime-facing
//! types used by embedded hosts, workers, provider drivers, integrations, and
//! the control plane.
//!
//! The crate is deliberately storage-agnostic. Agent execution is expressed in
//! terms of focused contracts such as [`MessageRetriever`], [`ToolExecutor`],
//! [`EventEmitter`], and [`ProviderStore`], while host crates decide whether
//! those traits are backed by memory, PostgreSQL, gRPC, or another system.
//! Per-turn contracts are grouped by concern in modules such as
//! [`tool_context`], [`execution_loading`], [`provider_resolution`],
//! [`session_files`], [`durability`], and [`event_emitter`]; there is no
//! catch-all service-traits module.
//!
//! Portable file/folder agents are available through `agent_package` with the
//! `agent-package` feature. Native disk effects require `agent-package-fs`;
//! neither is enabled by default. Framework applications use `everruns::AgentPackage`.
//!
//! # Main Surfaces
//!
//! - Agent, harness, session, message, and event models
//! - Capability and tool traits for composing agent behavior
//! - Provider-neutral execution inputs and effect contracts
//! - Context assembly for the shared `input -> reason -> act` execution flow
//! - Neutral storage, event, capability, and host-service contracts
//!
//! Optional modules preserve the default portable kernel: `engine` supplies
//! Input/Reason/Act algorithms, `builtins` supplies portable policies, and
//! `host` supplies effectful orchestration over injected services. `mcp`,
//! `ag-ui`, and `a2a` select protocol adaptation and their explicit transports.
//!
//! Environment implementations remain in `everruns-integrations-*` and the
//! application facade selects batteries. Core never depends on concrete vendor
//! drivers, integration crates, the control plane, or a database connection.
//! Hosted capabilities live in `everruns-capabilities`; persisted product
//! records live exclusively in the server.
//!
//! Deterministic simulation lives in `everruns-llmsim`; the in-memory agentic
//! loop, writable test doubles, and demo fixture capabilities live in
//! `everruns-test-support`. Core carries neither implementation.
//!
//! Hosts assemble capabilities, drivers, egress, utility LLM and filesystem
//! services through `host::HostComposition`. Provider identity, registration,
//! typed IDs and LLM wire values are imported from `everruns-contracts`.
//!
//! # Example
//!
//! ```
//! use everruns_core::CapabilityRegistry;
//! use everruns_contracts::DriverRegistry;
//!
//! let capabilities = CapabilityRegistry::new();
//! assert!(capabilities.is_empty());
//!
//! let drivers = DriverRegistry::new();
//! assert!(drivers.registered_providers().is_empty());
//! ```

// Published library code is safe-only. Unit tests use Rust 2024's unsafe
// environment mutation APIs under process-wide test locks.
#![cfg_attr(not(any(test, feature = "host")), forbid(unsafe_code))]
#![cfg_attr(not(test), deny(unsafe_code))]
#![deny(rustdoc::broken_intra_doc_links)]

// Runtime types (tool definitions, capability types)
pub use everruns_contracts::runtime::annotation_hook;
pub use everruns_contracts::runtime::capability_mcp_server;
pub use everruns_contracts::runtime::capability_types;
pub mod tool_fingerprint;
use everruns_contracts::tool_types;

// User-defined hooks (see knowledge/runtime-resources/user-hooks.md)
pub mod hook_adapter;
pub use everruns_contracts::runtime::hook_executor;
pub mod lifecycle_hooks;
pub use everruns_contracts::runtime::user_hook_types;

// Deployment configuration
pub mod decision_driver;
pub use everruns_contracts::runtime::decisions;
pub use everruns_contracts::runtime::deployment;
pub use everruns_contracts::runtime::egress;
pub use everruns_contracts::runtime::exec_tool_result;
pub use everruns_contracts::runtime::execution_context;
pub mod execution_snapshot;
pub use everruns_contracts::runtime::utility_llm;

// Execution feature decisions (EVE-878): the org/product feature-flag records
// and management logic (`FeatureFlags`, `FeatureFlagMap`, the API catalog, org
// opt-in resolution) moved to the `crates/server/src/records/`. Core keeps only
// the resolved registration-time decisions consumed by the capability
// registry builders.
pub use everruns_contracts::runtime::execution_features;
pub use everruns_contracts::runtime::feature_flag_grade;
pub use everruns_contracts::runtime::localization;

// Telemetry conventions (neutral gen-ai span metadata contracts)
pub mod telemetry;
pub use everruns_contracts::runtime::tool_narration;

// Event listeners (pluggable observability backends)
pub mod event_listeners;

// Error reporter (vendor-neutral embedder hook)
pub mod error_reporter;

// Shared classification for database failures that strike many subsystems at
// once, so one incident reports as one incident (EVE-1071).
pub mod database_failure;

// Observability implementations live behind the opt-in `otel`/`braintrust` host features
// (EVE-651, EVE-876): exporter listeners (Braintrust, OpenTelemetry), the
// CompositeEventListener fan-out, and OpenTelemetry/OTLP initialization. They
// use the `EventListener` trait, event types, and neutral gen-AI conventions
// in `telemetry`; the default kernel carries no exporter dependencies.

// Typed ID system (type-safe prefixed identifiers)
// See knowledge/foundations/id-schema.md for specification
use everruns_contracts::typed_id;

// Budget types (budgets, ledger, rules, actions)
pub use everruns_contracts::runtime::background;
pub use everruns_contracts::runtime::budget;

// Domain entity types
// These are DB-agnostic entity types used by both API and worker
pub use everruns_contracts::runtime::agent_definition;
// Authored packages belong with portable definitions, not in another publish slot.
#[cfg(feature = "agent-package")]
pub mod agent_package;
pub mod ard_attachment;
pub mod session_mcp_servers;
pub use everruns_contracts::runtime::capability_dto;
// EVE-878: the persisted eval aggregates (`Eval`, `EvalCase`, `EvalRun`,
// `EvalCaseResult`, `EvalRunDataset`, targets/scorers and their lifecycle
// enums) moved to the `crates/server/src/records/` — they are product
// management/reporting records that never participate in a turn.
use everruns_contracts::model_profiles;
pub use everruns_contracts::runtime::events;
pub use everruns_contracts::runtime::finalized_tool_calls;
pub use everruns_contracts::runtime::harness_definition;
pub use everruns_contracts::runtime::leased_resource;
pub use everruns_contracts::runtime::mcp_deferred;
pub use everruns_contracts::runtime::mcp_proxy;
pub use everruns_contracts::runtime::mcp_server;
pub use everruns_contracts::runtime::mount_fs;
pub use everruns_contracts::runtime::network_access;
// EVE-879: the OAuth 2.1 protocol client moved to the MCP adapter crate — MCP
// login/refresh is its only consumer, and the kernel carries no token-exchange
// plumbing.
// EVE-878: the observer records (`Observer`, `TraceScore`, judge
// configuration, match rules and their lifecycle enums) moved to the
// `crates/server/src/records/` — online scoring watches completed turns from the
// hosted control plane and never participates in a turn.
use everruns_contracts::model_spec;
use everruns_contracts::provider;
pub use everruns_contracts::runtime::organization;
pub use everruns_contracts::runtime::payment;
pub use everruns_contracts::runtime::principal;
pub use everruns_contracts::runtime::session;
pub use everruns_contracts::runtime::session_file;
pub use everruns_contracts::runtime::session_path;
pub use everruns_contracts::runtime::session_resource;
pub use everruns_contracts::runtime::session_schedule;
pub use everruns_contracts::runtime::session_task;
pub use everruns_contracts::runtime::skill;
pub use everruns_contracts::runtime::system_allowlist;
use everruns_contracts::runtime_provider;
pub mod task_observer;
pub mod wake_queue;
pub mod workspace_policy;
pub mod workspace_roots;

// Multi-platform channel abstractions (thread context, delivery, routing)
pub use everruns_contracts::runtime::channel;
// The shared channel host and reply delivery.
#[cfg(feature = "channels")]
pub mod channel_runtime;

// Permissions model (policies, rules, caller context)
pub use everruns_contracts::runtime::conversation;
pub mod permissions;
pub use everruns_contracts::runtime::resource_names;

// URL validation for SSRF prevention (shared utility)

// Plugin compiler (directory → declarative capability definition)
// See knowledge/integrations/plugins.md
pub mod plugins;

/// Durable orchestration state for the opt-in OpenAI Agents API backend.
pub mod agents_api_store;
pub use everruns_contracts::runtime::capabilities;
pub use everruns_contracts::runtime::command;
pub use everruns_contracts::runtime::command_host;
pub mod compaction_checkpoint;
pub use everruns_contracts::runtime::browser_use;
pub use everruns_contracts::runtime::compaction_policy;
pub use everruns_contracts::runtime::computer_use;
pub mod config;
pub use everruns_contracts::runtime::config_layer;
pub mod context_report;
pub mod output_truncation;
pub use everruns_contracts::runtime::dependency_blocker;
/// Shared lease and persistence contracts for native asynchronous tools.
pub mod native_async_store;
use everruns_contracts::driver_registry;
use everruns_contracts::error;
pub mod guardrail_checks;
pub mod guardrail_gallery;
pub use everruns_contracts::runtime::llm_error_hook;
// Adapters from core domain types to provider driver types. Lives on the core
// side to keep the crate dependency one-directional (core -> everruns-contracts).
pub mod llm_conversions;
pub use everruns_contracts::runtime::message;
pub use everruns_contracts::runtime::message_filter;
pub use everruns_contracts::runtime::message_retriever;
mod tool_call_integrity;
pub use everruns_contracts::runtime::connection_services;
pub use everruns_contracts::runtime::delegation_services;
pub use tool_call_integrity::{
    retain_complete_llm_tool_exchanges, retain_complete_llm_tool_exchanges_for_request,
    retain_complete_message_tool_exchanges,
};
pub mod durability;
pub use everruns_contracts::runtime::event_emitter;
pub use everruns_contracts::runtime::execution_loading;
pub mod file_services;
pub use everruns_contracts::runtime::image_services;
pub use everruns_contracts::runtime::outline;
pub use everruns_contracts::runtime::output_guardrail;
pub mod path_identity;
pub mod provider_resolution;
pub use everruns_contracts::runtime::resource_ownership;
pub use everruns_contracts::runtime::runtime_agent;
pub mod runtime_context;
/// Deployment-owned decision checks, answered by the source the org picked.
pub mod system_decisions;
pub use everruns_contracts::runtime::session_files;
pub use everruns_contracts::runtime::session_services;
/// Narrow child-session delegation contract: core owns the host-neutral
/// interface and a host adapter supplies the implementation.
pub use everruns_contracts::runtime::subagent_delegation;
pub use everruns_contracts::runtime::tool_context;
pub use everruns_contracts::runtime::tool_execution;
pub use everruns_contracts::runtime::tool_hooks;
pub use everruns_contracts::runtime::tool_output_sanitizer;
pub use everruns_contracts::runtime::tools;
pub use everruns_contracts::runtime::truncation_info;

// Private doubles for collocated unit tests. Public application backends live
// in everruns_core::host; reusable deterministic fixtures live in test-support.
#[cfg(test)]
mod test_fixtures;

// Stable completion semantics; execution state and planning live in everruns_core::engine.
pub mod turn;
pub mod turn_completion;

// Note: Chat Driver implementations (AnthropicChatDriver, OpenAIChatDriver) live in
// everruns-drivers, one feature per vendor, and depend only on everruns-contracts.
// This enables dependency inversion - hosts register the drivers they enable at startup.

// Re-exports for convenience
pub use command_host::{
    CommandHost, CommandTurnContext, DisabledCommandHost, SessionCompletion,
    SessionCompletionError, SessionCompletionRequest, SessionCompletionStream,
};
pub use config_layer::{
    AgentConfigOverlay, merge_capabilities, merge_initial_files, normalize_initial_file_path,
};
pub use connection_services::UserConnectionResolver;
pub use delegation_services::{SpawnClaimResult, SubagentNestingPolicy, SubagentSpawnStore};
pub use durability::{
    DurableToolResultStore, PartialStreamState, PartialStreamStore, StreamHeartbeater,
    StreamProgress, ToolCallClaimResult,
};
#[cfg(test)]
pub(crate) use error::Result;
pub use event_emitter::EventEmitter;
pub use execution_loading::{HarnessStore, SessionStore};
pub use execution_snapshot::{ResolvedExecutionSnapshot, SnapshotMcpServer};
pub use image_services::{ImageResolver, ResolvedImage};
pub use llm_error_hook::{
    LlmErrorContext, LlmErrorHook, LlmErrorHookOutcome, LlmErrorHookServices,
};
pub use message::{
    AnnotationSource, ContentPart, ContentType, Controls, ExternalActor, ImageContentPart,
    ImageFileContentPart, InputContentPart, ReasoningConfig, RuntimeMessage, RuntimeMessageRole,
    TextAnnotation, TextContentPart, ToolCallContentPart, ToolResultContentPart,
    VerificationStatus, VerificationVerdict,
};
pub use message_filter::{
    ExcludedNoticeTransform, FilterContext, InjectedMessage, InjectionPosition, MessageFilter,
    MessageFilterProvider, MessageQuery, PrependTransform,
};
pub use message_retriever::{InputMessage, MessageHistory, MessageRetriever};
pub use mount_fs::{DisplayPolicy, MountFs, WORKSPACE_MOUNT, scoped_prompt_file_store};
pub use provider_resolution::ProviderStore;
pub use runtime_agent::{RuntimeAgent, RuntimeAgentBuilder};
pub use runtime_context::{
    AssembledTurnContext, ResolvedModelExecution, ResolvedRuntimeCapabilities,
    ResolvedTurnContextInput, TurnContextRequest, TurnContextResolver,
    assemble_resolved_turn_context, resolve_runtime_capabilities, resolve_snapshot_capabilities,
};
pub use session_files::{RuntimeArtifactFileSystem, SessionFileSystem, WorkspaceScopedFileSystem};
pub use session_services::{
    KeyInfo, LeasedResourceStore, SecretInfo, SessionResourceRegistry, SessionStorageStore,
};
pub use tool_context::{ReasoningEffortHandle, ToolContext};
pub use tool_execution::{OutboundToolRateLimiter, ToolExecutor};
pub use workspace_policy::{WorkspacePolicy, WorkspacePolicyBuilder, WorkspacePolicyError};
pub use workspace_roots::{
    ADDITIONAL_ROOTS_MOUNT, PRIMARY_WORKSPACE_ROOT_NAME, RelPath, ResolvedPath, WorkspaceRoot,
    WorkspaceRootSet,
};

// Channel abstraction re-exports
pub use channel::{
    ChannelAgentSurface, ChannelDeliveryAdapter, ChannelStreamDelivery, ChannelViewContext,
    DeliveryContext as ChannelDeliveryContext, DeliveryResult as ChannelDeliveryResult,
    InboundAttachment, InboundChannelEvent, OutboundChannelMessage, Participant, SessionBinding,
    ThreadContext,
};

// Narrow subagent-session delegation contract (EVE-839). The full hosted
// `PlatformStore` and its management capabilities live in `everruns-capabilities`.
pub use resource_ownership::{
    LEASED_RESOURCE_EXTERNAL_ID_KEY, LEASED_RESOURCE_ID_KEY, LEASED_RESOURCE_PROVIDER_KEY,
    LEASED_RESOURCE_TYPE_KEY, list_owned_external_resource_ids,
    ownership_tracking_unavailable_error, require_owned_external_resource,
    resource_not_owned_error, verify_owned_external_resource_if_available,
};
pub use subagent_delegation::{
    PlatformCreateSessionRequest, PlatformMessage, SubagentSessionDelegate,
};

// Event listener re-exports
pub use background::{
    BackgroundEventSink, BackgroundExecutableTool, BackgroundOutcome, BackgroundProgress,
};
pub use event_listeners::{EventListener, NoopEventListener};

// Error reporter re-exports
pub use error_reporter::{
    ErrorReport, ErrorReporter, ErrorScope, ErrorSeverity, NoopErrorReporter, SharedErrorReporter,
};

// Database failure classification re-exports (EVE-1071).
pub use database_failure::{DatabaseFailureKind, log_database_failure};

// Outbound egress service re-exports
pub use egress::{
    DisabledEgressService, EgressByteStream, EgressError, EgressRequest, EgressRequestKind,
    EgressResponse, EgressResult, EgressScope, EgressService, EgressSigning, EgressStreamResponse,
    ScopedEgressService,
};
pub use system_allowlist::{
    AllowGroup, EGRESS_POLICY_ENV, EgressAccess, EgressPolicyDenial, EgressPolicyGrant,
    EgressPolicyMode, OPEN_READ_MAX_URL_LEN, SYSTEM_ALLOWLIST_ENABLED_ENV, SystemAllowlist,
    SystemEgressPolicy,
};

// EVE-879: the system email contract and its concrete senders (Resend,
// disabled/noop, `SystemEmailConfig`) moved to the `crates/server/src/records/` —
// email delivery is a hosted product side effect, never consumed during a
// turn. The OAuth 2.1 protocol client moved to `everruns-mcp` (its only
// consumer); it now lives in the optional `mcp` module. The connector catalog
// lives in `everruns-contracts`.
pub use decision_driver::{
    DecisionDriver, DecisionDriverCapabilities, NativePrimitives, SingleDriverService,
};
pub use decisions::{
    DecisionAnswer, DecisionOutcome, DecisionQuestion, DecisionRequest, DecisionUsage,
    DecisionsService, DisabledDecisionsService,
};
pub use utility_llm::{
    DisabledUtilityLlmService, UTILITY_LLM_MODEL, UtilityLlmReasoningEffort, UtilityLlmRequest,
    UtilityLlmService,
};

// Private provider-contract imports used by kernel implementation modules.
pub(crate) use driver_registry::ProviderOpaqueContext;
#[cfg(test)]
pub(crate) use driver_registry::{LlmCallConfig, LlmResponseStream};

// Transport-neutral native compaction contracts. Concrete OpenAI/OpenResponses
// protocol drivers live in everruns-contracts and the focused provider crates.
#[cfg(test)]
pub(crate) use everruns_contracts::compact::CompactOutputItem;

// Tool abstraction re-exports
pub use tools::{
    CliSpelling, Tool, ToolExecutionResult, ToolInternalError, ToolRegistry, ToolRegistryBuilder,
};

// EVE-881: `BuiltInHarnessDefinition`, `BuiltInHarnessRole`, and
// `BuiltInCapabilityDefinition` moved to the `crates/server/src/records/` —
// product provisioning templates are platform/server composition, not
// Framework execution configuration.
// EVE-887: the composition root moved to `everruns-host` as `HostComposition`.
// It now lives behind core’s opt-in `host` feature.
// Selecting a deployment's capabilities, drivers and host services is
// composition, not kernel execution configuration; core owns the registries
// and service contracts, and the layer that runs a turn owns the bundle.
// Managed sandbox state and provider SPIs live in contracts; hosted lifecycle
// orchestration lives in capabilities. The kernel reaches the sandbox through
// neutral context services.

pub use capabilities::SystemPromptContext;
pub use capabilities::{
    AgentBlueprint, AppliedCapabilities, BlueprintModel, Capability, CapabilityRegistry,
    CapabilityRegistryBuilder, CapabilityStatus, CollectedCapabilities,
    DECLARATIVE_CAPABILITY_PREFIX, DependencyError, IntegrationPlugin, MAX_RESOLVED_CAPABILITIES,
    MountAccess, MountDirectoryBuilder, MountEntry, MountPoint, MountSource, ResolvedCapabilities,
    RiskLevel, ToolCallHook, ToolDefinitionHook, apply_capabilities, collect_capabilities,
    collect_capabilities_with_configs, compute_features, declarative_capability_id,
    declarative_capability_info, get_dependencies, hydrate_declarative_capability_config,
    hydrate_plugin_capability_config, is_declarative_capability, parse_declarative_capability_id,
    plugin_capability_info, resolve_dependencies, validate_declarative_capability_definition,
};
pub use capabilities::{
    DeclarativeCapabilityDefinition, DeclarativeCapabilityFile, DeclarativeCapabilitySkill,
};
pub use capabilities::{
    SKILL_CAPABILITY_PREFIX, SKILLS_DISCOVERY_PATH, SkillCapabilityIdExt, SkillContribution,
    SkillInstructions, SkillMeta, SkillSource, discover_skills_from_entries, is_skill_capability,
    parse_skill_capability_id, reconstruct_skill_md, skill_capability_id,
};
pub use compaction_checkpoint::{
    ANTHROPIC_COMPACTION_CHECKPOINT_FORMAT_VERSION, COMPACTION_CHECKPOINT_FORMAT_VERSION,
    CompactionCheckpoint, CompactionCheckpointPayload, CompactionCheckpointStore,
    ProactiveCompactionAttempt, ProactiveCompactionAttemptTracker,
};

pub use execution_context::ExecutionContext;

#[cfg(test)]
pub(crate) use tool_types::{BuiltinTool, ToolCall};

pub(crate) use everruns_contracts::CapabilityRef as AgentCapabilityConfig;

// Domain entity re-exports
// Provider persistence rows live in `crates/server/src/records`; runtime provider contracts live in contracts.
// EVE-877: the stored `Agent` persistence records, their
// lifecycle enums, and the public-name/persistence helpers moved to
// the `crates/server/src/records/`. Core keeps only the portable authored
// execution configuration consumed during a turn.
pub use agent_definition::AgentDefinition;
// EVE-841: the app and agent-trigger control-plane records moved to the
// `crates/server/src/records/`. They are hosted orchestration records not consumed
// during a turn, so core no longer defines or re-exports them.
pub use ard_attachment::{
    ARD_ATTACHMENT_KV_PREFIX, ARD_ATTACHMENT_RESOURCE_KIND, ARD_DISCOVERY_KV_PREFIX, ArdAttachment,
    ArdAttachmentTarget, apply_session_attachments, attachment_kv_key, load_session_attachments,
    merge_attachment_into_session, urn_slug,
};
pub use capability_dto::{AgentCapability, CapabilityInfo};
pub use compaction_policy::{
    CompactionPolicy, CompactionSettings, CompactionStrategy as PolicyCompactionStrategy,
    ObservationMaskingResult as PolicyObservationMaskingResult,
};
pub use context_report::{
    ContextReportContribution, ContextReportSection, SessionContextReport,
    build_session_context_report, build_session_context_report_from_generation,
};
pub use events::{
    ACT_COMPLETED, ACT_STARTED, ActCompletedData, ActStartedData, CONTEXT_COMPACTED,
    CONTEXT_COMPACTING, CONTEXT_COMPACTION_FAILED, CONTEXT_COMPACTION_SKIPPED, CompactionFailStage,
    CompactionReason, CompactionSkipReason, CompactionStepData, CompactionTrigger,
    ContextCompactedData, ContextCompactingData, ContextCompactionFailedData,
    ContextCompactionSkippedData, Event, EventBuilder, EventContext, EventData, EventRequest,
    FILE_WRITTEN, FileWrittenData, INPUT_MESSAGE, InputMessageData, LLM_GENERATION,
    LlmCompactionInfo, LlmCostComponent, LlmGenerationData, LlmGenerationMetadata,
    LlmGenerationOutput, LlmRetryInfo, ModelMetadata, OUTPUT_MESSAGE_COMPLETED,
    OUTPUT_MESSAGE_DELTA, OUTPUT_MESSAGE_REPLACED, OUTPUT_MESSAGE_STARTED,
    OutputMessageCompletedData, OutputMessageDeltaData, OutputMessageReplacedData,
    OutputMessageStartedData, REASON_COMPLETED, REASON_ITEM, REASON_RECOVERED, REASON_STARTED,
    REASON_THINKING_COMPLETED, REASON_THINKING_DELTA, REASON_THINKING_STARTED, ReasonCompletedData,
    ReasonItemData, ReasonRecoveredData, ReasonStartedData, ReasonThinkingCompletedData,
    ReasonThinkingDeltaData, ReasonThinkingStartedData, RecoveryMode, SESSION_ACTIVATED,
    SESSION_IDLED, SESSION_MODEL_CHANGED, SESSION_STARTED, SESSION_TITLE_UPDATED,
    SessionActivatedData, SessionIdledData, SessionModelChangedData, SessionStartedData,
    SessionTitleUpdatedData, TOOL_CALL_REQUESTED, TOOL_COMPLETED, TOOL_OUTPUT_DELTA, TOOL_PROGRESS,
    TOOL_STARTED, TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED, TURN_SEALED, TURN_STARTED,
    TokenUsage, ToolCallRequestedData, ToolCallSummary, ToolCompletedData, ToolOutputDeltaData,
    ToolProgressData, ToolStartedData, TurnCancelledData, TurnCompletedData, TurnFailedData,
    TurnSealedData, TurnStartedData, VALID_EVENT_TYPES,
};
pub use finalized_tool_calls::{
    FinalizedToolCallRejection, FinalizedToolCallsContext, FinalizedToolCallsHook,
};
pub use guardrail_checks::{
    CompiledJudgeCheck, GuardrailAction, GuardrailEngine, GuardrailHit, GuardrailMode,
    GuardrailOnFail, GuardrailRule, GuardrailStage, GuardrailsConfig, MAX_JUDGE_PROMPT_LEN,
};
pub use guardrail_gallery::{
    DataEgress, GuardrailGalleryItem, find_guardrail_gallery_item, guardrail_gallery,
};
pub use session_mcp_servers::{
    SESSION_MCP_SERVER_KV_PREFIX, SessionMcpServer, SessionMcpServerSource, get_session_mcp_server,
    load_session_mcp_servers, put_session_mcp_server, remove_session_mcp_server,
    session_mcp_server_kv_key,
};
// EVE-881: the stored `Harness` persistence record, its lifecycle enum, the
// chain-merge helpers, and the built-in provisioning templates moved to the
// `crates/server/src/records/`. Core keeps only the portable harness execution
// configuration consumed during a turn.
pub use capability_mcp_server::{
    CapabilityMcpServer, CapabilityMcpServers, capability_mcp_servers_to_scoped,
};
pub use harness_definition::HarnessDefinition;
pub use leased_resource::{
    LEASED_RESOURCES_FEATURE, LeasedResource, LeasedResourceStatus, UpsertLeasedResource,
};
pub use mcp_deferred::{
    DEFERRED_MCP_REVEAL_KV_PREFIX, DeferredMcpServerTool, deferred_mcp_server_definition,
    deferred_mcp_server_prefix, partition_deferred_mcp_servers, reveal_deferred_mcp_server,
    revealed_mcp_servers,
};
pub use mcp_proxy::{
    McpCallIdentity, McpProxyTool, McpServerTools, McpToolInvoker, ScopedMcpToolInvoker,
    build_mcp_proxy_tools,
};
pub use mcp_server::{
    MCP_PROTOCOL_VERSION_2025_03, MCP_PROTOCOL_VERSION_2025_06, MCP_PROTOCOL_VERSION_2026_07,
    McpConnectInChat, McpContent, McpElicitationPolicy, McpError, McpProtocolMode,
    McpSecretBindingMetadata, McpServerActsAs, McpServerAuthMode, McpServerPresetRef,
    McpServerTransportType, McpToolAnnotations, McpToolCallParams, McpToolCallRequest,
    McpToolCallResponse, McpToolCallResult, McpToolDefinition, McpToolLabel, McpToolsListRequest,
    McpToolsListResponse, McpToolsListResult, ScopedMcpServer, ScopedMcpServers,
    apply_mcp_secret_binding_schemas, is_mcp_tool, mcp_oauth_provider_id_for_uuid,
    mcp_oauth_session_secret_name, mcp_tool_name, merge_scoped_mcp_servers,
    normalize_mcp_error_code, parse_mcp_tool_name, sanitize_mcp_server_name,
    scoped_mcp_servers_is_empty,
};
// EVE-837/EVE-845: `Organization`, `OrgMembership`, the `ANONYMOUS_USER_*`
// constants, and the public-id generation/validation helpers moved to the
// `crates/server/src/records/`. `OrgRole` (portable turn authorization via
// `permissions`), the `DEFAULT_ORG_*` constants, and the internal<->public id
// conversion helper stay here because core's permissions/auth layer and runtime
// name them.
pub use organization::{
    DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole, org_public_id_from_internal,
};
// EVE-838: the payment accounting records (`PaymentAccount`, `PaymentPolicy`,
// `PaymentAttempt`, `PaymentOwnerType`, `PaymentStatus`) moved to the
// `crates/server/src/records/`. The capability-internal execution contract below
// stays here — it is bound to the `PaymentAuthority` trait and `ToolContext`.
pub use payment::{MachinePaymentRequest, MachinePaymentResponse, PaymentMethod, PaymentRail};
// EVE-837/EVE-845: `Principal` and the `PrincipalStatus` lifecycle enum moved to
// the `crates/server/src/records/`. `PrincipalSummary` and the `PrincipalKind` that
// backs it stay here — they are embedded by `Session`/`SessionSchedule`/
// `VirtualUser`.
pub use principal::{PrincipalKind, PrincipalSummary};
pub(crate) use runtime_provider::ProviderKey;
// EVE-882: the persisted `Session` aggregate and its product lifecycle enums
// (`SessionStatus`, `SessionSource`, `SessionActivity`, participants) moved to
// the `crates/server/src/records/`. Core keeps only the portable execution view
// and the neutral execution state consumed during a turn.
pub use session::{ExecutionSession, SessionExecutionState, SessionSeedMode, SubagentStatus};
pub use session_file::{
    FileInfo, FileStat, GREP_MAX_CONTEXT_LINES, GREP_MAX_RETURN_BYTES, GrepContextBlock,
    GrepContextLine, GrepMatch, GrepOptions, GrepResult, GrepSearchResult, InitialFile,
    SessionFile,
};
pub use session_resource::{
    RegisterSessionResource, SessionResourceEntry, SessionResourceFilter, SessionResourceStatus,
};
// EVE-897: the session SQL database store and its value types moved to
// `everruns-contracts`. Values and trait travel together — the value types are
// the trait's signature vocabulary — and nothing in the kernel names either:
// the capability resolves the store as a typed context extension.
pub use session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskState, SessionTaskUpdate, TASK_KIND_AGENT_HANDOFF, TASK_KIND_BACKGROUND_TOOL,
    TASK_KIND_EXTERNAL_AG_UI, TASK_KIND_EXTERNAL_AGENT, TASK_KIND_MONITOR, TASK_KIND_SESSION,
    TASK_KIND_SUBAGENT, TaskArtifact, TaskError, TaskExecutor, TaskExecutorPlugin,
    TaskInputRequest, TaskLinks, TaskMessage, TaskMessageDirection, TaskMessagePart, TaskProgress,
    TaskSink, TaskWakePolicy, apply_task_update, find_task_executor,
};
pub use skill::{
    ParsedSkillMd, SkillContent, SkillFileEntry, SkillValidationResult, parse_skill_md,
    validate_skill_md, validate_skill_name,
};
pub use task_observer::{ObservingTaskRegistry, TaskTransition, TaskTransitionObserver};
pub(crate) use typed_id::{AgentId, HarnessId};
pub use wake_queue::{PendingWake, SessionWakeQueue, wake_text_for};

// Permissions re-exports
pub use permissions::{
    Caller, DefaultPermissionResolver, Permission, PermissionResolver, Policy,
    PolicyConfigResponse, PolicyError, ResourceConfigResponse, Rule, SkillPermissionAction,
    SkillPermissionPattern, SkillPermissionRule, check_skill_permission, evaluate_policies,
    evaluate_policies_with, parse_skill_permission_rule, role_has_permission, role_permissions,
};

// Dependency blocker re-exports
pub use dependency_blocker::DependencyBlocker;

// Deployment configuration
pub use deployment::DeploymentGrade;

// Execution feature decisions (EVE-878): `FeatureFlags` and the management
// catalog live in `crates/server/src/records`; core re-exports only the resolved
// execution-facing values.
pub use execution_features::{
    ExecutionFeatureDecisions, InternalFeatureFlags, feature_flag_available,
};
pub use feature_flag_grade::{FeatureFlagDefinition, FeatureFlagGrade};

pub use everruns_contracts::runtime::sandbox_context;

/// AG-UI wire values, event projection, and optional HTTP client.
#[cfg(feature = "ag-ui")]
pub mod ag_ui;
/// Portable first-party capability implementations.
#[cfg(feature = "builtins")]
pub mod builtins;
/// Credential verification for agent endpoints (OIDC/JWKS, introspection).
#[cfg(feature = "channel-auth")]
pub mod channel_auth;
/// Portable turn planning and Input/Reason/Act algorithms.
#[cfg(feature = "engine")]
pub mod engine;
/// Effectful host orchestration over injected services and registries.
#[cfg(feature = "host")]
// The inherited host module contains audited Unix ownership and containment calls.
#[allow(unsafe_code)]
pub mod host;
/// MCP client, auth, and tool adaptation over injected transports.
#[cfg(feature = "mcp")]
pub mod mcp;

#[cfg(feature = "a2a")]
pub mod a2a;

/// Shared voice loop for voice channels.
#[cfg(feature = "voice")]
pub mod voice;
