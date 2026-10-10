//! Sessions as actors: one runner per session at a time, across processes.
//!
//! Decisions (actor-based design, step 4):
//! - A session is an actor whose state is its event log. Any process with
//!   the log and the agent can run it, so what keeps two processes from
//!   running one session at once is a **lease**, not where the session was
//!   opened. [`SessionLeases`] is that contract: acquire, renew, release,
//!   with a fence number that grows each time the lease changes hands.
//! - The lease covers a turn, not the session's lifetime. [`ActorRunner`]
//!   takes it before a turn starts, renews it while the turn runs (a turn
//!   waiting on a person keeps it), and releases it when the turn ends. A
//!   session between turns, or parked on client-side tool results, holds
//!   nothing, so another process may pick it up.
//! - A holder is the lease store instance: one per process (or per worker),
//!   not per session. The same holder taking a lease it already holds gets
//!   it back with the same fence, so engines in one process that share
//!   backends never block each other.
//! - A lost lease (the holder stalled past the lease's lifetime and another
//!   holder took it) stops the turn at once rather than letting two runners
//!   write. Fencing the log appends themselves with the lease's fence number
//!   comes with the bucket store, where a stalled writer is a real risk.
//! - Clocks: leases expire by wall time in a shared store and by the
//!   process clock in memory; both are native-host concerns, so this module
//!   lives in `host`, outside the wasm-portable kernel.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::SessionId;
use tokio::time::Instant;
use uuid::Uuid;

use super::runtime::InProcessRuntime;
use super::turn_backend::{InProcessBackend, TurnBackend, TurnRequest, TurnTicket};

/// How long a session lease lasts unless renewed.
pub const DEFAULT_SESSION_LEASE_TTL: Duration = Duration::from_secs(30);

/// A held session lease.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SessionLease {
    /// The session the lease is for.
    pub session_id: SessionId,
    /// Grows each time the lease passes to another holder; a writer can
    /// present it so a store rejects a stale one.
    pub fence: u64,
}

impl SessionLease {
    /// A lease on `session_id` with `fence`.
    pub fn new(session_id: SessionId, fence: u64) -> Self {
        Self { session_id, fence }
    }
}

/// Which process runs a session right now.
///
/// **Experimental.** One store instance is one holder. Implementations:
/// [`InMemorySessionLeases`] for a single process, and the framework's local
/// SQLite store for processes that share a data directory.
#[async_trait]
pub trait SessionLeases: Send + Sync {
    /// Take `session_id`'s lease for `ttl`. `None` while another holder's
    /// lease on it has not expired. Taking a lease this holder already holds
    /// extends it and keeps its fence.
    async fn acquire(&self, session_id: SessionId, ttl: Duration) -> Result<Option<SessionLease>>;

    /// Extend `lease` by `ttl`. `false` when this holder no longer holds it
    /// (it expired and another holder took it).
    async fn renew(&self, lease: &SessionLease, ttl: Duration) -> Result<bool>;

    /// Give `lease` up. Releasing a lease this holder no longer holds does
    /// nothing.
    async fn release(&self, lease: &SessionLease) -> Result<()>;
}

/// [`SessionLeases`] kept in this process.
///
/// Clones are the same holder. [`another_holder`](Self::another_holder)
/// shares the table under a different holder, which is how a test stands in
/// for a second process.
#[derive(Clone)]
pub struct InMemorySessionLeases {
    holder: Uuid,
    table: Arc<Mutex<HashMap<SessionId, Held>>>,
}

#[derive(Debug, Clone, Copy)]
struct Held {
    holder: Uuid,
    fence: u64,
    expires_at: Instant,
}

impl InMemorySessionLeases {
    /// An empty table with one holder.
    pub fn new() -> Self {
        Self {
            holder: Uuid::new_v4(),
            table: Arc::default(),
        }
    }

    /// Another holder on the same table.
    pub fn another_holder(&self) -> Self {
        Self {
            holder: Uuid::new_v4(),
            table: self.table.clone(),
        }
    }

    fn table(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, Held>> {
        // Each critical section is one map read or write.
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for InMemorySessionLeases {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for InMemorySessionLeases {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InMemorySessionLeases")
            .field("holder", &self.holder)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl SessionLeases for InMemorySessionLeases {
    async fn acquire(&self, session_id: SessionId, ttl: Duration) -> Result<Option<SessionLease>> {
        let now = Instant::now();
        let mut table = self.table();
        let fence = match table.get(&session_id) {
            Some(held) if held.holder == self.holder => held.fence,
            Some(held) if held.expires_at > now => return Ok(None),
            Some(held) => held.fence + 1,
            None => 1,
        };
        table.insert(
            session_id,
            Held {
                holder: self.holder,
                fence,
                expires_at: now + ttl,
            },
        );
        Ok(Some(SessionLease::new(session_id, fence)))
    }

    async fn renew(&self, lease: &SessionLease, ttl: Duration) -> Result<bool> {
        let mut table = self.table();
        match table.get_mut(&lease.session_id) {
            Some(held) if held.holder == self.holder && held.fence == lease.fence => {
                held.expires_at = Instant::now() + ttl;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn release(&self, lease: &SessionLease) -> Result<()> {
        let mut table = self.table();
        if table
            .get(&lease.session_id)
            .is_some_and(|held| held.holder == self.holder && held.fence == lease.fence)
        {
            // Keep the fence so the next holder's is higher.
            if let Some(held) = table.get_mut(&lease.session_id) {
                held.holder = Uuid::nil();
                held.expires_at = Instant::now();
            }
        }
        Ok(())
    }
}

/// The error a turn fails with when another process runs the session.
pub fn session_held_elsewhere(session_id: SessionId) -> AgentLoopError {
    AgentLoopError::store(format!(
        "session {session_id} runs in another process: it holds the session's lease"
    ))
}

/// Runs a session's turns in this process while holding the session's lease.
///
/// **Experimental**, with [`TurnBackend`]. The turn itself runs as on
/// [`InProcessBackend`]: on the task that polls its ticket. Around it the
/// runner takes the session's lease (failing the start with
/// [`session_held_elsewhere`] when another holder has it), renews it at a
/// third of its lifetime while the turn runs, and releases it when the
/// ticket resolves or is dropped. A lease lost mid-turn stops the turn.
///
/// # Example
///
/// ```no_run
/// use std::sync::Arc;
///
/// use everruns_contracts::typed_id::TurnId;
/// use everruns_core::InputMessage;
/// use everruns_core::host::{
///     AcceptedTurnInput, ActorRunner, InMemorySessionLeases, InProcessRuntime, TurnBackend,
///     TurnInput, TurnRequest,
/// };
///
/// # async fn run() -> everruns_contracts::error::Result<()> {
/// let runtime = InProcessRuntime::builder()
///     .single_session(|session| session)
///     .build()
///     .await?;
/// let session_id = runtime.default_session_id().expect("single_session seeds one");
/// let runner = ActorRunner::new(runtime, Arc::new(InMemorySessionLeases::new()));
///
/// let input = AcceptedTurnInput::new(InputMessage::user("hello"));
/// let result = runner
///     .start_turn(TurnRequest::new(
///         session_id,
///         TurnId::new(),
///         TurnInput::Message(Box::new(input)),
///     ))
///     .await?
///     .await?;
/// println!("{}", result.response);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct ActorRunner {
    turns: InProcessBackend,
    leases: Arc<dyn SessionLeases>,
    ttl: Duration,
}

impl ActorRunner {
    /// Run turns on `runtime`, holding each session's lease in `leases`.
    pub fn new(runtime: InProcessRuntime, leases: Arc<dyn SessionLeases>) -> Self {
        Self {
            turns: InProcessBackend::new(runtime),
            leases,
            ttl: DEFAULT_SESSION_LEASE_TTL,
        }
    }

    /// Take leases for `ttl` (default [`DEFAULT_SESSION_LEASE_TTL`]).
    pub fn with_lease_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }
}

impl fmt::Debug for ActorRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActorRunner")
            .field("turns", &self.turns)
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

/// Releases a lease when the turn that holds it ends or is dropped.
struct HeldLease {
    leases: Arc<dyn SessionLeases>,
    lease: Option<SessionLease>,
}

impl HeldLease {
    async fn release(mut self) {
        if let Some(lease) = self.lease.take() {
            release(self.leases.as_ref(), &lease).await;
        }
    }
}

impl Drop for HeldLease {
    fn drop(&mut self) {
        // A dropped ticket stops its turn; give the lease up in the
        // background. Without a runtime it simply expires.
        let Some(lease) = self.lease.take() else {
            return;
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let leases = self.leases.clone();
            handle.spawn(async move { release(leases.as_ref(), &lease).await });
        }
    }
}

async fn release(leases: &dyn SessionLeases, lease: &SessionLease) {
    if let Err(error) = leases.release(lease).await {
        tracing::warn!(session_id = %lease.session_id, error = %error, "session lease release failed");
    }
}

/// Renew `lease` until it is lost; returns the error the turn then fails with.
async fn keep(leases: &dyn SessionLeases, lease: &SessionLease, ttl: Duration) -> AgentLoopError {
    let every = ttl / 3;
    loop {
        tokio::time::sleep(every).await;
        match leases.renew(lease, ttl).await {
            Ok(true) => {}
            Ok(false) => {
                return AgentLoopError::store(format!(
                    "session {} lost its lease to another process",
                    lease.session_id
                ));
            }
            // A store hiccup is retried at the next beat; the lease lasts
            // three beats.
            Err(error) => {
                tracing::warn!(session_id = %lease.session_id, error = %error, "session lease renewal failed");
            }
        }
    }
}

#[async_trait]
impl TurnBackend for ActorRunner {
    async fn start_turn(&self, request: TurnRequest) -> Result<TurnTicket> {
        let session_id = request.session_id;
        let turn_id = request.turn_id;
        if self.turns.is_running(session_id).await {
            // Let the in-process backend refuse it, without touching the
            // lease the running turn holds.
            return self.turns.start_turn(request).await;
        }
        let lease = self
            .leases
            .acquire(session_id, self.ttl)
            .await?
            .ok_or_else(|| session_held_elsewhere(session_id))?;
        let held = HeldLease {
            leases: self.leases.clone(),
            lease: Some(lease.clone()),
        };
        let ticket = match self.turns.start_turn(request).await {
            Ok(ticket) => ticket,
            Err(error) => {
                held.release().await;
                return Err(error);
            }
        };
        let leases = self.leases.clone();
        let ttl = self.ttl;
        let completion = async move {
            let result = tokio::select! {
                biased;
                result = ticket => result,
                lost = keep(leases.as_ref(), &lease, ttl) => Err(lost),
            };
            held.release().await;
            result
        };
        Ok(TurnTicket::new(session_id, turn_id, completion))
    }

    async fn cancel(&self, session_id: SessionId) -> Result<bool> {
        self.turns.cancel(session_id).await
    }

    async fn is_running(&self, session_id: SessionId) -> bool {
        self.turns.is_running(session_id).await
    }

    async fn active_count(&self) -> usize {
        self.turns.active_count().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_held_lease_blocks_another_holder_until_released() {
        let ours = InMemorySessionLeases::new();
        let theirs = ours.another_holder();
        let session_id = SessionId::new();
        let ttl = Duration::from_secs(30);

        let lease = ours.acquire(session_id, ttl).await.unwrap().unwrap();
        assert_eq!(lease.fence, 1);
        assert_eq!(
            ours.acquire(session_id, ttl).await.unwrap(),
            Some(lease.clone())
        );
        assert_eq!(theirs.acquire(session_id, ttl).await.unwrap(), None);

        ours.release(&lease).await.unwrap();
        let taken = theirs.acquire(session_id, ttl).await.unwrap().unwrap();
        assert_eq!(taken.fence, 2);
        assert!(!ours.renew(&lease, ttl).await.unwrap());
        assert!(theirs.renew(&taken, ttl).await.unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn an_expired_lease_passes_to_another_holder_with_a_higher_fence() {
        let ours = InMemorySessionLeases::new();
        let theirs = ours.another_holder();
        let session_id = SessionId::new();
        let ttl = Duration::from_secs(30);

        let lease = ours.acquire(session_id, ttl).await.unwrap().unwrap();
        tokio::time::advance(ttl + Duration::from_secs(1)).await;
        let taken = theirs.acquire(session_id, ttl).await.unwrap().unwrap();
        assert!(taken.fence > lease.fence);
        assert!(!ours.renew(&lease, ttl).await.unwrap());
        // Releasing a lost lease leaves the new holder's alone.
        ours.release(&lease).await.unwrap();
        assert_eq!(ours.acquire(session_id, ttl).await.unwrap(), None);
    }
}
