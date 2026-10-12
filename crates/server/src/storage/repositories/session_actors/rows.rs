// Rows the session actor repository reads and writes.

use crate::kernel_imports::contracts::typed_id::SessionId;
use chrono::{DateTime, Utc};
use everruns_server_macros::Columns;
use sqlx::FromRow;

use crate::storage::EventRow;

/// One `session_actors` row: whether a session has work for a worker, and who
/// runs it. Holds no turn state; the session's event log is that.
#[derive(Debug, Clone, FromRow, Columns)]
pub struct SessionActorRow {
    pub session_id: SessionId,
    pub org_id: i64,
    /// When the actor became runnable; `None` when it has nothing to do.
    pub runnable_since: Option<DateTime<Utc>>,
    /// Event sequence of the latest input the actor must see.
    pub inbox_seq: i64,
    pub lease_holder: Option<String>,
    /// Raised by every lease; a holder presents it to renew and release.
    pub lease_fence: i64,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub cancel_requested_at: Option<DateTime<Utc>>,
    /// A lease or renew told the holder about the pending cancel.
    pub cancel_reported: bool,
    /// The inbox cursor: input up to this sequence reached a turn.
    pub consumed_seq: i64,
    /// The input the next fresh turn starts from, when it is past the cursor.
    pub next_input_seq: i64,
    /// Leases since the last release (poison guard).
    pub attempts: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An actor a holder just leased.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct LeasedActorRow {
    pub session_id: SessionId,
    pub org_id: i64,
    pub fence: i64,
    pub inbox_seq: i64,
    /// The inbox cursor the previous holder left.
    pub consumed_seq: i64,
    /// The input the next fresh turn starts from, when past the cursor.
    pub next_input_seq: i64,
    /// Leases since the last release, this one included.
    pub attempts: i32,
    pub cancel_requested: bool,
}

/// What a cancel request did; see the repository header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorCancel {
    /// A live holder runs the actor; it records the cancel.
    Requested,
    /// The actor was runnable with no live holder: it is withdrawn, and the
    /// caller records the cancel.
    Withdrawn,
    /// Nothing runs or waits to run.
    Idle,
}

/// What a successful renew reports back to the holder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRow)]
pub struct ActorRenewalRow {
    pub inbox_seq: i64,
    pub cancel_requested: bool,
}

/// A bounded slice of a session's event log for an actor to fold.
#[derive(Debug, Clone)]
pub struct ActorJournalRows {
    /// Events in sequence order.
    pub events: Vec<EventRow>,
    /// More events follow the last one returned; read again after it.
    pub has_more: bool,
    /// The session's newest event sequence, `None` for an empty log.
    pub head_sequence: Option<i32>,
}
