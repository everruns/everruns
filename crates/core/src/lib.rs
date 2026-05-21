//! Core agent abstractions for Everruns.
//!
//! `everruns-core` is the shared contract crate for the
//! [Everruns](https://everruns.com) ecosystem. It defines the runtime-facing
//! types used by embedded hosts, workers, provider drivers, integrations, and
//! the control plane.
//!
//! The crate is deliberately storage-agnostic. Agent execution is expressed in
//! terms of traits such as [`MessageRetriever`], [`ToolExecutor`],
//! [`EventEmitter`], and [`LlmProviderStore`], while host crates decide whether
//! those traits are backed by memory, PostgreSQL, gRPC, or another system.
//!
//! # Main Surfaces
//!
//! - Agent, harness, session, message, event, and typed ID models
//! - Capability and tool traits for composing agent behavior
//! - Provider-neutral LLM messages, streams, and driver registration
//! - Context assembly for the shared `input -> reason -> act` execution flow
//! - In-memory helpers and `llmsim` for deterministic tests and examples
//!
//! # Example
//!
//! ```ignore
//! use everruns_core::{CapabilityRegistry, DriverRegistry, PlatformDefinition};
//! use everruns_core::capabilities::TestMathCapability;
//!
//! let mut capabilities = CapabilityRegistry::new();
//! capabilities.register(TestMathCapability);
//!
//! let platform = PlatformDefinition::new(capabilities, DriverRegistry::new());
//! assert!(platform.capability_registry().get("test_math").is_some());
//! ```

// Runtime types (tool definitions, capability types)
pub mod capability_types;
pub mod tool_types;

// Deployment configuration
pub mod deployment;
pub mod email;
pub mod exec_tool_result;

// Feature flags
pub mod feature_flags;
pub mod localization;

// Telemetry (OpenTelemetry with gen-ai semantic conventions)
pub mod telemetry;
pub mod tool_narration;

// Event listeners (pluggable observability backends)
pub mod event_listeners;

// Error reporter (vendor-neutral embedder hook)
pub mod error_reporter;

// Observation backends (OTel, etc.)
pub mod observation;

// Typed ID system (type-safe prefixed identifiers)
// See specs/id-schema.md for specification
pub mod typed_id;

// Audit logging types and trait (EVE-226)
pub mod audit;

// Budget types (budgets, ledger, rules, actions)
pub mod background;
pub mod budget;

// Domain entity types
// These are DB-agnostic entity types used by both API and worker
pub mod agent;
pub mod agent_identity;
pub mod app;
pub mod capability_dto;
pub mod connection_provider;
pub mod eval;
pub mod events;
pub mod harness;
pub mod leased_resource;
pub mod llm_model_profiles;
pub mod llm_models;
pub mod mcp_server;
pub mod memory_store;
pub mod model_router;
pub mod network_access;
pub mod organization;
pub mod payment;
pub mod principal;
pub mod reporting;
pub mod session;
pub mod session_file;
pub mod session_resource;
pub mod session_sandbox;
pub mod session_schedule;
pub mod session_sqldb;
pub mod skill;
pub mod volume;

// Multi-platform channel abstractions (thread context, delivery, routing)
pub mod channel;

// Permissions model (policies, rules, caller context)
pub mod permissions;
pub mod progress_reporting;
pub mod resource_names;

// URL validation for SSRF prevention (shared utility)
pub mod url_validation;

pub mod atoms;
pub mod capabilities;
pub mod command;
pub mod config_layer;
pub mod context_report;
pub mod dependency_blocker;
pub mod error;
pub mod llm_driver_helpers;
pub mod llm_driver_registry;
pub mod llm_retry;
pub mod message;
pub mod message_filter;
pub mod message_retriever;
pub mod openai_protocol;
pub mod openresponses_protocol;
pub mod openresponses_types;
pub mod outline;
pub mod output_guardrail;
pub mod platform_definition;
pub mod platform_store;
pub mod resource_ownership;
pub mod runtime_agent;
pub mod runtime_context;
pub mod tool_output_sanitizer;
pub mod tools;
pub mod traits;
pub mod truncation_info;
pub mod user_facing_error;

// In-memory implementations for examples and testing
pub mod memory;

// LLM Simulator driver for testing
pub mod llmsim_driver;

// In-memory agentic loop for testing and prototyping
pub mod in_memory_loop;

// Turn orchestration (state machine, context, outcomes)
pub mod turn;

// Note: LLM Driver implementations (AnthropicLlmDriver, OpenAILlmDriver) are now in
// separate crates (everruns-anthropic, everruns-openai) that depend on everruns-core.
// This enables dependency inversion - provider crates register their drivers at startup.

// Re-exports for convenience
pub use config_layer::{
    AgentConfigOverlay, merge_capabilities, merge_initial_files, normalize_initial_file_path,
};
pub use error::{AgentLoopError, Result, StoreResultExt, from_json, json_val};
pub use message::{
    ContentPart, ContentType, Controls, ExternalActor, ImageContentPart, ImageFileContentPart,
    InputContentPart, Message, MessageRole, ReasoningConfig, TextContentPart, ToolCallContentPart,
    ToolResultContentPart,
};
pub use message_filter::{
    ExcludedNoticeTransform, FilterContext, InjectedMessage, InjectionPosition, MessageFilter,
    MessageFilterProvider, MessageQuery, PrependTransform,
};
pub use message_retriever::{InputMessage, MessageRetriever};
pub use runtime_agent::{RuntimeAgent, RuntimeAgentBuilder};
pub use runtime_context::{
    AssembledTurnContext, ResolvedRuntimeCapabilities, assemble_turn_context, inspect_turn_context,
    resolve_runtime_capabilities,
};
pub use traits::{
    DisabledSessionFileSystemFactory, EventEmitter, HarnessStore, ImageResolver, KeyInfo,
    LeasedResourceStore, LlmProviderStore, ModelWithProvider, NoopEventEmitter, ResolvedImage,
    SecretInfo, SessionFileStore, SessionFileSystem, SessionFileSystemFactory,
    SessionFileSystemFactoryContext, SessionMutator, SessionResourceRegistry, SessionSqlDbStoreRef,
    SessionStorageStore, SessionStore, ToolContext, ToolExecutor, UserConnectionResolver,
};
pub use user_facing_error::{
    UserFacingError, UserFacingErrorContext, UserFacingErrorFields, classify_runtime_error_message,
    codes as user_facing_error_codes, trim_error_chain_prefixes,
};

// Memory store re-exports
pub use memory_store::{
    Memory, MemoryContentPart, MemoryImagePart, MemoryKind, MemoryLimits, MemoryQuery,
    MemoryStoreBackend, MemoryStoreEntity, MemoryTextPart,
};

// Channel abstraction re-exports
pub use channel::{
    ChannelDeliveryAdapter, ChannelReplyMode, DeliveryContext as ChannelDeliveryContext,
    DeliveryResult as ChannelDeliveryResult, InboundAttachment, InboundChannelEvent,
    OutboundChannelMessage, Participant, SessionRoutingStrategy, ThreadContext,
};

// Platform store re-exports
pub use platform_store::{PlatformMessage, PlatformStore};
pub use resource_ownership::{
    LEASED_RESOURCE_EXTERNAL_ID_KEY, LEASED_RESOURCE_ID_KEY, LEASED_RESOURCE_PROVIDER_KEY,
    LEASED_RESOURCE_TYPE_KEY, list_owned_external_resource_ids,
    ownership_tracking_unavailable_error, require_owned_external_resource,
    resource_not_owned_error, verify_owned_external_resource_if_available,
};

// Event listener re-exports
pub use background::{
    BackgroundEventSink, BackgroundExecutableTool, BackgroundOutcome, BackgroundProgress,
};
pub use event_listeners::{CompositeEventListener, EventListener, NoopEventListener};

// Error reporter re-exports
pub use error_reporter::{
    ErrorReport, ErrorReporter, ErrorScope, ErrorSeverity, NoopErrorReporter, SharedErrorReporter,
};

// System email re-exports
pub use email::{
    DisabledEmailSender, EmailAddress, EmailError, EmailMessage, EmailResult, EmailSender,
    EmailTag, EmailTemplate, GenericEmailTemplate, NoopEmailSender, RenderedEmail,
    ResendEmailConfig, ResendEmailSender, SYSTEM_EMAIL_FROM, SentEmail, SystemEmailConfig,
    system_email_from,
};

// LLM driver types re-exports
pub use llm_driver_registry::{
    BoxedLlmDriver, DiscoveredModel, DriverFactory, DriverRegistry, LlmCallConfig,
    LlmCallConfigBuilder, LlmCompletionMetadata, LlmContentPart, LlmDriver, LlmMessage,
    LlmMessageContent, LlmMessageRole, LlmResponse, LlmResponseStream, LlmStreamEvent,
    ProviderConfig, ProviderType,
};

// LLM retry types re-exports
pub use llm_retry::{LlmRetryConfig, RateLimitInfo, RateLimitType, RetryMetadata};

// OpenAI Protocol driver (Chat Completions API for backward compatibility)
pub use openai_protocol::OpenAIProtocolLlmDriver;

// Open Responses Protocol driver (https://www.openresponses.org/)
// Vendor-neutral API standard, recommended for new projects
pub use openresponses_protocol::{
    CompactContent, CompactContentPart, CompactInputItem, CompactOutputItem, CompactRequest,
    CompactResponse, CompactUsage, OpenResponsesProtocolLlmDriver, compact_output_to_messages,
    messages_to_compact_input,
};

// Tool abstraction re-exports
pub use tools::{
    EchoTool, FailingTool, SpawnBackgroundTool, Tool, ToolExecutionResult, ToolInternalError,
    ToolRegistry, ToolRegistryBuilder, ToolResultImage,
};

// Capability re-exports
// Connection provider plugin system (API key connections like Daytona)
pub use connection_provider::{
    ConnectionFormSchema, ConnectionProvider, ConnectionProviderPlugin, ConnectionProviderRegistry,
    ConnectionProviderRegistryBuilder, ConnectionType, ConnectionValidation, FieldType, FormField,
};
pub use platform_definition::{
    BuiltInCapabilityDefinition, BuiltInHarnessDefinition, BuiltInHarnessRole, PlatformDefinition,
    PlatformDefinitionBuilder,
};
pub use session_sandbox::{
    DEFAULT_SESSION_SANDBOX_IDLE_TIMEOUT_SECS, SESSION_SANDBOX_CAPABILITY_ID,
    SESSION_SANDBOX_SECRET_NAME, SessionSandboxConfig, SessionSandboxExecRequest,
    SessionSandboxExecResponse, SessionSandboxInitConfig, SessionSandboxInstance,
    SessionSandboxProvider, SessionSandboxProviderPlugin, SessionSandboxReadFileResponse,
    SessionSandboxState, SessionSandboxStatus, SessionSandboxStatusResponse,
    SessionSandboxWriteFileResponse, create_session_sandbox_provider, delete_session_sandbox,
    delete_session_sandbox_state, ensure_session_sandbox_running, load_session_sandbox_state,
    pause_session_sandbox, run_session_sandbox_init_if_needed, save_session_sandbox_state,
    session_sandbox_config_from_capabilities, session_sandbox_tool_hints,
};

pub use capabilities::SystemPromptContext;
pub use capabilities::{
    AddTool, AgentBlueprint, AgentCapabilityConfig, AppliedCapabilities, BlueprintModel,
    Capability, CapabilityId, CapabilityRegistry, CapabilityRegistryBuilder, CapabilityStatus,
    CollectedCapabilities, CurrentTimeCapability, DECLARATIVE_CAPABILITY_PREFIX, DeleteFileTool,
    DependencyError, DivideTool, FileSystemCapability, GetCurrentTimeTool, GetForecastTool,
    GetSessionInfoTool, GetWeatherTool, GrepFilesTool, HUMAN_INTENT_CAPABILITY_ID,
    HumanIntentCapability, INFINITY_CONTEXT_CAPABILITY_ID, InfinityContextCapability,
    IntegrationPlugin, ListDirectoryTool, MAX_RESOLVED_CAPABILITIES, MCP_CAPABILITY_PREFIX,
    McpCapability, MountAccess, MountDirectoryBuilder, MountEntry, MountPoint, MountSource,
    MultiplyTool, NoopCapability, OPENAI_TOOL_SEARCH_CAPABILITY_ID, OpenAiToolSearchCapability,
    PlatformManagementCapability, QueryHistoryTool, ReadFileTool, ResearchCapability,
    ResolvedCapabilities, RiskLevel, SampleDataCapability, SessionCapability,
    SessionSandboxCapability, SessionSqlDatabaseCapability, SqlExecuteTool, SqlQueryTool,
    SqlSchemaTool, StatFileTool, StatelessTodoListCapability, SubtractTool, TestMathCapability,
    TestWeatherCapability, ToolCallHook, ToolDefinitionHook, WriteFileTool, WriteSessionTitleTool,
    WriteTodosTool, apply_capabilities, collect_capabilities, collect_capabilities_with_configs,
    compute_features, declarative_capability_id, declarative_capability_info, get_dependencies,
    hydrate_declarative_capability_config, is_declarative_capability, is_mcp_capability,
    mcp_capability_id, parse_declarative_capability_id, parse_mcp_capability_id,
    resolve_dependencies, validate_declarative_capability_definition,
};
pub use capabilities::{
    AttachSkillCapability, SKILL_CAPABILITY_PREFIX, SKILLS_CAPABILITY_ID, SKILLS_DISCOVERY_PATH,
    SkillInstructions, SkillMeta, SkillSource, SkillsCapability, discover_skills_from_entries,
    is_skill_capability, parse_skill_capability_id, skill_capability_id,
};
pub use capabilities::{
    DeclarativeCapabilityDefinition, DeclarativeCapabilityFile, DeclarativeCapabilitySkill,
};

// Atoms re-exports (stateless atomic operations)
pub use atoms::{
    ActAtom, ActInput, ActResult, Atom, AtomContext, ClientSideToolHook, ConnectionSetupHook,
    InputAtom, InputAtomInput, InputAtomResult, PostActAction, PostActHook, PostToolExecHook,
    ReasonAtom, ReasonInput, ReasonResult, ToolCallResult,
};

// Tool types (runtime types defined in this crate)
pub use tool_types::{
    BuiltinTool, ClientSideTool, DeferrablePolicy, ToolCall, ToolDefinition, ToolHints, ToolPolicy,
    ToolResult,
};

// Note: CapabilityId and CapabilityStatus are re-exported via capabilities module

// Domain entity re-exports
// Note: LlmProvider entity is in llm_models module. Import as: everruns_core::llm_models::LlmProvider
pub use agent::{
    Agent, AgentStatus, AgentVersion, AgentVersionChangeKind, MAX_ADDRESSABLE_NAME_LEN,
    generate_agent_public_id, validate_addressable_name, validate_agent_public_id,
};
pub use agent_identity::{AgentIdentity, AgentIdentityStatus};
pub use app::{
    A2aChannelConfig, AgUiChannelConfig, AgUiToolVisibility, AgentVersionPolicy, App, AppChannel,
    AppEndpointAuthConfig, AppEndpointAuthMode, AppEndpointAuthProviderConfig,
    AppEndpointAuthRequirements, AppStatus, ChannelType, FcpChannelConfig, SessionStrategy,
    SlackChannelConfig, SlackReplyMode,
};
pub use capability_dto::{AgentCapability, CapabilityInfo};
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
    OutputMessageReplacedData, OutputMessageStartedData, REASON_COMPLETED, REASON_STARTED,
    REASON_THINKING_COMPLETED, REASON_THINKING_DELTA, REASON_THINKING_STARTED, ReasonCompletedData,
    ReasonStartedData, ReasonThinkingCompletedData, ReasonThinkingDeltaData,
    ReasonThinkingStartedData, SESSION_ACTIVATED, SESSION_IDLED, SESSION_STARTED,
    SessionActivatedData, SessionIdledData, SessionStartedData, TOOL_CALL_REQUESTED,
    TOOL_COMPLETED, TOOL_OUTPUT_DELTA, TOOL_PROGRESS, TOOL_STARTED, TURN_CANCELLED, TURN_COMPLETED,
    TURN_FAILED, TURN_STARTED, TokenUsage, ToolCallRequestedData, ToolCallSummary,
    ToolCompletedData, ToolOutputDeltaData, ToolProgressData, ToolStartedData, TurnCancelledData,
    TurnCompletedData, TurnFailedData, TurnStartedData, VALID_EVENT_TYPES,
};
pub use harness::{Harness, HarnessStatus, merge_harness, merge_harness_chain};
pub use leased_resource::{
    LEASED_RESOURCES_FEATURE, LeasedResource, LeasedResourceStatus, UpsertLeasedResource,
};
pub use llm_model_profiles::get_model_profile;
pub use llm_models::{
    CostTier, LlmModel, LlmModelCost, LlmModelLimits, LlmModelModalities, LlmModelProfile,
    LlmModelSource, LlmModelWithProvider, LlmProviderStatus, LlmProviderType, Modality,
    ReasoningEffort, ReasoningEffortConfig, ReasoningEffortValue,
};
pub use mcp_server::{
    McpContent, McpError, McpServer, McpServerAuthMode, McpServerStatus, McpServerTransportType,
    McpToolAnnotations, McpToolCallParams, McpToolCallRequest, McpToolCallResponse,
    McpToolCallResult, McpToolDefinition, McpToolsListRequest, McpToolsListResponse,
    McpToolsListResult, ScopedMcpServer, ScopedMcpServers, is_mcp_tool,
    mcp_oauth_provider_id_for_uuid, mcp_oauth_session_secret_name, mcp_tool_name,
    merge_scoped_mcp_servers, parse_mcp_tool_name, sanitize_mcp_server_name,
    scoped_mcp_servers_is_empty,
};
pub use organization::{
    ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID, ANONYMOUS_USER_NAME, DEFAULT_ORG_ID,
    DEFAULT_ORG_PUBLIC_ID, OrgMembership, OrgRole, Organization, generate_org_public_id,
    org_public_id_from_internal, validate_org_public_id,
};
pub use payment::{
    MachinePaymentRequest, MachinePaymentResponse, PaymentAccount, PaymentAttempt, PaymentMethod,
    PaymentOwnerType, PaymentPolicy, PaymentRail, PaymentStatus,
};
pub use principal::{Principal, PrincipalKind, PrincipalStatus, PrincipalSummary};
pub use session::{Session, SessionStatus, SubagentStatus};
pub use session_file::{FileInfo, FileStat, GrepMatch, GrepResult, InitialFile, SessionFile};
pub use session_resource::{
    RegisterSessionResource, SessionResourceEntry, SessionResourceFilter, SessionResourceStatus,
};
pub use session_sqldb::{
    ColumnSchema, DatabaseInfo, SessionSqlDbError, SessionSqlDbStore, SqlExecuteResult,
    SqlQueryResult, TableSchema,
};
pub use skill::{
    ParsedSkillMd, Skill, SkillContent, SkillFileEntry, SkillSourceType, SkillStatus, SkillUsage,
    SkillValidationResult, parse_skill_md, validate_skill_md, validate_skill_name,
};
pub use typed_id::{
    AgentId, AgentIdentityId, AgentVersionId, AppChannelId, AppId, DeclarativeCapabilityId,
    EvalCaseId, EvalId, EvalResultId, EvalRunId, EventId, ExecId, HarnessId, IdMarker,
    IdParseError, ImageId, KnowledgeBaseId, KnowledgeEntryId, LeasedResourceId, McpServerId,
    MemoryId, MemoryStoreId, MessageId, ModelId, NotificationId, OrgId, PaymentAccountId,
    PaymentAttemptId, PaymentPolicyId, PrincipalId, ProviderId, ScheduleId, SessionId, SkillId,
    TurnId, TypedId,
};

// Audit logging re-exports
pub use audit::{
    AgentAction, AuditAction, AuditDomain, AuditEvent, AuditEventBuilder, AuditLogger, AuditTarget,
    ManagementAction,
};

// Permissions re-exports
pub use permissions::{
    Caller, DefaultPermissionResolver, Permission, PermissionResolver, Policy,
    PolicyConfigResponse, PolicyError, ResourceConfigResponse, Rule, SkillPermissionAction,
    SkillPermissionPattern, SkillPermissionRule, check_skill_permission, evaluate_policies,
    evaluate_policies_with, parse_skill_permission_rule, role_has_permission, role_permissions,
};

// Dependency blocker re-exports
pub use dependency_blocker::{DependencyBlocker, detect_dependency_blocker};

// URL validation re-exports
pub use url_validation::{UrlValidationError, is_blocked_ip, validate_safe_url};

// Deployment configuration
pub use deployment::DeploymentGrade;

// Feature flags
pub use feature_flags::{FeatureFlags, InternalFeatureFlags};

// Observation backends
pub use observation::{BraintrustConfig, BraintrustListener, OtelEventListener};
