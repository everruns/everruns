// OAuth provider bindings in member-authored declarative capabilities.
//
// THREAT[TM-MCP-015]: Any org Member can write a declarative capability, and
// its MCP contributions are merged into a session without the strip that
// explicit attachments get. The runtime resolves an `oauth_provider_id` to
// the invoking person's own grant for that provider and sends it as a bearer
// token to the server's `url`; nothing on that path compares the two. A
// definition that pairs an attacker URL with a catalog server's provider id
// would therefore hand every user of the agent's token to the attacker.
//
// Decision: a declarative capability may name a provider only when it is an
// active org MCP server row (`mcp_oauth_<uuid>`) whose URL matches the
// contribution's URL, so the token can only go where its grant was issued.
// Write paths reject anything else; the runtime hydration path drops the
// binding (the server then runs unauthenticated) so rows stored before this
// check cannot keep exfiltrating. Plugins are not affected: their provider ids
// are server-assigned anchors (TM-PLUGIN-008).

use crate::kernel_imports::DeclarativeCapabilityDefinition;
use crate::storage::StorageBackend;
use uuid::Uuid;

/// Why a contributed server's provider binding is refused, or `None` if allowed.
async fn binding_problem(
    db: &StorageBackend,
    org_id: i64,
    url: &str,
    provider: &str,
) -> anyhow::Result<Option<String>> {
    let Some(server_id) = provider
        .strip_prefix("mcp_oauth_")
        .and_then(|id| id.parse::<Uuid>().ok())
    else {
        return Ok(Some(format!(
            "oauth_provider_id '{provider}' is not an MCP server sign-in in this organization"
        )));
    };
    let Some(row) = db.get_mcp_server(org_id, server_id).await? else {
        return Ok(Some(format!(
            "oauth_provider_id '{provider}' is not an MCP server sign-in in this organization"
        )));
    };
    if row.status != "active" {
        return Ok(Some(format!(
            "oauth_provider_id '{provider}' refers to an MCP server that is not active"
        )));
    }
    if !same_endpoint(url, &row.url) {
        return Ok(Some(format!(
            "oauth_provider_id '{provider}' belongs to {}, so it cannot be used for {url}",
            row.url
        )));
    }
    Ok(None)
}

/// Scheme, host, port and path must match; a trailing slash is ignored.
fn same_endpoint(left: &str, right: &str) -> bool {
    match (url::Url::parse(left), url::Url::parse(right)) {
        (Ok(left), Ok(right)) => {
            left.origin() == right.origin()
                && left.path().trim_end_matches('/') == right.path().trim_end_matches('/')
        }
        _ => false,
    }
}

/// Write-path check: refuse a definition that binds a server to a provider
/// whose grant was issued for a different endpoint.
pub(crate) async fn validate_declarative_oauth_bindings(
    db: &StorageBackend,
    org_id: i64,
    definition: &DeclarativeCapabilityDefinition,
) -> anyhow::Result<Result<(), String>> {
    let Some(servers) = &definition.mcp_servers else {
        return Ok(Ok(()));
    };
    for (name, server) in servers {
        if let Some(provider) = server.oauth_provider_id.as_deref()
            && let Some(problem) = binding_problem(db, org_id, &server.url, provider).await?
        {
            return Ok(Err(format!("MCP server '{name}': {problem}")));
        }
    }
    Ok(Ok(()))
}

/// Runtime backstop: clear provider bindings that would fail the write check.
pub(crate) async fn drop_unbound_declarative_oauth(
    db: &StorageBackend,
    org_id: i64,
    definition: &mut DeclarativeCapabilityDefinition,
) -> anyhow::Result<()> {
    let Some(servers) = definition.mcp_servers.as_mut() else {
        return Ok(());
    };
    for (name, server) in servers.iter_mut() {
        let Some(provider) = server.oauth_provider_id.clone() else {
            continue;
        };
        if let Some(problem) = binding_problem(db, org_id, &server.url, &provider).await? {
            tracing::warn!(
                capability = %definition.name,
                server = %name,
                %problem,
                "Dropping a declarative capability's OAuth provider binding"
            );
            server.oauth_provider_id = None;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::same_endpoint;

    #[test]
    fn same_endpoint_requires_origin_and_path() {
        assert!(same_endpoint(
            "https://mcp.linear.app/mcp",
            "https://mcp.linear.app/mcp/"
        ));
        assert!(!same_endpoint(
            "https://attacker.example/mcp",
            "https://mcp.linear.app/mcp"
        ));
        assert!(!same_endpoint(
            "http://mcp.linear.app/mcp",
            "https://mcp.linear.app/mcp"
        ));
        assert!(!same_endpoint(
            "https://mcp.linear.app:8443/mcp",
            "https://mcp.linear.app/mcp"
        ));
        assert!(!same_endpoint(
            "https://mcp.linear.app/other",
            "https://mcp.linear.app/mcp"
        ));
        assert!(!same_endpoint("not a url", "https://mcp.linear.app/mcp"));
    }
}
