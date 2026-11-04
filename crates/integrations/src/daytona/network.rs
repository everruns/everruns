//! Daytona-enforced outbound network rules.
//!
//! Daytona sandboxes reach the network from Daytona's runners, outside the
//! Everruns egress boundary, so the template network policy and the session
//! access list are pushed to Daytona instead (`networkBlockAll`,
//! `domainAllowList`, `networkAllowList` on create, `updateNetworkSettings`
//! on resume).
//!
//! Decisions:
//! - Open rules send nothing: Daytona tiers 1 and 2 reject network overrides,
//!   and an unrestricted sandbox must keep working there.
//! - Restricted rules fail closed: if Daytona refuses them, sandbox creation
//!   or resume fails rather than running with open egress.
//! - Verified live (2026-10-25): a domain allowlist blocks other hosts and
//!   direct IPs, `*.example.com` covers subdomains, a CIDR-only allowlist
//!   blocks every domain, and an update applies to the running sandbox.

use everruns_contracts::runtime::network_access::NetworkAccessList;
use everruns_contracts::runtime::sandbox_egress::{
    SandboxEgressMode, SandboxEgressRules, sandbox_egress_rules,
};
use everruns_contracts::runtime::{ToolContext, ToolExecutionResult};
use everruns_contracts::session_sandbox::{SessionSandboxConfig, SessionSandboxContext};
use serde_json::Value;
use tracing::warn;

use crate::daytona::api::management::types::{CreateSandbox, UpdateSandboxNetworkSettings};
use crate::daytona::client::DaytonaClient;

/// Rules for this session's sandbox: the template policy the server wrote to
/// `provider_config.network`, narrowed by the session access list.
pub(crate) fn egress_rules(
    context: &dyn SessionSandboxContext,
    config: &SessionSandboxConfig,
) -> Result<SandboxEgressRules, ToolExecutionResult> {
    rules_from(
        config.provider_config.get("network"),
        context.network_access().as_ref(),
    )
}

/// Rules for a sandbox a tool creates directly (`daytona_create_sandbox`,
/// the desktop): no template, only the session access list.
pub(crate) fn tool_egress_rules(
    context: &ToolContext,
) -> Result<SandboxEgressRules, ToolExecutionResult> {
    rules_from(None, context.network_access.as_ref())
}

/// Apply the [`tool_egress_rules`] to a create body.
pub(crate) fn apply_tool_rules(
    context: &ToolContext,
    body: &mut CreateSandbox,
) -> Result<(), ToolExecutionResult> {
    apply_to_create(body, &tool_egress_rules(context)?);
    Ok(())
}

fn rules_from(
    template_network: Option<&Value>,
    access: Option<&NetworkAccessList>,
) -> Result<SandboxEgressRules, ToolExecutionResult> {
    let rules = sandbox_egress_rules(template_network, access).map_err(|error| {
        ToolExecutionResult::tool_error(format!("Daytona network policy: {error}"))
    })?;
    if !rules.unenforced_blocks.is_empty() {
        warn!(
            blocks = ?rules.unenforced_blocks,
            "Daytona cannot enforce blocked hosts without an allowlist; tool egress still checks them"
        );
    }
    Ok(rules)
}

pub(crate) fn apply_to_create(body: &mut CreateSandbox, rules: &SandboxEgressRules) {
    let settings = settings(rules);
    body.network_block_all = settings.network_block_all;
    body.domain_allow_list = settings.domain_allow_list;
    body.network_allow_list = settings.network_allow_list;
}

/// Re-apply restricted rules to an existing sandbox, so a resumed sandbox
/// follows the current policy. Open rules are left alone (see module docs).
pub(crate) async fn apply_to_running(
    client: &DaytonaClient,
    sandbox_id: &str,
    rules: &SandboxEgressRules,
) -> Result<(), ToolExecutionResult> {
    if rules.mode == SandboxEgressMode::Open {
        return Ok(());
    }
    client
        .update_network_settings(sandbox_id, settings(rules))
        .await
        .map_err(|error| {
            ToolExecutionResult::tool_error(format!(
                "Daytona refused the sandbox network policy: {error}"
            ))
        })
}

fn settings(rules: &SandboxEgressRules) -> UpdateSandboxNetworkSettings {
    let joined = |list: &[String]| (!list.is_empty()).then(|| list.join(","));
    match &rules.mode {
        SandboxEgressMode::Open => UpdateSandboxNetworkSettings::default(),
        SandboxEgressMode::Blocked => UpdateSandboxNetworkSettings {
            network_block_all: Some(true),
            ..Default::default()
        },
        SandboxEgressMode::Allowlist { domains, cidrs } => UpdateSandboxNetworkSettings {
            network_block_all: Some(false),
            domain_allow_list: joined(domains),
            network_allow_list: joined(cidrs),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(mode: SandboxEgressMode) -> SandboxEgressRules {
        SandboxEgressRules {
            mode,
            unenforced_blocks: Vec::new(),
        }
    }

    #[test]
    fn open_rules_leave_the_create_body_untouched() {
        let mut body = CreateSandbox::default();
        apply_to_create(&mut body, &SandboxEgressRules::open());
        assert_eq!(body.network_block_all, None);
        assert_eq!(body.domain_allow_list, None);
        assert_eq!(body.network_allow_list, None);
    }

    #[test]
    fn blocked_rules_block_all() {
        let mut body = CreateSandbox::default();
        apply_to_create(&mut body, &rules(SandboxEgressMode::Blocked));
        assert_eq!(body.network_block_all, Some(true));
        assert_eq!(body.domain_allow_list, None);
    }

    #[test]
    fn allowlists_are_comma_joined() {
        let mut body = CreateSandbox::default();
        apply_to_create(
            &mut body,
            &rules(SandboxEgressMode::Allowlist {
                domains: vec!["github.com".into(), "*.github.com".into()],
                cidrs: vec!["10.0.0.0/8".into()],
            }),
        );
        assert_eq!(body.network_block_all, Some(false));
        assert_eq!(
            body.domain_allow_list.as_deref(),
            Some("github.com,*.github.com")
        );
        assert_eq!(body.network_allow_list.as_deref(), Some("10.0.0.0/8"));
    }
}
