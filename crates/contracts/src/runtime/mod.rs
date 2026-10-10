//! Runtime SPI: the capability, tool, and session contracts integrations implement.
//!
//! Moved here from `everruns-core` so integration crates depend on contracts alone;
//! core re-exports every module at its old path.

#![allow(unused_imports)]
// Root contracts (typed ids, errors, tool types) resolve here as they did at core root.
use crate::*;

pub mod agent_definition;
pub mod annotation_hook;
pub mod background;
pub mod browser_use;
pub mod budget;
pub mod capabilities;
pub mod capability_dto;
pub mod capability_mcp_server;
pub mod capability_types;
pub mod channel;
pub mod channel_driver;
pub mod command;
pub mod command_host;
pub mod compaction_policy;
pub mod computer_use;
pub mod config_layer;
pub mod connection_services;
pub mod conversation;
pub mod decisions;
pub mod delegation_services;
pub mod dependency_blocker;
pub mod deployment;
pub mod egress;
pub mod event_emitter;
pub mod events;
pub mod exec_tool_result;
pub mod execution_context;
pub mod execution_features;
pub mod execution_loading;
pub mod feature_flag_grade;
pub mod finalized_tool_calls;
pub mod harness_definition;
pub mod hook_executor;
pub mod image_services;
pub mod leased_resource;
pub mod llm_error_hook;
pub mod localization;
pub mod mcp_deferred;
pub mod mcp_proxy;
pub mod mcp_server;
pub mod message;
pub mod message_filter;
pub mod message_retriever;
pub mod mount_fs;
pub mod network_access;
pub mod org_egress_allowlist;
pub mod organization;
pub mod outline;
pub mod output_guardrail;
pub mod payment;
pub mod principal;
pub mod resource_names;
pub mod resource_ownership;
pub mod runtime_agent;
pub mod sandbox_context;
pub mod saved_scripts;
pub mod session;
pub mod session_file;
pub mod session_files;
pub mod session_path;
pub mod session_resource;
pub mod session_schedule;
pub mod session_services;
pub mod session_task;
pub mod skill;
pub mod subagent_delegation;
pub mod system_allowlist;
pub mod tool_context;
pub mod tool_execution;
pub mod tool_hooks;
pub mod tool_narration;
pub mod tool_output_sanitizer;
pub mod tools;
pub mod truncation_info;
pub mod user_hook_types;
pub mod utility_llm;

pub use self::agent_definition::AgentDefinition;
pub use self::background::{
    BackgroundEventSink, BackgroundExecutableTool, BackgroundOutcome, BackgroundProgress,
};
pub use self::capabilities::SystemPromptContext;
pub use self::capabilities::{
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
pub use self::capabilities::{
    DeclarativeCapabilityDefinition, DeclarativeCapabilityFile, DeclarativeCapabilitySkill,
};
pub use self::capabilities::{
    SKILL_CAPABILITY_PREFIX, SKILLS_DISCOVERY_PATH, SkillCapabilityIdExt, SkillContribution,
    SkillInstructions, SkillMeta, SkillSource, discover_skills_from_entries, is_skill_capability,
    parse_skill_capability_id, reconstruct_skill_md, skill_capability_id,
};
pub use self::capability_dto::{AgentCapability, CapabilityInfo};
pub use self::capability_mcp_server::{
    CapabilityMcpServer, CapabilityMcpServers, capability_mcp_servers_to_scoped,
};
pub use self::channel::{
    ChannelAgentSurface, ChannelDeliveryAdapter, ChannelStreamDelivery, ChannelViewContext,
    DeliveryContext as ChannelDeliveryContext, DeliveryResult as ChannelDeliveryResult,
    InboundAttachment, InboundChannelEvent, OutboundChannelMessage, Participant, SessionBinding,
    ThreadContext,
};
pub use self::command_host::{
    CommandHost, CommandTurnContext, DisabledCommandHost, SessionCompletion,
    SessionCompletionError, SessionCompletionRequest, SessionCompletionStream,
};
pub use self::compaction_policy::{
    CompactionPolicy, CompactionSettings, CompactionStrategy as PolicyCompactionStrategy,
    ObservationMaskingResult as PolicyObservationMaskingResult,
};
pub use self::config_layer::{
    AgentConfigOverlay, merge_capabilities, merge_initial_files, normalize_initial_file_path,
};
pub use self::connection_services::{
    McpResolvedCredential, ServiceApiKeyConnection, UserConnectionResolver,
};
pub use self::decisions::{
    DecisionAnswer, DecisionOutcome, DecisionQuestion, DecisionRequest, DecisionUsage,
    DecisionsService, DisabledDecisionsService,
};
pub use self::delegation_services::{SpawnClaimResult, SubagentNestingPolicy, SubagentSpawnStore};
pub use self::dependency_blocker::DependencyBlocker;
pub use self::deployment::DeploymentGrade;
pub use self::egress::{
    DisabledEgressService, EgressByteStream, EgressError, EgressRequest, EgressRequestKind,
    EgressResponse, EgressResult, EgressScope, EgressService, EgressSigning, EgressStreamResponse,
    ScopedEgressService,
};
pub use self::event_emitter::EventEmitter;
pub use self::events::{
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
pub use self::execution_context::ExecutionContext;
pub use self::execution_features::{
    ExecutionFeatureDecisions, InternalFeatureFlags, feature_flag_available,
};
pub use self::execution_loading::{HarnessStore, SessionStore};
pub use self::feature_flag_grade::{FeatureFlagDefinition, FeatureFlagGrade};
pub use self::finalized_tool_calls::{
    FinalizedToolCallRejection, FinalizedToolCallsContext, FinalizedToolCallsHook,
};
pub use self::harness_definition::HarnessDefinition;
pub use self::image_services::{ImageResolver, ResolvedImage};
pub use self::leased_resource::{
    LEASED_RESOURCES_FEATURE, LeasedResource, LeasedResourceStatus, UpsertLeasedResource,
};
pub use self::llm_error_hook::{
    LlmErrorContext, LlmErrorHook, LlmErrorHookOutcome, LlmErrorHookServices,
};
pub use self::mcp_deferred::{
    DEFERRED_MCP_REVEAL_KV_PREFIX, DeferredMcpServerTool, deferred_mcp_server_definition,
    deferred_mcp_server_prefix, deferred_mcp_server_tool_name,
    normalize_deferred_mcp_server_definition, partition_deferred_mcp_servers,
    reveal_deferred_mcp_server, revealed_mcp_servers,
};
pub use self::mcp_proxy::{
    McpCallIdentity, McpProxyTool, McpServerTools, McpToolInvoker, ScopedMcpToolInvoker,
    build_mcp_proxy_tools,
};
pub use self::mcp_server::{
    MCP_PROTOCOL_VERSION_2025_03, MCP_PROTOCOL_VERSION_2025_06, MCP_PROTOCOL_VERSION_2026_07,
    McpConnectInChat, McpContent, McpElicitationPolicy, McpError, McpProtocolMode,
    McpSecretBindingMetadata, McpServerActsAs, McpServerAuthMode, McpServerPresetRef,
    McpServerTransportType, McpToolAnnotations, McpToolCallParams, McpToolCallRequest,
    McpToolCallResponse, McpToolCallResult, McpToolDefinition, McpToolsListRequest,
    McpToolsListResponse, McpToolsListResult, ScopedMcpServer, ScopedMcpServers,
    apply_mcp_secret_binding_schemas, is_mcp_tool, mcp_oauth_provider_id_for_uuid,
    mcp_oauth_session_secret_name, mcp_tool_name, merge_scoped_mcp_servers,
    normalize_mcp_error_code, parse_mcp_tool_name, sanitize_mcp_server_name,
    scoped_mcp_servers_is_empty,
};
pub use self::message::{
    AnnotationSource, ContentPart, ContentType, Controls, ExternalActor, ImageContentPart,
    ImageFileContentPart, InputContentPart, ReasoningConfig, RuntimeMessage, RuntimeMessageRole,
    TextAnnotation, TextContentPart, ToolCallContentPart, ToolResultContentPart,
    VerificationStatus, VerificationVerdict,
};
pub use self::message_filter::{
    ExcludedNoticeTransform, FilterContext, InjectedMessage, InjectionPosition, MessageFilter,
    MessageFilterProvider, MessageQuery, PrependTransform,
};
pub use self::message_retriever::{InputMessage, MessageHistory, MessageRetriever};
pub use self::mount_fs::{DisplayPolicy, MountFs, WORKSPACE_MOUNT, scoped_prompt_file_store};
pub use self::org_egress_allowlist::{
    MAX_ORG_EGRESS_ALLOWLIST_PATTERNS, OrgEgressAllowlist, org_egress_extension,
    validate_org_egress_pattern, validate_org_egress_patterns,
};
pub use self::organization::{
    DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole, org_public_id_from_internal,
};
pub use self::payment::{
    MachinePaymentRequest, MachinePaymentResponse, PaymentMethod, PaymentRail,
};
pub use self::principal::{PrincipalKind, PrincipalSummary};
pub use self::resource_ownership::{
    LEASED_RESOURCE_EXTERNAL_ID_KEY, LEASED_RESOURCE_ID_KEY, LEASED_RESOURCE_PROVIDER_KEY,
    LEASED_RESOURCE_TYPE_KEY, list_owned_external_resource_ids,
    ownership_tracking_unavailable_error, require_owned_external_resource,
    resource_not_owned_error, verify_owned_external_resource_if_available,
};
pub use self::runtime_agent::{RuntimeAgent, RuntimeAgentBuilder};
pub use self::session::{ExecutionSession, SessionExecutionState, SessionSeedMode, SubagentStatus};
pub use self::session_file::{
    FileInfo, FileStat, GREP_MAX_CONTEXT_LINES, GREP_MAX_RETURN_BYTES, GrepContextBlock,
    GrepContextLine, GrepMatch, GrepOptions, GrepResult, GrepSearchResult, InitialFile,
    SessionFile,
};
pub use self::session_files::{
    RuntimeArtifactFileSystem, SessionFileSystem, WorkspaceScopedFileSystem,
};
pub use self::session_resource::{
    RegisterSessionResource, SessionResourceEntry, SessionResourceFilter, SessionResourceStatus,
};
pub use self::session_services::{
    KeyInfo, LeasedResourceStore, SecretInfo, SessionResourceRegistry, SessionStorageStore,
};
pub use self::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskState, SessionTaskUpdate, TASK_KIND_AGENT_HANDOFF, TASK_KIND_BACKGROUND_TOOL,
    TASK_KIND_EXTERNAL_AG_UI, TASK_KIND_EXTERNAL_AGENT, TASK_KIND_MONITOR, TASK_KIND_SESSION,
    TASK_KIND_SUBAGENT, TaskArtifact, TaskError, TaskExecutor, TaskExecutorPlugin,
    TaskInputRequest, TaskLinks, TaskMessage, TaskMessageDirection, TaskMessagePart, TaskProgress,
    TaskSink, TaskWakePolicy, apply_task_update, find_task_executor,
};
pub use self::skill::{
    ParsedSkillMd, SkillContent, SkillFileEntry, SkillValidationResult, parse_skill_md,
    validate_skill_md, validate_skill_name,
};
pub use self::subagent_delegation::{
    PlatformCreateSessionRequest, PlatformMessage, SubagentSessionDelegate,
};
pub use self::system_allowlist::{
    AllowGroup, EGRESS_POLICY_ENV, EgressAccess, EgressPolicyDenial, EgressPolicyGrant,
    EgressPolicyMode, OPEN_READ_MAX_URL_LEN, SYSTEM_ALLOWLIST_ENABLED_ENV, SystemAllowlist,
    SystemEgressPolicy,
};
pub use self::tool_context::{ReasoningEffortHandle, ToolContext};
pub use self::tool_execution::{OutboundToolRateLimiter, ToolExecutor};
pub use self::tools::{
    CliSpelling, Tool, ToolExecutionResult, ToolInternalError, ToolRegistry, ToolRegistryBuilder,
};
pub use self::utility_llm::{
    DisabledUtilityLlmService, UTILITY_LLM_MODEL, UtilityLlmReasoningEffort, UtilityLlmRequest,
    UtilityLlmService,
};
pub(crate) use crate::CapabilityRef as AgentCapabilityConfig;
#[cfg(test)]
pub(crate) use crate::compact::CompactOutputItem;
use crate::driver_registry;
use crate::error;
use crate::model_profiles;
use crate::model_spec;
use crate::provider;
use crate::runtime_provider;
use crate::tool_types;
use crate::typed_id;
use crate::user_facing_error;
