//! Control-plane shapes no single domain owns.
//!
//! Entity records live with their owning domain as `domains/<domain>/record.rs`
//! (Agent in `domains::agents::record`, Session in `domains::sessions::record`,
//! and so on). What stays here is cross-cutting:
//!
//! - `wire`: proto conversions spanning agents, harnesses and sessions.
//! - `capability_schema`: the OpenAPI surrogate for the core capability
//!   reference, embedded by the agent, harness and session records.
//! - `email`: the system email seam; no domain owns outbound mail, and SaaS
//!   overlays its own sender.
//! - `feature_flags`: org feature grades read by every layer.
//! - `model_router`: no model-router domain exists yet.

pub mod capability_schema;
pub mod email;
pub mod feature_flags;
pub mod model_router;
pub mod wire;

#[cfg(test)]
mod wire_tests;

pub use capability_schema::CapabilityRefSchema;
pub use email::{
    BasicEmailTemplate, DisabledEmailSender, EmailAddress, EmailError, EmailMessage, EmailResult,
    EmailSender, EmailTag, EmailTemplate, MinimalEmailTemplate, NoopEmailSender, RenderedEmail,
    SYSTEM_EMAIL_FROM, SentEmail, SystemEmailConfig, system_email_from,
};
pub use feature_flags::{
    API_FEATURE_FLAG_DEFINITIONS, FeatureFlagDefinition, FeatureFlagGrade, FeatureFlagMap,
    FeatureFlagPolicy, FeatureFlags,
};

pub use everruns_contracts::typed_id::AgentChannelId;

// kept for saas; remove after adoption
pub use crate::domains::organizations::record::generate_org_public_id;
// kept for saas; remove after adoption
pub use crate::domains::providers::record::Provider as ProviderRecord;
// kept for saas; remove after adoption
pub use crate::domains::sessions::record::SessionSource;
