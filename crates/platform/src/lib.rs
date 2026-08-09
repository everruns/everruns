//! Backend platform entities for Everruns.
//!
//! `everruns-platform` owns the durable backend aggregates and
//! accounting/observability records that were historically defined in
//! [`everruns-core`](https://docs.rs/everruns-core). It was carved out in
//! EVE-837 (identity: [`Organization`], [`Principal`]) and extended in EVE-838
//! (payment/reporting/audit records) to give the backend platform family its
//! own crate without disturbing the runtime-facing contract in `everruns-core`.
//!
//! # Dependency direction
//!
//! The dependency edge is strictly `platform -> core`. `everruns-core` never
//! depends on `everruns-platform`; that invariant is enforced by
//! `crates/core/tests/no_platform_dependency.rs`.
//!
//! # Types that stay in core
//!
//! Cross-cutting value types that core's runtime, permissions layer, and domain
//! models embed remain in `everruns-core` and are re-exported here so consumers
//! can reach the whole surface through `everruns_platform`:
//!
//! - [`OrgRole`] and the `DEFAULT_ORG_*` constants plus the internal<->public id
//!   conversion helper — used by core's permissions/auth layer during a turn.
//! - [`PrincipalKind`], [`PrincipalSummary`] — embedded in
//!   `Session`/`SessionSchedule`/`AgentIdentity`.
//! - The capability-internal payment execution contract (`PaymentRail`,
//!   `PaymentMethod`, `MachinePaymentRequest`, `MachinePaymentResponse`) — bound
//!   to core's `PaymentAuthority` trait and `ToolContext`. Re-exported from
//!   [`payment`].
//!
//! # Identity values owned here (EVE-845)
//!
//! [`OrgMembership`], the `ANONYMOUS_USER_*` seed constants, the org public-id
//! generation/validation helpers ([`generate_org_public_id`],
//! [`validate_org_public_id`]), and the [`PrincipalStatus`] lifecycle enum are
//! defined in this crate: no core code names them, so they moved out of the
//! runtime-facing contract in `everruns-core`.

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

pub use organization::{
    ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID, ANONYMOUS_USER_NAME, OrgMembership, Organization,
    generate_org_public_id, validate_org_public_id,
};
pub use platform_store::{
    PlatformCreateSessionRequest, PlatformMessage, PlatformStore, PlatformStoreExt,
    PlatformStoreSubagentDelegate,
};
pub use principal::{Principal, PrincipalStatus};

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
