//! `UserConnectionResolver` over the `worker_*_connection*` and
//! `worker_*_mcp_connection` commands.
//!
//! Decision: connection tokens are org-scoped work, so the resolver is built
//! for one org and every lookup runs as that org's internal caller; the server
//! confirms the session (or virtual user) belongs to it. Bindings travel as
//! params: `for_execution` adds the invocation, `for_mcp_operation` the MCP
//! attachment the server re-validates before it resolves an MCP grant.
//!
//! Tokens cross only in the command's answer, and a malformed answer is
//! reported without quoting it ([`super::call_secret`]). A control plane that
//! does not know a command answers `NotFound`, which fails the lookup closed:
//! nothing here falls back to another identity.

use super::{InternalCommandTransport, call, call_secret};
use crate::core::McpServerActsAs;
use crate::core::connection_services::{ServiceApiKeyConnection, UserConnectionResolver};
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::session_sandbox::SessionSandboxCredential;
use everruns_contracts::typed_id::SessionId;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

/// The connection resolver tools use, on either transport.
pub struct CommandConnectionResolver<T> {
    transport: Arc<T>,
    input_message_id: Option<Uuid>,
    mcp_server_prefix: Option<String>,
}

impl<T> Clone for CommandConnectionResolver<T> {
    fn clone(&self) -> Self {
        Self {
            transport: self.transport.clone(),
            input_message_id: self.input_message_id,
            mcp_server_prefix: self.mcp_server_prefix.clone(),
        }
    }
}

impl<T: InternalCommandTransport + 'static> CommandConnectionResolver<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport: Arc::new(transport),
            input_message_id: None,
            mcp_server_prefix: None,
        }
    }

    fn session(&self, session_id: SessionId, provider: &str) -> Value {
        json!({
            "session_id": session_id.to_string(),
            "provider": provider,
            "input_message_id": self.input_message_id,
        })
    }

    fn mcp(&self, session_id: SessionId, provider: &str, acts_as: McpServerActsAs) -> Value {
        json!({
            "session_id": session_id.to_string(),
            "provider": provider,
            "acts_as": acts_as.to_string(),
            "server_prefix": self.mcp_server_prefix,
            "input_message_id": self.input_message_id,
        })
    }
}

/// The service API-key connection, as it crosses the wire.
#[derive(serde::Deserialize)]
struct ServiceConnection {
    api_key: String,
    #[serde(default)]
    metadata: Option<Value>,
}

#[async_trait]
impl<T: InternalCommandTransport + 'static> UserConnectionResolver
    for CommandConnectionResolver<T>
{
    fn for_execution(&self, id: Uuid) -> Option<Arc<dyn UserConnectionResolver>> {
        let mut bound = self.clone();
        bound.input_message_id = Some(id);
        Some(Arc::new(bound))
    }

    fn for_mcp_operation(&self, server_prefix: &str) -> Option<Arc<dyn UserConnectionResolver>> {
        let mut bound = self.clone();
        bound.mcp_server_prefix = Some(server_prefix.to_string());
        Some(Arc::new(bound))
    }

    async fn get_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<String>> {
        call_secret(
            &*self.transport,
            "Get connection token",
            "worker_get_connection_token",
            self.session(session_id, provider),
        )
        .await
    }

    async fn get_sandbox_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
        credential: &SessionSandboxCredential,
    ) -> Result<Option<String>> {
        call_secret(
            &*self.transport,
            "Get sandbox connection token",
            "worker_get_sandbox_connection_token",
            json!({
                "session_id": session_id.to_string(),
                "provider": provider,
                "credential": credential,
            }),
        )
        .await
    }

    async fn get_mcp_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
        acts_as: McpServerActsAs,
    ) -> Result<Option<String>> {
        call_secret(
            &*self.transport,
            "Get MCP connection token",
            "worker_get_mcp_connection_token",
            self.mcp(session_id, provider, acts_as),
        )
        .await
    }

    async fn get_service_api_key_connection(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<ServiceApiKeyConnection>> {
        let connection: Option<ServiceConnection> = call_secret(
            &*self.transport,
            "Get service connection",
            "worker_get_service_api_key_connection",
            self.session(session_id, provider),
        )
        .await?;
        Ok(connection.map(|connection| ServiceApiKeyConnection {
            api_key: connection.api_key,
            metadata: connection.metadata,
        }))
    }

    async fn get_connection_user(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Uuid>> {
        call(
            &*self.transport,
            "Get connection owner",
            "worker_get_connection_user",
            self.session(session_id, provider),
        )
        .await
    }

    async fn invalidate_mcp_connection(
        &self,
        session_id: SessionId,
        provider: &str,
        acts_as: McpServerActsAs,
        rejected_credential_fingerprint: &str,
    ) -> Result<()> {
        let mut params = self.mcp(session_id, provider, acts_as);
        params["rejected_credential_fingerprint"] = json!(rejected_credential_fingerprint);
        call(
            &*self.transport,
            "Invalidate MCP connection",
            "worker_invalidate_mcp_connection",
            params,
        )
        .await
    }

    async fn get_connection_token_for_user(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<String>> {
        call_secret(
            &*self.transport,
            "Get connection token for user",
            "worker_get_virtual_user_connection_token",
            json!({ "virtual_user_id": user_id, "provider": provider }),
        )
        .await
    }

    async fn get_connection_token_for_connection(
        &self,
        connection_id: Uuid,
        virtual_user_id: Uuid,
        provider: &str,
    ) -> Result<Option<String>> {
        call_secret(
            &*self.transport,
            "Get connection token for connection",
            "worker_get_connection_token_for_connection",
            json!({
                "virtual_user_id": virtual_user_id,
                "connection_id": connection_id,
                "provider": provider,
            }),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_internal_protocol::proto;
    use std::sync::Mutex;

    /// Records each call and answers with a canned result.
    struct Recorded {
        calls: Arc<Mutex<Vec<(String, Value)>>>,
        answer: std::result::Result<Value, proto::CommandError>,
    }

    #[async_trait]
    impl InternalCommandTransport for Recorded {
        async fn execute_internal_command(
            &self,
            name: &str,
            params: Value,
        ) -> Result<std::result::Result<Value, proto::CommandError>> {
            self.calls.lock().unwrap().push((name.to_string(), params));
            Ok(self.answer.clone())
        }
    }

    type Calls = Arc<Mutex<Vec<(String, Value)>>>;

    fn resolver(
        answer: std::result::Result<Value, proto::CommandError>,
    ) -> (CommandConnectionResolver<Recorded>, Calls) {
        let calls = Calls::default();
        let resolver = CommandConnectionResolver::new(Recorded {
            calls: calls.clone(),
            answer,
        });
        (resolver, calls)
    }

    #[tokio::test]
    async fn bindings_travel_with_each_lookup() {
        let (resolver, calls) = resolver(Ok(json!("token")));
        let session = SessionId::new();
        let invocation = Uuid::new_v4();

        let plain = resolver.get_connection_token(session, "github").await;
        assert_eq!(plain.unwrap().as_deref(), Some("token"));
        let bound = resolver
            .for_execution(invocation)
            .unwrap()
            .for_mcp_operation("docs")
            .unwrap();
        bound
            .get_mcp_connection_token(session, "mcp_oauth_x", McpServerActsAs::User)
            .await
            .unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls[0].0, "worker_get_connection_token");
        assert_eq!(calls[0].1["session_id"], session.to_string());
        assert_eq!(calls[0].1["provider"], "github");
        assert!(calls[0].1["input_message_id"].is_null());
        assert_eq!(calls[1].0, "worker_get_mcp_connection_token");
        assert_eq!(calls[1].1["input_message_id"], invocation.to_string());
        assert_eq!(calls[1].1["server_prefix"], "docs");
        assert_eq!(calls[1].1["acts_as"], "user");
    }

    /// `user_or_service` asks for each concrete identity in turn, as the
    /// RPC-based resolver did, and reports the one that answered.
    #[tokio::test]
    async fn user_or_service_asks_each_identity() {
        let (resolver, calls) = resolver(Ok(Value::Null));
        let credential = resolver
            .get_mcp_connection_credential(
                SessionId::new(),
                "mcp_oauth_x",
                McpServerActsAs::UserOrService,
            )
            .await
            .unwrap();
        assert!(credential.is_none());
        let asked: Vec<String> = calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, params)| params["acts_as"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(asked, ["user", "service"]);
    }

    #[tokio::test]
    async fn cleanup_lookups_name_the_virtual_user() {
        let (resolver, calls) = resolver(Ok(json!("token")));
        let owner = Uuid::new_v4();
        let connection = Uuid::new_v4();
        resolver
            .get_connection_token_for_user(owner, "daytona")
            .await
            .unwrap();
        resolver
            .get_connection_token_for_connection(connection, owner, "daytona")
            .await
            .unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(calls[0].0, "worker_get_virtual_user_connection_token");
        assert_eq!(calls[0].1["virtual_user_id"], owner.to_string());
        assert_eq!(calls[1].0, "worker_get_connection_token_for_connection");
        assert_eq!(calls[1].1["connection_id"], connection.to_string());
    }

    #[tokio::test]
    async fn service_connections_and_owners_decode() {
        let (resolver, _) = resolver(Ok(json!({ "api_key": "k", "metadata": { "a": 1 } })));
        let connection = resolver
            .get_service_api_key_connection(SessionId::new(), "brave")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(connection.api_key, "k");
        assert_eq!(connection.metadata, Some(json!({ "a": 1 })));

        let owner = Uuid::new_v4();
        let (resolver, _) = resolver_for(json!(owner));
        assert_eq!(
            resolver
                .get_connection_user(SessionId::new(), "github")
                .await
                .unwrap(),
            Some(owner)
        );
    }

    fn resolver_for(answer: Value) -> (CommandConnectionResolver<Recorded>, Calls) {
        resolver(Ok(answer))
    }

    /// An older control plane answers an unknown command with `NotFound`; the
    /// lookup fails rather than falling back to another identity.
    #[tokio::test]
    async fn an_unknown_command_fails_closed() {
        let (resolver, calls) = resolver(Err(proto::CommandError {
            kind: proto::command_error::Kind::NotFound as i32,
            message: "Unknown command: worker_get_mcp_connection_token".to_string(),
        }));
        resolver
            .for_mcp_operation("docs")
            .unwrap()
            .get_mcp_connection_token(SessionId::new(), "mcp_oauth_x", McpServerActsAs::User)
            .await
            .expect_err("fails closed");
        assert_eq!(calls.lock().unwrap().len(), 1, "no second lookup");
    }

    #[tokio::test]
    async fn a_malformed_token_answer_does_not_echo_it() {
        let (resolver, _) = resolver(Ok(json!(["leaked-token"])));
        let error = resolver
            .get_connection_token(SessionId::new(), "github")
            .await
            .expect_err("wrong shape");
        assert!(!error.to_string().contains("leaked-token"), "{error}");
    }
}
