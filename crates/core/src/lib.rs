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
//! # Main Surfaces
//!
//! - Agent, harness, session, message, and event models
//! - Capability and tool traits for composing agent behavior
//! - Provider-neutral execution inputs and effect contracts
//! - Context assembly for the shared `input -> reason -> act` execution flow
//! - Neutral storage, event, capability, and host-service contracts
//!
//! Concrete Input/Reason/Act algorithms and their phase I/O values live in
//! `everruns-engine`. Core owns only concern-specific contracts such as
//! [`ExecutionContext`] and [`tool_hooks`]; it exposes no generic atom
//! extension trait or executor compatibility module.
//!
//! Environment-backed implementations are intentionally separate: filesystem,
//! Bashkit, web fetch, and Lua live in `everruns-integrations-*`; MCP adaptation
//! lives in `everruns-mcp`; and concrete runtime HTTP transport lives in
//! `everruns-host`. Hosts select those edges explicitly instead of inheriting
//! them from this contract crate.
//!
//! Hosted product capabilities — including Knowledge Bases and Indexes,
//! Memories, delegation, schedules/tasks, user hooks, and platform management
//! — live in `everruns-platform`. Backend-neutral first-party implementations
//! live in `everruns-builtins`. Core exposes only neutral collection hooks,
//! registry algorithms, and type-keyed extension seams.
//!
//! Deterministic simulation lives in `everruns-llmsim`; the in-memory agentic
//! loop, writable test doubles, and demo fixture capabilities live in
//! `everruns-test-support`. Core carries neither implementation.
//!
//! Composition is not core's job either. The execution surface an embedder
//! assembles — capability registry, provider registry, egress, utility LLM and
//! the session filesystem factory — is `everruns_host::HostComposition`
//! (EVE-887); core owns the execution contracts, not the bundle that selects a
//! deployment's shape. Provider identity, driver registration, typed IDs, and
//! LLM wire abstractions live in `everruns-provider` and are imported from
//! that crate directly.
//!
//! # Example
//!
//! ```
//! use everruns_core::CapabilityRegistry;
//! use everruns_provider::DriverRegistry;
//!
//! let capabilities = CapabilityRegistry::new();
//! assert!(capabilities.is_empty());
//!
//! let drivers = DriverRegistry::new();
//! assert!(drivers.registered_providers().is_empty());
//! ```

// Published library code is safe-only. Unit tests use Rust 2024's unsafe
// environment mutation APIs under process-wide test locks.
#![cfg_attr(not(test), forbid(unsafe_code))]
#![deny(rustdoc::broken_intra_doc_links)]

// Runtime types (tool definitions, capability types)
pub mod annotation_hook;
pub mod capability_types;
pub mod tool_fingerprint;
use everruns_provider::tool_types;

// User-defined hooks (see knowledge/runtime-resources/user-hooks.md)
pub mod hook_adapter;
pub mod hook_executor;
pub mod lifecycle_hooks;
pub mod user_hook_types;

// Deployment configuration
pub mod deployment;
pub mod egress;
pub mod exec_tool_result;
pub mod execution_context;
pub mod execution_snapshot;
pub mod utility_llm;

// Execution feature decisions (EVE-878): the org/product feature-flag records
// and management logic (`FeatureFlags`, `FeatureFlagMap`, the API catalog, org
// opt-in resolution) moved to the `everruns-platform` crate. Core keeps only
// the resolved registration-time decisions consumed by the capability
// registry builders.
pub mod execution_features;
pub mod localization;

// Telemetry conventions (neutral gen-ai span metadata contracts)
pub mod telemetry;
pub mod tool_narration;

// Event listeners (pluggable observability backends)
pub mod event_listeners;

// Error reporter (vendor-neutral embedder hook)
pub mod error_reporter;

// Observability implementations live behind `everruns-host/observability`
// (EVE-651, EVE-876): exporter listeners (Braintrust, OpenTelemetry), the
// CompositeEventListener fan-out, and OpenTelemetry/OTLP initialization. They
// depend on core only for the `EventListener` trait, event types, and the
// neutral gen-AI span conventions in `telemetry`.

// Typed ID system (type-safe prefixed identifiers)
// See knowledge/foundations/id-schema.md for specification
use everruns_provider::typed_id;

// Budget types (budgets, ledger, rules, actions)
pub mod background;
pub mod budget;

// Domain entity types
// These are DB-agnostic entity types used by both API and worker
pub mod agent_definition;
pub mod agent_identity;
pub mod ard_attachment;
pub mod capability_dto;
// EVE-878: the persisted eval aggregates (`Eval`, `EvalCase`, `EvalRun`,
// `EvalCaseResult`, `EvalRunDataset`, targets/scorers and their lifecycle
// enums) moved to the `everruns-platform` crate — they are product
// management/reporting records that never participate in a turn.
pub mod events;
pub mod finalized_tool_calls;
pub mod harness_definition;
pub mod leased_resource;
pub mod mcp_proxy;
pub mod mcp_server;
use everruns_provider::model_profiles;
pub mod model_router;
pub mod mount_fs;
pub mod network_access;
// EVE-879: the OAuth 2.1 protocol client moved to the MCP adapter crate — MCP
// login/refresh is its only consumer, and the kernel carries no token-exchange
// plumbing.
// EVE-878: the observer records (`Observer`, `TraceScore`, judge
// configuration, match rules and their lifecycle enums) moved to the
// `everruns-platform` crate — online scoring watches completed turns from the
// hosted control plane and never participates in a turn.
pub mod organization;
pub mod payment;
pub mod principal;
use everruns_provider::model_spec;
use everruns_provider::provider;
use everruns_provider::runtime_provider;
pub mod session;
pub mod session_file;
pub mod session_path;
pub mod session_resource;
pub mod session_schedule;
pub mod session_task;
pub mod skill;
pub mod system_allowlist;
pub mod task_observer;
pub mod wake_queue;
pub mod workspace_policy;
pub mod workspace_roots;

// Multi-platform channel abstractions (thread context, delivery, routing)
pub mod channel;

// Permissions model (policies, rules, caller context)
pub mod permissions;
pub mod progress_reporting;
pub mod resource_names;

// URL validation for SSRF prevention (shared utility)

// Plugin compiler (directory → declarative capability definition)
// See knowledge/integrations/plugins.md
pub mod plugins;

pub mod capabilities;
pub mod command;
pub mod command_host;
pub mod compaction_checkpoint;
pub mod compaction_policy;
pub mod config;
pub mod config_layer;
pub mod context_report;
pub mod dependency_blocker;
use everruns_provider::driver_registry;
use everruns_provider::error;
pub mod guardrail_checks;
pub mod guardrail_gallery;
pub mod llm_error_hook;
// Adapters from core domain types to provider driver types. Lives on the core
// side to keep the crate dependency one-directional (core -> everruns-provider).
pub mod llm_conversions;
pub mod message;
pub mod message_filter;
pub mod message_retriever;
mod tool_call_integrity;
pub use tool_call_integrity::{
    retain_complete_llm_tool_exchanges, retain_complete_llm_tool_exchanges_for_request,
    retain_complete_message_tool_exchanges,
};
pub mod connection_services;
pub mod delegation_services;
pub mod durability;
pub mod event_emitter;
pub mod execution_loading;
pub mod image_services;
pub mod outline;
pub mod output_guardrail;
pub mod path_identity;
pub mod provider_resolution;
pub mod resource_ownership;
pub mod runtime_agent;
pub mod runtime_context;
pub mod session_files;
pub mod session_services;
/// Narrow child-session delegation contract: core owns the host-neutral
/// interface and a host adapter supplies the implementation.
pub mod subagent_delegation;
pub mod tool_context;
pub mod tool_execution;
pub mod tool_hooks;
pub mod tool_output_sanitizer;
pub mod tools;
pub mod truncation_info;
use everruns_provider::user_facing_error;

// Private doubles for collocated unit tests. Public application backends live
// in everruns-host; reusable deterministic fixtures live in test-support.
#[cfg(test)]
mod test_fixtures;

// Stable completion semantics; execution state and planning live in everruns-engine.
pub mod turn;
pub mod turn_completion;

// Note: Chat Driver implementations (AnthropicChatDriver, OpenAIChatDriver) are now in
// separate crates (everruns-anthropic, everruns-openai) that depend on everruns-core.
// This enables dependency inversion - provider crates register their drivers at startup.

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
pub(crate) use error::{AgentLoopError, Result};
pub use event_emitter::EventEmitter;
pub use execution_loading::{HarnessStore, SessionStore};
pub use execution_snapshot::{ResolvedExecutionSnapshot, SnapshotMcpServer};
pub use image_services::{ImageResolver, ResolvedImage};
pub use llm_error_hook::{
    LlmErrorContext, LlmErrorHook, LlmErrorHookOutcome, LlmErrorHookServices,
};
pub use message::{
    AnnotationSource, ContentPart, ContentType, Controls, ExternalActor, ImageContentPart,
    ImageFileContentPart, InputContentPart, Message, MessageRole, ReasoningConfig, TextAnnotation,
    TextContentPart, ToolCallContentPart, ToolResultContentPart, VerificationStatus,
    VerificationVerdict,
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
pub use session_files::{SessionFileSystem, WorkspaceScopedFileSystem};
pub use session_services::{
    KeyInfo, LeasedResourceStore, SecretInfo, SessionResourceRegistry, SessionStorageStore,
};
pub use tool_context::{ReasoningEffortHandle, ToolContext};
pub use tool_execution::{OutboundToolRateLimiter, ToolExecutor};
pub(crate) use user_facing_error::ErrorDisclosure;
pub use workspace_policy::{WorkspacePolicy, WorkspacePolicyBuilder, WorkspacePolicyError};
pub use workspace_roots::{
    ADDITIONAL_ROOTS_MOUNT, PRIMARY_WORKSPACE_ROOT_NAME, RelPath, ResolvedPath, WorkspaceRoot,
    WorkspaceRootSet,
};

// Channel abstraction re-exports
pub use channel::{
    ChannelDeliveryAdapter, ChannelReplyMode, DeliveryContext as ChannelDeliveryContext,
    DeliveryResult as ChannelDeliveryResult, InboundAttachment, InboundChannelEvent,
    OutboundChannelMessage, Participant, SessionRoutingStrategy, ThreadContext,
};

// Narrow subagent-session delegation contract (EVE-839). The full hosted
// `PlatformStore` and its management capabilities live in `everruns-platform`.
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

// Outbound egress service re-exports
pub use egress::{
    DisabledEgressService, EgressByteStream, EgressError, EgressRequest, EgressRequestKind,
    EgressResponse, EgressResult, EgressService, EgressSigning, EgressStreamResponse,
};
pub use system_allowlist::{AllowGroup, SYSTEM_ALLOWLIST_ENABLED_ENV, SystemAllowlist};

// EVE-879: the system email contract and its concrete senders (Resend,
// disabled/noop, `SystemEmailConfig`) moved to the `everruns-platform` crate —
// email delivery is a hosted product side effect, never consumed during a
// turn. The OAuth 2.1 protocol client moved to `everruns-mcp` (its only
// consumer), and the connector catalog moved to `everruns-platform`.
pub use utility_llm::{
    DisabledUtilityLlmService, UTILITY_LLM_MODEL, UtilityLlmReasoningEffort, UtilityLlmRequest,
    UtilityLlmService,
};

// Private provider-contract imports used by kernel implementation modules.
pub(crate) use driver_registry::{
    LlmCallConfig, LlmMessage, LlmMessageRole, LlmResponse, LlmResponseStream,
    ProviderOpaqueContext,
};

// Transport-neutral native compaction contracts. Concrete OpenAI/OpenResponses
// protocol drivers live in everruns-provider and the focused provider crates.
#[cfg(test)]
pub(crate) use everruns_provider::compact::CompactOutputItem;

// Tool abstraction re-exports
pub use tools::{Tool, ToolExecutionResult, ToolInternalError, ToolRegistry, ToolRegistryBuilder};

// EVE-881: `BuiltInHarnessDefinition`, `BuiltInHarnessRole`, and
// `BuiltInCapabilityDefinition` moved to the `everruns-platform` crate —
// product provisioning templates are platform/server composition, not
// Framework execution configuration.
// EVE-887: the composition root moved to `everruns-host` as `HostComposition`.
// Selecting a deployment's capabilities, drivers and host services is
// composition, not kernel execution configuration; core owns the registries
// and service contracts, and the layer that runs a turn owns the bundle.
// EVE-880: the managed session sandbox record, its provider SPI and lifecycle
// helpers moved to the `everruns-platform` crate. One provider-backed sandbox
// per session is control-plane state — a turn reaches it through the sandbox
// capability, never through the kernel.

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
    COMPACTION_CHECKPOINT_FORMAT_VERSION, CompactionCheckpoint, CompactionCheckpointPayload,
    CompactionCheckpointStore, ProactiveCompactionAttempt, ProactiveCompactionAttemptTracker,
};

pub use execution_context::ExecutionContext;

pub(crate) use tool_types::ToolDefinition;
#[cfg(test)]
pub(crate) use tool_types::{BuiltinTool, ToolCall};

pub(crate) use everruns_capability::CapabilityRef as AgentCapabilityConfig;

// Domain entity re-exports
// Provider entities live in `everruns-provider`; import them from that crate.
// EVE-877: the stored `Agent`/`AgentVersion` persistence records, their
// lifecycle/versioning enums, and the public-name/persistence helpers moved to
// the `everruns-platform` crate. Core keeps only the portable authored
// execution configuration consumed during a turn.
pub use agent_definition::AgentDefinition;
pub use agent_identity::{AgentIdentity, AgentIdentityStatus};
// EVE-841: the app and agent-trigger control-plane records moved to the
// `everruns-platform` crate. They are hosted orchestration records not consumed
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
    CONTEXT_COMPACTING, CompactionReason, CompactionStepData, ContextCompactedData,
    ContextCompactingData, Event, EventBuilder, EventContext, EventData, EventRequest,
    FILE_WRITTEN, FileWrittenData, INPUT_MESSAGE, InputMessageData, LLM_GENERATION,
    LlmCompactionInfo, LlmGenerationData, LlmGenerationMetadata, LlmGenerationOutput, LlmRetryInfo,
    ModelMetadata, OUTPUT_MESSAGE_COMPLETED, OUTPUT_MESSAGE_DELTA, OUTPUT_MESSAGE_REPLACED,
    OUTPUT_MESSAGE_STARTED, OutputMessageCompletedData, OutputMessageDeltaData,
    OutputMessageReplacedData, OutputMessageStartedData, REASON_COMPLETED, REASON_ITEM,
    REASON_RECOVERED, REASON_STARTED, REASON_THINKING_COMPLETED, REASON_THINKING_DELTA,
    REASON_THINKING_STARTED, ReasonCompletedData, ReasonItemData, ReasonRecoveredData,
    ReasonStartedData, ReasonThinkingCompletedData, ReasonThinkingDeltaData,
    ReasonThinkingStartedData, RecoveryMode, SESSION_ACTIVATED, SESSION_IDLED,
    SESSION_MODEL_CHANGED, SESSION_STARTED, SESSION_TITLE_UPDATED, SessionActivatedData,
    SessionIdledData, SessionModelChangedData, SessionStartedData, SessionTitleUpdatedData,
    TOOL_CALL_REQUESTED, TOOL_COMPLETED, TOOL_OUTPUT_DELTA, TOOL_PROGRESS, TOOL_STARTED,
    TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED, TURN_SEALED, TURN_STARTED, TokenUsage,
    ToolCallRequestedData, ToolCallSummary, ToolCompletedData, ToolOutputDeltaData,
    ToolProgressData, ToolStartedData, TurnCancelledData, TurnCompletedData, TurnFailedData,
    TurnSealedData, TurnStartedData, VALID_EVENT_TYPES,
};
pub use finalized_tool_calls::{FinalizedToolCallsContext, FinalizedToolCallsHook};
pub use guardrail_checks::{
    CompiledJudgeCheck, GuardrailAction, GuardrailHit, GuardrailMode, GuardrailOnFail,
    GuardrailRule, GuardrailStage, GuardrailsConfig, MAX_JUDGE_PROMPT_LEN,
};
pub use guardrail_gallery::{
    DataEgress, GuardrailGalleryItem, find_guardrail_gallery_item, guardrail_gallery,
};
// EVE-881: the stored `Harness` persistence record, its lifecycle enum, the
// chain-merge helpers, and the built-in provisioning templates moved to the
// `everruns-platform` crate. Core keeps only the portable harness execution
// configuration consumed during a turn.
pub use harness_definition::HarnessDefinition;
pub use leased_resource::{
    LEASED_RESOURCES_FEATURE, LeasedResource, LeasedResourceStatus, UpsertLeasedResource,
};
pub use mcp_proxy::{McpProxyTool, McpToolInvoker, ScopedMcpToolInvoker, build_mcp_proxy_tools};
pub use mcp_server::{
    MCP_PROTOCOL_VERSION_2025_03, MCP_PROTOCOL_VERSION_2025_06, MCP_PROTOCOL_VERSION_2026_07,
    McpContent, McpError, McpProtocolMode, McpSecretBindingMetadata, McpServer, McpServerAuthMode,
    McpServerStatus, McpServerTransportType, McpToolAnnotations, McpToolCallParams,
    McpToolCallRequest, McpToolCallResponse, McpToolCallResult, McpToolDefinition,
    McpToolsListRequest, McpToolsListResponse, McpToolsListResult, ScopedMcpServer,
    ScopedMcpServers, apply_mcp_secret_binding_schemas, is_mcp_tool,
    mcp_oauth_provider_id_for_uuid, mcp_oauth_session_secret_name, mcp_tool_name,
    merge_scoped_mcp_servers, normalize_mcp_error_code, parse_mcp_tool_name,
    sanitize_mcp_server_name, scoped_mcp_servers_is_empty,
};
// EVE-837/EVE-845: `Organization`, `OrgMembership`, the `ANONYMOUS_USER_*`
// constants, and the public-id generation/validation helpers moved to the
// `everruns-platform` crate. `OrgRole` (portable turn authorization via
// `permissions`), the `DEFAULT_ORG_*` constants, and the internal<->public id
// conversion helper stay here because core's permissions/auth layer and runtime
// name them.
pub use organization::{
    DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole, org_public_id_from_internal,
};
// EVE-838: the payment accounting records (`PaymentAccount`, `PaymentPolicy`,
// `PaymentAttempt`, `PaymentOwnerType`, `PaymentStatus`) moved to the
// `everruns-platform` crate. The capability-internal execution contract below
// stays here — it is bound to the `PaymentAuthority` trait and `ToolContext`.
pub use payment::{MachinePaymentRequest, MachinePaymentResponse, PaymentMethod, PaymentRail};
// EVE-837/EVE-845: `Principal` and the `PrincipalStatus` lifecycle enum moved to
// the `everruns-platform` crate. `PrincipalSummary` and the `PrincipalKind` that
// backs it stay here — they are embedded by `Session`/`SessionSchedule`/
// `AgentIdentity`.
pub use principal::{PrincipalKind, PrincipalSummary};
pub(crate) use runtime_provider::ProviderKey;
// EVE-882: the persisted `Session` aggregate and its product lifecycle enums
// (`SessionStatus`, `SessionSource`, `SessionActivity`, participants) moved to
// the `everruns-platform` crate. Core keeps only the portable execution view
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
// `everruns-platform`. Records and trait travel together — the value types are
// the trait's signature vocabulary — and nothing in the kernel names either:
// the capability resolves the store as a typed context extension.
pub use session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskState, SessionTaskUpdate, TASK_KIND_AGENT_HANDOFF, TASK_KIND_BACKGROUND_TOOL,
    TASK_KIND_EXTERNAL_AGENT, TASK_KIND_MONITOR, TASK_KIND_SESSION, TASK_KIND_SUBAGENT,
    TaskArtifact, TaskError, TaskExecutor, TaskExecutorPlugin, TaskInputRequest, TaskLinks,
    TaskMessage, TaskMessageDirection, TaskMessagePart, TaskProgress, TaskSink, TaskWakePolicy,
    apply_task_update, find_task_executor,
};
pub use skill::{
    ParsedSkillMd, Skill, SkillContent, SkillFileEntry, SkillSourceType, SkillStatus, SkillUsage,
    SkillValidationResult, parse_skill_md, validate_skill_md, validate_skill_name,
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
// catalog live in `everruns-platform`; core re-exports only the resolved
// execution-facing values.
pub use execution_features::{ExecutionFeatureDecisions, InternalFeatureFlags};
