//! [`SessionHosting`] over the control plane's session actors: the
//! execution-only profile a worker runs the Engine on (data plane v2 step 4).
//!
//! Decisions:
//! - Execution only. Commands, admissions and queries stay on the control
//!   plane's own code; this profile refuses them. It serves what activation
//!   needs: the claim, the binding, receipts, definitions and fenced ports.
//! - The claim hands over a lease the worker's batch loop already holds
//!   ([`PlatformHosting::offer`]), so running on the Engine costs no extra
//!   lease round trip. The Engine's driver names its own holder; every lease
//!   call goes out under this worker's ([`HolderStore`]).

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use everruns::hosting::spi::session_commands::{SessionCommand, SessionConfiguration};
use everruns::hosting::{
    AdmissionReceipt, AgentBuild, AgentDefinition, AgentDefinitions, HostingStatus, RuntimeAgent,
    SessionActivation, SessionBinding, SessionEpoch, SessionHosting, SessionQuerySnapshot,
};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{MessageId, SessionId};
use serde_json::Value;

use super::backends::{EventStamp, PlatformEventLog, PlatformSessionStore};
use super::deltas::PlatformDeltas;
use super::extensions::{PlatformTurn, with_platform_services};
use crate::actor_lease_store::{
    ActorFence, ActorJournal, ActorLeaseRenewal, ActorRelease, ActorRenewal, LeasedActor,
    SessionActorStore, actor_lease_lost,
};
use crate::core::host::HostBackends;
use crate::engine_sessions::{EngineBinding, EngineFence, EngineSessions};
use crate::worker_adapters::WorkerAdapters;

/// The Environment reference the Engine binds every defined agent's
/// sessions to (`DEFINITION_ENVIRONMENT` in the Engine).
pub const DEFINITION_ENVIRONMENT: &str = "agent-default";

/// A lease waiting for the Engine's claim.
struct Offer {
    actor: LeasedActor,
    binding: EngineBinding,
    signals: LeaseSignals,
}

/// The worker's signals to one lease: renew now (a cancel landed) and hand
/// off at the next step (the worker drains).
#[derive(Clone, Default)]
pub struct LeaseSignals {
    pub renew_poke: Option<Arc<tokio::sync::Notify>>,
    pub hand_off: Option<Arc<AtomicBool>>,
}

/// What the last claim of a session bound, for status and query reads.
#[derive(Clone)]
struct Claimed {
    binding: SessionBinding,
    configuration: SessionConfiguration,
    suspended_reason: Option<String>,
    wake_at_ms: Option<i64>,
    created_at: String,
}

/// The execution-only profile. One per worker; clones share it.
pub struct PlatformHosting<A: WorkerAdapters> {
    adapters: A,
    sessions: Arc<dyn EngineSessions>,
    leases: Arc<dyn SessionActorStore>,
    holder: String,
    offered: Arc<Mutex<HashMap<SessionId, Offer>>>,
    claimed: Arc<Mutex<HashMap<SessionId, Claimed>>>,
    definitions: Arc<PlatformDefinitions>,
}

impl<A: WorkerAdapters> Clone for PlatformHosting<A> {
    fn clone(&self) -> Self {
        Self {
            adapters: self.adapters.clone(),
            sessions: self.sessions.clone(),
            leases: self.leases.clone(),
            holder: self.holder.clone(),
            offered: self.offered.clone(),
            claimed: self.claimed.clone(),
            definitions: self.definitions.clone(),
        }
    }
}

impl<A: WorkerAdapters> PlatformHosting<A> {
    pub fn new(
        adapters: A,
        sessions: Arc<dyn EngineSessions>,
        leases: Arc<dyn SessionActorStore>,
        holder: impl Into<String>,
    ) -> Self {
        Self {
            adapters,
            definitions: Arc::new(PlatformDefinitions::new(sessions.clone())),
            sessions,
            leases,
            holder: holder.into(),
            offered: Arc::default(),
            claimed: Arc::default(),
        }
    }

    /// Hand a held lease, claimed with `binding`, to the next
    /// [`SessionHosting::claim`] of its session.
    pub fn offer(&self, actor: LeasedActor, binding: EngineBinding, signals: LeaseSignals) {
        self.offered
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                actor.session_id,
                Offer {
                    actor,
                    binding,
                    signals,
                },
            );
    }

    /// Forget what this worker knows of `session` once its activation ended.
    pub fn forget(&self, session: SessionId) {
        self.offered
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&session);
        self.claimed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&session);
    }

    fn claimed(&self, session: SessionId) -> Result<Claimed> {
        self.claimed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&session)
            .cloned()
            .ok_or(AgentLoopError::SessionNotFound(session))
    }

    /// The published revisions this profile reads.
    pub fn agent_definitions(&self) -> &PlatformDefinitions {
        &self.definitions
    }

    fn holder_store(&self) -> Arc<dyn SessionActorStore> {
        Arc::new(HolderStore {
            inner: self.leases.clone(),
            holder: self.holder.clone(),
        })
    }
}

fn execution_only() -> AgentLoopError {
    AgentLoopError::config(
        "this worker host only executes sessions; commands and queries go through the control plane",
    )
}

fn binding_of(binding: &EngineBinding) -> SessionBinding {
    SessionBinding {
        framework: binding.runtime,
        behavior: AgentBuild::new(binding.agent_key.clone(), binding.revision.clone()),
        harness: None,
        environment: Some(DEFINITION_ENVIRONMENT.to_string()),
        metadata: Value::Null,
    }
}

#[async_trait]
impl<A: WorkerAdapters> SessionHosting for PlatformHosting<A> {
    async fn command(&self, _session: SessionId, _command: SessionCommand) -> Result<Value> {
        Err(execution_only())
    }

    async fn query_snapshot(&self, session: SessionId) -> Result<SessionQuerySnapshot> {
        let claimed = self.claimed(session)?;
        Ok(SessionQuerySnapshot {
            configuration: claimed.configuration,
            binding: claimed.binding,
            high_watermark: 0,
            execution: None,
            rejected_inputs: Vec::new(),
        })
    }

    async fn status(&self, session: SessionId) -> Result<HostingStatus> {
        let claimed = self.claimed(session)?;
        Ok(HostingStatus {
            executing: true,
            now_ms: chrono::Utc::now().timestamp_millis(),
            wake_at: claimed.wake_at_ms,
            created_at: claimed.created_at,
            binding: claimed.binding,
            runnable: false,
            suspended: claimed.suspended_reason.is_some(),
            suspension_reason: claimed.suspended_reason,
            open_turn: None,
            waiting: Vec::new(),
        })
    }

    async fn receipt(
        &self,
        _session: SessionId,
        _message: MessageId,
    ) -> Result<Option<AdmissionReceipt>> {
        Err(execution_only())
    }

    fn reader(&self, session: SessionId) -> HostBackends {
        let _ = session;
        HostBackends::in_memory().with_event_log(Arc::new(PlatformEventLog::reader(
            self.adapters.clone(),
            self.leases.clone(),
        )))
    }

    async fn record(&self, session: SessionId, key: &str) -> Result<Option<Value>> {
        self.sessions.record(session, key).await
    }

    async fn claim(
        &self,
        session: SessionId,
        _holder: &str,
        _ttl: Duration,
    ) -> Result<Option<SessionActivation>> {
        let Some(offer) = self
            .offered
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&session)
        else {
            return Ok(None);
        };
        let binding = binding_of(&offer.binding);
        let configuration: SessionConfiguration =
            serde_json::from_value(offer.binding.configuration.clone()).map_err(|e| {
                AgentLoopError::store(format!("invalid session configuration: {e}"))
            })?;
        self.claimed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                session,
                Claimed {
                    binding: binding.clone(),
                    configuration: configuration.clone(),
                    suspended_reason: offer.binding.suspended_reason.clone(),
                    wake_at_ms: offer.binding.wake_at_ms,
                    created_at: offer.binding.created_at.clone(),
                },
            );
        let fence = Arc::new(Mutex::new(ActorFence::new(
            session,
            self.holder.clone(),
            offer.actor.fence,
        )));
        let stamp = EventStamp {
            agent_id: offer.binding.event_agent_id,
            harness_id: offer.binding.event_harness_id,
        };
        let deltas = PlatformDeltas::start(self.adapters.clone(), fence.clone(), stamp.clone());
        let backends = HostBackends::in_memory()
            .with_event_log(Arc::new(PlatformEventLog::writer(
                self.adapters.clone(),
                self.leases.clone(),
                fence.clone(),
                stamp,
            )))
            .with_session_store(Arc::new(PlatformSessionStore::new(
                self.adapters.clone(),
                offer.actor.org_id,
                configuration,
            )))
            .with_storage_store(self.adapters.storage_store(offer.actor.org_id));
        let backends = match self.adapters.compaction_checkpoint_store() {
            Some(store) => backends.with_compaction_checkpoint_store(store),
            None => backends,
        };
        // The lane checked the revision builds before offering it, so its
        // definition is cached; a missing one only gates fewer platform tools.
        let capabilities = match self.definitions.definition(&binding.behavior).await {
            Ok(Some(definition)) => definition
                .agent_package()
                .map(|package| package.manifest().capabilities.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let backends = with_platform_services(
            backends,
            &self.adapters,
            PlatformTurn {
                org_id: offer.actor.org_id,
                session_id: session,
                agent_id: offer.binding.event_agent_id,
                capabilities,
            },
        );
        let epoch = PlatformEpoch {
            session,
            lease: EngineFence {
                holder: self.holder.clone(),
                fence: offer.actor.fence,
            },
            fence,
            sessions: self.sessions.clone(),
            store: self.holder_store(),
            backends,
            renew_poke: offer.signals.renew_poke.clone(),
            hand_off: offer.signals.hand_off.clone(),
            wake_at_ms: offer.binding.wake_at_ms,
            deltas,
        };
        Ok(Some(SessionActivation {
            binding,
            actor: offer.actor,
            epoch: Arc::new(epoch),
        }))
    }

    fn definitions(&self) -> Option<&dyn AgentDefinitions> {
        Some(self.definitions.as_ref())
    }
}

/// One claimed epoch: every write presents its lease.
struct PlatformEpoch {
    session: SessionId,
    lease: EngineFence,
    fence: Arc<Mutex<ActorFence>>,
    sessions: Arc<dyn EngineSessions>,
    store: Arc<dyn SessionActorStore>,
    backends: HostBackends,
    renew_poke: Option<Arc<tokio::sync::Notify>>,
    hand_off: Option<Arc<AtomicBool>>,
    wake_at_ms: Option<i64>,
    deltas: Arc<PlatformDeltas>,
}

impl PlatformEpoch {
    fn lost(&self, what: &str) -> AgentLoopError {
        actor_lease_lost(self.session, format!("lease lost before {what}"))
    }
}

#[async_trait]
impl SessionEpoch for PlatformEpoch {
    fn renew_poke(&self) -> Option<Arc<tokio::sync::Notify>> {
        self.renew_poke.clone()
    }

    fn hand_off(&self) -> Option<Arc<AtomicBool>> {
        self.hand_off.clone()
    }

    fn claimed_wake_at(&self) -> Option<Option<i64>> {
        Some(self.wake_at_ms)
    }

    fn live_deltas(&self) -> Option<Arc<dyn everruns::hosting::LiveDeltaSink>> {
        Some(self.deltas.clone())
    }

    async fn schedule(&self, wake_at: Option<i64>) -> Result<()> {
        if self
            .sessions
            .schedule(self.session, &self.lease, wake_at)
            .await?
        {
            Ok(())
        } else {
            Err(self.lost("scheduling"))
        }
    }

    fn backends(&self) -> HostBackends {
        self.backends.clone()
    }

    fn actor_store(&self) -> Arc<dyn SessionActorStore> {
        self.store.clone()
    }

    fn observe_fence(&self, fence: &ActorFence) {
        // The driver's fence carries its lost signal under the Engine's own
        // holder name; writes go out under this worker's.
        let mut ours = fence.clone();
        ours.holder = self.lease.holder.clone();
        *self.fence.lock().unwrap_or_else(|p| p.into_inner()) = ours;
    }

    async fn record(&self, key: &str) -> Result<Option<Value>> {
        self.sessions.record(self.session, key).await
    }

    async fn records(&self, prefix: &str) -> Result<Vec<(String, Value)>> {
        self.sessions.records(self.session, prefix).await
    }

    async fn commit(&self, key: &str, value: Value, absent: bool) -> Result<bool> {
        self.sessions
            .commit(self.session, &self.lease, key, &value, absent)
            .await?
            .ok_or_else(|| self.lost("writing a receipt"))
    }

    async fn suspend(&self, reason: &str) -> Result<()> {
        if self
            .sessions
            .suspend(self.session, &self.lease, reason)
            .await?
        {
            Ok(())
        } else {
            Err(self.lost("suspending"))
        }
    }
}

/// The worker's lease store under the worker's own holder name.
struct HolderStore {
    inner: Arc<dyn SessionActorStore>,
    holder: String,
}

#[async_trait]
impl SessionActorStore for HolderStore {
    async fn renew_actor_lease(
        &self,
        session_id: SessionId,
        _holder: &str,
        fence: u64,
        ttl: Duration,
        consumed_seq: i64,
    ) -> Result<Option<ActorRenewal>> {
        self.inner
            .renew_actor_lease(session_id, &self.holder, fence, ttl, consumed_seq)
            .await
    }

    async fn renew_actor_leases(
        &self,
        _holder: &str,
        ttl: Duration,
        leases: &[ActorLeaseRenewal],
    ) -> Result<Vec<Result<Option<ActorRenewal>>>> {
        self.inner
            .renew_actor_leases(&self.holder, ttl, leases)
            .await
    }

    async fn release_actor(
        &self,
        session_id: SessionId,
        _holder: &str,
        fence: u64,
        consumed_inbox_seq: i64,
    ) -> Result<ActorRelease> {
        self.inner
            .release_actor(session_id, &self.holder, fence, consumed_inbox_seq)
            .await
    }

    async fn hand_off_actor(
        &self,
        session_id: SessionId,
        _holder: &str,
        fence: u64,
        consumed_inbox_seq: i64,
    ) -> Result<Option<ActorRelease>> {
        self.inner
            .hand_off_actor(session_id, &self.holder, fence, consumed_inbox_seq)
            .await
    }

    async fn read_actor_journal(
        &self,
        session_id: SessionId,
        after_sequence: Option<i32>,
        limit: u32,
    ) -> Result<ActorJournal> {
        self.inner
            .read_actor_journal(session_id, after_sequence, limit)
            .await
    }
}

/// Published revisions, fetched from the control plane once each. A
/// revision never changes, so a cached one is never stale.
pub struct PlatformDefinitions {
    sessions: Arc<dyn EngineSessions>,
    cache: moka::sync::Cache<AgentBuild, AgentDefinition>,
}

/// Revisions one worker keeps. Each is a few kilobytes of JSON.
const DEFINITION_CACHE: u64 = 10_000;

impl PlatformDefinitions {
    fn new(sessions: Arc<dyn EngineSessions>) -> Self {
        Self {
            sessions,
            cache: moka::sync::Cache::new(DEFINITION_CACHE),
        }
    }
}

#[async_trait]
impl AgentDefinitions for PlatformDefinitions {
    async fn publish_definition(&self, _definition: &AgentDefinition) -> Result<()> {
        Err(AgentLoopError::config(
            "the control plane publishes platform agent definitions",
        ))
    }

    async fn definition(&self, build: &AgentBuild) -> Result<Option<AgentDefinition>> {
        if let Some(definition) = self.cache.get(build) {
            return Ok(Some(definition));
        }
        let Some(value) = self
            .sessions
            .definition(&build.agent, &build.build_id)
            .await?
        else {
            return Ok(None);
        };
        let runtime_agent: RuntimeAgent = serde_json::from_value(value)
            .map_err(|e| AgentLoopError::store(format!("invalid runtime agent: {e}")))?;
        let definition =
            AgentDefinition::new(build.agent.clone(), build.build_id.clone(), runtime_agent);
        self.cache.insert(build.clone(), definition.clone());
        Ok(Some(definition))
    }
}
