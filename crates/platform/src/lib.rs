//! Backend control-plane entities and store contracts for the Everruns Platform.
//!
//! This crate owns organization, principal, app/channel, trigger, payment,
//! reporting, audit, and hosted-management values used by server and platform
//! backends. It is part of the [Everruns](https://everruns.com) ecosystem;
//! normal Framework applications use `everruns` rather than platform records.
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
//! Backend/API-only records live here. Cross-cutting identity and payment
//! values needed during a turn remain in `everruns-core` and are re-exported
//! where a unified platform-facing import is useful. The dependency direction
//! remains `platform -> core`.

pub mod audit;
pub mod organization;
pub mod payment;
pub mod principal;
pub mod reporting;

// Hosted management seam and capabilities carved out of `everruns-core` (EVE-839).
pub mod capabilities;
pub mod platform_store;

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

pub use agent::{
    Agent, AgentStatus, AgentVersion, AgentVersionChangeKind, MAX_ADDRESSABLE_NAME_LEN,
    generate_agent_public_id, validate_addressable_name, validate_agent_public_id,
};
pub use harness::{
    BuiltInCapabilityDefinition, BuiltInHarnessDefinition, BuiltInHarnessRole, Harness,
    HarnessStatus, harness_for_role, merge_harness, merge_harness_chain, resolve_execution_harness,
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
