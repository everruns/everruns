// Session MCP servers on the control plane. Spec: knowledge/integrations/user-mcp-servers.md (D6).
//
// Decision: every path that builds a session's MCP surface folds the session's
// run-time records first: the gRPC turn context, gRPC MCP prefix resolution,
// and the in-process (direct worker) turn context and prefix resolution. One
// fold point, `everruns_core::apply_session_attachments`, reads session MCP
// server records (ARD MCP targets, chat-only user servers) and ARD external
// agents. A path that skipped it would list a server's tools in one place and
// fail to resolve them in another.

use super::user_servers::UserMcpConnectionStatus;
use crate::domains::common::*;
use crate::domains::session_storage::queries::{parse_owned_session_id, verify_session_ownership};
use crate::records::Session;
use everruns_core::session_services::SessionStorageStore;
use everruns_core::{SessionMcpServer, SessionMcpServerSource};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Fold the session's run-time records into its `mcpServers` and capabilities.
pub async fn fold_session_records(storage: &dyn SessionStorageStore, session: &mut Session) {
    // Records merge into the portable config layer (EVE-882); copy the merged
    // fields back onto the stored record so scoped-MCP resolution sees them.
    let mut execution_session = session.execution_session();
    everruns_core::apply_session_attachments(storage, &mut execution_session).await;
    session.mcp_servers = execution_session.mcp_servers;
    session.capabilities = execution_session.capabilities;
}

// ============================================================================
// Session API: chat-only servers
// ============================================================================
//
// Decision: the session API shows and removes the servers a person added "for
// this chat only" (`user_mcp` add with `scope: "chat"`, source `UserMcp`). ARD
// attachments are left out: their `ard_attach:` record drives ARD's own
// listing and cap, so removing only the session record here would split them.
// Sign-in state is the viewer's, because a chat-only server signs in as
// whoever is chatting. Same authority as session storage: `SESSION_VIEW` to
// list, `SESSION_MANAGE` to remove, and the session must belong to the
// caller's organization.

/// An MCP server added to one chat only.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ChatMcpServer {
    /// Server name; also the tool prefix the agent sees.
    pub name: String,
    /// Endpoint URL, for display.
    pub url: String,
    /// Catalog preset name, for servers added from the catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_name: Option<String>,
    /// Whether the person viewing has signed in to it.
    pub connection: UserMcpConnectionStatus,
}

fn session_store(ctx: &Ctx) -> crate::storage::DbSessionStorageStore {
    // Session MCP records are plain KV values; no secret is read or written.
    crate::storage::DbSessionStorageStore::new_without_encryption(ctx.db.database().clone())
}

/// The viewer's sign-in state for one chat-only server.
async fn viewer_connection(
    ctx: &Ctx,
    record: &SessionMcpServer,
) -> Result<UserMcpConnectionStatus, CommandError> {
    let providers =
        match super::user_manage::sign_in_providers(&ctx.db, ctx.org_id(), &record.server).await {
            Ok(providers) => providers,
            // The catalog preset is gone: nothing can sign in to it.
            Err(everruns_core::mcp::UserMcpStoreError::Invalid(_)) => {
                return Ok(UserMcpConnectionStatus::NotConnected);
            }
            Err(error) => return Err(CommandError::internal(anyhow::anyhow!(error.to_string()))),
        };
    let Some((provider, _)) = providers else {
        return Ok(UserMcpConnectionStatus::NotNeeded);
    };
    // Chat-only servers sign in as the person chatting; a service login is
    // not the viewer's to give, so it reads as not needed from here.
    if !record.server.acts_as.uses_user_grant() {
        return Ok(UserMcpConnectionStatus::NotNeeded);
    }
    let Some(user_id) = ctx.caller.user_id else {
        return Ok(UserMcpConnectionStatus::NotConnected);
    };
    let Ok(viewer) = ctx.db.default_virtual_user(ctx.org_id(), user_id).await else {
        return Ok(UserMcpConnectionStatus::NotConnected);
    };
    let connected = ctx
        .db
        .get_virtual_user_connection(viewer.id, &provider)
        .await
        .map_err(CommandError::internal)?
        .is_some();
    Ok(if connected {
        UserMcpConnectionStatus::Connected
    } else {
        UserMcpConnectionStatus::NotConnected
    })
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct ListChatMcpServers {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "list_session_mcp_servers",
    category = "sessions",
    description = "List the MCP servers added to one chat only.",
    method = "GET",
    path = "/v1/sessions/{session_id}/mcp-servers",
    positional = "session_id",
    policy = crate::domains::sessions::SESSION_VIEW,
)]
impl Command for ListChatMcpServers {
    type Output = Vec<ChatMcpServer>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<ChatMcpServer>, CommandError> {
        let session_id = parse_owned_session_id(&self.session_id)?;
        verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;
        let mut records: Vec<_> =
            everruns_core::load_session_mcp_servers(&session_store(ctx), session_id)
                .await
                .into_iter()
                .filter(|record| record.source == SessionMcpServerSource::UserMcp)
                .collect();
        records.sort_by(|a, b| a.name.cmp(&b.name));
        let mut servers = Vec::with_capacity(records.len());
        for record in records {
            servers.push(ChatMcpServer {
                connection: viewer_connection(ctx, &record).await?,
                catalog_name: record
                    .server
                    .preset
                    .as_ref()
                    .map(|preset| preset.catalog_name().to_string()),
                url: record.server.url,
                name: record.name,
            });
        }
        Ok(servers)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
/// Remove an MCP server that was added to one chat only.
pub struct RemoveChatMcpServer {
    /// Session the server was added to.
    #[schema(example = "session_01933b5a000070008000000000000001")]
    pub session_id: String,
    /// Server name.
    #[schema(example = "linear")]
    pub name: String,
}

#[command(
    name = "remove_session_mcp_server",
    category = "sessions",
    description = "Remove an MCP server added to one chat only. Its tools leave from the next turn.",
    method = "DELETE",
    path = "/v1/sessions/{session_id}/mcp-servers/{name}",
    policy = crate::domains::sessions::SESSION_MANAGE,
)]
impl Command for RemoveChatMcpServer {
    type Output = bool;

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        let session_id = parse_owned_session_id(&self.session_id)?;
        verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;
        let store = session_store(ctx);
        let chat_only = everruns_core::get_session_mcp_server(&store, session_id, &self.name)
            .await
            .is_some_and(|record| record.source == SessionMcpServerSource::UserMcp);
        if !chat_only {
            return Ok(false);
        }
        everruns_core::remove_session_mcp_server(&store, session_id, &self.name)
            .await
            .map_err(|error| CommandError::internal(anyhow::anyhow!(error.to_string())))
    }
}

#[cfg(test)]
#[path = "session_servers_tests.rs"]
mod tests;
