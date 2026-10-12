// Validation for the PACT Delegated profile config (PACT 1.0 §5). Errors are
// plain messages; `validation.rs` prefixes them with the config path.

use crate::domains::agent_channels::record::pact_delegation::PactDelegationConfig;

/// Scopes one endpoint may offer. Each is a consent-page checkbox.
const MAX_SCOPES: usize = 50;
const MAX_SCOPE_ID_LEN: usize = 128;
const MAX_SCOPE_DESCRIPTION_LEN: usize = 500;
const MAX_TOOLS_PER_SCOPE: usize = 100;
const MAX_TOOL_NAME_LEN: usize = 256;
/// Same cap as a personal agent's inline JWKS.
const MAX_INLINE_JWKS_BYTES: usize = 16 * 1024;

/// Pages the user is sent to: HTTPS, or plain HTTP on a loopback host for local development: users'
/// browsers visit these pages, so a loopback URL only reaches the user's own
/// machine.
fn https_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    match url.scheme() {
        "https" => true,
        "http" => {
            matches!(url.host(), Some(url::Host::Domain("localhost")))
                || matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
                || matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback())
        }
        _ => false,
    }
}

/// RFC 6749 §3.3 scope-token: `%x21 / %x23-5B / %x5D-7E`.
fn valid_scope_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_SCOPE_ID_LEN
        && id
            .bytes()
            .all(|b| b == 0x21 || (0x23..=0x5B).contains(&b) || (0x5D..=0x7E).contains(&b))
}

pub(super) fn validate_delegation(config: &PactDelegationConfig) -> Result<(), String> {
    // The server sends users to these pages and fetches the JWKS, so only
    // HTTPS is stored; the JWKS fetch still refuses private addresses
    // (`everruns_core::channel_auth`, `build_pinned_client`).
    if !https_url(&config.login_url) {
        return Err("login_url must be an https URL".into());
    }
    if config.login_issuer.trim().is_empty() {
        return Err("login_issuer must be non-empty".into());
    }
    match (config.login_jwks_uri.as_deref(), config.login_jwks.as_ref()) {
        // Fetched by the server, so HTTPS only, loopback included.
        (Some(uri), None) if uri.starts_with("https://") && url::Url::parse(uri).is_ok() => {}
        (Some(_), None) => return Err("login_jwks_uri must be an https URL".into()),
        (None, Some(jwks)) => {
            if serde_json::to_vec(jwks).map_or(0, |bytes| bytes.len()) > MAX_INLINE_JWKS_BYTES {
                return Err("inline login_jwks must be at most 16 KiB".into());
            }
            if serde_json::from_value::<jsonwebtoken::jwk::JwkSet>(jwks.clone()).is_err() {
                return Err("login_jwks must be a JWKS document ({\"keys\": [...]})".into());
            }
        }
        _ => return Err("set exactly one of login_jwks_uri or login_jwks".into()),
    }
    if let Some(url) = config.connected_url.as_deref()
        && !https_url(url)
    {
        return Err("connected_url must be an https URL".into());
    }
    if let Some(server) = config.mcp_server.as_deref()
        && (server.trim().is_empty() || server.len() > MAX_TOOL_NAME_LEN)
    {
        return Err("mcp_server must be a catalog MCP server name".into());
    }
    if config.scopes.is_empty() {
        return Err("scopes must list at least one scope".into());
    }
    if config.scopes.len() > MAX_SCOPES {
        return Err("scopes lists more than 50 scopes".into());
    }
    let mut seen = std::collections::HashSet::new();
    for scope in &config.scopes {
        if !valid_scope_id(&scope.id) {
            return Err(format!(
                "scope id {:?} must be 1-128 printable ASCII characters without spaces, quotes or backslashes",
                scope.id
            ));
        }
        if !seen.insert(scope.id.as_str()) {
            return Err(format!("scope id {:?} is listed twice", scope.id));
        }
        if scope.tools.len() > MAX_TOOLS_PER_SCOPE
            || scope
                .tools
                .iter()
                .any(|tool| tool.trim().is_empty() || tool.len() > MAX_TOOL_NAME_LEN)
        {
            return Err(format!(
                "scope {:?} lists at most 100 tools, each a non-empty name of at most 256 characters",
                scope.id
            ));
        }
        let description = scope.description.trim();
        if description.is_empty() || description.len() > MAX_SCOPE_DESCRIPTION_LEN {
            return Err(format!(
                "scope {:?} needs a description of 1-500 characters",
                scope.id
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config(value: serde_json::Value) -> PactDelegationConfig {
        let mut base = json!({
            "login_url": "https://brand.example/login",
            "login_issuer": "https://brand.example",
            "login_jwks_uri": "https://brand.example/.well-known/jwks.json",
            "scopes": [{ "id": "orders:read", "description": "Look up your orders" }],
        });
        for (key, field) in value.as_object().unwrap() {
            base[key] = field.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn accepts_loopback_http_pages_only() {
        let local = config(json!({
            "login_url": "http://localhost:3004/login",
            "connected_url": "http://127.0.0.1:3004/connected",
        }));
        assert_eq!(validate_delegation(&local), Ok(()));
        let jwks = config(json!({ "login_jwks_uri": "http://localhost/jwks" }));
        assert!(validate_delegation(&jwks).is_err());
    }

    #[test]
    fn accepts_a_complete_config() {
        assert_eq!(validate_delegation(&config(json!({}))), Ok(()));
    }

    #[test]
    fn rejects_plain_http_login_and_jwks() {
        assert!(
            validate_delegation(&config(
                json!({ "login_url": "http://brand.example/login" })
            ))
            .is_err()
        );
        assert!(
            validate_delegation(&config(
                json!({ "login_jwks_uri": "http://brand.example/jwks" })
            ))
            .is_err()
        );
        assert!(
            validate_delegation(&config(
                json!({ "connected_url": "http://brand.example/done" })
            ))
            .is_err()
        );
    }

    #[test]
    fn needs_exactly_one_key_source() {
        let both = config(json!({ "login_jwks": { "keys": [] } }));
        assert!(validate_delegation(&both).is_err());
        let inline = config(json!({ "login_jwks_uri": null, "login_jwks": { "keys": [] } }));
        assert_eq!(validate_delegation(&inline), Ok(()));
    }

    #[test]
    fn rejects_bad_or_duplicate_scope_ids() {
        for id in ["", "has space", "quote\"d", "back\\slash"] {
            let bad = config(json!({ "scopes": [{ "id": id, "description": "x" }] }));
            assert!(
                validate_delegation(&bad).is_err(),
                "{id:?} should be rejected"
            );
        }
        let twice = config(json!({ "scopes": [
            { "id": "a", "description": "x" },
            { "id": "a", "description": "y" },
        ] }));
        assert!(validate_delegation(&twice).is_err());
        assert!(validate_delegation(&config(json!({ "scopes": [] }))).is_err());
        let blank_tool =
            config(json!({ "scopes": [{ "id": "a", "description": "x", "tools": [" "] }] }));
        assert!(validate_delegation(&blank_tool).is_err());
    }
}
