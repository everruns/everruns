//! Sessions as actors, on the worker: lease runnable sessions and run each
//! one's whole turn (actor-based design, step 6c).
//!
//! Decisions:
//! - The control plane's runnable index is the only queue for turns. Intake
//!   marks a session's actor runnable in the transaction that stored its
//!   input; this loop leases actors (`ActorLeaseStore`), and each lease runs
//!   one turn to its end on this worker (`turn`), with no step handed back to
//!   a queue between reason and act.
//! - The session log is the only turn state. A lease folds the log's tail and
//!   resumes the open turn, or starts a fresh one from the first unconsumed
//!   input, so a crash anywhere costs a lease TTL and nothing else.
//! - Execution slots are the worker's: actors and queued activities share
//!   `in_flight`, its cap, the model-wait slot release, operator drain and the
//!   shutdown grace. The loop has its own wake-up (`session_actor`
//!   notifications and finished actors), and cancel notifications poke every
//!   running lease to renew now, which is how a holder learns of a cancel.
//! - The worker's host stays the one queued phases used: `WorkerRuntimeHost`
//!   with its phase setup reads, turn read cache and write-behind events,
//!   built once per turn and re-armed at each phase boundary (`ActorHosts`).
//!   Stream heartbeats are not needed: the lease renewer keeps the turn alive.

mod inbox;
mod lease;
mod turn;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::tool_approval_types::APPROVE_TOOL_CALL_TOOL;
use everruns_contracts::tool_types::CONFIRM_URL_ELICITATION_TOOL;
use everruns_contracts::typed_id::{AgentId, MessageId, SessionId};
use tokio::sync::{Notify, watch};
use tokio::task::JoinSet;
use tracing::{debug, info, warn};

use crate::actor_lease_store::{ActorLeaseStore, LeasedActor};
use crate::engine::model_wait::{self, ModelWaitSlots};
use crate::host::{DEFAULT_SESSION_LEASE_TTL, RuntimeHostAdapter, TurnDrive, WaitsOnPerson};
use crate::phase_reads::PhaseIds;
use crate::runtime_host::WorkerRuntimeHost;
use crate::turn_reads::TurnReads;
use crate::worker_adapters::WorkerAdapters;

pub use turn::NO_PROGRESS_SEAL_REASON;

/// Wake-up payload for a newly leasable actor (the server's
/// `SESSION_ACTOR_NOTIFICATION`).
pub const SESSION_ACTOR_NOTIFICATION: &str = "session_actor";
/// Wake-up payload for a cancel on a leased actor.
pub const SESSION_ACTOR_CANCEL_NOTIFICATION: &str = "session_actor_cancel";

/// The turn a host is built for.
#[derive(Debug, Clone, Copy)]
pub struct HostTurn {
    pub org_id: i64,
    pub session_id: SessionId,
    pub agent_id: Option<AgentId>,
    /// `None` while the lease is still reading which turn it runs.
    pub input_message_id: Option<MessageId>,
}

/// How a running turn learns it must stop.
#[derive(Clone)]
pub struct TurnSignals {
    /// The turn must stop: the lease was lost or a cancel was requested.
    pub stop: watch::Receiver<bool>,
    /// A person asked to cancel the turn.
    pub cancel: watch::Receiver<bool>,
    /// The lease every event of the turn is appended under.
    pub fence: Option<crate::actor_lease_store::ActorFence>,
}

/// A phase of a turn, as the host's setup reads see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Reason,
    Act,
}

/// The hosts a session actor's turns run on.
#[async_trait]
pub trait ActorHosts: Send + Sync + 'static {
    type Host: RuntimeHostAdapter + Clone + Send + Sync + 'static;

    /// A host for `turn`, stopping its tools on `signals`.
    fn host(&self, turn: HostTurn, signals: &TurnSignals) -> Self::Host;

    /// A phase of `drive`'s turn begins on `host`.
    fn begin_phase(&self, _host: &Self::Host, _drive: &TurnDrive, _phase: Phase) {}

    /// Wait until the events `host` queued are stored.
    async fn flush(&self, _host: &Self::Host) {}

    /// Drop the events `host` queued and has not stored (the lease is lost).
    fn abandon(&self, _host: &Self::Host) {}

    /// `drive`'s turn ended on this worker (its lease is not released yet).
    async fn turn_ended(&self, _drive: &TurnDrive) {}

    /// Calls that wait on a person before they run, so resume reruns them.
    fn waits_on_person(&self) -> Option<WaitsOnPerson> {
        None
    }
}

/// Settings of one worker's actor loop.
#[derive(Debug, Clone)]
pub struct ActorRuntimeConfig {
    /// Lease holder id (the worker id).
    pub holder: String,
    pub lease_ttl: Duration,
    /// Seal a turn whose lease count since the last release passes this.
    pub poison_attempts: u32,
    /// Journal page size; 0 = the control plane's default.
    pub journal_page: u32,
    /// Cap on leases one org holds at once (`None` = the control plane's).
    pub max_leased_per_org: Option<u32>,
}

impl ActorRuntimeConfig {
    pub fn new(holder: impl Into<String>) -> Self {
        Self {
            holder: holder.into(),
            lease_ttl: DEFAULT_SESSION_LEASE_TTL,
            poison_attempts: crate::durable::persistence::no_progress_seal_threshold_from_env(),
            journal_page: 0,
            max_leased_per_org: None,
        }
    }
}

/// The worker's execution slots, shared with its queued activities.
#[derive(Clone)]
pub struct ActorSlots {
    pub in_flight: Arc<AtomicUsize>,
    pub max_concurrent: usize,
    pub claim_batch: usize,
    pub model_waits: Arc<dyn ModelWaitSlots>,
    /// The registry reported the worker draining: lease nothing.
    pub draining: Arc<AtomicBool>,
    pub poll_interval: Duration,
    pub poll_backoff_max: Duration,
    pub shutdown_grace: Duration,
}

/// Leases session actors and runs their turns.
pub struct ActorRuntime<H: ActorHosts> {
    hosts: Arc<H>,
    store: Arc<dyn ActorLeaseStore>,
    config: ActorRuntimeConfig,
    /// Fired on a cancel notification: every running lease renews now.
    renew_poke: Arc<Notify>,
    /// Wakes the lease loop: new runnable actors, or a finished one.
    wake: Arc<Notify>,
}

impl<H: ActorHosts> Clone for ActorRuntime<H> {
    fn clone(&self) -> Self {
        Self {
            hosts: self.hosts.clone(),
            store: self.store.clone(),
            config: self.config.clone(),
            renew_poke: self.renew_poke.clone(),
            wake: self.wake.clone(),
        }
    }
}

impl<H: ActorHosts> ActorRuntime<H> {
    pub fn new(hosts: H, store: Arc<dyn ActorLeaseStore>, config: ActorRuntimeConfig) -> Self {
        Self {
            hosts: Arc::new(hosts),
            store,
            config,
            renew_poke: Arc::new(Notify::new()),
            wake: Arc::new(Notify::new()),
        }
    }

    /// Notify to wake the lease loop.
    pub fn wake(&self) -> Arc<Notify> {
        self.wake.clone()
    }

    /// Notify (with `notify_waiters`) to make every running lease renew now.
    pub fn renew_poke(&self) -> Arc<Notify> {
        self.renew_poke.clone()
    }

    /// Lease up to `limit` actors.
    pub async fn lease(&self, limit: usize) -> everruns_contracts::error::Result<Vec<LeasedActor>> {
        self.store
            .lease_actors(
                &self.config.holder,
                u32::try_from(limit).unwrap_or(u32::MAX),
                self.config.lease_ttl,
                self.config.max_leased_per_org,
            )
            .await
    }

    /// Lease and run actors until `shutdown`, then give running turns the
    /// grace period and abort the rest (their leases expire and another
    /// worker resumes them from the log).
    pub async fn run(&self, slots: ActorSlots, mut shutdown: watch::Receiver<bool>) {
        let base = slots.poll_interval.min(slots.poll_backoff_max);
        let mut backoff = base;
        let mut running = JoinSet::new();
        loop {
            if *shutdown.borrow() {
                break;
            }
            while let Some(result) = running.try_join_next() {
                log_join(result);
            }
            let leased = match self.lease_into(&slots, &mut running).await {
                Ok(leased) => leased,
                Err(error) => {
                    warn!(error = %error, "Failed to lease session actors");
                    0
                }
            };
            if leased > 0 {
                backoff = base;
                continue;
            }
            let mut woken = false;
            tokio::select! {
                _ = tokio::time::sleep(backoff) => {}
                _ = self.wake.notified() => woken = true,
                _ = shutdown.changed() => break,
            }
            backoff = if woken {
                base
            } else {
                backoff
                    .checked_mul(2)
                    .unwrap_or(slots.poll_backoff_max)
                    .min(slots.poll_backoff_max)
            };
        }
        if !running.is_empty() {
            info!(
                running = running.len(),
                "Waiting for running session turns before stopping"
            );
        }
        let finished = tokio::time::timeout(slots.shutdown_grace, async {
            while let Some(result) = running.join_next().await {
                log_join(result);
            }
        })
        .await;
        if finished.is_err() {
            warn!(
                aborted = running.len(),
                "Shutdown grace elapsed; aborting session turns (their leases expire)"
            );
            running.abort_all();
            while running.join_next().await.is_some() {}
        }
    }

    async fn lease_into(
        &self,
        slots: &ActorSlots,
        running: &mut JoinSet<()>,
    ) -> everruns_contracts::error::Result<usize> {
        if slots.draining.load(Ordering::SeqCst) {
            return Ok(0);
        }
        let available = slots
            .max_concurrent
            .saturating_sub(slots.in_flight.load(Ordering::SeqCst));
        let limit = available.min(slots.claim_batch);
        if limit == 0 {
            return Ok(0);
        }
        let actors = self.lease(limit).await?;
        let leased = actors.len();
        if leased > 0 {
            debug!(holder = %self.config.holder, leased, "Leased session actors");
        }
        for actor in actors {
            let runtime = self.clone();
            let guard = InFlight::enter(slots.in_flight.clone());
            let model_waits = slots.model_waits.clone();
            running.spawn(async move {
                let _guard = guard;
                model_wait::scope(model_waits, runtime.run_actor(actor)).await;
                // The release may have left the actor runnable (input arrived
                // as its turn ended), and a slot just opened.
                runtime.wake.notify_one();
            });
        }
        Ok(leased)
    }
}

fn log_join(result: Result<(), tokio::task::JoinError>) {
    if let Err(error) = result
        && !error.is_cancelled()
    {
        warn!(error = %error, "Session actor task failed");
    }
}

struct InFlight(Arc<AtomicUsize>);

impl InFlight {
    fn enter(in_flight: Arc<AtomicUsize>) -> Self {
        in_flight.fetch_add(1, Ordering::SeqCst);
        Self(in_flight)
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// [`ActorHosts`] over the worker's adapters.
pub struct WorkerActorHosts<A: WorkerAdapters> {
    adapters: A,
    turns: Arc<TurnReads>,
}

impl<A: WorkerAdapters> WorkerActorHosts<A> {
    pub fn new(adapters: A) -> Self {
        Self {
            adapters,
            turns: Arc::default(),
        }
    }
}

/// Decision: besides `ask_user` and the calls whose tool requires approval or
/// runs on the client (core reruns those itself), the synthetic person-facing
/// calls wait on a person: an approval prompt and a URL elicitation consent.
/// A platform approval-gated call is not one of them: its wait settles as a
/// recorded `tool.completed` (`tool_approval_required`), so a call of it left
/// unfinished may already have run after the approval, and resume reruns it
/// only when its tool says that is safe.
fn waits_on_person(call: &everruns_contracts::tool_types::ToolCall) -> bool {
    call.name == APPROVE_TOOL_CALL_TOOL || call.name == CONFIRM_URL_ELICITATION_TOOL
}

#[async_trait]
impl<A: WorkerAdapters> ActorHosts for WorkerActorHosts<A> {
    type Host = WorkerRuntimeHost<A>;

    fn host(&self, turn: HostTurn, signals: &TurnSignals) -> Self::Host {
        let event_metadata = turn.agent_id.map(|agent_id| {
            let mut metadata = serde_json::Map::new();
            metadata.insert(
                "agent_id".to_string(),
                serde_json::Value::String(agent_id.to_string()),
            );
            metadata
        });
        let host = WorkerRuntimeHost::with_event_metadata(self.adapters.clone(), event_metadata)
            .with_turn_cancellation(signals.stop.clone(), signals.cancel.clone())
            .with_actor_fence(signals.fence.clone());
        match turn.input_message_id {
            Some(message_id) => host.with_turn_reads(self.turns.slot(
                turn.org_id,
                turn.session_id.uuid(),
                message_id.uuid(),
            )),
            None => host,
        }
    }

    fn begin_phase(&self, host: &Self::Host, drive: &TurnDrive, phase: Phase) {
        host.begin_phase(Some(PhaseIds {
            org_id: drive.org_id,
            session_id: drive.session_id.uuid(),
            harness_id: drive.harness_id.uuid(),
            agent_id: drive.agent_id.map(|id| id.uuid()),
            input_message_id: (phase == Phase::Reason).then(|| drive.input_message_id.uuid()),
        }));
    }

    async fn flush(&self, host: &Self::Host) {
        host.flush_events().await;
    }

    fn abandon(&self, host: &Self::Host) {
        host.abandon_events();
    }

    async fn turn_ended(&self, drive: &TurnDrive) {
        self.turns.end(
            drive.org_id,
            drive.session_id.uuid(),
            drive.input_message_id.uuid(),
        );
    }

    fn waits_on_person(&self) -> Option<WaitsOnPerson> {
        Some(Arc::new(waits_on_person))
    }
}
