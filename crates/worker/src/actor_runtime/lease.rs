//! A held actor lease: renewal, the inbox cursor and the journal reads a
//! turn's inbox makes.
//!
//! Decisions:
//! - The cursor is the highest event sequence the holder examined for input.
//!   Every turn input up to it reached a turn (the fresh turn's own input, or
//!   steering handed to a reason). Renew and release commit it, so a holder
//!   that dies mid-turn leaves the next one a cursor that delivers only input
//!   the model never saw.
//! - A turn input is an `input.message` without a turn id. Everything a turn
//!   or a resolution writes into the conversation carries the turn's id, so
//!   it is part of that turn's history, never input that starts or steers one.
//! - Lease loss is observed, never assumed away: a renew that finds another
//!   holder's fence, or no successful renew for a whole TTL, flips `lost`, and
//!   the turn stops without writing anything more.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

use everruns_contracts::error::Result;
use everruns_contracts::typed_id::{MessageId, SessionId};
use tokio::sync::{Notify, watch};

use crate::actor_lease_store::{ActorLeaseStore, ActorRelease, LeasedActor};
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
    inbox_seq: AtomicI64,
    last_renewed: Mutex<Instant>,
    lost: watch::Sender<bool>,
    cancel: watch::Sender<bool>,
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
            inbox_seq: AtomicI64::new(actor.inbox_seq),
            last_renewed: Mutex::new(Instant::now()),
            lost: watch::channel(false).0,
            cancel: watch::channel(actor.cancel_requested).0,
        }
    }

    pub(crate) fn cursor(&self) -> i64 {
        self.cursor.load(Ordering::SeqCst)
    }

    pub(crate) fn advance(&self, sequence: i64) {
        self.cursor.fetch_max(sequence, Ordering::SeqCst);
    }

    pub(crate) fn inbox_seq(&self) -> i64 {
        self.inbox_seq.load(Ordering::SeqCst)
    }

    /// Input arrived past the cursor (or the inbox moved for a resolution).
    pub(crate) fn has_pending(&self) -> bool {
        self.inbox_seq() > self.cursor()
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

    /// Extend the lease and commit the cursor. A lost lease flips `lost`; a
    /// store error does too once no renew succeeded for a whole TTL.
    pub(crate) async fn renew(&self) -> Result<()> {
        let renewal = self
            .store
            .renew_actor_lease(
                self.session_id,
                &self.holder,
                self.fence,
                self.ttl,
                self.cursor(),
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
                if renewal.cancel_requested {
                    self.cancel.send_replace(true);
                }
                Ok(())
            }
            Ok(None) => {
                self.lost.send_replace(true);
                Ok(())
            }
            Err(error) => {
                let stale = self
                    .last_renewed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .elapsed()
                    >= self.ttl;
                if stale {
                    self.lost.send_replace(true);
                }
                Err(error)
            }
        }
    }

    /// Give the actor back with the cursor as consumed input.
    pub(crate) async fn release(&self) -> Result<ActorRelease> {
        self.store
            .release_actor(self.session_id, &self.holder, self.fence, self.cursor())
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
pub(crate) async fn renew_loop(lease: Arc<Lease>, poke: Arc<Notify>) {
    let period = lease.ttl / 3;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(period) => {}
            _ = poke.notified() => {}
        }
        if let Err(error) = lease.renew().await {
            tracing::warn!(session_id = %lease.session_id, error = %error, "Actor lease renew failed");
        }
        if lease.is_lost() {
            tracing::warn!(session_id = %lease.session_id, "Actor lease lost; stopping the turn");
            return;
        }
    }
}
