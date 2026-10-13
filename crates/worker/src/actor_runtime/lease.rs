//! A held actor lease: renewal, the inbox cursor and the journal reads a
//! turn's inbox makes.
//!
//! Decisions:
//! - The cursor is the highest event sequence the holder examined for input.
//!   Every turn input up to it reached a turn (the fresh turn's own input, or
//!   steering handed to a reason). Renew and release commit it, so a holder
//!   that dies mid-turn leaves the next one a cursor that delivers only input
//!   the model never saw.
//! - A fresh turn's own input is not committed until the turn is in the log.
//!   The cursor moves over it at once (so the turn's inbox never hands it
//!   back as steering), but `turn.started` is a write-behind event stored at
//!   the first phase boundary, and setup before it reads the control plane
//!   (and can fail on it). A renew in that window used to commit the input,
//!   so a holder that failed or died before the turn started left the next
//!   one a consumed input and no open turn: the message was never answered.
//!   Renew and release commit below that input until the turn's events are
//!   stored ([`Lease::turn_stored`]).
//! - A turn input is an `input.message` without a turn id. Everything a turn
//!   or a resolution writes into the conversation carries the turn's id, so
//!   it is part of that turn's history, never input that starts or steers one.
//! - Lease loss is observed, never assumed away: a renew that finds another
//!   holder's fence, no successful renew for a whole TTL, or an event append
//!   the control plane refused for its fence flips `lost`, and the turn stops
//!   without writing anything more.
//! - Every event the turn appends carries the lease's holder and fence
//!   (`fence`), checked by the control plane in the insert's transaction, so
//!   a write still in flight when the lease was lost cannot land after
//!   another holder took the session.
//! - A cancel targets the turn it was requested for: the control plane reports
//!   the inbox position it was requested at, and the lease raises `cancel`
//!   only for a turn whose input is at or before it. The cancel stays on the
//!   row until a release, and a holder that recorded it can die before
//!   releasing; a position needs nothing to run after `turn.cancelled` to stop
//!   it reaching the follow-up message's turn, where clearing it would.
//!   Any cancel requested while a turn runs targets it, since the turn's input
//!   is in the inbox by then.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{MessageId, SessionId};
use tokio::sync::{Notify, watch};

use crate::actor_lease_store::{ActorFence, ActorLeaseStore, ActorRelease, LeasedActor};
use crate::core::events::{Event, EventData};

/// The message id of `event` when it is input that starts or steers a turn.
pub(crate) fn turn_input(event: &Event) -> Option<MessageId> {
    match &event.data {
        EventData::InputMessage(data) if event.context.turn_id.is_none() => Some(data.message.id),
        _ => None,
    }
}

pub(crate) fn sequence(event: &Event) -> i64 {
    event.sequence.map_or(0, i64::from)
}

/// The input a fresh turn starts from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FreshInput {
    pub sequence: i64,
    pub message_id: MessageId,
}

pub(crate) struct Lease {
    store: Arc<dyn ActorLeaseStore>,
    holder: String,
    pub(crate) session_id: SessionId,
    fence: u64,
    ttl: Duration,
    page: u32,
    cursor: AtomicI64,
    /// The sequence of the fresh turn's input while its `turn.started` may
    /// not be stored yet; 0 when there is none. See the module notes.
    unstarted_input: AtomicI64,
    inbox_seq: AtomicI64,
    last_renewed: Mutex<Instant>,
    lost: Arc<watch::Sender<bool>>,
    /// Raised once a pending cancel targets this lease's turn.
    cancel: watch::Sender<bool>,
    target: Mutex<CancelTarget>,
}

/// What decides whether a pending cancel ends this lease's turn.
#[derive(Debug, Default)]
struct CancelTarget {
    /// The inbox position of the latest reported cancel.
    pending: Option<i64>,
    /// The sequence of the running turn's input, once the turn is known; see
    /// [`Lease::open_turn_input`] for an open turn's.
    turn_input: Option<i64>,
}

impl Lease {
    pub(crate) fn new(
        store: Arc<dyn ActorLeaseStore>,
        holder: String,
        actor: &LeasedActor,
        ttl: Duration,
        page: u32,
    ) -> Self {
        Self {
            store,
            holder,
            session_id: actor.session_id,
            fence: actor.fence,
            ttl,
            page,
            cursor: AtomicI64::new(actor.consumed_seq),
            unstarted_input: AtomicI64::new(0),
            inbox_seq: AtomicI64::new(actor.inbox_seq),
            last_renewed: Mutex::new(Instant::now()),
            lost: Arc::new(watch::channel(false).0),
            cancel: watch::channel(false).0,
            target: Mutex::new(CancelTarget {
                pending: actor.cancel_inbox_seq,
                turn_input: None,
            }),
        }
    }

    fn update_target(&self, update: impl FnOnce(&mut CancelTarget)) {
        let mut target = self
            .target
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut target);
        if let (Some(position), Some(input)) = (target.pending, target.turn_input)
            && input <= position
        {
            self.cancel.send_replace(true);
        }
    }

    /// The lease runs the turn started from the input at `input_seq`: a
    /// pending cancel at or past it ends the turn.
    pub(crate) fn begin_turn(&self, input_seq: i64) {
        self.update_target(|target| target.turn_input = Some(input_seq));
    }

    /// The inbox position of the latest reported cancel, if any.
    pub(crate) fn pending_cancel(&self) -> Option<i64> {
        self.target
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pending
    }

    /// The input a recorded cancel consumes: what was in the inbox when it
    /// was requested, as a withdraw consumes. Later input starts its own turn.
    pub(crate) fn cancelled_inbox(&self) -> i64 {
        let inbox = self.inbox_seq();
        self.pending_cancel()
            .map_or(inbox, |position| position.min(inbox))
    }

    pub(crate) fn cursor(&self) -> i64 {
        self.cursor.load(Ordering::SeqCst)
    }

    pub(crate) fn advance(&self, sequence: i64) {
        self.cursor.fetch_max(sequence, Ordering::SeqCst);
    }

    /// A fresh turn starts from the input at `sequence`: move the cursor over
    /// it, but commit below it until [`Lease::turn_stored`].
    pub(crate) fn start_fresh(&self, sequence: i64) {
        self.unstarted_input.store(sequence, Ordering::SeqCst);
        self.advance(sequence);
    }

    /// The turn's queued events (its `turn.started` among them) are stored:
    /// its input may be committed.
    pub(crate) fn turn_stored(&self) {
        self.unstarted_input.store(0, Ordering::SeqCst);
    }

    /// The cursor renew and release commit.
    pub(crate) fn committed_cursor(&self) -> i64 {
        let cursor = self.cursor();
        match self.unstarted_input.load(Ordering::SeqCst) {
            0 => cursor,
            input => cursor.min(input - 1),
        }
    }

    pub(crate) fn inbox_seq(&self) -> i64 {
        self.inbox_seq.load(Ordering::SeqCst)
    }

    /// Input arrived past the cursor (or the inbox moved for a resolution).
    pub(crate) fn has_pending(&self) -> bool {
        self.inbox_seq() > self.cursor()
    }

    /// The fence this lease's event appends carry; a refused append flips
    /// the lease lost.
    pub(crate) fn fence(&self) -> ActorFence {
        ActorFence::new(self.session_id, self.holder.clone(), self.fence)
            .with_lost_signal(self.lost.clone())
    }

    pub(crate) fn lost(&self) -> watch::Receiver<bool> {
        self.lost.subscribe()
    }

    pub(crate) fn is_lost(&self) -> bool {
        *self.lost.borrow()
    }

    pub(crate) fn cancel(&self) -> watch::Receiver<bool> {
        self.cancel.subscribe()
    }

    pub(crate) fn cancel_requested(&self) -> bool {
        *self.cancel.borrow()
    }

    /// Extend the lease and commit the cursor. A lost lease flips `lost`, as
    /// does a deleted session (`SessionNotFound`, EVE-1235); a store error
    /// does too once no renew succeeded for a whole TTL.
    pub(crate) async fn renew(&self) -> Result<()> {
        let renewal = self
            .store
            .renew_actor_lease(
                self.session_id,
                &self.holder,
                self.fence,
                self.ttl,
                self.committed_cursor(),
            )
            .await;
        match renewal {
            Ok(Some(renewal)) => {
                *self
                    .last_renewed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Instant::now();
                self.inbox_seq
                    .fetch_max(renewal.inbox_seq, Ordering::SeqCst);
                if let Some(position) = renewal.cancel_inbox_seq {
                    self.update_target(|target| target.pending = Some(position));
                }
                Ok(())
            }
            Ok(None) => {
                self.lost.send_replace(true);
                Ok(())
            }
            Err(error @ AgentLoopError::SessionNotFound(_)) => {
                self.lost.send_replace(true);
                Err(error)
            }
            Err(error) => {
                if self.since_renewed() >= self.ttl {
                    self.lost.send_replace(true);
                }
                Err(error)
            }
        }
    }

    /// Time since the last successful renew (or the lease was taken).
    pub(crate) fn since_renewed(&self) -> Duration {
        self.last_renewed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .elapsed()
    }

    /// Give the actor back with the cursor as consumed input.
    pub(crate) async fn release(&self) -> Result<ActorRelease> {
        self.store
            .release_actor(
                self.session_id,
                &self.holder,
                self.fence,
                self.committed_cursor(),
            )
            .await
    }

    /// The log's tail from the session's latest `turn.started`: what a fold
    /// of its open turn needs.
    pub(crate) async fn read_tail(&self) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        let mut after = None;
        loop {
            let journal = self
                .store
                .read_actor_journal(self.session_id, after, self.page)
                .await?;
            let more = journal.has_more && !journal.events.is_empty();
            after = journal.events.last().and_then(|event| event.sequence);
            events.extend(journal.events);
            if !more {
                return Ok(events);
            }
        }
    }

    /// Scan the log past the cursor, advancing it over every event read.
    /// Collects turn inputs, or, with `stop_at_input`, stops in front of the
    /// first one without consuming it.
    async fn scan(&self, stop_at_input: bool) -> Result<Vec<MessageId>> {
        let mut inputs = Vec::new();
        while self.has_pending() {
            let after = i32::try_from(self.cursor()).unwrap_or(i32::MAX);
            let journal = self
                .store
                .read_actor_journal(self.session_id, Some(after), self.page)
                .await?;
            if journal.events.is_empty() {
                // Nothing past the cursor: the inbox points at the log's head.
                self.advance(self.inbox_seq());
                break;
            }
            for event in &journal.events {
                if let Some(message_id) = turn_input(event) {
                    if stop_at_input {
                        return Ok(inputs);
                    }
                    inputs.push(message_id);
                }
                self.advance(sequence(event));
            }
            if !journal.has_more {
                break;
            }
        }
        Ok(inputs)
    }

    /// Every turn input past the cursor: steering for the running turn.
    pub(crate) async fn take_inputs(&self) -> Result<Vec<MessageId>> {
        self.scan(false).await
    }

    /// Move the cursor over events that are no turn's input (a resolution's
    /// writes, the turn's own events), stopping in front of the next input,
    /// so a release leaves the actor runnable only for real input.
    pub(crate) async fn skip_to_next_input(&self) -> Result<()> {
        self.scan(true).await.map(drop)
    }

    /// The sequence of the open turn's input (`input_message_id`, the turn
    /// started at `started_at`) as far as the pending cancel needs it: 0, which
    /// any cancel targets, unless the input came after the cancel. The tail
    /// starts at `turn.started`, so the input is looked for only when the turn
    /// started after the cancel's position, between the two.
    pub(crate) async fn open_turn_input(
        &self,
        input_message_id: MessageId,
        started_at: Option<i64>,
    ) -> Result<i64> {
        let (Some(position), Some(started_at)) = (self.pending_cancel(), started_at) else {
            return Ok(0);
        };
        if started_at <= position {
            return Ok(0);
        }
        let mut after = i32::try_from(position).unwrap_or(i32::MAX);
        loop {
            let journal = self
                .store
                .read_actor_journal(self.session_id, Some(after), self.page)
                .await?;
            for event in &journal.events {
                if sequence(event) >= started_at {
                    return Ok(0);
                }
                if turn_input(event) == Some(input_message_id) {
                    return Ok(sequence(event));
                }
            }
            match journal.events.last().and_then(|event| event.sequence) {
                Some(last) if journal.has_more => after = last,
                _ => return Ok(0),
            }
        }
    }

    /// The input a fresh turn starts from: the first turn input at or after
    /// `next_input_seq` when that is past the cursor (inputs before it were
    /// never marked, like a channel's history backfill), else the first turn
    /// input past the cursor. `None` when there is none.
    pub(crate) async fn fresh_input(&self, next_input_seq: i64) -> Result<Option<FreshInput>> {
        let consumed = self.cursor();
        let start_from = if next_input_seq > consumed {
            next_input_seq
        } else {
            consumed + 1
        };
        let mut first_after = None;
        let mut after = i32::try_from(consumed).unwrap_or(i32::MAX);
        loop {
            let journal = self
                .store
                .read_actor_journal(self.session_id, Some(after), self.page)
                .await?;
            for event in &journal.events {
                if let Some(message_id) = turn_input(event) {
                    let input = FreshInput {
                        sequence: sequence(event),
                        message_id,
                    };
                    if input.sequence >= start_from {
                        return Ok(Some(input));
                    }
                    first_after.get_or_insert(input);
                }
            }
            match journal.events.last().and_then(|event| event.sequence) {
                Some(last) if journal.has_more => after = last,
                _ => return Ok(first_after),
            }
        }
    }
}

/// Renew `lease` every third of its TTL, or at once when `poke` fires (a
/// cancel landed), until it is lost or this task is aborted.
///
/// Decision: after a failed renew the next one comes after a tenth of the TTL,
/// not a third. The lease is lost once no renew succeeds for a whole TTL (the
/// control plane hands the session on then, so the turn must stop by then);
/// retrying sooner lets a short control-plane blip end inside that window
/// instead of costing the turn and the model call it was waiting on.
pub(crate) async fn renew_loop(lease: Arc<Lease>, poke: Arc<Notify>) {
    let period = lease.ttl / 3;
    let retry = lease.ttl / 10;
    let mut failing = false;
    loop {
        let wait = if failing { retry } else { period };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = poke.notified() => {}
        }
        let since_renewed = lease.since_renewed();
        match lease.renew().await {
            Ok(()) => {
                if failing {
                    tracing::info!(
                        session_id = %lease.session_id,
                        since_renewed_ms = since_renewed.as_millis() as u64,
                        "Actor lease renewed after failed renews"
                    );
                }
                failing = false;
            }
            Err(AgentLoopError::SessionNotFound(_)) => {
                tracing::info!(session_id = %lease.session_id, "Session deleted, stopping turn");
                return;
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %lease.session_id,
                    since_renewed_ms = since_renewed.as_millis() as u64,
                    error = %error,
                    "Actor lease renew failed"
                );
                failing = true;
            }
        }
        if lease.is_lost() {
            tracing::warn!(session_id = %lease.session_id, "Actor lease lost; stopping the turn");
            return;
        }
    }
}
