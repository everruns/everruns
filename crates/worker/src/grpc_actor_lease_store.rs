//! [`ActorLeaseStore`] over the worker link: the gRPC side of the session
//! actor RPCs (`LeaseActors`, `RenewActorLease`, `ReleaseActor`,
//! `ReadActorJournal`) and the detached tool call RPCs.

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{SessionId, TurnId};
use everruns_internal_protocol::proto;
use uuid::Uuid;

use crate::actor_lease_store::{
    ActorJournal, ActorLeaseStore, ActorRelease, ActorRenewal, DetachedCallKey, LeasedActor,
    SessionActorStore,
};
use crate::grpc_adapters::{
    GrpcClient, grpc_missing_field, grpc_status_to_error, proto_event_to_core, uuid_to_proto,
};

/// A lease life as the wire carries it: whole milliseconds, at least one, so
/// a short TTL never reads as "server default" (0).
fn ttl_ms(ttl: std::time::Duration) -> u32 {
    u32::try_from(ttl.as_millis()).unwrap_or(u32::MAX).max(1)
}

fn fence_to_wire(fence: u64) -> Result<i64> {
    i64::try_from(fence).map_err(|_| AgentLoopError::store(format!("fence {fence} out of range")))
}

fn fence_from_wire(fence: i64) -> Result<u64> {
    u64::try_from(fence)
        .map_err(|_| AgentLoopError::store(format!("gRPC response error: negative fence {fence}")))
}

/// [`ActorLeaseStore`] over the worker link.
#[derive(Clone)]
pub struct GrpcActorLeaseStore {
    client: GrpcClient,
}

impl GrpcActorLeaseStore {
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ActorLeaseStore for GrpcActorLeaseStore {
    async fn lease_actors(
        &self,
        holder: &str,
        limit: u32,
        ttl: std::time::Duration,
        max_leased_per_org: Option<u32>,
    ) -> Result<Vec<LeasedActor>> {
        let response = self
            .client
            .inner
            .client()
            .lease_actors(proto::LeaseActorsRequest {
                holder: holder.to_string(),
                limit,
                ttl_ms: ttl_ms(ttl),
                max_leased_per_org: max_leased_per_org.unwrap_or(0),
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        response
            .actors
            .into_iter()
            .map(|actor| {
                let session_id = actor
                    .session_id
                    .as_ref()
                    .ok_or_else(|| grpc_missing_field("leased actor session_id"))?;
                let session_id = Uuid::parse_str(&session_id.value).map_err(|e| {
                    AgentLoopError::store(format!("Invalid leased actor session_id: {e}"))
                })?;
                Ok(LeasedActor {
                    session_id: SessionId::from_uuid(session_id),
                    org_id: actor.org_id,
                    fence: fence_from_wire(actor.fence)?,
                    inbox_seq: actor.inbox_seq,
                    cancel_inbox_seq: cancel_target(actor.cancel_requested, actor.cancel_inbox_seq),
                    consumed_seq: actor.consumed_seq,
                    next_input_seq: actor.next_input_seq,
                    attempts: actor.attempts,
                })
            })
            .collect()
    }

    async fn detach_tool_call(
        &self,
        call: &DetachedCallKey,
        turn_id: TurnId,
        holder: &str,
        fence: u64,
        ttl: std::time::Duration,
    ) -> Result<bool> {
        let response = self
            .client
            .inner
            .client()
            .detach_tool_call(proto::DetachToolCallRequest {
                call: Some(call_to_wire(call)),
                turn_id: Some(uuid_to_proto(turn_id.uuid())),
                holder: holder.to_string(),
                fence: fence_to_wire(fence)?,
                ttl_ms: ttl_ms(ttl),
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        Ok(response.detached)
    }

    async fn renew_detached_tool_calls(
        &self,
        holder: &str,
        ttl: std::time::Duration,
        calls: &[DetachedCallKey],
    ) -> Result<Vec<DetachedCallKey>> {
        let response = self
            .client
            .inner
            .client()
            .renew_detached_tool_calls(proto::RenewDetachedToolCallsRequest {
                holder: holder.to_string(),
                ttl_ms: ttl_ms(ttl),
                calls: calls.iter().map(call_to_wire).collect(),
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        response.live.into_iter().map(call_from_wire).collect()
    }

    async fn complete_detached_tool_call(
        &self,
        call: &DetachedCallKey,
        holder: &str,
    ) -> Result<Option<bool>> {
        let response = self
            .client
            .inner
            .client()
            .complete_detached_tool_call(proto::CompleteDetachedToolCallRequest {
                call: Some(call_to_wire(call)),
                holder: holder.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        Ok(response.completed.then_some(response.resumable))
    }

    async fn expire_detached_tool_calls(
        &self,
        holder: &str,
        calls: &[DetachedCallKey],
    ) -> Result<()> {
        self.client
            .inner
            .client()
            .expire_detached_tool_calls(proto::ExpireDetachedToolCallsRequest {
                holder: holder.to_string(),
                calls: calls.iter().map(call_to_wire).collect(),
            })
            .await
            .map_err(grpc_status_to_error)?;
        Ok(())
    }
}

fn call_to_wire(call: &DetachedCallKey) -> proto::DetachedToolCallKey {
    proto::DetachedToolCallKey {
        session_id: Some(uuid_to_proto(call.session_id.uuid())),
        tool_call_id: call.tool_call_id.clone(),
    }
}

fn call_from_wire(call: proto::DetachedToolCallKey) -> Result<DetachedCallKey> {
    let session_id = call
        .session_id
        .as_ref()
        .ok_or_else(|| grpc_missing_field("detached call session_id"))?;
    let session_id = Uuid::parse_str(&session_id.value)
        .map_err(|e| AgentLoopError::store(format!("Invalid detached call session_id: {e}")))?;
    Ok(DetachedCallKey {
        session_id: SessionId::from_uuid(session_id),
        tool_call_id: call.tool_call_id,
    })
}

#[async_trait]
impl SessionActorStore for GrpcActorLeaseStore {
    async fn renew_actor_lease(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        ttl: std::time::Duration,
        consumed_seq: i64,
    ) -> Result<Option<ActorRenewal>> {
        let response = self
            .client
            .inner
            .client()
            .renew_actor_lease(proto::RenewActorLeaseRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                holder: holder.to_string(),
                fence: fence_to_wire(fence)?,
                ttl_ms: ttl_ms(ttl),
                consumed_seq,
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        Ok(response.held.then_some(ActorRenewal {
            inbox_seq: response.inbox_seq,
            cancel_inbox_seq: cancel_target(response.cancel_requested, response.cancel_inbox_seq),
        }))
    }

    async fn release_actor(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        consumed_inbox_seq: i64,
    ) -> Result<ActorRelease> {
        let response = self
            .client
            .inner
            .client()
            .release_actor(proto::ReleaseActorRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                holder: holder.to_string(),
                fence: fence_to_wire(fence)?,
                consumed_inbox_seq,
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        Ok(if response.released {
            ActorRelease::Released {
                still_runnable: response.still_runnable,
            }
        } else {
            ActorRelease::LeaseLost
        })
    }

    async fn hand_off_actor(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        consumed_inbox_seq: i64,
    ) -> Result<Option<ActorRelease>> {
        let response = match self
            .client
            .inner
            .client()
            .hand_off_actor(proto::ReleaseActorRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                holder: holder.to_string(),
                fence: fence_to_wire(fence)?,
                consumed_inbox_seq,
            })
            .await
        {
            Ok(response) => response.into_inner(),
            // A server from before hand-off: the lease expires instead.
            Err(status) if status.code() == tonic::Code::Unimplemented => return Ok(None),
            Err(status) => return Err(grpc_status_to_error(status)),
        };
        Ok(Some(if response.released {
            ActorRelease::Released {
                still_runnable: true,
            }
        } else {
            ActorRelease::LeaseLost
        }))
    }

    async fn read_actor_journal(
        &self,
        session_id: SessionId,
        after_sequence: Option<i32>,
        limit: u32,
    ) -> Result<ActorJournal> {
        let response = self
            .client
            .inner
            .client()
            .read_actor_journal(proto::ReadActorJournalRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                after_sequence,
                limit,
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        Ok(ActorJournal {
            events: response
                .events
                .into_iter()
                .map(proto_event_to_core)
                .collect::<Result<_>>()?,
            has_more: response.has_more,
            head_sequence: response.head_sequence,
        })
    }
}

/// The inbox position a reported cancel targets. A control plane that predates
/// the position reports none: its cancel targets whatever turn runs, as it did.
fn cancel_target(cancel_requested: bool, cancel_inbox_seq: Option<i64>) -> Option<i64> {
    cancel_requested.then(|| cancel_inbox_seq.unwrap_or(i64::MAX))
}
