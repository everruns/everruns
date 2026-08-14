// Agent domain — commands, queries, types.
//
// See knowledge/foundations/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod analysis;
pub mod check_rules;
pub mod checks;
pub mod commands;
pub mod credentials;
pub mod health_check;
pub mod queries;
pub mod types;

pub use commands::*;
pub use health_check::{AgentHealthCheckService, HealthCheckRunContext};

/// Reduce utility-provider failures to the shared actionable error taxonomy.
/// Raw provider text is for server logs only: it can contain request context or
/// credential-adjacent details and must never reach agent-check API responses.
pub(crate) fn safe_agent_check_error(
    error: &str,
) -> everruns_provider::user_facing_error::UserFacingError {
    everruns_provider::user_facing_error::classify_runtime_error_message(
        error,
        &everruns_provider::user_facing_error::UserFacingErrorContext::default(),
    )
}

/// Policy: View agents (read-only).
pub const AGENT_VIEW: Policy = Policy {
    id: "agent.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

/// Policy: Manage agents (create, update, copy, delete).
pub const AGENT_MANAGE: Policy = Policy {
    id: "agent.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

/// Policy: Run agent health checks.
///
/// Health checks create real sessions and messages in the background, so require
/// the same session-execution boundary as eval runs.
pub const AGENT_HEALTH_CHECK_RUN: Policy = Policy {
    id: "agent.health_check.run",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgSessionsManage),
    ],
};

pub const AGENT_DANGEROUS: Policy = Policy {
    id: "agent.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgAgentsDangerous),
    ],
};
