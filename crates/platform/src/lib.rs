//! Backend control-plane entities and store contracts for the
//! [Everruns](https://everruns.com) Platform.
//!
//! This crate owns organization, principal, app/channel, trigger, payment,
//! reporting, audit, and hosted capability implementations used by server and
//! platform backends. Hosted knowledge, memory, delegation, task, user-hook,
//! and management capabilities are composed through
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
    Agent, AgentStatus, AgentVersion, AgentVersionChangeKind, BuiltInAgentDefinition,
    MAX_ADDRESSABLE_NAME_LEN, generate_agent_public_id, validate_addressable_name,
    validate_agent_public_id,
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
    InMemoryVectorStore, KnowledgeIndexCitation, KnowledgeIndexSearch, VectorMatch, VectorQuery,
    VectorRecord, VectorStore, VectorStoreExt, index_namespace,
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
