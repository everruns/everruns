//! Coalesces one holder's lease renewals into batched store calls.
//!
//! Decisions:
//! - Group commit, not a timer: a renewal goes out at once when none is in
//!   flight; renewals that arrive while one is in flight wait for it and go
//!   out together in the next call. An idle holder pays no added latency; a
//!   busy one sends one call (and one statement) for many leases.
//! - Every lease keeps its own expiry, fence and cursor: the batch is only
//!   transport. A lost lease, a deleted session and a failed renewal come
//!   back per lease exactly as a single renewal reports them; a failed batch
//!   fails each of its renewals, which the lease's renewer retries.
//! - The renewers' periodic renewals line up on one clock (see
//!   `lease::renew_loop`), so a holder's running turns renew in the same
//!   batch rather than spread across the period.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::SessionId;
use tokio::sync::oneshot;

use super::store::{
    ActorJournal, ActorLeaseRenewal, ActorRelease, ActorRenewal, SessionActorStore,
};

/// Most leases in one call.
const MAX_BATCH: usize = 500;

type Reply = oneshot::Sender<Result<Option<ActorRenewal>>>;

struct Waiting {
    holder: String,
    ttl: Duration,
    lease: ActorLeaseRenewal,
    reply: Reply,
}

#[derive(Default)]
struct Queue {
    flushing: bool,
    waiting: Vec<Waiting>,
}

/// A [`SessionActorStore`] whose renewals are batched; every other call goes
/// straight to the wrapped store.
pub struct BatchedRenewals {
    inner: Arc<dyn SessionActorStore>,
    queue: Arc<Mutex<Queue>>,
}

impl BatchedRenewals {
    pub fn new(inner: Arc<dyn SessionActorStore>) -> Self {
        Self {
            inner,
            queue: Arc::default(),
        }
    }
}

/// Hands the flush over if a flush task dies mid-call (a panic in the store),
/// so the next renewal leads a new one instead of waiting forever.
struct Unstick {
    queue: Arc<Mutex<Queue>>,
    armed: bool,
}

impl Drop for Unstick {
    fn drop(&mut self) {
        if self.armed {
            let mut queue = self
                .queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            queue.flushing = false;
        }
    }
}

/// Send what is waiting until nothing is.
async fn flush(inner: Arc<dyn SessionActorStore>, queue: Arc<Mutex<Queue>>) {
    let mut unstick = Unstick {
        queue: queue.clone(),
        armed: true,
    };
    loop {
        let waiting = {
            let mut queue = queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if queue.waiting.is_empty() {
                queue.flushing = false;
                unstick.armed = false;
                return;
            }
            std::mem::take(&mut queue.waiting)
        };
        // One call per holder and TTL; a driver has one of each.
        let mut groups: Vec<Vec<Waiting>> = Vec::new();
        for item in waiting {
            match groups
                .iter_mut()
                .find(|group| group[0].holder == item.holder && group[0].ttl == item.ttl)
            {
                Some(group) => group.push(item),
                None => groups.push(vec![item]),
            }
        }
        for group in groups.into_iter().flat_map(chunked) {
            let leases: Vec<ActorLeaseRenewal> = group.iter().map(|item| item.lease).collect();
            let renewed = inner
                .renew_actor_leases(&group[0].holder, group[0].ttl, &leases)
                .await;
            match renewed {
                Ok(results) if results.len() == group.len() => {
                    for (item, result) in group.into_iter().zip(results) {
                        let _ = item.reply.send(result);
                    }
                }
                Ok(results) => {
                    let message = format!(
                        "lease renewal batch answered {} of {} leases",
                        results.len(),
                        group.len()
                    );
                    for item in group {
                        let _ = item.reply.send(Err(AgentLoopError::store(message.clone())));
                    }
                }
                Err(error) => {
                    let message = format!("lease renewal batch failed: {error}");
                    for item in group {
                        let _ = item.reply.send(Err(AgentLoopError::store(message.clone())));
                    }
                }
            }
        }
    }
}

/// Calls of at most [`MAX_BATCH`] leases (the control plane caps one).
fn chunked(mut group: Vec<Waiting>) -> Vec<Vec<Waiting>> {
    let mut chunks = Vec::new();
    while group.len() > MAX_BATCH {
        let rest = group.split_off(MAX_BATCH);
        chunks.push(std::mem::replace(&mut group, rest));
    }
    chunks.push(group);
    chunks
}

#[async_trait]
impl SessionActorStore for BatchedRenewals {
    async fn renew_actor_lease(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        ttl: Duration,
        consumed_seq: i64,
    ) -> Result<Option<ActorRenewal>> {
        let (reply, answer) = oneshot::channel();
        let lead = {
            let mut queue = self
                .queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            queue.waiting.push(Waiting {
                holder: holder.to_string(),
                ttl,
                lease: ActorLeaseRenewal {
                    session_id,
                    fence,
                    consumed_seq,
                },
                reply,
            });
            !std::mem::replace(&mut queue.flushing, true)
        };
        if lead {
            // Its own task: a caller that gives up waiting (the renewal's
            // timeout) must not strand the renewals queued behind it.
            drop(everruns_contracts::rt::spawn(flush(
                self.inner.clone(),
                self.queue.clone(),
            )));
        }
        answer
            .await
            .unwrap_or_else(|_| Err(AgentLoopError::store("lease renewal batch stopped")))
    }

    async fn renew_actor_leases(
        &self,
        holder: &str,
        ttl: Duration,
        leases: &[ActorLeaseRenewal],
    ) -> Result<Vec<Result<Option<ActorRenewal>>>> {
        self.inner.renew_actor_leases(holder, ttl, leases).await
    }

    async fn release_actor(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        consumed_inbox_seq: i64,
    ) -> Result<ActorRelease> {
        self.inner
            .release_actor(session_id, holder, fence, consumed_inbox_seq)
            .await
    }

    async fn record_actor_progress(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
    ) -> Result<bool> {
        self.inner
            .record_actor_progress(session_id, holder, fence)
            .await
    }

    async fn hand_off_actor(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        consumed_inbox_seq: i64,
    ) -> Result<Option<ActorRelease>> {
        self.inner
            .hand_off_actor(session_id, holder, fence, consumed_inbox_seq)
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

#[cfg(test)]
#[path = "renewals_tests.rs"]
mod tests;
