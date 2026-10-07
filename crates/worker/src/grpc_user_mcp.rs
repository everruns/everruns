//! The worker end of the `user_mcp` manage tools' store
//! (knowledge/integrations/user-mcp-servers.md).
//!
//! Bound to one org and session. The worker forwards the call and the turn's
//! input message; the control plane resolves the person and re-checks that the
//! agent may manage their servers.

use async_trait::async_trait;
use everruns_capabilities::capabilities::user_mcp::{
    UserMcpCallInvoker, UserMcpStoreCall, UserMcpStoreError, UserMcpStoreReply, UserMcpStoreResult,
};
use everruns_internal_protocol::proto;
use uuid::Uuid;

use crate::grpc_adapters::GrpcClient;

pub struct GrpcUserMcpInvoker {
    client: GrpcClient,
    org_id: i64,
    session_id: everruns_contracts::typed_id::SessionId,
}

impl GrpcUserMcpInvoker {
    pub fn new(
        client: GrpcClient,
        org_id: i64,
        session_id: everruns_contracts::typed_id::SessionId,
    ) -> Self {
        Self {
            client,
            org_id,
            session_id,
        }
    }
}

#[async_trait]
impl UserMcpCallInvoker for GrpcUserMcpInvoker {
    async fn invoke(
        &self,
        input_message: Option<Uuid>,
        call: UserMcpStoreCall,
    ) -> UserMcpStoreResult<UserMcpStoreReply> {
        use proto::invoke_user_mcp_store_response::Result as Wire;
        let call_json = serde_json::to_vec(&call)
            .map_err(|error| UserMcpStoreError::Internal(error.to_string()))?;
        let request = proto::InvokeUserMcpStoreRequest {
            org_id: self.org_id,
            session_id: Some(crate::grpc_adapters::uuid_to_proto(self.session_id.uuid())),
            input_message_id: input_message.map(crate::grpc_adapters::uuid_to_proto),
            call_json,
        };
        let mut client = self.client.inner.client();
        let response = client
            .invoke_user_mcp_store(request)
            .await
            .map_err(|status| UserMcpStoreError::Internal(status.message().to_string()))?
            .into_inner();
        let decode = |error: serde_json::Error| UserMcpStoreError::Internal(error.to_string());
        match response.result {
            Some(Wire::ReplyJson(reply)) => serde_json::from_slice(&reply).map_err(decode),
            Some(Wire::ErrorJson(error)) => Err(serde_json::from_slice(&error).map_err(decode)?),
            None => Err(UserMcpStoreError::Internal(
                "control plane answered without a result".into(),
            )),
        }
    }
}
