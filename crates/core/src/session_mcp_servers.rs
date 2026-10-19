//! Session MCP servers: MCP servers added to one session at run time.
//!
//! Decision: there is one record for "this MCP server joins this chat", no
//! matter who adds it. ARD's `attach_resource` writes one for an MCP target,
//! and `user_mcp`'s add tool writes one when the person picks "this chat
//! only". Turn-context assembly folds every record into the session's
//! `mcpServers` layer (see [`crate::apply_session_attachments`]), so the
//! existing scoped-MCP resolution, deferral and credential paths treat them
//! like any session-configured server. Records apply from the next turn, and
//! deleting one drops its tools from the next turn.
//!
//! Records are keyed by server name, so two writers adding the same name keep
//! one record (last write wins), exactly like two `mcpServers` entries with the
//! same key. The key prefix is reserved from the user-facing `kv_store` tool:
//! a session actor that could write here would add any server it liked.

use serde::{Deserialize, Serialize};

use crate::mcp_server::ScopedMcpServer;
use crate::session::ExecutionSession;
use crate::session_services::SessionStorageStore;
use crate::typed_id::SessionId;

/// Session KV key prefix of session MCP server records.
pub const SESSION_MCP_SERVER_KV_PREFIX: &str = "session_mcp:";

/// Who added a session MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionMcpServerSource {
    /// Attached by ARD `attach_resource` from a catalog entry.
    Ard {
        /// URN of the attached catalog entry.
        urn: String,
    },
    /// Added by the person through `user_mcp` for this chat only.
    UserMcp,
}

/// An MCP server that joins one session's turns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionMcpServer {
    /// Logical server name: the `mcpServers` key and the `mcp_{name}__` prefix.
    pub name: String,
    /// The server definition, as in `mcpServers`.
    pub server: ScopedMcpServer,
    /// Who added it.
    pub source: SessionMcpServerSource,
}

/// The KV key of the record for `name`.
pub fn session_mcp_server_kv_key(name: &str) -> String {
    format!("{SESSION_MCP_SERVER_KV_PREFIX}{name}")
}

/// Write `record`, replacing any record with the same name.
pub async fn put_session_mcp_server(
    storage: &dyn SessionStorageStore,
    session_id: SessionId,
    record: &SessionMcpServer,
) -> everruns_contracts::error::Result<()> {
    let serialized = serde_json::to_string(record)
        .map_err(|e| everruns_contracts::error::AgentLoopError::Internal(e.into()))?;
    storage
        .set_value(
            session_id,
            &session_mcp_server_kv_key(&record.name),
            &serialized,
        )
        .await
}

/// The record for `name`, if one exists and parses.
pub async fn get_session_mcp_server(
    storage: &dyn SessionStorageStore,
    session_id: SessionId,
    name: &str,
) -> Option<SessionMcpServer> {
    let raw = storage
        .get_value(session_id, &session_mcp_server_kv_key(name))
        .await
        .ok()??;
    serde_json::from_str(&raw).ok()
}

/// Delete the record for `name`. `false` when there was none.
pub async fn remove_session_mcp_server(
    storage: &dyn SessionStorageStore,
    session_id: SessionId,
    name: &str,
) -> everruns_contracts::error::Result<bool> {
    storage
        .delete_value(session_id, &session_mcp_server_kv_key(name))
        .await
}

/// Every session MCP server record of a session. Best-effort: a record that
/// fails to read or parse is skipped with a warning rather than failing the
/// turn.
pub async fn load_session_mcp_servers(
    storage: &dyn SessionStorageStore,
    session_id: SessionId,
) -> Vec<SessionMcpServer> {
    let keys = match storage.list_keys(session_id).await {
        Ok(keys) => keys,
        Err(e) => {
            tracing::warn!("failed to list session keys for session MCP servers: {e}");
            return Vec::new();
        }
    };
    let mut records = Vec::new();
    for info in keys {
        if !info.key.starts_with(SESSION_MCP_SERVER_KV_PREFIX) {
            continue;
        }
        match storage.get_value(session_id, &info.key).await {
            Ok(Some(raw)) => match serde_json::from_str::<SessionMcpServer>(&raw) {
                Ok(record) => records.push(record),
                Err(e) => {
                    tracing::warn!(key = %info.key, "skipping malformed session MCP server: {e}")
                }
            },
            Ok(None) => {}
            Err(e) => tracing::warn!(key = %info.key, "failed to read session MCP server: {e}"),
        }
    }
    records
}

/// Fold records into the session's `mcpServers` layer. A record wins over a
/// session-configured server of the same name, as the latest session write.
pub fn merge_session_mcp_servers(session: &mut ExecutionSession, records: &[SessionMcpServer]) {
    for record in records {
        session
            .mcp_servers
            .insert(record.name.clone(), record.server.clone());
    }
}

#[cfg(test)]
#[path = "session_mcp_servers_tests.rs"]
mod tests;
