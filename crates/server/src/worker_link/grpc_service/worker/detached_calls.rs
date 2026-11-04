//! Detached tool calls: a call a worker took off its turn so the turn parks
//! while the call runs (knowledge/execution/detached-tool-calls.md).
//!
//! Dedicated RPCs like the session actor ones next door: the renew covers
//! every call one worker holds, across orgs. The in-process path
//! (`DirectWorkerAdapters`) calls the same repository methods.

use std::time::Duration;

use crate::worker_link::grpc_service::worker::support::internal_status;
use crate::worker_link::grpc_service::*;
use everruns_contracts::typed_id::SessionId;

/// Record life when a request passes none.
const DEFAULT_DETACHED_CALL_TTL: Duration = Duration::from_secs(30);

fn requested_ttl(ttl_ms: u32) -> Duration {
    if ttl_ms == 0 {
        DEFAULT_DETACHED_CALL_TTL
    } else {
        Duration::from_millis(u64::from(ttl_ms))
    }
}

fn require_holder(holder: &str) -> Result<(), Status> {
    if holder.trim().is_empty() {
        return Err(Status::invalid_argument("Missing holder"));
    }
    Ok(())
}

fn call_key(call: Option<&proto::DetachedToolCallKey>) -> Result<(SessionId, String), Status> {
    let call = call.ok_or_else(|| Status::invalid_argument("Missing call"))?;
    if call.tool_call_id.is_empty() {
        return Err(Status::invalid_argument("Missing tool_call_id"));
    }
    Ok((
        SessionId::from_uuid(parse_uuid(call.session_id.as_ref())?),
        call.tool_call_id.clone(),
    ))
}

fn call_keys(calls: &[proto::DetachedToolCallKey]) -> Result<Vec<(SessionId, String)>, Status> {
    calls.iter().map(|call| call_key(Some(call))).collect()
}

impl WorkerServiceImpl {
    pub(crate) async fn handle_detach_tool_call(
        &self,
        request: Request<proto::DetachToolCallRequest>,
    ) -> Result<Response<proto::DetachToolCallResponse>, Status> {
        let req = request.into_inner();
        require_holder(&req.holder)?;
        let (session_id, tool_call_id) = call_key(req.call.as_ref())?;
        let turn_id = parse_uuid(req.turn_id.as_ref())?;
        let detached = self
            .db
            .detach_tool_call(
                session_id,
                &tool_call_id,
                turn_id,
                &req.holder,
                req.fence,
                requested_ttl(req.ttl_ms),
            )
            .await
            .map_err(|e| internal_status("Failed to detach tool call", e))?;
        Ok(Response::new(proto::DetachToolCallResponse { detached }))
    }

    pub(crate) async fn handle_renew_detached_tool_calls(
        &self,
        request: Request<proto::RenewDetachedToolCallsRequest>,
    ) -> Result<Response<proto::RenewDetachedToolCallsResponse>, Status> {
        let req = request.into_inner();
        require_holder(&req.holder)?;
        let live = self
            .db
            .renew_detached_tool_calls(
                &req.holder,
                requested_ttl(req.ttl_ms),
                &call_keys(&req.calls)?,
            )
            .await
            .map_err(|e| internal_status("Failed to renew detached tool calls", e))?;
        Ok(Response::new(proto::RenewDetachedToolCallsResponse {
            live: live
                .into_iter()
                .map(|(session_id, tool_call_id)| proto::DetachedToolCallKey {
                    session_id: Some(everruns_internal_protocol::uuid_to_proto_uuid(
                        session_id.uuid(),
                    )),
                    tool_call_id,
                })
                .collect(),
        }))
    }

    pub(crate) async fn handle_complete_detached_tool_call(
        &self,
        request: Request<proto::CompleteDetachedToolCallRequest>,
    ) -> Result<Response<proto::CompleteDetachedToolCallResponse>, Status> {
        let req = request.into_inner();
        require_holder(&req.holder)?;
        let (session_id, tool_call_id) = call_key(req.call.as_ref())?;
        let completed = self
            .db
            .complete_detached_tool_call(session_id, &tool_call_id, &req.holder)
            .await
            .map_err(|e| internal_status("Failed to complete detached tool call", e))?;
        Ok(Response::new(proto::CompleteDetachedToolCallResponse {
            completed: completed.is_some(),
            resumable: completed.unwrap_or(false),
        }))
    }

    pub(crate) async fn handle_expire_detached_tool_calls(
        &self,
        request: Request<proto::ExpireDetachedToolCallsRequest>,
    ) -> Result<Response<proto::ExpireDetachedToolCallsResponse>, Status> {
        let req = request.into_inner();
        require_holder(&req.holder)?;
        self.db
            .expire_detached_tool_calls(&req.holder, &call_keys(&req.calls)?)
            .await
            .map_err(|e| internal_status("Failed to expire detached tool calls", e))?;
        Ok(Response::new(proto::ExpireDetachedToolCallsResponse {}))
    }
}
