//! The control plane's Engine session authority, as a worker sees it (data
//! plane v2 step 4): a leased session's Engine binding and the Engine's
//! session state on its actor row, every write fenced by the lease.
//!
//! The gRPC worker link implements [`EngineSessions`] in [`GrpcEngineSessions`];
//! the in-process worker calls the same repository methods directly.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};
use everruns_internal_protocol::proto;
use serde_json::Value;

use crate::grpc_adapters::{GrpcClient, grpc_status_to_error, uuid_to_proto};

/// A claimed session's Engine binding. Names no platform record.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineBinding {
    pub agent_key: String,
    pub revision: String,
    /// The session runtime format the binding was created with.
    pub runtime: u32,
    pub suspended_reason: Option<String>,
    /// Milliseconds since the epoch.
    pub wake_at_ms: Option<i64>,
    /// The Engine's `SessionConfiguration` JSON: the session's model and
    /// session capabilities.
    pub configuration: Value,
    /// RFC 3339.
    pub created_at: String,
    /// The platform agent and harness the session's events name. The
    /// worker's event port stamps them; the Engine never sees them.
    pub event_agent_id: Option<AgentId>,
    pub event_harness_id: Option<HarnessId>,
    /// The platform workspace the session's files live in.
    pub files_workspace_id: uuid::Uuid,
}

/// The answer to [`EngineSessions::claim`].
#[derive(Debug, Clone, PartialEq)]
pub enum EngineClaim {
    /// The lease is not the caller's any more: stop without writing.
    Lost,
    /// Run this lease on the worker's own host.
    Legacy,
    /// Run it on the Engine.
    Bound(Box<EngineBinding>),
}

/// The lease a write presents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineFence {
    pub holder: String,
    pub fence: u64,
}

/// Engine session state over the control plane.
#[async_trait]
pub trait EngineSessions: Send + Sync + 'static {
    /// Claim `session`'s binding for the lease `fence` names: between turns
    /// it moves to its key's current revision; an unbound session of an
    /// enrolled org binds here.
    async fn claim(&self, session: SessionId, fence: &EngineFence) -> Result<EngineClaim>;
    /// A published revision's `RuntimeAgent` declaration.
    async fn definition(&self, agent_key: &str, revision: &str) -> Result<Option<Value>>;
    /// One receipt.
    async fn record(&self, session: SessionId, key: &str) -> Result<Option<Value>>;
    /// Receipts whose key starts with `prefix`, in key order.
    async fn records(&self, session: SessionId, prefix: &str) -> Result<Vec<(String, Value)>>;
    /// Write a receipt (with `absent`, only a new key). `None` when the lease
    /// is lost; otherwise whether it was written.
    async fn commit(
        &self,
        session: SessionId,
        fence: &EngineFence,
        key: &str,
        value: &Value,
        absent: bool,
    ) -> Result<Option<bool>>;
    /// Suspend the session. Whether the lease was held.
    async fn suspend(&self, session: SessionId, fence: &EngineFence, reason: &str) -> Result<bool>;
    /// Set or clear the session's timer. Whether the lease was held.
    async fn schedule(
        &self,
        session: SessionId,
        fence: &EngineFence,
        wake_at_ms: Option<i64>,
    ) -> Result<bool>;
}

/// The leases the control plane routed to the Engine
/// (`LeasedActor.engine`). The lease store records them as it leases; the
/// actor loop takes each one as it starts the lease.
///
/// Decision: a side table rather than a field of the shared `LeasedActor`,
/// which the Framework's hosting contract exposes and which must not carry
/// a platform routing decision.
#[derive(Default)]
pub struct EngineRoutes(Mutex<HashSet<(SessionId, u64)>>);

impl EngineRoutes {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    /// The lease of `session` at `fence` runs on the Engine.
    pub fn route(&self, session: SessionId, fence: u64) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert((session, fence));
    }

    /// Whether the lease runs on the Engine; forgets it either way.
    pub fn take(&self, session: SessionId, fence: u64) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&(session, fence))
    }
}

/// A worker's link to the Engine session authority: the authority, and the
/// table its lease store records Engine routes in.
#[derive(Clone)]
pub struct EngineLink {
    pub sessions: Arc<dyn EngineSessions>,
    pub routes: Arc<EngineRoutes>,
}

fn fence_to_wire(fence: &EngineFence) -> Result<proto::ActorFence> {
    Ok(proto::ActorFence {
        holder: fence.holder.clone(),
        fence: i64::try_from(fence.fence)
            .map_err(|_| AgentLoopError::store(format!("fence {} out of range", fence.fence)))?,
        detached_tool_call_id: None,
    })
}

fn json(text: &str) -> Result<Value> {
    serde_json::from_str(text)
        .map_err(|e| AgentLoopError::store(format!("invalid Engine session JSON: {e}")))
}

/// [`EngineSessions`] over the worker link.
#[derive(Clone)]
pub struct GrpcEngineSessions {
    client: GrpcClient,
}

impl GrpcEngineSessions {
    pub fn new(client: GrpcClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl EngineSessions for GrpcEngineSessions {
    async fn claim(&self, session: SessionId, fence: &EngineFence) -> Result<EngineClaim> {
        let response = self
            .client
            .inner
            .client()
            .claim_engine_session(proto::ClaimEngineSessionRequest {
                session_id: Some(uuid_to_proto(session.uuid())),
                fence: Some(fence_to_wire(fence)?),
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        if response.lost {
            return Ok(EngineClaim::Lost);
        }
        let Some(binding) = response.binding else {
            return Ok(EngineClaim::Legacy);
        };
        Ok(EngineClaim::Bound(Box::new(EngineBinding {
            agent_key: binding.agent_key,
            revision: binding.revision,
            runtime: binding.runtime,
            suspended_reason: binding.suspended_reason,
            wake_at_ms: binding.wake_at_ms,
            configuration: json(&binding.configuration_json)?,
            created_at: binding.created_at,
            event_agent_id: binding
                .event_agent_id
                .as_deref()
                .map(str::parse)
                .transpose()
                .map_err(|_| AgentLoopError::store("invalid event agent id"))?,
            event_harness_id: binding
                .event_harness_id
                .as_deref()
                .map(str::parse)
                .transpose()
                .map_err(|_| AgentLoopError::store("invalid event harness id"))?,
            files_workspace_id: binding
                .files_workspace_id
                .parse()
                .map_err(|_| AgentLoopError::store("invalid files workspace id"))?,
        })))
    }

    async fn definition(&self, agent_key: &str, revision: &str) -> Result<Option<Value>> {
        let response = self
            .client
            .inner
            .client()
            .get_engine_agent_definition(proto::GetEngineAgentDefinitionRequest {
                agent_key: agent_key.to_string(),
                revision: revision.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        response.runtime_agent_json.as_deref().map(json).transpose()
    }

    async fn record(&self, session: SessionId, key: &str) -> Result<Option<Value>> {
        let records = self.read(session, Some(key), "").await?;
        Ok(records.into_iter().next().map(|(_, value)| value))
    }

    async fn records(&self, session: SessionId, prefix: &str) -> Result<Vec<(String, Value)>> {
        self.read(session, None, prefix).await
    }

    async fn commit(
        &self,
        session: SessionId,
        fence: &EngineFence,
        key: &str,
        value: &Value,
        absent: bool,
    ) -> Result<Option<bool>> {
        let response = self
            .client
            .inner
            .client()
            .commit_engine_session_record(proto::CommitEngineSessionRecordRequest {
                session_id: Some(uuid_to_proto(session.uuid())),
                fence: Some(fence_to_wire(fence)?),
                record: Some(proto::EngineSessionRecord {
                    key: key.to_string(),
                    value_json: value.to_string(),
                }),
                absent,
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        Ok(response.held.then_some(response.written))
    }

    async fn suspend(&self, session: SessionId, fence: &EngineFence, reason: &str) -> Result<bool> {
        let response = self
            .client
            .inner
            .client()
            .suspend_engine_session(proto::SuspendEngineSessionRequest {
                session_id: Some(uuid_to_proto(session.uuid())),
                fence: Some(fence_to_wire(fence)?),
                reason: reason.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        Ok(response.held)
    }

    async fn schedule(
        &self,
        session: SessionId,
        fence: &EngineFence,
        wake_at_ms: Option<i64>,
    ) -> Result<bool> {
        let response = self
            .client
            .inner
            .client()
            .schedule_engine_session(proto::ScheduleEngineSessionRequest {
                session_id: Some(uuid_to_proto(session.uuid())),
                fence: Some(fence_to_wire(fence)?),
                wake_at_ms,
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        Ok(response.held)
    }
}

impl GrpcEngineSessions {
    async fn read(
        &self,
        session: SessionId,
        key: Option<&str>,
        prefix: &str,
    ) -> Result<Vec<(String, Value)>> {
        let response = self
            .client
            .inner
            .client()
            .read_engine_session_records(proto::ReadEngineSessionRecordsRequest {
                session_id: Some(uuid_to_proto(session.uuid())),
                key: key.map(str::to_string),
                prefix: prefix.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();
        response
            .records
            .into_iter()
            .map(|record| Ok((record.key, json(&record.value_json)?)))
            .collect()
    }
}
