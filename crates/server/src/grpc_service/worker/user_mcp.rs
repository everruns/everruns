//! The `user_mcp` manage tools' store (knowledge/integrations/user-mcp-servers.md).
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only.
//!
//! THREAT[TM-AGENT-017]: the worker names only the org, session and input
//! message. The person, and whether the agent may manage their servers at all,
//! are re-derived here from storage; a worker cannot name a person.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_invoke_user_mcp_store(
        &self,
        request: Request<proto::InvokeUserMcpStoreRequest>,
    ) -> Result<Response<proto::InvokeUserMcpStoreResponse>, Status> {
        use proto::invoke_user_mcp_store_response::Result as Wire;
        let req = request.into_inner();
        if req.call_json.len() > MAX_EXECUTE_COMMAND_PARAMS_BYTES {
            return Err(Status::resource_exhausted(
                "User MCP store call is too large",
            ));
        }
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let input_message = req
            .input_message_id
            .as_ref()
            .map(|id| parse_uuid(Some(id)))
            .transpose()?;
        let call: everruns_core::mcp::UserMcpStoreCall = serde_json::from_slice(&req.call_json)
            .map_err(|error| Status::invalid_argument(format!("Invalid call_json: {error}")))?;
        let outcome = crate::domains::mcp_servers::user_manage::invoke_user_mcp_store_for_session(
            &self.db,
            self.encryption.as_deref(),
            self.capability_service.registry(),
            req.org_id,
            session_id.into(),
            input_message,
            self.storage_store().ok().map(|store| store.as_ref()),
            call,
        )
        .await;
        let result = match outcome {
            Ok(reply) => Wire::ReplyJson(serde_json::to_vec(&reply).map_err(|error| {
                internal_status("Failed to encode user MCP store reply", error)
            })?),
            Err(error) => {
                if let everruns_core::mcp::UserMcpStoreError::Internal(message) = &error {
                    tracing::error!(%session_id, org_id = req.org_id, error = %message, "User MCP store call failed");
                }
                Wire::ErrorJson(serde_json::to_vec(&error).map_err(|error| {
                    internal_status("Failed to encode user MCP store error", error)
                })?)
            }
        };
        Ok(Response::new(proto::InvokeUserMcpStoreResponse {
            result: Some(result),
        }))
    }
}
