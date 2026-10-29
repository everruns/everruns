//! Customer-hosted execution is a separate trust boundary, not a flavor of
//! `Caller::internal`. Never add a forwarding catch-all to the trusted service.

use std::{sync::Arc, time::Duration};

use everruns_contracts::typed_id::SessionId;
use everruns_internal_protocol::{proto, proto_uuid_to_uuid, uuid_to_proto_uuid};
use proto::custom_worker_service_server::CustomWorkerService;
use sha2::{Digest, Sha256};
use tonic::{Request, Response, Status};

use crate::{services::EventService, storage::StorageBackend};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod ingress_floor_tests;

#[derive(Clone)]
pub struct CustomWorkerServiceImpl {
    db: Arc<StorageBackend>,
    events: Arc<EventService>,
}

#[derive(sqlx::FromRow)]
struct Deployment {
    id: uuid::Uuid,
    org_id: i64,
    agent_id: uuid::Uuid,
}

fn unavailable(error: impl std::fmt::Display) -> Status {
    tracing::error!(%error, "custom worker storage operation failed");
    Status::internal("Custom worker storage unavailable")
}

fn session_id(value: Option<&proto::Uuid>) -> Result<SessionId, Status> {
    let value = value.ok_or_else(|| Status::invalid_argument("Missing session id"))?;
    proto_uuid_to_uuid(value)
        .map(SessionId::from_uuid)
        .map_err(|_| Status::invalid_argument("Invalid session id"))
}

fn validate_holder(holder: &str) -> Result<(), Status> {
    if holder.is_empty()
        || holder.len() > 128
        || !holder
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
    {
        return Err(Status::invalid_argument(
            "Holder must be 1-128 ASCII letters, digits, hyphens or underscores",
        ));
    }
    Ok(())
}

impl Deployment {
    // A customer-chosen holder can never collide with a trusted worker or
    // another deployment, even if the customer guesses its worker id.
    fn holder(&self, supplied: &str) -> Result<String, Status> {
        validate_holder(supplied)?;
        Ok(format!("custom:{}:{supplied}", self.id))
    }
}

impl CustomWorkerServiceImpl {
    pub fn new(db: Arc<StorageBackend>, events: Arc<EventService>) -> Self {
        Self { db, events }
    }

    pub fn into_server(
        self,
    ) -> proto::custom_worker_service_server::CustomWorkerServiceServer<Self> {
        proto::custom_worker_service_server::CustomWorkerServiceServer::new(self)
            .max_decoding_message_size(1024 * 1024)
            .max_encoding_message_size(everruns_internal_protocol::MAX_GRPC_MESSAGE_SIZE)
    }

    async fn authenticate<T>(&self, request: &Request<T>) -> Result<Deployment, Status> {
        let token = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .filter(|v| (32..=256).contains(&v.len()))
            .ok_or_else(|| Status::unauthenticated("Custom worker credential required"))?;
        // THREAT[TM-AUTHZ-002]: deployment tokens and live leases are not
        // verified assignment authority. This separate listener must not
        // reopen the protected plane. Recheck each call, including reconnects.
        if self
            .db
            .worker_ingress_minimum_version()
            .await
            .map_err(unavailable)?
            .is_some()
        {
            return Err(Status::permission_denied(
                "Protected worker ingress does not support custom worker authority",
            ));
        }
        // Only a digest is stored in the CP. No reusable infrastructure
        // credential is ever given to a customer worker.
        let digest = Sha256::digest(token.as_bytes());
        sqlx::query_as::<_, Deployment>(
            "SELECT id, org_id, agent_id FROM custom_workers WHERE token_hash = $1 AND enabled",
        )
        .bind(digest.as_slice())
        .fetch_optional(self.db.pool())
        .await
        .map_err(unavailable)?
        .ok_or_else(|| Status::unauthenticated("Invalid custom worker credential"))
    }

    async fn set_status(
        &self,
        deployment: &Deployment,
        session: SessionId,
        holder: &str,
        fence: i64,
        status: &str,
    ) -> Result<(), Status> {
        // A stalled writer must not change the replacement holder's status.
        sqlx::query("UPDATE sessions s SET status = $1, updated_at = NOW() WHERE s.id = $2 AND s.org_id = $3 AND EXISTS (SELECT 1 FROM session_actors a WHERE a.session_id = s.id AND a.lease_holder = $4 AND a.lease_fence = $5 AND a.lease_expires_at > NOW())")
            .bind(status).bind(session).bind(deployment.org_id).bind(holder).bind(fence)
            .execute(self.db.pool()).await.map_err(unavailable)?;
        Ok(())
    }

    async fn authorize_lease(
        &self,
        deployment: &Deployment,
        session: SessionId,
        fence: Option<&proto::ActorFence>,
    ) -> Result<String, Status> {
        let fence = fence.ok_or_else(|| Status::permission_denied("Live actor lease required"))?;
        if fence.detached_tool_call_id.is_some() {
            return Err(Status::permission_denied(
                "Detached leases are not supported",
            ));
        }
        let holder = deployment.holder(&fence.holder)?;
        let authorized: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM session_actors a JOIN sessions s ON s.id = a.session_id \
             WHERE s.id = $1 AND s.org_id = $2 AND s.agent_id = $3 \
             AND a.lease_holder = $4 AND a.lease_fence = $5 AND a.lease_expires_at > NOW())",
        )
        .bind(session)
        .bind(deployment.org_id)
        .bind(deployment.agent_id)
        .bind(&holder)
        .bind(fence.fence)
        .fetch_one(self.db.pool())
        .await
        .map_err(unavailable)?;
        if !authorized {
            return Err(Status::permission_denied(
                "Session is not held by this custom worker",
            ));
        }
        Ok(holder)
    }
}

#[tonic::async_trait]
impl CustomWorkerService for CustomWorkerServiceImpl {
    async fn lease_actors(
        &self,
        request: Request<proto::LeaseActorsRequest>,
    ) -> Result<Response<proto::LeaseActorsResponse>, Status> {
        let deployment = self.authenticate(&request).await?;
        let req = request.into_inner();
        let holder = deployment.holder(&req.holder)?;
        // Fixed limits: the customer cannot reserve an arbitrarily long lease
        // or use the trusted fleet's fairness controls.
        let rows = self
            .db
            .lease_session_actors_for_worker(
                &holder,
                i64::from(req.limit.min(10)),
                Duration::from_secs(30),
                32,
                Some(deployment.id),
            )
            .await
            .map_err(unavailable)?;
        Ok(Response::new(proto::LeaseActorsResponse {
            actors: rows
                .into_iter()
                .map(|r| proto::LeasedActor {
                    session_id: Some(uuid_to_proto_uuid(r.session_id.uuid())),
                    org_id: r.org_id,
                    fence: r.fence,
                    inbox_seq: r.inbox_seq,
                    consumed_seq: r.consumed_seq,
                    next_input_seq: r.next_input_seq,
                    attempts: r.attempts as u32,
                    cancel_requested: r.cancel_requested(),
                    cancel_inbox_seq: r.cancel_inbox_seq,
                })
                .collect(),
        }))
    }

    async fn renew_actor_lease(
        &self,
        request: Request<proto::RenewActorLeaseRequest>,
    ) -> Result<Response<proto::RenewActorLeaseResponse>, Status> {
        let deployment = self.authenticate(&request).await?;
        let req = request.into_inner();
        let session = session_id(req.session_id.as_ref())?;
        let holder = self
            .authorize_lease(
                &deployment,
                session,
                Some(&proto::ActorFence {
                    holder: req.holder,
                    fence: req.fence,
                    detached_tool_call_id: None,
                }),
            )
            .await?;
        let renewal = self
            .db
            .renew_session_actor_lease(
                session,
                &holder,
                req.fence,
                Duration::from_secs(30),
                req.consumed_seq,
            )
            .await
            .map_err(unavailable)?;
        Ok(Response::new(match renewal {
            Some(r) => proto::RenewActorLeaseResponse {
                held: true,
                inbox_seq: r.inbox_seq,
                cancel_requested: r.cancel_requested(),
                cancel_inbox_seq: r.cancel_inbox_seq,
            },
            None => proto::RenewActorLeaseResponse::default(),
        }))
    }

    async fn release_actor(
        &self,
        request: Request<proto::ReleaseActorRequest>,
    ) -> Result<Response<proto::ReleaseActorResponse>, Status> {
        let deployment = self.authenticate(&request).await?;
        let req = request.into_inner();
        let session = session_id(req.session_id.as_ref())?;
        let holder = self
            .authorize_lease(
                &deployment,
                session,
                Some(&proto::ActorFence {
                    holder: req.holder,
                    fence: req.fence,
                    detached_tool_call_id: None,
                }),
            )
            .await?;
        // Also repairs a crash after committing the terminal event but before
        // updating the disposable status projection.
        self.set_status(&deployment, session, &holder, req.fence, "idle")
            .await?;
        let released = self
            .db
            .release_session_actor(session, &holder, req.fence, req.consumed_inbox_seq)
            .await
            .map_err(unavailable)?;
        Ok(Response::new(proto::ReleaseActorResponse {
            released: released.is_some(),
            still_runnable: released.unwrap_or(false),
        }))
    }

    async fn read_journal(
        &self,
        request: Request<proto::CustomWorkerJournalRequest>,
    ) -> Result<Response<proto::ReadActorJournalResponse>, Status> {
        let deployment = self.authenticate(&request).await?;
        let req = request.into_inner();
        let journal = req
            .journal
            .ok_or_else(|| Status::invalid_argument("Missing journal request"))?;
        let session = session_id(journal.session_id.as_ref())?;
        self.authorize_lease(&deployment, session, req.actor_fence.as_ref())
            .await?;
        let rows = self
            .db
            .read_session_actor_journal(
                session,
                journal.after_sequence,
                i64::from(journal.limit.min(100)),
            )
            .await
            .map_err(unavailable)?;
        let mut events = Vec::new();
        let mut size = 0;
        let mut cut = false;
        for row in rows.events {
            let event =
                everruns_internal_protocol::schema_event_to_proto(&EventService::row_to_event(row));
            size += prost::Message::encoded_len(&event);
            if size > everruns_internal_protocol::MAX_GRPC_MESSAGE_SIZE / 4 && !events.is_empty() {
                cut = true;
                break;
            }
            events.push(event);
        }
        Ok(Response::new(proto::ReadActorJournalResponse {
            events,
            has_more: rows.has_more || cut,
            head_sequence: rows.head_sequence,
        }))
    }

    async fn emit_event(
        &self,
        request: Request<proto::EmitEventRequest>,
    ) -> Result<Response<proto::EmitEventResponse>, Status> {
        let deployment = self.authenticate(&request).await?;
        let req = request.into_inner();
        let event = req
            .event
            .ok_or_else(|| Status::invalid_argument("Missing event"))?;
        let mut event = everruns_internal_protocol::proto_event_request_to_schema(event)
            .map_err(|_| Status::invalid_argument("Invalid event"))?;
        validate_event(&event)?;
        let holder = self
            .authorize_lease(&deployment, event.session_id, req.actor_fence.as_ref())
            .await?;
        normalize_event(&deployment, &mut event)?;
        let input = event
            .context
            .input_message_id
            .ok_or_else(|| Status::invalid_argument("Missing input"))?;
        if !self
            .db
            .runtime_invocation_exists(event.session_id, input.uuid())
            .await
            .map_err(unavailable)?
        {
            return Err(Status::permission_denied("Unknown session invocation"));
        }
        let fence = req
            .actor_fence
            .ok_or_else(|| Status::permission_denied("Live actor lease required"))?;
        let status = match event.event_type.as_str() {
            "turn.started" => Some("active"),
            "turn.completed" | "turn.failed" | "turn.cancelled" => Some("idle"),
            _ => None,
        };
        let session = event.session_id;
        let mut batch = Vec::new();
        let turn = event
            .context
            .turn_id
            .ok_or_else(|| Status::invalid_argument("Missing turn"))?;
        if status == Some("active") {
            batch.push(everruns_core::EventRequest::new(
                session,
                event.context.clone(),
                everruns_core::SessionActivatedData {
                    turn_id: turn,
                    input_message_id: input,
                },
            ));
        } else if status == Some("idle") {
            batch.push(everruns_core::EventRequest::new(
                session,
                event.context.clone(),
                everruns_core::SessionIdledData {
                    turn_id: turn,
                    iterations: None,
                    usage: None,
                },
            ));
        }
        batch.push(event);
        let (_, stored) = self
            .events
            .emit_batch_fenced(
                batch,
                crate::services::ActorAppendFence {
                    holder: &holder,
                    fence: fence.fence,
                    detached_tool_call: None,
                },
            )
            .await
            .map_err(|error| {
                if error
                    .downcast_ref::<crate::storage::ActorLeaseFenceRejected>()
                    .is_some()
                {
                    Status::failed_precondition("Actor lease lost")
                } else {
                    unavailable(error)
                }
            })?;
        if let Some(status) = status {
            self.set_status(&deployment, session, &holder, fence.fence, status)
                .await?;
        }
        Ok(Response::new(proto::EmitEventResponse {
            event: stored
                .as_ref()
                .map(everruns_internal_protocol::schema_event_to_proto),
        }))
    }
}

fn validate_event(event: &everruns_core::EventRequest) -> Result<(), Status> {
    // Customer code reports execution only. Intake, approvals, identity,
    // configuration, schedules and resource mutations remain CP-owned.
    let allowed = matches!(
        event.event_type.as_str(),
        "turn.started"
            | "turn.completed"
            | "turn.failed"
            | "turn.cancelled"
            | "act.started"
            | "act.completed"
            | "reason.thinking.started"
            | "reason.thinking.completed"
            | "reason.started"
            | "reason.completed"
            | "reason.thinking.delta"
            | "reason.item"
            | "output.message.started"
            | "output.message.delta"
            | "output.message.completed"
            | "tool.started"
            | "tool.completed"
            | "tool.output.delta"
            | "tool.progress"
            | "llm.generation"
    );
    if !allowed
        || event.data.event_type() != event.event_type
        || event.context.turn_id.is_none()
        || event.context.input_message_id.is_none()
    {
        return Err(Status::permission_denied(
            "Event is not a custom worker execution event",
        ));
    }
    use everruns_core::EventData;
    let payload_turn = match &event.data {
        EventData::TurnStarted(d) => {
            if Some(d.input_message_id) != event.context.input_message_id {
                return Err(Status::invalid_argument("Input identity mismatch"));
            }
            Some(d.turn_id)
        }
        EventData::TurnCompleted(d) => Some(d.turn_id),
        EventData::TurnFailed(d) => Some(d.turn_id),
        EventData::TurnCancelled(d) => Some(d.turn_id),
        EventData::OutputMessageStarted(d) => Some(d.turn_id),
        EventData::OutputMessageDelta(d) => Some(d.turn_id),
        EventData::OutputMessageCompleted(d) => {
            if d.message.role != everruns_core::RuntimeMessageRole::Agent
                || d.message.external_actor.is_some()
            {
                return Err(Status::permission_denied("Only agent output is accepted"));
            }
            None
        }
        _ => None,
    };
    if payload_turn.is_some() && payload_turn != event.context.turn_id {
        return Err(Status::invalid_argument("Turn identity mismatch"));
    }
    Ok(())
}

/// Global observer span maps must never accept another deployment's correlation
/// ids. One input owns one custom turn; CP-derived ids survive replica restarts.
fn normalize_event(
    deployment: &Deployment,
    event: &mut everruns_core::EventRequest,
) -> Result<(), Status> {
    use everruns_contracts::typed_id::{ExecId, TurnId};
    let input = event
        .context
        .input_message_id
        .ok_or_else(|| Status::invalid_argument("Missing input"))?;
    let supplied_turn = event
        .context
        .turn_id
        .ok_or_else(|| Status::invalid_argument("Missing turn"))?;
    let scoped = |kind: &str, value: &str| {
        let mut hash = Sha256::new();
        hash.update(deployment.id.as_bytes());
        hash.update(event.session_id.uuid().as_bytes());
        hash.update(input.uuid().as_bytes());
        hash.update(kind.as_bytes());
        hash.update([0]);
        hash.update(value.as_bytes());
        let digest = hash.finalize();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        uuid::Uuid::from_bytes(bytes)
    };
    let turn = TurnId::from_uuid(scoped("turn", ""));
    let span = |value: &str| {
        if value == supplied_turn.to_string() || value == supplied_turn.uuid().to_string() {
            turn.to_string()
        } else {
            scoped("span", value).simple().to_string()
        }
    };
    let span_id = event.context.span_id.as_deref().map(span);
    let parent = event.context.parent_span_id.as_deref().map(span);
    let exec = event
        .context
        .exec_id
        .map(|id| ExecId::from_uuid(scoped("exec", &id.to_string())));
    let trace = scoped("trace", "").simple().to_string();
    event.context.turn_id = Some(turn);
    event.context.exec_id = exec;
    event.context.trace_id = Some(trace);
    event.context.span_id = span_id;
    event.context.parent_span_id = parent;
    // Customer compute uses customer-owned model access. Reported usage may
    // never masquerade as spend on a CP-managed provider account.
    if let everruns_core::EventData::LlmGeneration(data) = &mut event.data {
        data.metadata.provider_id = None;
    }
    let mut metadata = event
        .metadata
        .take()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    metadata.remove("everruns_provider_id");
    metadata.insert("custom_worker_id".into(), serde_json::json!(deployment.id));
    event.metadata = Some(serde_json::Value::Object(metadata));
    let mut data = serde_json::to_value(&event.data).map_err(unavailable)?;
    if data.get("turn_id").is_some() {
        data["turn_id"] = serde_json::json!(turn);
    }
    if event.event_type == "turn.started" {
        data["agent_id"] = serde_json::json!(everruns_contracts::typed_id::AgentId::from_uuid(
            deployment.agent_id
        ));
    }
    event.data = everruns_core::events::deserialize_event_data(&event.event_type, data);
    Ok(())
}
