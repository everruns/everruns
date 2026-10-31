//! Shared session ownership and journal ports for native workers and object hosts.
//!
//! Decisions:
//! - Each session is an actor: a worker leases a runnable session and runs its
//!   whole turn, and the session event log is the only turn state, so resume
//!   is a fold of the log ([`SessionActorStore::read_actor_journal`]).
//! - Platform workers never touch the database. The control plane serves this port
//!   over gRPC (`GrpcActorLeaseStore`) and, in DEV_MODE, in process through
//!   the same repository (`DirectWorkerAdapters`).
//! - Fleet discovery is cross-org on the platform. A known-session object host
//!   scopes its implementation to one tenant/session without a fleet discovery port.
//! - Fences are `u64`, as in core's `SessionLease`, so a leased actor maps onto
//!   it directly.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{SessionId, TurnId};

use crate::events::Event;

/// The marker the control plane puts in a refused fenced append (the gRPC
/// status message of `FAILED_PRECONDITION`).
pub const ACTOR_LEASE_LOST: &str = "actor lease lost";

/// The lease a holder's event appends are made under.
///
/// Decision: every event a leased turn writes carries its holder and fence,
/// and the control plane stores it only while that is still the holder's
/// unexpired lease, checked in the insert's transaction. A holder that lost
/// its lease (expired, taken, released) but still has a write in flight
/// cannot land it after another holder took the session. The refusal flips
/// the lease lost (`on_lost`), so the turn stops as when a renew finds the
/// fence moved.
#[derive(Clone)]
pub struct ActorFence {
    pub session_id: SessionId,
    pub holder: String,
    pub fence: u64,
    /// A detached tool call's appends: checked against the call's detached
    /// record instead of the actor lease (`fence` is unused), because its turn
    /// parked and released the lease (see `engine::tool_detach`).
    pub detached_tool_call_id: Option<String>,
    on_lost: Option<Arc<tokio::sync::watch::Sender<bool>>>,
}

impl std::fmt::Debug for ActorFence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActorFence")
            .field("session_id", &self.session_id)
            .field("holder", &self.holder)
            .field("fence", &self.fence)
            .field("detached_tool_call_id", &self.detached_tool_call_id)
            .finish_non_exhaustive()
    }
}

impl ActorFence {
    pub fn new(session_id: SessionId, holder: impl Into<String>, fence: u64) -> Self {
        Self {
            session_id,
            holder: holder.into(),
            fence,
            detached_tool_call_id: None,
            on_lost: None,
        }
    }

    /// The fence of `holder`'s detached call `tool_call_id` of `session_id`.
    pub fn detached_call(
        session_id: SessionId,
        holder: impl Into<String>,
        tool_call_id: impl Into<String>,
    ) -> Self {
        Self {
            detached_tool_call_id: Some(tool_call_id.into()),
            ..Self::new(session_id, holder, 0)
        }
    }

    /// Flip `lost` to `true` when an append is refused.
    pub fn with_lost_signal(mut self, lost: Arc<tokio::sync::watch::Sender<bool>>) -> Self {
        self.on_lost = Some(lost);
        self
    }

    /// Whether the lease this fence belongs to is known to be lost.
    pub fn is_lost(&self) -> bool {
        self.on_lost.as_ref().is_some_and(|lost| *lost.borrow())
    }

    /// The fence as the control plane stores it.
    pub fn fence_i64(&self) -> i64 {
        i64::try_from(self.fence).unwrap_or(i64::MAX)
    }

    /// Note the outcome of an append: a refusal marks the lease lost.
    pub fn observe<T>(&self, result: &Result<T>) {
        if let Err(error) = result
            && is_actor_lease_lost(error)
            && let Some(lost) = &self.on_lost
        {
            lost.send_replace(true);
        }
    }
}

/// An append refused because the holder's actor lease is gone.
#[derive(Debug)]
pub struct ActorLeaseLost {
    pub session_id: SessionId,
    pub detail: String,
}

impl std::fmt::Display for ActorLeaseLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{ACTOR_LEASE_LOST}: session {} ({})",
            self.session_id, self.detail
        )
    }
}

impl std::error::Error for ActorLeaseLost {}

/// The error a fenced append returns when the lease is no longer the
/// holder's. Retryable (`Internal`): the turn stops and the next holder
/// resumes it from the log.
pub fn actor_lease_lost(session_id: SessionId, detail: impl Into<String>) -> AgentLoopError {
    AgentLoopError::Internal(anyhow::Error::new(ActorLeaseLost {
        session_id,
        detail: detail.into(),
    }))
}

/// Whether `error` is a refused fenced append ([`actor_lease_lost`]).
pub fn is_actor_lease_lost(error: &AgentLoopError) -> bool {
    matches!(error, AgentLoopError::Internal(inner) if inner.downcast_ref::<ActorLeaseLost>().is_some())
}

/// An actor a worker just leased.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LeasedActor {
    pub session_id: SessionId,
    pub org_id: i64,
    /// Raised by every lease; present it to renew and release.
    pub fence: u64,
    /// Event sequence of the latest input the actor must see.
    pub inbox_seq: i64,
    /// A cancel is pending: the inbox position it targets. It ends the turn
    /// whose input is at or before it, never one started from later input.
    pub cancel_inbox_seq: Option<i64>,
    /// The inbox cursor the previous holder left: input up to this sequence
    /// reached a turn.
    pub consumed_seq: i64,
    /// The input the next fresh turn starts from, when past the cursor.
    pub next_input_seq: i64,
    /// Leases since the last release, this one included (poison guard).
    pub attempts: u32,
}

/// What a successful lease renewal reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActorRenewal {
    /// Event sequence of the latest input the actor must see.
    pub inbox_seq: i64,
    /// A cancel is pending: the inbox position it targets (see
    /// [`LeasedActor::cancel_inbox_seq`]).
    pub cancel_inbox_seq: Option<i64>,
}

/// The outcome of releasing an actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ActorRelease {
    /// Released. `still_runnable`: newer input (or a cancel this holder never
    /// saw) is waiting, and the next lease call picks the actor up.
    Released { still_runnable: bool },
    /// The lease was already lost; nothing changed.
    LeaseLost,
}

/// A page of a session's event log for an actor to fold.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ActorJournal {
    /// Events in sequence order.
    pub events: Vec<Event>,
    /// More events follow the last one; read again after it.
    pub has_more: bool,
    /// The session's newest event sequence; `None` for an empty log.
    pub head_sequence: Option<i32>,
}

/// A tool call a holder took off its turn (see `engine::tool_detach`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DetachedCallKey {
    pub session_id: SessionId,
    pub tool_call_id: String,
}

/// The control plane's runnable index and lease protocol.
#[async_trait]
pub trait ActorLeaseStore: SessionActorStore {
    /// Lease up to `limit` runnable actors to `holder` for `ttl`, oldest
    /// first, except that an org holding `max_leased_per_org` leases or more
    /// (counting leases already held) goes after every other org: a fairness
    /// order, not a limit (`None` = the control plane's default).
    async fn lease_actors(
        &self,
        holder: &str,
        limit: u32,
        ttl: Duration,
        max_leased_per_org: Option<u32>,
    ) -> Result<Vec<LeasedActor>>;

    /// Record `call` of turn `turn_id` as running on `holder`, which leases
    /// the actor with `fence`, for `ttl`. While the record lives the actor is
    /// not leased. `false` when the store keeps no records or the lease is
    /// gone: the call stays with its turn.
    async fn detach_tool_call(
        &self,
        _call: &DetachedCallKey,
        _turn_id: TurnId,
        _holder: &str,
        _fence: u64,
        _ttl: Duration,
    ) -> Result<bool> {
        Ok(false)
    }

    /// Extend `holder`'s live records among `calls` by `ttl`; returns those
    /// still live. The holder stops every other call.
    async fn renew_detached_tool_calls(
        &self,
        _holder: &str,
        _ttl: Duration,
        _calls: &[DetachedCallKey],
    ) -> Result<Vec<DetachedCallKey>> {
        Ok(Vec::new())
    }

    /// `call`'s `tool.completed` is stored: drop its record. `None` when the
    /// record was gone (cancelled or expired), else whether it was the
    /// session's last, which made the actor runnable.
    async fn complete_detached_tool_call(
        &self,
        _call: &DetachedCallKey,
        _holder: &str,
    ) -> Result<Option<bool>> {
        Ok(None)
    }

    /// Expire `holder`'s records among `calls` now: it stopped them, and the
    /// next lease resumes their turns from the log.
    async fn expire_detached_tool_calls(
        &self,
        _holder: &str,
        _calls: &[DetachedCallKey],
    ) -> Result<()> {
        Ok(())
    }
}

/// Ownership and journal operations for one known session.
#[async_trait]
pub trait SessionActorStore: Send + Sync {
    /// Extend `holder`'s lease on `session_id`, recording that its turn
    /// consumed input up to `consumed_seq` (the cursor only moves forward).
    /// `None` when the lease is lost (another holder took it after it
    /// expired, or a cancel withdrew it): stop the turn without writing.
    /// Reports input that arrived and a pending cancel.
    async fn renew_actor_lease(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        ttl: Duration,
        consumed_seq: i64,
    ) -> Result<Option<ActorRenewal>>;

    /// Give up `holder`'s lease on `session_id` after consuming input up to
    /// `consumed_inbox_seq`. Newer input keeps the actor runnable for the
    /// next lease.
    async fn release_actor(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        consumed_inbox_seq: i64,
    ) -> Result<ActorRelease>;

    /// Hand the lease back mid-turn, between steps, leaving the actor
    /// runnable so the next lease resumes the open turn from the log at once
    /// (a draining holder). `None` when this store cannot: the holder then
    /// lets the lease expire, which resumes the turn too, one TTL later.
    async fn hand_off_actor(
        &self,
        _session_id: SessionId,
        _holder: &str,
        _fence: u64,
        _consumed_inbox_seq: i64,
    ) -> Result<Option<ActorRelease>> {
        Ok(None)
    }

    /// The tail of `session_id`'s event log to fold: events after
    /// `after_sequence`, or from the latest `turn.started` (the whole log when
    /// no turn started yet). Page with [`ActorJournal::has_more`]. `limit` 0 =
    /// the control plane's default page size.
    async fn read_actor_journal(
        &self,
        session_id: SessionId,
        after_sequence: Option<i32>,
        limit: u32,
    ) -> Result<ActorJournal>;
}
