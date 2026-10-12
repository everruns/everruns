// Connected AI clients: the external MCP clients (Claude, ChatGPT, Cursor, ...)
// a person approved to act as them on `/mcp`, and revoking them.
//
// Spec: knowledge/integrations/mcp-connected-clients.md (phase 1).
//
// Decision: user-scoped, not a domain command. A grant spans every
// organization the person belongs to, and like personal access tokens it is a
// credential: it must not be listable or revocable from the MCP surface it
// governs (commands are exposed over MCP `execute`), so the REST routes call
// this module directly and only for the signed-in person's own grants.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::BTreeSet;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::storage::{OAuthGrantWithClientRow, StorageBackend};

/// What a connected client may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectedClientAccess {
    /// Read-only tools and commands.
    ReadOnly,
    /// Everything the person can do, including running agents.
    ReadAndRun,
}

/// An external MCP client the person approved.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConnectedClient {
    /// Grant id. Pass it to the revoke endpoint.
    pub id: Uuid,
    /// Name the client registered with. Self-declared: show it with the
    /// redirect hosts, which tell a real client apart from a lookalike.
    pub client_name: String,
    /// Hosts the client's registered redirect URIs point at.
    pub redirect_hosts: Vec<String>,
    pub access: ConnectedClientAccess,
    /// True when the client may act in every organization the person belongs to.
    pub all_organizations: bool,
    /// When the person approved the client.
    pub created_at: DateTime<Utc>,
    /// Last request the client made on `/mcp`. Updated at most every few
    /// minutes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
}

/// List response for connected clients.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectedClientsResponse {
    pub data: Vec<ConnectedClient>,
}

/// Unique hosts of the registered redirect URIs, in registration order of
/// first appearance. Custom schemes (`cursor://anysphere.cursor-retrieval/...`)
/// carry their authority as the host; a URI without one is shown as is.
fn redirect_hosts(redirect_uris: &serde_json::Value) -> Vec<String> {
    let uris: Vec<String> = serde_json::from_value(redirect_uris.clone()).unwrap_or_default();
    let mut seen = BTreeSet::new();
    uris.iter()
        .map(|uri| {
            url::Url::parse(uri)
                .ok()
                .and_then(|url| url.host_str().map(str::to_string))
                .unwrap_or_else(|| uri.clone())
        })
        .filter(|host| seen.insert(host.clone()))
        .collect()
}

impl From<OAuthGrantWithClientRow> for ConnectedClient {
    fn from(row: OAuthGrantWithClientRow) -> Self {
        Self {
            id: row.id,
            redirect_hosts: redirect_hosts(&row.redirect_uris),
            client_name: row.client_name,
            access: if row.access == "read_only" {
                ConnectedClientAccess::ReadOnly
            } else {
                ConnectedClientAccess::ReadAndRun
            },
            all_organizations: row.allowed_org_ids.is_none(),
            created_at: row.created_at,
            last_used_at: row.last_used_at,
        }
    }
}

/// The person's connected clients, most recently used first.
pub async fn list(db: &StorageBackend, user_id: Uuid) -> anyhow::Result<Vec<ConnectedClient>> {
    let rows = db.list_active_oauth_grants_for_user(user_id).await?;
    Ok(rows.into_iter().map(ConnectedClient::from).collect())
}

/// Revoke one of the person's connected clients: the grant is marked revoked,
/// its refresh tokens are deleted, and `/mcp` rejects its access tokens.
/// Returns the client id, or `None` when the grant is not the person's or is
/// already revoked.
pub async fn revoke(
    db: &StorageBackend,
    user_id: Uuid,
    grant_id: Uuid,
) -> anyhow::Result<Option<String>> {
    Ok(db
        .revoke_oauth_grant(grant_id, user_id)
        .await?
        .map(|grant| grant.client_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_hosts_dedupe_and_keep_custom_scheme_authority() {
        let hosts = redirect_hosts(&serde_json::json!([
            "https://claude.ai/api/mcp/auth_callback",
            "https://claude.ai/other",
            "http://127.0.0.1:6274/callback",
            "cursor://anysphere.cursor-retrieval/oauth/user-everruns/callback",
        ]));
        assert_eq!(
            hosts,
            vec!["claude.ai", "127.0.0.1", "anysphere.cursor-retrieval"]
        );
    }

    #[test]
    fn redirect_hosts_tolerate_bad_data() {
        assert!(redirect_hosts(&serde_json::json!({"not": "a list"})).is_empty());
        assert_eq!(
            redirect_hosts(&serde_json::json!(["not a url"])),
            vec!["not a url"]
        );
    }
}
