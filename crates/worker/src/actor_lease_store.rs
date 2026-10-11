//! Session actor leases and journal reads, as the worker sees them
//! (actor-based design).
//!
//! Decisions:
//! - Each session is an actor: a worker leases a runnable session and runs its
//!   whole turn, and the session event log is the only turn state, so resume
//!   is a fold of the log ([`ActorLeaseStore::read_actor_journal`]).
//! - Workers never touch the database. The control plane serves this port
//!   over gRPC (`GrpcActorLeaseStore`) and, in DEV_MODE, in process through
//!   the same repository (`DirectWorkerAdapters`).
//! - Cross-org, like the leased resource sweeper: a lease call picks work from
//!   every org, so the port carries no org scope.
//! - Fences are `u64`, as in core's `SessionLease`, so a leased actor maps onto
//!   it directly.

use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionId;

use crate::core::events::Event;

/// An actor a worker just leased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeasedActor {
    pub session_id: SessionId,
    pub org_id: i64,
    /// Raised by every lease; present it to renew and release.
    pub fence: u64,
    /// Event sequence of the latest input the actor must see.
    pub inbox_seq: i64,
    /// A cancel is pending for the turn this lease runs.
    pub cancel_requested: bool,
    /// The inbox cursor the previous holder left: input up to this sequence
    /// reached a turn.
    pub consumed_seq: i64,
    /// The input the next fresh turn starts from, when past the cursor.
    pub next_input_seq: i64,
    /// Leases since the last release, this one included (poison guard).
    pub attempts: u32,
}

/// What a successful lease renewal reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorRenewal {
    /// Event sequence of the latest input the actor must see.
    pub inbox_seq: i64,
    /// A cancel is pending for the running turn.
    pub cancel_requested: bool,
}

/// The outcome of releasing an actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorRelease {
    /// Released. `still_runnable`: newer input (or a cancel this holder never
    /// saw) is waiting, and the next lease call picks the actor up.
    Released { still_runnable: bool },
    /// The lease was already lost; nothing changed.
    LeaseLost,
}

/// A page of a session's event log for an actor to fold.
#[derive(Debug, Clone)]
pub struct ActorJournal {
    /// Events in sequence order.
    pub events: Vec<Event>,
    /// More events follow the last one; read again after it.
    pub has_more: bool,
    /// The session's newest event sequence; `None` for an empty log.
    pub head_sequence: Option<i32>,
}

/// The control plane's runnable index and lease protocol.
#[async_trait]
pub trait ActorLeaseStore: Send + Sync {
    /// Lease up to `limit` runnable actors to `holder` for `ttl`, oldest
    /// first. `max_leased_per_org` caps the leases one org holds at once,
    /// counting leases already held (`None` = the control plane's default).
    async fn lease_actors(
        &self,
        holder: &str,
        limit: u32,
        ttl: Duration,
        max_leased_per_org: Option<u32>,
    ) -> Result<Vec<LeasedActor>>;

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
