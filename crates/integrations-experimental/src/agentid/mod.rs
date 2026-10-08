#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! AgentID sign-in for Everruns agents: let an agent finish another app's
//! AgentID waiting page with its own AgentMail inbox.
//!
//! An app that supports AgentID shows a waiting page with a short-lived
//! `auth_token`. The agent hands that token to [`AgentIdCapability`]'s
//! `agentid_authorize` tool, which asks AgentMail to approve the sign-in for
//! the agent's inbox (`POST /v0/inboxes/{inbox_id}/authorize`). The app then
//! sees the agent signed in under its AgentID.
//!
//! The AgentMail key and inbox belong to the agent, not to whoever is talking
//! to it: the tool reads only the agent's service-account `agentmail`
//! connection and never falls back to the invoking user's connections or to a
//! management user's.
//!
//! # Example
//!
//! ```
//! use everruns_contracts::runtime::capabilities::Capability;
//! use everruns_integrations_experimental::agentid::AgentIdCapability;
//!
//! let capability = AgentIdCapability;
//! assert_eq!(capability.id(), "agentid");
//! ```

pub mod connection;
mod tools;

use everruns_contracts::connector::ConnectorPlugin;
use everruns_contracts::runtime::capabilities::{
    Capability, CapabilityStatus, IntegrationPlugin, RiskLevel,
};
use everruns_contracts::runtime::tools::Tool;

pub use connection::AgentMailConnector;
pub use tools::AgentIdAuthorizeTool;

/// Connection provider id for the agent's AgentMail key and inbox.
pub const AGENTMAIL_PROVIDER: &str = "agentmail";

/// AgentMail API base. Fixed: the tool never sends the key anywhere else.
const AGENTMAIL_API_BASE: &str = "https://api.agentmail.to/v0";

/// Feature flags this module's plugins name, with their default rollout grades;
/// the hosted platform lists them in its feature flag settings, and
/// `FEATURE_<NAME>` overrides the grade per deployment.
pub const FEATURE_FLAGS: &[everruns_contracts::runtime::FeatureFlagDefinition] =
    &[everruns_contracts::runtime::FeatureFlagDefinition {
        name: "agentid",
        label: "AgentID sign-in",
        description: "Let agents sign in to apps that support AgentID, with an AgentMail inbox \
                      connected to the agent's service account.",
        grade: everruns_contracts::runtime::FeatureFlagGrade::Adoption,
    }];

/// Capability plugins this module contributes to a hosted catalog.
pub const CAPABILITY_PLUGINS: &[IntegrationPlugin] = &[IntegrationPlugin {
    feature_flag: Some("agentid"),
    factory: || Box::new(AgentIdCapability),
}];

/// Connector plugins this module contributes to a hosted catalog.
pub const CONNECTOR_PLUGINS: &[ConnectorPlugin] = &[ConnectorPlugin {
    feature_flag: Some("agentid"),
    factory: || Box::new(AgentMailConnector),
}];

const SYSTEM_PROMPT: &str = "When an app asks you to sign in with AgentID and shows a waiting page \
with an auth token, call `agentid_authorize` with that token. It approves the sign-in for your own \
AgentMail inbox; the app then sees you signed in. Only pass a token the app itself gave you.";

/// Lets an agent approve AgentID sign-ins with its own AgentMail inbox.
pub struct AgentIdCapability;

impl Capability for AgentIdCapability {
    fn id(&self) -> &str {
        "agentid"
    }

    fn name(&self) -> &str {
        "[Experimental] AgentID sign-in"
    }

    fn description(&self) -> &str {
        "Sign in to apps that support AgentID as this agent, using the AgentMail inbox \
         connected to the agent's service account. EXPERIMENTAL: This capability may change."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Medium
    }

    fn icon(&self) -> Option<&str> {
        Some("agentid")
    }

    fn category(&self) -> Option<&str> {
        Some("Integrations")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(AgentIdAuthorizeTool)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capability_exposes_only_the_authorize_tool() {
        let cap = AgentIdCapability;
        assert_eq!(cap.id(), "agentid");
        let names: Vec<String> = cap.tools().iter().map(|t| t.name().to_string()).collect();
        assert_eq!(names, vec!["agentid_authorize"]);
        assert!(cap.tools()[0].requires_context());
    }

    #[test]
    fn the_capability_and_connector_share_the_flag() {
        assert_eq!(CAPABILITY_PLUGINS[0].feature_flag, Some("agentid"));
        assert_eq!(CONNECTOR_PLUGINS[0].feature_flag, Some("agentid"));
        assert_eq!(FEATURE_FLAGS[0].name, "agentid");
    }
}
