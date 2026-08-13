//! Backend control-plane entities and store contracts for the
//! [Everruns](https://everruns.com) Platform.
//!
//! This crate owns organization, principal, app/channel, trigger, payment,
//! reporting, audit, and hosted capability implementations used by server and
//! platform backends. Hosted knowledge, memory, delegation, task, user-hook,
//! and management capabilities, plus the portable and environment catalogs
//! selected by the hosted product, are composed through
//! [`capabilities::hosted_capability_registry`]. Normal Framework applications
//! use `everruns`; its default registry does not advertise these product-owned
//! capabilities.
//!
//! # Example
//!
//! ```
//! use everruns_platform::{Organization, Principal};
//!
//! fn accepts_platform_values(_organization: &Organization, _principal: &Principal) {}
//! # let _ = accepts_platform_values;
//! ```
//!
//! # Layer boundary
//!
//! Backend/API-only records and service-backed capability implementations live
//! here. Cross-cutting identity and payment values needed during a turn remain
//! in `everruns-core` and are re-exported where a unified platform-facing import
//! is useful. The dependency direction remains `platform -> core`.

pub mod audit;
pub mod organization;
pub mod payment;
pub mod principal;
pub mod reporting;

// Hosted management seam and capabilities carved out of `everruns-core` (EVE-839).
pub mod capabilities;
pub mod knowledge_store;
pub mod memory;
pub mod platform_store;
pub mod vector_store;

// Hosted control-plane orchestration records carved out of `everruns-core`
// (EVE-841). `App`/`AppChannel` and their channel configs, plus `AgentTrigger`,
// are persisted/API records not consumed during a turn. Turn-consumed neutral
// values (`DeploymentGrade`, `SessionSchedule` and its store) stay in core.
pub mod agent_trigger;
pub mod app;

// Stored Agent/AgentVersion persistence records carved out of `everruns-core`
// (EVE-877). Execution consumes only `everruns_core::AgentDefinition`, produced
// by `Agent::execution_definition` at the platform loading seam.
pub mod agent;

// Stored Harness persistence records and built-in provisioning templates
// carved out of `everruns-core` (EVE-881). Execution consumes only
// `everruns_core::HarnessDefinition`, produced by `Harness::execution_definition`
// after `merge_harness_chain` resolves parent inheritance at the platform
// loading seam.
pub mod harness;

// Stored Session persistence record and product lifecycle enums carved out of
// `everruns-core` (EVE-882). Execution consumes only the portable
// `everruns_core::ExecutionSession`, produced by `Session::execution_session`
// at the platform loading seam; the neutral `SessionExecutionState` maps
// to/from the stored `SessionStatus` at the adapter boundary.
pub mod session;

// Org-scoped Workspace record carved out of `everruns-core` (EVE-880). A turn
// resolves workspace *paths* through core's roots/policy types; the stored
// working-area row with its lifecycle status is control-plane state.
pub mod workspace;

// Background tool runs carved out of `everruns-core` (EVE-888): the
// `spawn_background` tool, its session-task mirroring and schedule path, the
// event sink, admission-control permits and the reattach entry point. The
// kernel keeps the neutral `BackgroundExecutableTool`/`BackgroundEventSink`
// contracts; creating session tasks and schedules is hosted behaviour.
pub mod background_run;

// Session metadata mutation carved out of `everruns-core` (EVE-897). The
// kernel never mutated a session; it only carried the trait so a hosted
// capability could reach it. That capability resolves the store as a typed
// extension now.
pub mod session_mutator;

// Session SQL database store carved out of `everruns-core` (EVE-897). The
// value types are the signature vocabulary of `SessionSqlDbStore`, so records
// and trait moved together once the kernel stopped naming the store on
// `ToolContext`; the capability resolves it as a typed extension.
pub mod session_sqldb;

// Managed per-session sandbox carved out of `everruns-core` (EVE-880): the
// persisted state record, the provider SPI integration crates register through
// inventory, and the create/resume/pause/init lifecycle helpers. Execution
// reaches a sandbox only through the `session_sandbox` capability, which lives
// beside this module.
pub mod session_sandbox;

// Management/reporting aggregates carved out of `everruns-core` (EVE-878):
// persisted eval definitions/runs/results/datasets, observer records with
// judge configuration and trace-score lifecycle, and the org/product
// feature-flag records with their management logic. None of these participate
// directly in a turn; execution keeps only the resolved decisions in
// `everruns_core::execution_features`.
pub mod eval;
pub mod feature_flags;
pub mod observer;

// Connector catalog (user-scoped API key / OAuth connections) and the system
// email contract with its concrete senders, carved out of `everruns-core`
// (EVE-879). Both are hosted control-plane services: the server renders
// connector form schemas and resolves connections, and email delivery is a
// product/ops side effect. Nothing consumes them during a turn.
pub mod connector;
pub mod email;

pub use agent::{
    Agent, AgentStatus, AgentVersion, AgentVersionChangeKind, MAX_ADDRESSABLE_NAME_LEN,
    generate_agent_public_id, validate_addressable_name, validate_agent_public_id,
};
pub use harness::{
    BuiltInCapabilityDefinition, BuiltInHarnessDefinition, BuiltInHarnessRole, Harness,
    HarnessStatus, harness_for_role, merge_harness, merge_harness_chain, resolve_execution_harness,
};
pub use knowledge_store::{
    KnowledgeIndexSearchExt, KnowledgeIndexSearchHit, KnowledgeSearchHit, KnowledgeStore,
    KnowledgeStoreExt,
};
pub use memory::{
    Memory, MemoryConfig, MemoryFile, MemoryMountAccess, MemoryMountConfig, MemoryScope,
    MemoryStatus, validate_memory_config, validate_mount_config_shape,
};
pub use organization::{
    ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID, ANONYMOUS_USER_NAME, OrgMembership, Organization,
    generate_org_public_id, validate_org_public_id,
};
pub use platform_store::{
    PlatformCreateSessionRequest, PlatformMessage, PlatformStore, PlatformStoreExt,
    PlatformStoreSubagentDelegate,
};
pub use principal::{Principal, PrincipalStatus};
pub use session::{
    Session, SessionActivity, SessionParticipant, SessionParticipantKind, SessionParticipantRole,
    SessionSource, SessionStatus,
};

// Session metadata mutation (EVE-897).
pub use session_mutator::{SessionMutator, SessionMutatorExt};

// Session SQL database (EVE-897).
pub use session_sqldb::{
    ColumnSchema, DatabaseInfo, SessionSqlDbError, SessionSqlDbStore, SessionSqlDbStoreExt,
    SqlExecuteResult, SqlQueryResult, TableSchema,
};

// Managed per-session sandbox (EVE-880).
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

// Management/reporting aggregates (EVE-878).
pub use eval::{
    ArtifactSpec, CaseResultStatus, Eval, EvalCase, EvalCaseResult, EvalDatasetStatus,
    EvalInputMessage, EvalRun, EvalRunDataset, EvalRunSource, EvalRunStatus, EvalRunSummaryView,
    EvalStatus, EvalTarget, RunSummary, Score, Scorer,
};
pub use feature_flags::{
    API_FEATURE_FLAG_DEFINITIONS, FeatureFlagDefinition, FeatureFlagMap, FeatureFlags,
};
pub use observer::{
    LlmJudgeConfig, Observer, ObserverMatch, ObserverScope, ObserverScorerConfig, ObserverStatus,
    ScorerMethod, TraceScore, TraceScoreStatus,
};
pub use vector_store::{
    EmbeddingCallUsage, InMemoryVectorStore, KnowledgeIndexCitation, KnowledgeIndexSearch,
    KnowledgeIndexSearchOutcome, VectorMatch, VectorQuery, VectorRecord, VectorStore,
    VectorStoreExt, index_namespace,
};

// Hosted control-plane orchestration records (EVE-841).
pub use agent_trigger::{AgentTrigger, AgentTriggerType, ScheduleTriggerConfig};
pub use app::{
    A2aChannelConfig, AgUiChannelConfig, AgUiToolVisibility, AgentVersionPolicy,
    ApiEndpointChannelConfig, App, AppChannel, AppEndpointAuthConfig, AppEndpointAuthMode,
    AppEndpointAuthProviderConfig, AppEndpointAuthRequirements, AppStatus, CaptchaProvider,
    ChannelType, FcpChannelConfig, InvocationSessionMode, PublicChatBranding,
    PublicChatCaptchaConfig, PublicChatChannelConfig, SessionStrategy, SlackChannelConfig,
    SlackReplyMode,
};

// Payment accounting records (EVE-838). The execution-contract types
// (PaymentRail/PaymentMethod/MachinePaymentRequest/MachinePaymentResponse) stay
// in core and are re-exported through `payment`.
pub use payment::{PaymentAccount, PaymentAttempt, PaymentOwnerType, PaymentPolicy, PaymentStatus};

// Audit logging records and traits (EVE-838).
pub use audit::{
    AgentAction, AuditAction, AuditDomain, AuditEvent, AuditEventBuilder, AuditLogger, AuditTarget,
    HasAuditTargetId, ManagementAction,
};

// Re-export the identity value types and multitenancy constants that remain in
// `everruns-core`, so `everruns_platform` exposes the complete identity surface.
pub use everruns_core::{
    DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole, PrincipalKind, PrincipalSummary,
    org_public_id_from_internal,
};

// Connector plugin system re-exports (EVE-879).
pub use connector::{
    Connector, ConnectorFormSchema, ConnectorPlugin, ConnectorRegistry, ConnectorRegistryBuilder,
    ConnectorType, ConnectorValidation, FieldType, FormField,
};

// System email re-exports (EVE-879).
pub use email::{
    BasicEmailTemplate, DisabledEmailSender, EmailAddress, EmailError, EmailMessage, EmailResult,
    EmailSender, EmailTag, EmailTemplate, MinimalEmailTemplate, NoopEmailSender, RenderedEmail,
    ResendEmailConfig, ResendEmailSender, SYSTEM_EMAIL_FROM, SentEmail, SystemEmailConfig,
    system_email_from,
};
