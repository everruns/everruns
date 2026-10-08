// Catalog presets whose service credential is an existing agent connection.
//
// Spec: knowledge/integrations/user-mcp-servers.md (D4). A preset may name a
// connection provider (`service_connection_provider`); an attachment acting as
// `service`, or `user_or_service` falling back to the agent, then uses that
// connection on the agent's service virtual user instead of an MCP OAuth grant.
// For GitHub that is the agent's own GitHub App installation, so an agent with
// a GitHub App needs no second GitHub login for the GitHub MCP server.
//
// THREAT[TM-TOOL-059]: naming a connection sends that connection's token to the
// preset's URL. Each provider is therefore pinned to the hosts that legitimately
// accept its tokens, checked when the preset is saved and again every time a
// token is resolved, so editing a preset cannot forward an agent's GitHub token
// (or any other connection) to a host the provider never issued it for.

/// Providers a preset may name, and the MCP hosts each one's tokens may go to.
const CONNECTION_BACKED_PROVIDERS: &[(&str, &[&str])] = &[("github", &["api.githubcopilot.com"])];

/// Providers a preset may name as its service credential source.
pub fn supported_providers() -> impl Iterator<Item = &'static str> {
    CONNECTION_BACKED_PROVIDERS
        .iter()
        .map(|(provider, _)| *provider)
}

/// Whether `provider`'s tokens may be sent to the MCP server at `url`.
pub fn token_may_reach(provider: &str, url: &str) -> bool {
    let Some((_, hosts)) = CONNECTION_BACKED_PROVIDERS
        .iter()
        .find(|(candidate, _)| *candidate == provider)
    else {
        return false;
    };
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.host_str().is_some_and(|host| {
            hosts
                .iter()
                .any(|allowed| host.eq_ignore_ascii_case(allowed))
        })
}

/// Validate a preset's `service_connection_provider` against its URL. An empty
/// value clears it.
pub fn normalize(provider: Option<&str>, url: &str) -> Result<Option<String>, String> {
    let Some(provider) = provider.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if !supported_providers().any(|supported| supported == provider) {
        return Err(format!(
            "service_connection_provider '{provider}' is not supported; supported: {}",
            supported_providers().collect::<Vec<_>>().join(", ")
        ));
    }
    if !token_may_reach(provider, url) {
        return Err(format!(
            "A {provider} connection can only back MCP servers on hosts that accept its tokens"
        ));
    }
    Ok(Some(provider.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_tokens_only_reach_the_github_mcp_host() {
        assert!(token_may_reach(
            "github",
            "https://api.githubcopilot.com/mcp/"
        ));
        assert!(!token_may_reach(
            "github",
            "http://api.githubcopilot.com/mcp/"
        ));
        assert!(!token_may_reach("github", "https://evil.example.com/mcp"));
        assert!(!token_may_reach(
            "github",
            "https://api.githubcopilot.com.evil.example.com/mcp"
        ));
        assert!(!token_may_reach(
            "slack",
            "https://api.githubcopilot.com/mcp/"
        ));
    }

    #[test]
    fn normalize_accepts_supported_pairs_and_clears_empty_values() {
        assert_eq!(
            normalize(Some("github"), "https://api.githubcopilot.com/mcp/").unwrap(),
            Some("github".to_string())
        );
        assert_eq!(normalize(Some(" "), "https://example.com").unwrap(), None);
        assert_eq!(normalize(None, "https://example.com").unwrap(), None);
        assert!(
            normalize(Some("github"), "https://mcp.example.com/mcp")
                .unwrap_err()
                .contains("hosts that accept its tokens")
        );
        assert!(
            normalize(Some("slack"), "https://api.githubcopilot.com/mcp/")
                .unwrap_err()
                .contains("not supported")
        );
    }
}
