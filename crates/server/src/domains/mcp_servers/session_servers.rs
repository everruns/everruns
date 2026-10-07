// Session MCP servers on the control plane. Spec: knowledge/integrations/user-mcp-servers.md (D6).
//
// Decision: every path that builds a session's MCP surface folds the session's
// run-time records first: the gRPC turn context, gRPC MCP prefix resolution,
// and the in-process (direct worker) turn context and prefix resolution. One
// fold point, `everruns_core::apply_session_attachments`, reads session MCP
// server records (ARD MCP targets, chat-only user servers) and ARD external
// agents. A path that skipped it would list a server's tools in one place and
// fail to resolve them in another.

use crate::records::Session;
use everruns_core::session_services::SessionStorageStore;

/// Fold the session's run-time records into its `mcpServers` and capabilities.
pub async fn fold_session_records(storage: &dyn SessionStorageStore, session: &mut Session) {
    // Records merge into the portable config layer (EVE-882); copy the merged
    // fields back onto the stored record so scoped-MCP resolution sees them.
    let mut execution_session = session.execution_session();
    everruns_core::apply_session_attachments(storage, &mut execution_session).await;
    session.mcp_servers = execution_session.mcp_servers;
    session.capabilities = execution_session.capabilities;
}
