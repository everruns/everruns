//! The worker's org-scoped command transport.
//!
//! `GrpcAdapter`'s bridge onto `ExecuteCommand`, shared by the two surfaces
//! that reach the server through domain commands rather than bespoke RPCs:
//! `grpc_sqldb_adapter` and `grpc_files_adapter`. Its own module because it is
//! the mechanism both depend on, and because `grpc_adapters.rs` is on the
//! source-size ratchet's debt list.

use crate::grpc_adapters::{
    COMMAND_API_VERSION_V1, GrpcAdapter, grpc_missing_field, grpc_status_to_error, uuid_to_proto,
};
use everruns_internal_protocol::proto;
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::typed_id::SessionId;

impl GrpcAdapter {
    /// The org this adapter speaks for, or an error naming the surface that
    /// needs one. Callers that reach the command transport go through here.
    pub(crate) fn require_org(&self, surface: &str) -> Result<i64> {
        self.org_id.ok_or_else(|| {
            AgentLoopError::store(format!(
                "{surface} requires an org-scoped adapter; this one is the cross-org sweeper context"
            ))
        })
    }

    /// Run a registered domain command as the org's internal caller.
    ///
    /// `user_id: None` makes the server resolve `Caller::internal(org_id)`, which
    /// is what the worker is: trusted, acting for the org rather than for a
    /// person. The command still runs through `Command::run`, so policy applies
    /// here exactly as it does for HTTP and MCP callers.
    /// `acting_for_session` states which session's runtime is speaking. It does
    /// not change the caller; it gates that session's private user-memory mount
    /// and nothing else. `None` outside a specific session's turn.
    pub(crate) async fn execute_session_command(
        &self,
        surface: &str,
        name: &str,
        params: serde_json::Value,
        acting_for_session: Option<SessionId>,
    ) -> Result<std::result::Result<serde_json::Value, proto::CommandError>> {
        let org_id = self.require_org(surface)?;
        let mut client = self.client.inner.lock().await;
        let response = client
            .execute_command(proto::ExecuteCommandRequest {
                input_message_id: None,
                platform_session_id: None,
                acting_for_session_id: acting_for_session.map(|id| uuid_to_proto(id.uuid())),

                name: name.to_string(),
                api_version: COMMAND_API_VERSION_V1.to_string(),
                params_json: serde_json::to_vec(&params).map_err(|error| {
                    AgentLoopError::store(format!("JSON serialization failed: {error}"))
                })?,
                org_id,
                user_id: None,
                idempotency_key: None,
                metadata: Default::default(),
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();

        match response
            .result
            .ok_or_else(|| grpc_missing_field("No command result in response"))?
        {
            proto::execute_command_response::Result::OkJson(ok_json) => {
                let value = serde_json::from_slice(&ok_json).map_err(|error| {
                    AgentLoopError::store(format!("Failed to decode command response: {error}"))
                })?;
                Ok(Ok(value))
            }
            proto::execute_command_response::Result::Error(error) => Ok(Err(error)),
        }
    }
}
