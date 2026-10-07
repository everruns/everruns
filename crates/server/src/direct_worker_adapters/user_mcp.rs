// MCP servers for one in-process worker turn, including the person's own.
// Spec: knowledge/integrations/user-mcp-servers.md.

use super::DirectWorkerAdapters;
use crate::domains::mcp_servers::McpServerResolved;
use crate::domains::mcp_servers::scoped_mcp::resolve_scoped_mcp_server_with_capabilities;
use crate::domains::mcp_servers::user_layer::{
    UserMcpTurn, merge_turn_scoped_mcp_servers, user_mcp_layer,
};
use crate::kernel_imports::ScopedMcpServers;
use crate::records::{Agent, Harness, Session};

impl DirectWorkerAdapters {
    /// Empty unless the agent uses the person's servers and the turn's input
    /// message was sent by exactly one person.
    async fn user_mcp_layer(
        &self,
        org_id: i64,
        harness: &Harness,
        agent: Option<&Agent>,
        session: &Session,
    ) -> ScopedMcpServers {
        user_mcp_layer(&UserMcpTurn {
            db: &self.db,
            encryption: self.encryption.as_deref(),
            org_id,
            harness,
            agent,
            session,
            registry: &self.capability_registry,
            input_message: self.input_message_id,
        })
        .await
    }

    /// The turn's effective scoped MCP servers.
    pub(super) async fn turn_mcp_servers(
        &self,
        org_id: i64,
        harness: &Harness,
        agent: Option<&Agent>,
        session: &Session,
    ) -> ScopedMcpServers {
        let user_layer = self.user_mcp_layer(org_id, harness, agent, session).await;
        merge_turn_scoped_mcp_servers(
            harness,
            agent,
            session,
            &self.capability_registry,
            &user_layer,
        )
    }

    /// Resolve the turn's scoped MCP server whose tool prefix is `server_prefix`.
    pub(super) async fn resolve_turn_mcp_server(
        &self,
        org_id: i64,
        harness: &Harness,
        agent: Option<&Agent>,
        session: &Session,
        server_prefix: &str,
    ) -> anyhow::Result<Option<McpServerResolved>> {
        let user_layer = self.user_mcp_layer(org_id, harness, agent, session).await;
        resolve_scoped_mcp_server_with_capabilities(
            &self.mcp_server_service,
            org_id,
            harness,
            agent,
            session,
            server_prefix,
            &self.capability_registry,
            &user_layer,
        )
        .await
    }
}

/// Runs the `user_mcp` manage tools' calls in process, for one session.
struct DirectUserMcpInvoker {
    db: std::sync::Arc<crate::storage::StorageBackend>,
    encryption: Option<std::sync::Arc<crate::storage::EncryptionService>>,
    registry: everruns_core::capabilities::CapabilityRegistry,
    org_id: i64,
    session_id: everruns_contracts::typed_id::SessionId,
}

#[async_trait::async_trait]
impl everruns_capabilities::capabilities::UserMcpCallInvoker for DirectUserMcpInvoker {
    async fn invoke(
        &self,
        input_message: Option<uuid::Uuid>,
        call: everruns_core::mcp::UserMcpStoreCall,
    ) -> everruns_core::mcp::UserMcpStoreResult<everruns_core::mcp::UserMcpStoreReply> {
        crate::domains::mcp_servers::user_manage::invoke_user_mcp_store_for_session(
            &self.db,
            self.encryption.as_deref(),
            &self.registry,
            self.org_id,
            self.session_id,
            input_message,
            call,
        )
        .await
    }
}

impl DirectWorkerAdapters {
    pub(super) fn user_mcp_invoker_for(
        &self,
        org_id: i64,
        session_id: everruns_contracts::typed_id::SessionId,
    ) -> std::sync::Arc<dyn everruns_capabilities::capabilities::UserMcpCallInvoker> {
        std::sync::Arc::new(DirectUserMcpInvoker {
            db: self.db.clone(),
            encryption: self.encryption.clone(),
            registry: self.capability_registry.clone(),
            org_id,
            session_id,
        })
    }
}
