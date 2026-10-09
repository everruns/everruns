//! Control-plane records and their server-owned service contracts.

pub mod agent;
pub mod agent_channel;
pub mod agent_script;
pub mod agent_trigger;
pub mod app;
pub mod audit;
pub mod budget;
pub mod capability_schema;
pub mod email;
pub mod eval;
pub mod exposure;
pub mod feature_flags;
pub mod harness;
pub mod memory;
pub mod model;
pub mod observer;
pub mod organization;
pub mod pact_delegation;
pub mod payment;
pub mod principal;
pub mod provider;
pub mod reporting;
pub mod sandbox_template;
pub mod session;
pub mod slack_channel;
pub mod slack_provisioning;
pub mod workspace;

pub use agent::{
    Agent, AgentAvatar, AgentStatus, MAX_ADDRESSABLE_NAME_LEN, generate_agent_public_id,
    validate_addressable_name, validate_agent_public_id,
};
pub use agent_channel::{
    A2aChannelConfig, AGENTID_ISSUER, AgUiChannelConfig, AgentChannel, ApiChannelConfig,
    CaptchaProvider, ChannelAuthConfig, ChannelAuthMode, ChannelAuthProviderConfig,
    ChannelAuthRequirements, ChannelStatus, ChannelType, FcpChannelConfig, PublicChatBranding,
    PublicChatCaptchaConfig, PublicChatChannelConfig, SlackReplyMode,
};
pub use agent_script::AgentScript;
pub use agent_trigger::{
    AgentTrigger, AgentTriggerDelivery, AgentTriggerType, GitHubTriggerConfig,
    McpEventTriggerConfig, ScheduleTriggerConfig, TriggerDeliveryStatus, TriggerEventFilter,
    TriggerFilterCondition, WebhookTriggerConfig,
};
pub use app::{App, AppStatus};
pub use audit::{
    AgentAction, AuditAction, AuditDomain, AuditEvent, AuditEventBuilder, AuditLogger, AuditTarget,
    HasAuditTargetId, ManagementAction,
};
pub use budget::{Budget, LedgerEntry};
pub use capability_schema::CapabilityRefSchema;
pub use email::{
    BasicEmailTemplate, DisabledEmailSender, EmailAddress, EmailError, EmailMessage, EmailResult,
    EmailSender, EmailTag, EmailTemplate, MinimalEmailTemplate, NoopEmailSender, RenderedEmail,
    SYSTEM_EMAIL_FROM, SentEmail, SystemEmailConfig, system_email_from,
};
pub use eval::{
    ArtifactSpec, CaseResultStatus, Eval, EvalCase, EvalCaseResult, EvalDatasetStatus,
    EvalInputMessage, EvalRun, EvalRunDataset, EvalRunSource, EvalRunStatus, EvalRunSummaryView,
    EvalStatus, EvalTarget, RunSummary, Score, Scorer,
};
pub use exposure::PublicToolVisibility;
pub use feature_flags::{
    API_FEATURE_FLAG_DEFINITIONS, FeatureFlagDefinition, FeatureFlagGrade, FeatureFlagMap,
    FeatureFlagPolicy, FeatureFlags,
};
pub use harness::{
    BuiltInCapabilityDefinition, BuiltInHarnessDefinition, BuiltInHarnessRole, ConversationStarter,
    Harness, HarnessStatus, harness_for_role, merge_harness, merge_harness_chain,
    resolve_execution_harness,
};
pub use memory::{
    Memory, MemoryConfig, MemoryFile, MemoryMountAccess, MemoryMountConfig, MemoryScope,
    MemoryStatus, validate_memory_config, validate_mount_config_shape,
};
pub use observer::{
    LlmJudgeConfig, Observer, ObserverMatch, ObserverScope, ObserverScorerConfig, ObserverStatus,
    ScorerMethod, TraceScore, TraceScoreStatus,
};
pub use organization::{
    ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID, ANONYMOUS_USER_NAME, OrgMembership, Organization,
    generate_org_public_id, validate_org_public_id,
};
pub use payment::{PaymentAccount, PaymentAttempt, PaymentOwnerType, PaymentPolicy, PaymentStatus};
pub use principal::{Principal, PrincipalStatus};
pub use session::{
    Session, SessionActivity, SessionParticipant, SessionParticipantKind, SessionParticipantRole,
    SessionSource, SessionStatus,
};
pub use slack_channel::{SlackChannelConfig, SlackResponsePolicy};

pub use model::{Model, ModelSource, ModelWithProvider};
pub use provider::{Provider as ProviderRecord, ProviderStatus};
#[cfg(test)]
mod channel_wire_names_tests;

pub use everruns_contracts::typed_id::AgentChannelId;
pub use everruns_core::channel::SessionBinding;
pub use everruns_core::{
    DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole, PrincipalKind, PrincipalSummary,
    org_public_id_from_internal,
};

pub mod wire;

#[cfg(test)]
mod wire_tests;

pub use sandbox_template::*;
pub mod mcp_server;
pub mod model_router;
pub mod skill;
pub mod virtual_user;
pub use mcp_server::{McpServer, McpServerStatus};
pub use skill::{Skill, SkillSourceType, SkillStatus, SkillUsage};
pub use virtual_user::{VirtualUser, VirtualUserStatus, VirtualUserUsage};
