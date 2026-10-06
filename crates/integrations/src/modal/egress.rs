//! Outbound network rules for Modal sandboxes: allowlists Modal enforces, and
//! connection credentials Modal injects into outbound HTTPS requests.
//!
//! Decisions:
//! - Keyless egress: a connection's token goes into a Modal Secret and Modal
//!   adds it as a header to matching requests outside the sandbox
//!   (`OutboundPolicy`). The agent can use the API but never sees the token.
//! - The domains a token may be sent to are fixed here per connection, never
//!   taken from the caller. A caller-chosen domain would let anyone who can
//!   edit a template or prompt the agent send a user's token to their own host.
//! - Modal does not combine an outbound policy with a blocked network or a
//!   domain allowlist, so injection requires open egress or a CIDR-only list.
//! - The Secret is anonymous and owned by the shared app, so whoever creates
//!   it deletes it when the sandbox ends; its ID rides in the cleanup lease.

use std::collections::HashMap;
use std::net::IpAddr;

use base64::Engine as _;
use everruns_contracts::session_sandbox::SessionSandboxContext;
use everruns_contracts::tools::ToolExecutionResult;
use serde_json::Value;
use tracing::warn;

use super::client::{HeaderReplacement, ModalClient, NetworkAccess};

/// Connections whose tokens Modal can inject, and where.
pub const INJECTABLE_CONNECTIONS: &[&str] = &["github"];

const MAX_ALLOWLIST_ENTRIES: usize = 128;

/// Secret values (`KEY`, value) and the header rules that reference them.
type ConnectionRules = (Vec<(String, String)>, Vec<HeaderReplacement>);

/// Header rules and secret values for one connection token.
fn connection_rules(connection: &str, token: &str) -> Option<ConnectionRules> {
    match connection {
        // REST API takes a bearer token; git over HTTPS takes basic auth with
        // the token as the password.
        "github" => {
            let basic =
                base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
            Some((
                vec![
                    ("GITHUB_BEARER".to_string(), token.to_string()),
                    ("GITHUB_BASIC".to_string(), basic),
                ],
                vec![
                    header("api.github.com", "Authorization", "Bearer $GITHUB_BEARER"),
                    header("github.com", "Authorization", "Basic $GITHUB_BASIC"),
                ],
            ))
        }
        _ => None,
    }
}

fn header(domain: &str, name: &str, value: &str) -> HeaderReplacement {
    HeaderReplacement {
        domain: domain.to_string(),
        secret_id: None,
        headers: vec![(name.to_string(), value.to_string())],
    }
}

/// Validated outbound rules for one sandbox.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EgressSpec {
    /// `None` keeps Modal's default, open egress.
    pub network: Option<NetworkAccess>,
    /// Connections whose tokens Modal injects.
    pub inject_connections: Vec<String>,
}

impl EgressSpec {
    /// Check the combination Modal accepts and the connection names.
    pub fn validate(&self) -> Result<(), String> {
        for connection in &self.inject_connections {
            if !INJECTABLE_CONNECTIONS.contains(&connection.as_str()) {
                return Err(format!(
                    "connection '{connection}' cannot be injected; supported: {}",
                    INJECTABLE_CONNECTIONS.join(", ")
                ));
            }
        }
        match &self.network {
            Some(NetworkAccess::Allowlist { domains, cidrs }) => {
                if domains.is_empty() && cidrs.is_empty() {
                    return Err("a network allowlist needs at least one domain or CIDR".into());
                }
                if domains.len() + cidrs.len() > MAX_ALLOWLIST_ENTRIES {
                    return Err(format!(
                        "a network allowlist may hold at most {MAX_ALLOWLIST_ENTRIES} entries"
                    ));
                }
                if let Some(bad) = domains.iter().find(|d| !is_valid_domain(d)) {
                    return Err(format!("'{bad}' is not a domain (use '*.' for subdomains)"));
                }
                if let Some(bad) = cidrs.iter().find(|c| !is_valid_cidr(c)) {
                    return Err(format!("'{bad}' is not a CIDR block"));
                }
                if !self.inject_connections.is_empty() && !domains.is_empty() {
                    return Err(
                        "Modal cannot inject credentials when egress is limited to domains; use CIDRs or open egress"
                            .into(),
                    );
                }
            }
            Some(NetworkAccess::Blocked) if !self.inject_connections.is_empty() => {
                return Err("credential injection needs network access".into());
            }
            _ => {}
        }
        Ok(())
    }

    /// Read the `network` and `inject_connections` keys of a JSON object.
    ///
    /// `network` is `{"mode": "open" | "blocked" | "allowlist", "domains": [..],
    /// "cidrs": [..]}`.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let network = match value.get("network").filter(|v| !v.is_null()) {
            None => None,
            Some(network) => Some(match network.get("mode").and_then(Value::as_str) {
                Some("open") => NetworkAccess::Open,
                Some("blocked") => NetworkAccess::Blocked,
                Some("allowlist") => NetworkAccess::Allowlist {
                    domains: string_list(network, "domains")?,
                    cidrs: string_list(network, "cidrs")?,
                },
                _ => return Err("network.mode must be open, blocked, or allowlist".into()),
            }),
        };
        let inject_connections = string_list(value, "inject_connections")?;
        let spec = Self {
            network,
            inject_connections,
        };
        spec.validate()?;
        Ok(spec)
    }

    /// Resolve tokens, store them in a Modal Secret, and return its ID with
    /// the header rules that reference it. `(None, [])` when nothing is injected.
    pub async fn prepare(
        &self,
        client: &ModalClient,
        app_id: &str,
        context: &dyn SessionSandboxContext,
    ) -> Result<(Option<String>, Vec<HeaderReplacement>), ToolExecutionResult> {
        if self.inject_connections.is_empty() {
            return Ok((None, Vec::new()));
        }
        let mut env = HashMap::new();
        let mut replacements = Vec::new();
        for connection in &self.inject_connections {
            let token = match context.connection_token(connection).await? {
                Some(token) if !token.trim().is_empty() => token,
                // THREAT[TM-AGENT-016]: never ask for credentials in chat.
                _ => return Err(ToolExecutionResult::connection_required(connection.clone())),
            };
            let Some((values, rules)) = connection_rules(connection, token.trim()) else {
                return Err(ToolExecutionResult::tool_error(format!(
                    "connection '{connection}' cannot be injected"
                )));
            };
            env.extend(values);
            replacements.extend(rules);
        }
        let secret_id = client
            .create_secret(app_id, &env)
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        for rule in &mut replacements {
            rule.secret_id = Some(secret_id.clone());
        }
        Ok((Some(secret_id), replacements))
    }
}

/// Delete the egress Secret, if any; a missing one is already gone.
pub async fn delete_egress_secret(client: &ModalClient, secret_id: Option<&str>) {
    let Some(secret_id) = secret_id else {
        return;
    };
    if let Err(err) = client.delete_secret(secret_id).await
        && !err.contains("not found")
    {
        warn!(secret_id, error = %err, "Failed to delete Modal egress secret");
    }
}

fn string_list(value: &Value, key: &str) -> Result<Vec<String>, String> {
    match value.get(key).filter(|v| !v.is_null()) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    Some(s) if !out.iter().any(|o| o == s) => out.push(s.to_string()),
                    Some(_) => {}
                    None => return Err(format!("'{key}' must be a list of strings")),
                }
            }
            Ok(out)
        }
        Some(_) => Err(format!("'{key}' must be a list of strings")),
    }
}

/// A hostname, optionally with a leading `*.`, as Modal's clients accept.
pub fn is_valid_domain(domain: &str) -> bool {
    let host = domain.strip_prefix("*.").unwrap_or(domain);
    let labels: Vec<&str> = host.split('.').collect();
    host.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// An IPv4 or IPv6 network in CIDR notation.
pub fn is_valid_cidr(cidr: &str) -> bool {
    let Some((addr, prefix)) = cidr.split_once('/') else {
        return false;
    };
    let Ok(addr) = addr.parse::<IpAddr>() else {
        return false;
    };
    let max = if addr.is_ipv4() { 32 } else { 128 };
    prefix.parse::<u8>().is_ok_and(|p| p <= max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn domains_and_cidrs_are_checked() {
        assert!(is_valid_domain("api.github.com"));
        assert!(is_valid_domain("*.pypi.org"));
        for bad in [
            "*",
            "localhost",
            "a..b",
            "-a.com",
            "https://x.com",
            "x.com/p",
            "a.*.com",
        ] {
            assert!(!is_valid_domain(bad), "{bad}");
        }
        assert!(is_valid_cidr("10.0.0.0/8"));
        assert!(is_valid_cidr("2001:db8::/32"));
        for bad in ["10.0.0.0", "10.0.0.0/33", "x/8", "10.0.0.0/-1"] {
            assert!(!is_valid_cidr(bad), "{bad}");
        }
    }

    #[test]
    fn parses_network_modes() {
        let spec = EgressSpec::from_json(&json!({
            "network": {"mode": "allowlist", "domains": ["pypi.org", "pypi.org"], "cidrs": ["10.0.0.0/8"]}
        }))
        .unwrap();
        assert_eq!(
            spec.network,
            Some(NetworkAccess::Allowlist {
                domains: vec!["pypi.org".into()],
                cidrs: vec!["10.0.0.0/8".into()],
            })
        );
        assert_eq!(
            EgressSpec::from_json(&json!({})).unwrap(),
            EgressSpec::default()
        );
        assert_eq!(
            EgressSpec::from_json(&json!({"network": {"mode": "blocked"}}))
                .unwrap()
                .network,
            Some(NetworkAccess::Blocked)
        );
    }

    #[test]
    fn rejects_what_modal_cannot_enforce() {
        for (bad, expected) in [
            (json!({"network": {"mode": "closed"}}), "network.mode"),
            (json!({"network": {"mode": "allowlist"}}), "at least one"),
            (
                json!({"network": {"mode": "allowlist", "domains": ["*"]}}),
                "not a domain",
            ),
            (
                json!({"network": {"mode": "allowlist", "cidrs": ["1.2.3.4"]}}),
                "CIDR",
            ),
            (
                json!({"inject_connections": ["openai"]}),
                "cannot be injected",
            ),
            (
                json!({"inject_connections": ["github"], "network": {"mode": "blocked"}}),
                "needs network",
            ),
            (
                json!({"inject_connections": ["github"], "network": {"mode": "allowlist", "domains": ["github.com"]}}),
                "limited to domains",
            ),
        ] {
            let err = EgressSpec::from_json(&bad).unwrap_err();
            assert!(err.contains(expected), "{bad}: {err}");
        }
        assert!(
            EgressSpec::from_json(&json!({
                "inject_connections": ["github"],
                "network": {"mode": "allowlist", "cidrs": ["140.82.112.0/20"]}
            }))
            .is_ok()
        );
    }

    #[test]
    fn github_rules_keep_the_token_out_of_header_templates() {
        let (env, rules) = connection_rules("github", "ghp_x").unwrap();
        let env: HashMap<_, _> = env.into_iter().collect();
        assert_eq!(env["GITHUB_BEARER"], "ghp_x");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(&env["GITHUB_BASIC"])
                .unwrap(),
            b"x-access-token:ghp_x"
        );
        assert_eq!(
            rules.iter().map(|r| r.domain.as_str()).collect::<Vec<_>>(),
            ["api.github.com", "github.com"]
        );
        assert!(
            rules
                .iter()
                .flat_map(|r| &r.headers)
                .all(|(_, v)| !v.contains("ghp_x"))
        );
    }
}
