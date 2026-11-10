//! Engine sessions over the worker link (data plane v2 step 4): the Engine's
//! session state on the session actor row, every write fenced by the lease.
//!
//! Decision: dedicated RPCs like the actor lease RPCs, for the same reason:
//! they act on a lease, not as an org. The bearer token on the worker link is
//! the authorization. The in-process path calls the same repository methods.

use crate::domains::agent_definitions::sessions::{EngineClaim, claim};
use crate::worker_link::grpc_service::worker::support::internal_status;
use crate::worker_link::grpc_service::*;
use everruns_contracts::typed_id::SessionId;

/// Most receipts one prefix read returns. A session keeps a few per turn it
/// has open or waiting; a scan past this is a bug, not a page.
const MAX_ENGINE_RECORDS: i64 = 1000;

fn fence(fence: Option<&proto::ActorFence>) -> Result<(&str, i64), Status> {
    let fence = fence.ok_or_else(|| Status::invalid_argument("Missing fence"))?;
    if fence.holder.trim().is_empty() {
        return Err(Status::invalid_argument("Missing holder"));
    }
    Ok((fence.holder.as_str(), fence.fence))
}

fn session(id: Option<&proto::Uuid>) -> Result<SessionId, Status> {
    Ok(SessionId::from_uuid(parse_uuid(id)?))
}

fn json(text: &str) -> Result<serde_json::Value, Status> {
    serde_json::from_str(text).map_err(|_| Status::invalid_argument("Invalid JSON"))
}

fn write(held: bool, written: bool) -> Response<proto::EngineSessionWriteResponse> {
    Response::new(proto::EngineSessionWriteResponse { held, written })
}

impl WorkerServiceImpl {
    pub(crate) async fn handle_claim_engine_session(
        &self,
        request: Request<proto::ClaimEngineSessionRequest>,
    ) -> Result<Response<proto::ClaimEngineSessionResponse>, Status> {
        let req = request.into_inner();
        let session_id = session(req.session_id.as_ref())?;
        let (holder, fence) = fence(req.fence.as_ref())?;
        let claimed = claim(&self.db, session_id, holder, fence)
            .await
            .map_err(|e| internal_status("Failed to claim Engine session", e))?;
        Ok(Response::new(match claimed {
            EngineClaim::Lost => proto::ClaimEngineSessionResponse {
                binding: None,
                lost: true,
            },
            EngineClaim::Legacy => proto::ClaimEngineSessionResponse {
                binding: None,
                lost: false,
            },
            EngineClaim::Bound(session) => proto::ClaimEngineSessionResponse {
                binding: Some(proto::EngineSessionBinding {
                    agent_key: session.binding.agent_key,
                    revision: session.binding.revision,
                    runtime: u32::try_from(session.binding.runtime).unwrap_or(0),
                    suspended_reason: session.binding.suspended_reason,
                    wake_at_ms: session.binding.wake_at.map(|at| at.timestamp_millis()),
                    configuration_json: session.configuration.to_string(),
                    created_at: session.created_at.to_rfc3339(),
                }),
                lost: false,
            },
        }))
    }

    pub(crate) async fn handle_get_engine_agent_definition(
        &self,
        request: Request<proto::GetEngineAgentDefinitionRequest>,
    ) -> Result<Response<proto::GetEngineAgentDefinitionResponse>, Status> {
        let req = request.into_inner();
        let definition = self
            .db
            .engine_agent_definition(&req.agent_key, &req.revision)
            .await
            .map_err(|e| internal_status("Failed to read Engine agent definition", e))?;
        Ok(Response::new(proto::GetEngineAgentDefinitionResponse {
            runtime_agent_json: definition.map(|value| value.to_string()),
        }))
    }

    pub(crate) async fn handle_read_engine_session_records(
        &self,
        request: Request<proto::ReadEngineSessionRecordsRequest>,
    ) -> Result<Response<proto::ReadEngineSessionRecordsResponse>, Status> {
        let req = request.into_inner();
        let session_id = session(req.session_id.as_ref())?;
        let records = match req.key {
            Some(key) => self
                .db
                .engine_session_record(session_id, &key)
                .await
                .map(|value| value.map(|value| (key, value)).into_iter().collect())
                .map_err(|e| internal_status("Failed to read Engine receipt", e))?,
            None => self
                .db
                .engine_session_records(session_id, &req.prefix, MAX_ENGINE_RECORDS)
                .await
                .map_err(|e| internal_status("Failed to read Engine receipts", e))?,
        };
        Ok(Response::new(proto::ReadEngineSessionRecordsResponse {
            records: records
                .into_iter()
                .map(|(key, value)| proto::EngineSessionRecord {
                    key,
                    value_json: value.to_string(),
                })
                .collect(),
        }))
    }

    pub(crate) async fn handle_commit_engine_session_record(
        &self,
        request: Request<proto::CommitEngineSessionRecordRequest>,
    ) -> Result<Response<proto::EngineSessionWriteResponse>, Status> {
        let req = request.into_inner();
        let session_id = session(req.session_id.as_ref())?;
        let (holder, fence) = fence(req.fence.as_ref())?;
        let record = req
            .record
            .ok_or_else(|| Status::invalid_argument("Missing record"))?;
        let value = json(&record.value_json)?;
        let outcome = self
            .db
            .commit_engine_session_record(
                session_id,
                holder,
                fence,
                &record.key,
                &value,
                req.absent,
            )
            .await
            .map_err(|e| internal_status("Failed to write Engine receipt", e))?;
        Ok(write(outcome.is_some(), outcome.unwrap_or(false)))
    }

    pub(crate) async fn handle_suspend_engine_session(
        &self,
        request: Request<proto::SuspendEngineSessionRequest>,
    ) -> Result<Response<proto::EngineSessionWriteResponse>, Status> {
        let req = request.into_inner();
        let session_id = session(req.session_id.as_ref())?;
        let (holder, fence) = fence(req.fence.as_ref())?;
        let held = self
            .db
            .suspend_engine_session(session_id, holder, fence, &req.reason)
            .await
            .map_err(|e| internal_status("Failed to suspend Engine session", e))?;
        Ok(write(held, held))
    }

    pub(crate) async fn handle_schedule_engine_session(
        &self,
        request: Request<proto::ScheduleEngineSessionRequest>,
    ) -> Result<Response<proto::EngineSessionWriteResponse>, Status> {
        let req = request.into_inner();
        let session_id = session(req.session_id.as_ref())?;
        let (holder, fence) = fence(req.fence.as_ref())?;
        let wake_at = req
            .wake_at_ms
            .map(|ms| {
                chrono::DateTime::from_timestamp_millis(ms)
                    .ok_or_else(|| Status::invalid_argument("Invalid wake time"))
            })
            .transpose()?;
        let held = self
            .db
            .schedule_engine_session(session_id, holder, fence, wake_at)
            .await
            .map_err(|e| internal_status("Failed to schedule Engine session", e))?;
        Ok(write(held, held))
    }
}
