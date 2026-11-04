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
//! - When the host routes sandboxes through its tenant egress proxy
//!   (`EVERRUNS_EGRESS_PROXY_SOCKS_ADDRESS`), every sandbox that may reach
//!   the network also gets `network.egressProxy`: E2B filters against
//!   `allowOut`/`denyOut` first, then tunnels what is left over SOCKS5 to the
//!   proxy, which runs the same checks as for tool traffic and counts the
//!   bytes per org and session. The credential goes to E2B's API only, never
//!   into the sandbox, and E2B never returns the password. E2B fails closed
//!   when the proxy is unreachable. A blocked sandbox gets no proxy: nothing
//!   leaves it. UDP, including DNS, is not tunneled and leaves from E2B.
//! - Credentials expire. A session sandbox's lasts its timeout plus an hour
//!   and is replaced on every resume (which `PUT /network` does in place); a
//!   sandbox that outlives it loses egress, it never falls back to direct.

use everruns_contracts::runtime::network_access::NetworkAccessList;
use everruns_contracts::runtime::sandbox_egress::{
    SandboxEgressMode, SandboxEgressRules, sandbox_egress_rules,
};
use everruns_contracts::runtime::{SandboxProxyCredential, ToolContext, ToolExecutionResult};
use everruns_contracts::session_sandbox::{SessionSandboxConfig, SessionSandboxContext};
use serde_json::{Value, json};
use std::time::Duration;
use tracing::warn;

use crate::e2b::client::E2BClient;

const ALL_IPV4: &str = "0.0.0.0/0";
/// How long past the sandbox's own timeout its proxy credential stays valid,
/// so a request in flight at the deadline is not cut by the credential.
const CREDENTIAL_SLACK_SECS: u64 = 3_600;

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

    pub(crate) fn from_rules(
        rules: &SandboxEgressRules,
        proxy: Option<&SandboxProxyCredential>,
    ) -> Self {
        match &rules.mode {
            SandboxEgressMode::Blocked => Self {
                allow_internet_access: false,
                network: None,
            },
            SandboxEgressMode::Open | SandboxEgressMode::Allowlist { .. } => Self {
                allow_internet_access: true,
                network: network_update(rules, proxy),
            },
        }
    }
}

/// The `SandboxNetworkUpdateConfig` for these rules; `None` when open and
/// not routed, the one case that needs nothing beyond the defaults.
fn network_update(
    rules: &SandboxEgressRules,
    proxy: Option<&SandboxProxyCredential>,
) -> Option<Value> {
    let mut network = match &rules.mode {
        // Nothing leaves a blocked sandbox, so it never needs the proxy.
        SandboxEgressMode::Blocked => return Some(json!({ "denyOut": [ALL_IPV4] })),
        SandboxEgressMode::Open => json!({}),
        SandboxEgressMode::Allowlist { domains, cidrs } => json!({
            "allowOut": domains.iter().chain(cidrs).collect::<Vec<_>>(),
            "denyOut": [ALL_IPV4],
        }),
    };
    if let Some(proxy) = proxy {
        network["egressProxy"] = egress_proxy(proxy);
    }
    (network != json!({})).then_some(network)
}

/// `SandboxEgressProxyConfig` for one credential.
fn egress_proxy(proxy: &SandboxProxyCredential) -> Value {
    let mut config = json!({
        "address": proxy.address,
        "username": proxy.username,
        "password": proxy.password,
    });
    if proxy.tls {
        config["tls"] = json!({ "enabled": true });
    }
    config
}

/// The proxy credential for a sandbox that runs for `timeout_seconds`, when
/// the host routes sandboxes. Fails closed when routing is configured but
/// unusable.
pub(crate) fn proxy_credential(
    context: &dyn SessionSandboxContext,
    rules: &SandboxEgressRules,
    timeout_seconds: u64,
) -> Result<Option<SandboxProxyCredential>, ToolExecutionResult> {
    if matches!(rules.mode, SandboxEgressMode::Blocked) {
        return Ok(None);
    }
    context.sandbox_proxy_credential(Duration::from_secs(
        timeout_seconds.saturating_add(CREDENTIAL_SLACK_SECS),
    ))
}

/// The proxy credential for a sandbox a tool creates. `e2b_exec` keeps
/// extending such a sandbox's timeout, so it asks for two days, the most the
/// host mints; a tool sandbox busy for longer loses egress until resumed.
pub(crate) fn tool_proxy_credential(
    context: &ToolContext,
    rules: &SandboxEgressRules,
) -> Result<Option<SandboxProxyCredential>, ToolExecutionResult> {
    if matches!(rules.mode, SandboxEgressMode::Blocked) {
        return Ok(None);
    }
    SessionSandboxContext::sandbox_proxy_credential(context, Duration::from_secs(2 * 86_400))
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

/// Re-apply the rules, and a fresh proxy credential, to a running sandbox, so
/// a resumed sandbox follows the current policy. Open, unrouted rules are
/// left alone.
pub(crate) async fn apply_to_running(
    client: &E2BClient,
    sandbox_id: &str,
    rules: &SandboxEgressRules,
    proxy: Option<&SandboxProxyCredential>,
) -> Result<(), ToolExecutionResult> {
    let Some(network) = network_update(rules, proxy) else {
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
            E2BEgress::from_rules(&SandboxEgressRules::open(), None),
            E2BEgress::open()
        );
        assert_eq!(network_update(&SandboxEgressRules::open(), None), None);
    }

    #[test]
    fn blocked_rules_turn_internet_access_off() {
        let egress = E2BEgress::from_rules(&rules(SandboxEgressMode::Blocked), None);
        assert!(!egress.allow_internet_access);
        assert_eq!(egress.network, None);
        assert_eq!(
            network_update(&rules(SandboxEgressMode::Blocked), None),
            Some(json!({"denyOut": ["0.0.0.0/0"]}))
        );
    }

    #[test]
    fn allowlists_deny_everything_else() {
        let egress = E2BEgress::from_rules(
            &rules(SandboxEgressMode::Allowlist {
                domains: vec!["github.com".into(), "*.github.com".into()],
                cidrs: vec!["10.0.0.0/8".into()],
            }),
            None,
        );
        assert!(egress.allow_internet_access);
        assert_eq!(
            egress.network,
            Some(json!({
                "allowOut": ["github.com", "*.github.com", "10.0.0.0/8"],
                "denyOut": ["0.0.0.0/0"],
            }))
        );
    }

    fn credential(tls: bool) -> SandboxProxyCredential {
        SandboxProxyCredential {
            address: "egress.example.com:1080".into(),
            tls,
            username: "everruns".into(),
            password: "s1.claims.sig".into(),
            expires_at: 1_900_000_000,
        }
    }

    #[test]
    fn routed_sandboxes_tunnel_through_the_proxy() {
        let proxy = credential(true);
        let open = E2BEgress::from_rules(&SandboxEgressRules::open(), Some(&proxy));
        assert!(open.allow_internet_access);
        assert_eq!(
            open.network,
            Some(json!({
                "egressProxy": {
                    "address": "egress.example.com:1080",
                    "username": "everruns",
                    "password": "s1.claims.sig",
                    "tls": {"enabled": true},
                },
            }))
        );

        // An allowlist still filters first; the proxy carries what passes.
        let allowlist = E2BEgress::from_rules(
            &rules(SandboxEgressMode::Allowlist {
                domains: vec!["github.com".into()],
                cidrs: vec![],
            }),
            Some(&credential(false)),
        );
        assert_eq!(
            allowlist.network,
            Some(json!({
                "allowOut": ["github.com"],
                "denyOut": ["0.0.0.0/0"],
                "egressProxy": {
                    "address": "egress.example.com:1080",
                    "username": "everruns",
                    "password": "s1.claims.sig",
                },
            }))
        );

        // Nothing leaves a blocked sandbox, so it gets no credential.
        let blocked = E2BEgress::from_rules(&rules(SandboxEgressMode::Blocked), Some(&proxy));
        assert!(!blocked.allow_internet_access);
        assert_eq!(blocked.network, None);
        assert_eq!(
            network_update(&rules(SandboxEgressMode::Blocked), Some(&proxy)),
            Some(json!({"denyOut": ["0.0.0.0/0"]}))
        );
    }
}
