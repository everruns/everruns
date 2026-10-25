//! E2B-enforced outbound network rules.
//!
//! E2B sandboxes reach the network from E2B's infrastructure, outside the
//! Everruns egress boundary, so the template network policy and the session
//! access list are pushed to E2B instead: `allowInternetAccess` and
//! `network.allowOut` / `network.denyOut` on create, `PUT
//! /sandboxes/{id}/network` on resume.
//!
//! Decisions:
//! - E2B lets allowed entries win over denied ones, so an allowlist is
//!   `denyOut: ["0.0.0.0/0"]` plus the allowed domains and CIDRs.
//! - Open rules send nothing beyond `allowInternetAccess: true`, the
//!   behavior every sandbox had before.
//! - Restricted rules fail closed: if E2B refuses them, creation or resume
//!   fails rather than running with open egress.

use everruns_contracts::runtime::network_access::NetworkAccessList;
use everruns_contracts::runtime::sandbox_egress::{
    SandboxEgressMode, SandboxEgressRules, sandbox_egress_rules,
};
use everruns_contracts::runtime::{ToolContext, ToolExecutionResult};
use everruns_contracts::session_sandbox::{SessionSandboxConfig, SessionSandboxContext};
use serde_json::{Value, json};
use tracing::warn;

use crate::e2b::client::E2BClient;

const ALL_IPV4: &str = "0.0.0.0/0";

/// What a create request carries for one sandbox's egress.
#[derive(Debug, Clone, PartialEq)]
pub struct E2BEgress {
    pub allow_internet_access: bool,
    /// `SandboxNetworkConfig`, when restricted.
    pub network: Option<Value>,
}

impl E2BEgress {
    pub fn open() -> Self {
        Self {
            allow_internet_access: true,
            network: None,
        }
    }

    pub(crate) fn from_rules(rules: &SandboxEgressRules) -> Self {
        match &rules.mode {
            SandboxEgressMode::Open => Self::open(),
            SandboxEgressMode::Blocked => Self {
                allow_internet_access: false,
                network: None,
            },
            SandboxEgressMode::Allowlist { .. } => Self {
                allow_internet_access: true,
                network: network_update(rules),
            },
        }
    }
}

/// The `SandboxNetworkUpdateConfig` for restricted rules; `None` when open.
fn network_update(rules: &SandboxEgressRules) -> Option<Value> {
    match &rules.mode {
        SandboxEgressMode::Open => None,
        SandboxEgressMode::Blocked => Some(json!({ "denyOut": [ALL_IPV4] })),
        SandboxEgressMode::Allowlist { domains, cidrs } => Some(json!({
            "allowOut": domains.iter().chain(cidrs).collect::<Vec<_>>(),
            "denyOut": [ALL_IPV4],
        })),
    }
}

/// Rules for a session sandbox: the template policy the server wrote to
/// `provider_config.network`, narrowed by the session access list.
pub(crate) fn session_rules(
    context: &dyn SessionSandboxContext,
    config: &SessionSandboxConfig,
) -> Result<SandboxEgressRules, ToolExecutionResult> {
    rules_from(
        config.provider_config.get("network"),
        context.network_access().as_ref(),
    )
}

/// Rules for a sandbox a tool creates directly (`e2b_create_sandbox`, the
/// desktop): no template, only the session access list.
pub(crate) fn tool_rules(context: &ToolContext) -> Result<SandboxEgressRules, ToolExecutionResult> {
    rules_from(None, context.network_access.as_ref())
}

fn rules_from(
    template_network: Option<&Value>,
    access: Option<&NetworkAccessList>,
) -> Result<SandboxEgressRules, ToolExecutionResult> {
    let rules = sandbox_egress_rules(template_network, access)
        .map_err(|error| ToolExecutionResult::tool_error(format!("E2B network policy: {error}")))?;
    if !rules.unenforced_blocks.is_empty() {
        warn!(
            blocks = ?rules.unenforced_blocks,
            "E2B cannot enforce blocked hosts without an allowlist; tool egress still checks them"
        );
    }
    Ok(rules)
}

/// Re-apply restricted rules to a running sandbox, so a resumed sandbox
/// follows the current policy. Open rules are left alone.
pub(crate) async fn apply_to_running(
    client: &E2BClient,
    sandbox_id: &str,
    rules: &SandboxEgressRules,
) -> Result<(), ToolExecutionResult> {
    let Some(network) = network_update(rules) else {
        return Ok(());
    };
    client
        .update_network(sandbox_id, network)
        .await
        .map_err(|error| {
            ToolExecutionResult::tool_error(format!(
                "E2B refused the sandbox network policy: {error}"
            ))
        })
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
    fn open_rules_keep_the_open_create_request() {
        assert_eq!(
            E2BEgress::from_rules(&SandboxEgressRules::open()),
            E2BEgress::open()
        );
        assert_eq!(network_update(&SandboxEgressRules::open()), None);
    }

    #[test]
    fn blocked_rules_turn_internet_access_off() {
        let egress = E2BEgress::from_rules(&rules(SandboxEgressMode::Blocked));
        assert!(!egress.allow_internet_access);
        assert_eq!(egress.network, None);
        assert_eq!(
            network_update(&rules(SandboxEgressMode::Blocked)),
            Some(json!({"denyOut": ["0.0.0.0/0"]}))
        );
    }

    #[test]
    fn allowlists_deny_everything_else() {
        let egress = E2BEgress::from_rules(&rules(SandboxEgressMode::Allowlist {
            domains: vec!["github.com".into(), "*.github.com".into()],
            cidrs: vec!["10.0.0.0/8".into()],
        }));
        assert!(egress.allow_internet_access);
        assert_eq!(
            egress.network,
            Some(json!({
                "allowOut": ["github.com", "*.github.com", "10.0.0.0/8"],
                "denyOut": ["0.0.0.0/0"],
            }))
        );
    }
}
