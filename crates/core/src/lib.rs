// Agent Loop Abstraction
//
// This crate provides a DB-agnostic, streamable, and decomposable implementation
// of an agentic loop (LLM call → tool execution → repeat).

//
// Key design decisions:
// - Uses traits (MessageRetriever, ToolExecutor) for pluggable backends
// - MessageRetriever is retrieval-only; messages are stored via EventEmitter
// - Can be decomposed into steps for durable activity execution
// - Configuration via RuntimeAgent (can be built from Agent entity or created directly)
// - Tools are defined via a Tool trait for flexibility (function-style tools)
// - ToolRegistry implements ToolExecutor for easy tool management
// - Error handling distinguishes between user-visible and internal errors
// - Capabilities provide modular functionality units for composing agent behavior
// - Domain entity types (Agent, Session, LlmProvider, etc.) are defined here
// - Tool types are defined here as runtime types

// Runtime types (tool definitions, capability types)
pub mod capability_types;
pub mod tool_types;

// Deployment configuration
pub mod deployment;

// Feature flags
pub mod feature_flags;

// Telemetry (OpenTelemetry with gen-ai semantic conventions)
pub mod telemetry;
pub mod tool_narration;

// Event listeners (pluggable observability backends)
pub mod event_listeners;

// Observation backends (OTel, etc.)
pub mod observation;

// Typed ID system (type-safe prefixed identifiers)
// See specs/id-schema.md for specification
pub mod typed_id;

// Domain entity types
// These are DB-agnostic entity types used by both API and worker
pub mod agent;
pub mod app;
pub mod capability_dto;
pub mod connection_provider;
pub mod events;
pub mod harness;
pub mod leased_resource;
pub mod llm_model_profiles;
pub mod llm_models;
pub mod mcp_server;
pub mod organization;
pub mod session;
pub mod session_file;
pub mod session_schedule;
pub mod session_sqldb;
pub mod skill;

// Permissions model (policies, rules, caller context)
pub mod permissions;

// URL validation for SSRF prevention (shared utility)
pub mod url_validation;

pub mod atoms;
pub mod capabilities;
pub mod command;
pub mod error;
pub mod llm_driver_registry;
pub mod llm_retry;
pub mod message;
pub mod message_filter;
pub mod message_retriever;
pub mod openai_protocol;
pub mod openresponses_protocol;
pub mod openresponses_types;
pub mod platform_definition;
pub mod platform_store;
pub mod runtime_agent;
pub mod tools;
pub mod traits;

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
pub use error::{AgentLoopError, Result};
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
pub use traits::{
    EventEmitter, HarnessStore, ImageResolver, KeyInfo, LeasedResourceStore, LlmProviderStore,
    ModelWithProvider, NoopEventEmitter, ResolvedImage, SecretInfo, SessionFileStore,
    SessionMutator, SessionSqlDbStoreRef, SessionStorageStore, SessionStore, ToolContext,
    ToolExecutor, UserConnectionResolver,
};

// Platform store re-exports
pub use platform_store::{PlatformMessage, PlatformStore};

// Event listener re-exports
pub use event_listeners::{CompositeEventListener, EventListener, NoopEventListener};

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
    EchoTool, FailingTool, Tool, ToolExecutionResult, ToolInternalError, ToolRegistry,
    ToolRegistryBuilder, ToolResultImage,
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

pub use capabilities::SystemPromptContext;
pub use capabilities::{
    AddTool, AgentCapabilityConfig, AppliedCapabilities, Capability, CapabilityId,
    CapabilityRegistry, CapabilityRegistryBuilder, CapabilityStatus, CollectedCapabilities,
    CurrentTimeCapability, DeleteFileTool, DependencyError, DivideTool, FileSystemCapability,
    GetCurrentTimeTool, GetForecastTool, GetSessionInfoTool, GetWeatherTool, GrepFilesTool,
    INFINITY_CONTEXT_CAPABILITY_ID, InfinityContextCapability, IntegrationPlugin,
    ListDirectoryTool, MAX_RESOLVED_CAPABILITIES, MCP_CAPABILITY_PREFIX, McpCapability,
    MountAccess, MountDirectoryBuilder, MountEntry, MountPoint, MountSource, MultiplyTool,
    NoopCapability, OPENAI_TOOL_SEARCH_CAPABILITY_ID, OpenAiToolSearchCapability,
    PlatformManagementCapability, QueryHistoryTool, ReadFileTool, ResearchCapability,
    ResolvedCapabilities, RiskLevel, SampleDataCapability, SessionCapability,
    SessionSqlDatabaseCapability, SqlExecuteTool, SqlQueryTool, SqlSchemaTool, StatFileTool,
    StatelessTodoListCapability, SubtractTool, TestMathCapability, TestWeatherCapability,
    WriteFileTool, WriteSessionTitleTool, WriteTodosTool, apply_capabilities, collect_capabilities,
    collect_capabilities_with_configs, compute_features, get_dependencies, is_mcp_capability,
    mcp_capability_id, parse_mcp_capability_id, resolve_dependencies,
};
pub use capabilities::{
    AttachSkillCapability, SKILL_CAPABILITY_PREFIX, SKILLS_CAPABILITY_ID, SKILLS_DISCOVERY_PATH,
    SkillInstructions, SkillMeta, SkillSource, SkillsCapability, discover_skills_from_entries,
    is_skill_capability, parse_skill_capability_id, skill_capability_id,
};

// Atoms re-exports (stateless atomic operations)
pub use atoms::{
    ActAtom, ActInput, ActResult, Atom, AtomContext, InputAtom, InputAtomInput, InputAtomResult,
    ReasonAtom, ReasonInput, ReasonResult, ToolCallResult,
};

// Tool types (runtime types defined in this crate)
pub use tool_types::{
    BuiltinTool, ClientSideTool, DeferrablePolicy, ToolCall, ToolDefinition, ToolPolicy, ToolResult,
};

// Note: CapabilityId and CapabilityStatus are re-exported via capabilities module

// Domain entity re-exports
// Note: LlmProvider entity is in llm_models module. Import as: everruns_core::llm_models::LlmProvider
pub use agent::{Agent, AgentStatus, generate_agent_public_id, validate_agent_public_id};
pub use app::{App, AppStatus, ChannelType, SessionStrategy, SlackChannelConfig};
pub use capability_dto::{AgentCapability, CapabilityInfo};
pub use events::{
    ACT_COMPLETED, ACT_STARTED, ActCompletedData, ActStartedData, CONTEXT_COMPACTED,
    CONTEXT_COMPACTING, CompactionReason, CompactionStepData, ContextCompactedData,
    ContextCompactingData, Event, EventBuilder, EventContext, EventData, EventRequest,
    INPUT_MESSAGE, InputMessageData, LLM_GENERATION, LlmCompactionInfo, LlmGenerationData,
    LlmGenerationMetadata, LlmGenerationOutput, LlmRetryInfo, ModelMetadata,
    OUTPUT_MESSAGE_COMPLETED, OUTPUT_MESSAGE_DELTA, OUTPUT_MESSAGE_STARTED,
    OutputMessageCompletedData, OutputMessageDeltaData, OutputMessageStartedData, REASON_COMPLETED,
    REASON_STARTED, REASON_THINKING_COMPLETED, REASON_THINKING_DELTA, REASON_THINKING_STARTED,
    ReasonCompletedData, ReasonStartedData, ReasonThinkingCompletedData, ReasonThinkingDeltaData,
    ReasonThinkingStartedData, SESSION_ACTIVATED, SESSION_IDLED, SESSION_STARTED,
    SessionActivatedData, SessionIdledData, SessionStartedData, TOOL_CALL_REQUESTED,
    TOOL_COMPLETED, TOOL_STARTED, TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED, TURN_STARTED,
    TokenUsage, ToolCallRequestedData, ToolCallSummary, ToolCompletedData, ToolStartedData,
    TurnCancelledData, TurnCompletedData, TurnFailedData, TurnStartedData, VALID_EVENT_TYPES,
};
pub use harness::{Harness, HarnessStatus};
pub use leased_resource::{
    LEASED_RESOURCES_FEATURE, LeasedResource, LeasedResourceStatus, UpsertLeasedResource,
};
pub use llm_model_profiles::get_model_profile;
pub use llm_models::{
    CostTier, LlmModel, LlmModelCost, LlmModelLimits, LlmModelModalities, LlmModelProfile,
    LlmModelSource, LlmModelStatus, LlmModelWithProvider, LlmProviderStatus, LlmProviderType,
    Modality, ReasoningEffort, ReasoningEffortConfig, ReasoningEffortValue,
};
pub use mcp_server::{
    McpContent, McpError, McpServer, McpServerStatus, McpServerTransportType, McpToolCallParams,
    McpToolCallRequest, McpToolCallResponse, McpToolCallResult, McpToolDefinition,
    McpToolsListRequest, McpToolsListResponse, McpToolsListResult, is_mcp_tool, mcp_tool_name,
    parse_mcp_tool_name,
};
pub use organization::{
    ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID, ANONYMOUS_USER_NAME, DEFAULT_ORG_ID,
    DEFAULT_ORG_PUBLIC_ID, OrgMembership, OrgRole, Organization, generate_org_public_id,
    org_public_id_from_internal, validate_org_public_id,
};
pub use session::{Session, SessionStatus, SubagentStatus};
pub use session_file::{FileInfo, FileStat, GrepMatch, GrepResult, InitialFile, SessionFile};
pub use session_sqldb::{
    ColumnSchema, DatabaseInfo, SessionSqlDbError, SessionSqlDbStore, SqlExecuteResult,
    SqlQueryResult, TableSchema,
};
pub use skill::{
    ParsedSkillMd, Skill, SkillContent, SkillFileEntry, SkillSourceType, SkillStatus,
    SkillValidationResult, parse_skill_md, validate_skill_md, validate_skill_name,
};
pub use typed_id::{
    AgentId, AppId, EventId, ExecId, HarnessId, IdMarker, IdParseError, ImageId, LeasedResourceId,
    McpServerId, MessageId, ModelId, NotificationId, OrgId, ProviderId, ScheduleId, SessionId,
    SkillId, TurnId, TypedId,
};

// Permissions re-exports
pub use permissions::{
    Caller, DefaultPermissionResolver, Permission, PermissionResolver, Policy,
    PolicyConfigResponse, PolicyError, ResourceConfigResponse, Rule, SkillPermissionAction,
    SkillPermissionPattern, SkillPermissionRule, check_skill_permission, evaluate_policies,
    evaluate_policies_with, parse_skill_permission_rule, role_has_permission, role_permissions,
};

// URL validation re-exports
pub use url_validation::{UrlValidationError, validate_safe_url};

// Deployment configuration
pub use deployment::DeploymentGrade;

// Feature flags
pub use feature_flags::FeatureFlags;

// Observation backends
pub use observation::{BraintrustConfig, BraintrustListener, OtelEventListener};
