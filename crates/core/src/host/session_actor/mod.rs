//! Backend-neutral driver for one leased session, shared by native workers and object hosts.
//! Fleet polling stays with the scheduler; this driver runs a known leased session.

mod inbox;
mod lease;
pub mod store;
#[cfg(test)]
mod tests;
mod turn;

use crate::host::{DEFAULT_SESSION_LEASE_TTL, RuntimeHostAdapter, TurnDrive, WaitsOnPerson};
use async_trait::async_trait;
use everruns_contracts::typed_id::{AgentId, MessageId, SessionId, TurnId};
use std::sync::Arc;
use std::time::Duration;
use store::SessionActorStore;
use tokio::sync::{Notify, watch};
pub use turn::NO_PROGRESS_SEAL_REASON;

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
    pub fence: Option<store::ActorFence>,
}

/// A phase of a turn, as the host's setup reads see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Reason,
    Act,
}

/// The hosts a session actor's turns run on.
/// Result of an application's pre-turn gate. Rejection is already retained
/// by the host before the shared driver advances the rejected input cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputPreparation {
    /// Planning may begin under the current epoch.
    Ready,
    /// The command has a durable rejection receipt; no model/tool may use it.
    Rejected,
}

#[async_trait]
pub trait ActorHosts: Send + Sync + 'static {
    type Host: RuntimeHostAdapter + Clone + Send + Sync + 'static;

    /// A host for `turn`, stopping its tools on `signals`.
    fn host(&self, turn: HostTurn, signals: &TurnSignals) -> Self::Host;

    /// Recover host-owned finalization before admitting another logical turn,
    /// and finish it under the live epoch before release. A terminal runtime
    /// event alone need not be the application's finalized outcome.
    async fn checkpoint(
        &self,
        _host: &Self::Host,
        _tail: &[crate::events::Event],
    ) -> everruns_contracts::error::Result<()> {
        Ok(())
    }

    /// Retain a logical turn identity before start hooks or `turn.started`.
    async fn fresh_turn_id(
        &self,
        _input: MessageId,
        proposed: TurnId,
    ) -> everruns_contracts::error::Result<TurnId> {
        Ok(proposed)
    }

    /// Await application gates under the same execution epoch as the turn.
    async fn before_turn(
        &self,
        _host: &Self::Host,
        _drive: &TurnDrive,
        _input: Option<&crate::RuntimeMessage>,
    ) -> everruns_contracts::error::Result<InputPreparation> {
        Ok(InputPreparation::Ready)
    }

    /// Record assignment before a reason can consume admitted steering.
    async fn inputs_consumed(
        &self,
        _drive: &TurnDrive,
        _inputs: &[MessageId],
    ) -> everruns_contracts::error::Result<()> {
        Ok(())
    }

    /// A phase of `drive`'s turn begins on `host`.
    fn begin_phase(&self, _host: &Self::Host, _drive: &TurnDrive, _phase: Phase) {}

    /// Wait until the events `host` queued are stored; an error when one of
    /// those stores gave up (the log has a gap; see the host write-behind queue).
    async fn flush(&self, _host: &Self::Host) -> everruns_contracts::error::Result<()> {
        Ok(())
    }

    /// Drop the events `host` queued and has not stored (the lease is lost).
    fn abandon(&self, _host: &Self::Host) {}

    /// `drive`'s turn ended on this worker (its lease is not released yet).
    async fn turn_ended(&self, _drive: &TurnDrive) {}

    /// Calls that wait on a person before they run, so resume reruns them.
    fn waits_on_person(&self) -> Option<WaitsOnPerson> {
        None
    }
}

/// Default bound on the phases of a lease that write nothing (setup before a
/// turn is driven, the flush and checkpoint after it). Generous: setup is a
/// handful of control-plane reads and the application's start gates.
pub const DEFAULT_ACTOR_STALL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Default number of times one lease runs its turn again after a retryable
/// (control-plane) failure before it lets the lease expire; see `turn.rs`.
pub const DEFAULT_ACTOR_CONTROL_PLANE_RETRIES: u32 = 5;
/// Default wait before the first such retry; it doubles with each one, so the
/// default retries span about 30 s, past a pool or failover blip.
pub const DEFAULT_ACTOR_CONTROL_PLANE_BACKOFF: Duration = Duration::from_secs(1);

/// Configuration of a single-session execution driver.
#[derive(Debug, Clone)]
pub struct SessionActorConfig {
    pub holder: String,
    pub lease_ttl: Duration,
    pub poison_attempts: u32,
    pub journal_page: u32,
    /// How long a lease may sit in a phase that writes nothing before it
    /// stops renewing and lets the lease expire (the stall watchdog).
    pub stall_timeout: Duration,
    /// Times one lease runs its turn again after a retryable failure (a store
    /// or transport error) before it lets the lease expire.
    pub control_plane_retries: u32,
    /// Wait before the first such retry; doubles with each one.
    pub control_plane_backoff: Duration,
}

impl SessionActorConfig {
    pub fn new(holder: impl Into<String>) -> Self {
        Self {
            holder: holder.into(),
            lease_ttl: DEFAULT_SESSION_LEASE_TTL,
            poison_attempts: 3,
            journal_page: 256,
            stall_timeout: DEFAULT_ACTOR_STALL_TIMEOUT,
            control_plane_retries: DEFAULT_ACTOR_CONTROL_PLANE_RETRIES,
            control_plane_backoff: DEFAULT_ACTOR_CONTROL_PLANE_BACKOFF,
        }
    }
}

/// Executes a leased session with the shared turn loop and log-based recovery.
pub struct SessionActorDriver<H: ActorHosts> {
    hosts: Arc<H>,
    store: Arc<dyn SessionActorStore>,
    config: SessionActorConfig,
    renew_poke: Arc<Notify>,
}

impl<H: ActorHosts> SessionActorDriver<H> {
    pub fn new(
        hosts: Arc<H>,
        store: Arc<dyn SessionActorStore>,
        config: SessionActorConfig,
    ) -> Self {
        Self {
            hosts,
            store,
            config,
            renew_poke: Arc::new(Notify::new()),
        }
    }

    /// Wake the renewer on a cancellation notification.
    pub fn with_renew_poke(mut self, poke: Arc<Notify>) -> Self {
        self.renew_poke = poke;
        self
    }
}
