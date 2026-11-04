//! boxd-enforced outbound network rules.
//!
//! boxd machines reach the internet from boxd's hosts, outside the Everruns
//! egress boundary, so the template network policy and the session access
//! list are pushed to boxd as the machine's egress allowlist: in the machine
//! spec from birth, and with `SetVmEgressAllow` on resume.
//!
//! Decisions:
//! - The mapping is direct: open is an empty list (unrestricted), blocked is
//!   `["none"]`, an allowlist is its domains and IPv4 ranges. boxd follows the
//!   list for DNS too, so a blocked name cannot carry data out in a lookup.
//! - boxd refuses some entries Everruns accepts: IPv6 (machines have no IPv6
//!   egress), private ranges, and wildcards on shared-hosting domains. Those
//!   fail creation with boxd's message rather than being dropped, so a policy
//!   never silently widens or narrows.
//! - boxd offers no outbound proxy hook, so open egress is direct and the
//!   Everruns tenant egress proxy does not see it.

use everruns_contracts::runtime::network_access::NetworkAccessList;
use everruns_contracts::runtime::sandbox_egress::{
    SandboxEgressMode, SandboxEgressRules, sandbox_egress_rules,
};
use everruns_contracts::runtime::{ToolContext, ToolExecutionResult};
use everruns_contracts::session_sandbox::{SessionSandboxConfig, SessionSandboxContext};
use serde_json::Value;
use tracing::warn;

/// boxd's cap on allowlist entries.
pub const MAX_ENTRIES: usize = 256;

/// The allowlist boxd should hold for `rules`.
pub fn entries(rules: &SandboxEgressRules) -> Result<Vec<String>, String> {
    match &rules.mode {
        SandboxEgressMode::Open => Ok(Vec::new()),
        SandboxEgressMode::Blocked => Ok(vec!["none".to_string()]),
        SandboxEgressMode::Allowlist { domains, cidrs } => {
            if let Some(v6) = cidrs.iter().find(|cidr| cidr.contains(':')) {
                return Err(format!(
                    "boxd machines have no IPv6 egress, so '{v6}' cannot be allowed"
                ));
            }
            let mut entries: Vec<String> = Vec::new();
            for entry in domains.iter().chain(cidrs) {
                if !entries.contains(entry) {
                    entries.push(entry.clone());
                }
            }
            if entries.is_empty() {
                return Ok(vec!["none".to_string()]);
            }
            if entries.len() > MAX_ENTRIES {
                return Err(format!(
                    "boxd allows at most {MAX_ENTRIES} egress entries, the policy has {}",
                    entries.len()
                ));
            }
            Ok(entries)
        }
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

/// Rules for a machine a tool creates: only the session access list.
pub(crate) fn tool_rules(context: &ToolContext) -> Result<SandboxEgressRules, ToolExecutionResult> {
    rules_from(None, context.network_access.as_ref())
}

fn rules_from(
    template_network: Option<&Value>,
    access: Option<&NetworkAccessList>,
) -> Result<SandboxEgressRules, ToolExecutionResult> {
    let rules = sandbox_egress_rules(template_network, access).map_err(|error| {
        ToolExecutionResult::tool_error(format!("boxd network policy: {error}"))
    })?;
    if !rules.unenforced_blocks.is_empty() {
        warn!(
            blocks = ?rules.unenforced_blocks,
            "boxd cannot enforce blocked hosts without an allowlist; tool egress still checks them"
        );
    }
    Ok(rules)
}

/// The entries for `rules`, as a tool error when boxd cannot express them.
pub(crate) fn entries_for(rules: &SandboxEgressRules) -> Result<Vec<String>, ToolExecutionResult> {
    entries(rules).map_err(ToolExecutionResult::tool_error)
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
    fn open_is_unrestricted_and_blocked_is_none() {
        assert!(entries(&SandboxEgressRules::open()).unwrap().is_empty());
        assert_eq!(
            entries(&rules(SandboxEgressMode::Blocked)).unwrap(),
            vec!["none"]
        );
    }

    #[test]
    fn allowlists_map_one_to_one_without_duplicates() {
        let listed = entries(&rules(SandboxEgressMode::Allowlist {
            domains: vec![
                "github.com".into(),
                "*.github.com".into(),
                "github.com".into(),
            ],
            cidrs: vec!["203.0.113.0/24".into()],
        }))
        .unwrap();
        assert_eq!(listed, vec!["github.com", "*.github.com", "203.0.113.0/24"]);
    }

    #[test]
    fn what_boxd_cannot_express_fails_instead_of_widening() {
        let v6 = entries(&rules(SandboxEgressMode::Allowlist {
            domains: vec![],
            cidrs: vec!["2001:db8::/32".into()],
        }))
        .unwrap_err();
        assert!(v6.contains("IPv6"), "{v6}");

        let too_many = entries(&rules(SandboxEgressMode::Allowlist {
            domains: (0..=MAX_ENTRIES)
                .map(|i| format!("h{i}.example.com"))
                .collect(),
            cidrs: vec![],
        }))
        .unwrap_err();
        assert!(too_many.contains("at most"), "{too_many}");

        // An allowlist that ends up empty allows nothing, never everything.
        assert_eq!(
            entries(&rules(SandboxEgressMode::Allowlist {
                domains: vec![],
                cidrs: vec![]
            }))
            .unwrap(),
            vec!["none"]
        );
    }
}
