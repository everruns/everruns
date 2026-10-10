// MCP Server domain — commands, queries, types.
//
// See knowledge/foundations/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod connection_backed;
pub mod deferred;
pub mod events;
pub mod queries;
pub mod record;
pub mod scoped_mcp;
pub mod service;
pub mod session_servers;
pub mod tool_labels;
pub mod types;
pub mod user_layer;
pub mod user_manage;
pub mod user_servers;
pub mod worker_lookup;

pub use commands::*;
pub use events::McpEventsService;
pub use service::{
    McpServerOAuthSettings, McpServerResolved, McpServerService, McpServerSettings,
    McpServerWithTools,
};

/// Stored MCP credentials (API key, custom headers) are bound to the origin
/// they were issued for (EVE-1192). Both are write-only, so a caller who can
/// edit the URL but never saw the secrets must not be able to point them at a
/// host they control. An update that moves the server to a different origin
/// (scheme, host, or port) must therefore re-supply every credential it would
/// otherwise retain — or drop it (switch auth mode, send `headers: {}`).
/// Same-origin edits (path changes, renames) keep credentials. OAuth servers
/// are stricter still: their authority is immutable (see
/// `oauth_authority_retargeted`). Redirects never carry credentials either:
/// the egress client does not follow them.
///
/// Returns the rejection message when stored credentials would follow the
/// server to a new origin.
pub(crate) fn retained_credentials_retarget_error(
    existing_url: &str,
    existing_api_key_set: bool,
    existing_headers: &serde_json::Value,
    new_url: Option<&str>,
    api_key_retained: bool,
    headers_retained: bool,
) -> Option<&'static str> {
    let new_url = new_url?;
    if same_origin(existing_url, new_url) {
        return None;
    }
    if existing_api_key_set && api_key_retained {
        return Some(
            "Changing the MCP server origin requires re-entering its api_key; \
             stored credentials stay bound to the original origin",
        );
    }
    let has_headers = existing_headers
        .as_object()
        .is_some_and(|headers| !headers.is_empty());
    if has_headers && headers_retained {
        return Some(
            "Changing the MCP server origin requires re-supplying its headers \
             (send {} to clear them); stored headers stay bound to the original origin",
        );
    }
    None
}

/// Web origin equality (scheme, host, effective port). Unparseable URLs never
/// match, so they fail closed.
fn same_origin(a: &str, b: &str) -> bool {
    match (url::Url::parse(a), url::Url::parse(b)) {
        (Ok(a), Ok(b)) => {
            let (a, b) = (a.origin(), b.origin());
            a.is_tuple() && a == b
        }
        _ => false,
    }
}

pub const MCP_SERVER_VIEW: Policy = Policy {
    id: "mcp_server.view",
    rules: &[Rule::UserHasPermission(Permission::OrgMcpServersView)],
};
pub const MCP_SERVER_MANAGE: Policy = Policy {
    id: "mcp_server.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgMcpServersManage)],
};
pub const MCP_SERVER_DANGEROUS: Policy = Policy {
    id: "mcp_server.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgMcpServersManage),
        Rule::UserHasPermission(Permission::OrgMcpServersDangerous),
    ],
};
