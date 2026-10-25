//! Outbound rules a remote sandbox provider enforces for one sandbox.
//!
//! A remote sandbox (Daytona, E2B, ...) reaches the network from the
//! provider's infrastructure, so Everruns' egress boundary never sees its
//! traffic. These rules are what the provider is asked to enforce instead.
//!
//! Decisions:
//! - Two inputs narrow each other and neither widens: the sandbox template's
//!   network policy (`provider_config.network`, written by the server in the
//!   shape `{"mode": "open" | "blocked" | "allowlist", "domains": [...]}`)
//!   and the session's merged [`NetworkAccessList`].
//! - Providers filter hosts, not URLs. A URL-prefix rule becomes its host,
//!   which is wider than the rule; tool traffic still goes through the
//!   egress service, which checks the whole URL.
//! - A provider allowlist cannot carry exceptions. With an allowlist, allowed
//!   entries a block covers are dropped. Without one, blocks cannot be
//!   expressed and are returned in `unenforced_blocks` so the caller can say
//!   so instead of dropping them silently.
//! - An allowlist that ends up empty blocks everything: an empty allowlist
//!   must never read as "no restriction".

use std::net::IpAddr;

use serde_json::Value;

use crate::runtime::network_access::{NetworkAccessList, merge_network_access};

/// What a provider must enforce for one sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxEgressRules {
    pub mode: SandboxEgressMode,
    /// Blocked patterns the provider cannot express (no allowlist to remove
    /// them from).
    pub unenforced_blocks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxEgressMode {
    /// No provider-side restriction.
    Open,
    /// No outbound network at all.
    Blocked,
    /// Only these destinations. `domains` use `example.com` and
    /// `*.example.com` (subdomains only); a pattern that also covers the apex
    /// in Everruns' syntax is listed both ways. `cidrs` hold IP ranges.
    Allowlist {
        domains: Vec<String>,
        cidrs: Vec<String>,
    },
}

impl SandboxEgressRules {
    pub fn open() -> Self {
        Self {
            mode: SandboxEgressMode::Open,
            unenforced_blocks: Vec::new(),
        }
    }
}

/// Combine the template network policy and the session access list into the
/// rules a provider enforces. Errors on a template policy it cannot read, so
/// a malformed policy never turns into open egress.
pub fn sandbox_egress_rules(
    template_network: Option<&Value>,
    access: Option<&NetworkAccessList>,
) -> Result<SandboxEgressRules, String> {
    let template = match template_network {
        None | Some(Value::Null) => None,
        Some(network) => match network.get("mode").and_then(Value::as_str) {
            Some("open") => None,
            Some("blocked") => {
                return Ok(SandboxEgressRules {
                    mode: SandboxEgressMode::Blocked,
                    unenforced_blocks: Vec::new(),
                });
            }
            Some("allowlist") => {
                let domains = network
                    .get("domains")
                    .and_then(Value::as_array)
                    .ok_or("network.domains must be a list for mode allowlist")?
                    .iter()
                    .map(|domain| {
                        domain
                            .as_str()
                            .map(str::to_string)
                            .ok_or("network.domains entries must be strings")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                // An empty template allowlist allows nothing.
                Some(if domains.is_empty() {
                    NetworkAccessList::allow_only(["<none>"])
                } else {
                    NetworkAccessList::allow_only(domains)
                })
            }
            other => return Err(format!("unknown network.mode {other:?}")),
        },
    };

    let Some(merged) = merge_network_access(template.as_ref(), access) else {
        return Ok(SandboxEgressRules::open());
    };

    if merged.allowed.is_empty() {
        return Ok(SandboxEgressRules {
            mode: SandboxEgressMode::Open,
            unenforced_blocks: merged.blocked.clone(),
        });
    }

    // The merge already intersected the allowlists; only blocks are left to
    // apply, so probe each allowed host against the blocks alone.
    let blocks = NetworkAccessList {
        allowed: Vec::new(),
        blocked: merged.blocked.clone(),
    };
    let blocked_cidrs: Vec<String> = merged
        .blocked
        .iter()
        .filter_map(|pattern| pattern_host(pattern).and_then(|host| as_cidr(&host)))
        .collect();
    let mut domains = Vec::new();
    let mut cidrs = Vec::new();
    for pattern in &merged.allowed {
        let Some(host) = pattern_host(pattern) else {
            continue;
        };
        if let Some(cidr) = as_cidr(&host) {
            if !blocked_cidrs.contains(&cidr) {
                push_unique(&mut cidrs, cidr);
            }
            continue;
        }
        let apex = host.strip_prefix("*.").unwrap_or(&host);
        if !blocks.is_url_allowed(&probe_url(apex)) {
            continue;
        }
        push_unique(&mut domains, apex.to_string());
        if host.starts_with("*.") {
            push_unique(&mut domains, format!("*.{apex}"));
        }
    }

    let mode = if domains.is_empty() && cidrs.is_empty() {
        SandboxEgressMode::Blocked
    } else {
        SandboxEgressMode::Allowlist { domains, cidrs }
    };
    Ok(SandboxEgressRules {
        mode,
        unenforced_blocks: Vec::new(),
    })
}

/// The host a pattern names, lowercased; `None` for the "allow nothing"
/// sentinel or an unparsable URL prefix (which allows nothing either).
fn pattern_host(pattern: &str) -> Option<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern == "<none>" {
        return None;
    }
    if pattern.contains("://") {
        let url = url::Url::parse(pattern).ok()?;
        return url.host_str().map(|host| {
            host.trim_start_matches('[')
                .trim_end_matches(']')
                .to_lowercase()
        });
    }
    Some(pattern.to_lowercase())
}

fn as_cidr(host: &str) -> Option<String> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(match ip {
            IpAddr::V4(_) => format!("{ip}/32"),
            IpAddr::V6(_) => format!("{ip}/128"),
        });
    }
    let (address, prefix) = host.split_once('/')?;
    let ip = address.parse::<IpAddr>().ok()?;
    let bits: u8 = prefix.parse().ok()?;
    let max = if ip.is_ipv4() { 32 } else { 128 };
    (bits <= max).then(|| format!("{ip}/{bits}"))
}

fn probe_url(host: &str) -> String {
    if host.contains(':') {
        format!("https://[{host}]/")
    } else {
        format!("https://{host}/")
    }
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn allowlist(domains: &[&str], cidrs: &[&str]) -> SandboxEgressMode {
        SandboxEgressMode::Allowlist {
            domains: domains.iter().map(|d| d.to_string()).collect(),
            cidrs: cidrs.iter().map(|c| c.to_string()).collect(),
        }
    }

    #[test]
    fn nothing_set_is_open() {
        assert_eq!(
            sandbox_egress_rules(None, None).unwrap(),
            SandboxEgressRules::open()
        );
        assert_eq!(
            sandbox_egress_rules(Some(&json!({"mode": "open"})), None).unwrap(),
            SandboxEgressRules::open()
        );
    }

    #[test]
    fn blocked_template_wins_over_any_access_list() {
        let access = NetworkAccessList::allow_only(["github.com"]);
        let rules = sandbox_egress_rules(Some(&json!({"mode": "blocked"})), Some(&access)).unwrap();
        assert_eq!(rules.mode, SandboxEgressMode::Blocked);
    }

    #[test]
    fn session_allowlist_narrows_an_open_template() {
        let access = NetworkAccessList::allow_only([
            "*.github.com",
            "https://pypi.org/simple/",
            "10.0.0.0/8",
            "1.1.1.1",
        ]);
        let rules = sandbox_egress_rules(Some(&json!({"mode": "open"})), Some(&access)).unwrap();
        assert_eq!(
            rules.mode,
            allowlist(
                &["github.com", "*.github.com", "pypi.org"],
                &["10.0.0.0/8", "1.1.1.1/32"]
            )
        );
    }

    #[test]
    fn template_and_session_allowlists_intersect() {
        let template = json!({"mode": "allowlist", "domains": ["*.github.com", "pypi.org"]});
        let access = NetworkAccessList::allow_only(["api.github.com", "example.com"]);
        let rules = sandbox_egress_rules(Some(&template), Some(&access)).unwrap();
        assert_eq!(rules.mode, allowlist(&["api.github.com"], &[]));
    }

    #[test]
    fn disjoint_allowlists_block_everything() {
        let template = json!({"mode": "allowlist", "domains": ["pypi.org"]});
        let access = NetworkAccessList::allow_only(["example.com"]);
        let rules = sandbox_egress_rules(Some(&template), Some(&access)).unwrap();
        assert_eq!(rules.mode, SandboxEgressMode::Blocked);

        let empty = json!({"mode": "allowlist", "domains": []});
        assert_eq!(
            sandbox_egress_rules(Some(&empty), None).unwrap().mode,
            SandboxEgressMode::Blocked
        );
    }

    #[test]
    fn blocks_remove_allowed_entries_or_are_reported_unenforced() {
        let access = NetworkAccessList {
            allowed: vec!["github.com".into(), "evil.example".into()],
            blocked: vec!["evil.example".into()],
        };
        let rules = sandbox_egress_rules(None, Some(&access)).unwrap();
        assert_eq!(rules.mode, allowlist(&["github.com"], &[]));
        assert!(rules.unenforced_blocks.is_empty());

        let blocks_only = NetworkAccessList::block(["169.254.169.254"]);
        let rules = sandbox_egress_rules(None, Some(&blocks_only)).unwrap();
        assert_eq!(rules.mode, SandboxEgressMode::Open);
        assert_eq!(rules.unenforced_blocks, ["169.254.169.254"]);
    }

    #[test]
    fn malformed_template_policy_is_an_error_not_open() {
        assert!(sandbox_egress_rules(Some(&json!({"mode": "sometimes"})), None).is_err());
        assert!(sandbox_egress_rules(Some(&json!({"mode": "allowlist"})), None).is_err());
    }
}
