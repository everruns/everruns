//! Handing activations to executors that pull them with credits (data plane
//! v2 step 6, design.md "Executors").
//!
//! Decisions:
//! - Executors are a pool shared by every cell. An executor offers each
//!   cell some of its free slots as **credits** in a long poll
//!   (`/cell/exec/pull`); the cell's leader hands out an activation only
//!   against a credit, so an executor is never oversubscribed. Credits the
//!   cell can't use within the poll go back with an empty reply, and the
//!   executor offers them elsewhere.
//! - **Soft affinity**: a session goes first to the executor that ran it
//!   last, while that executor has a credit here, because its mirror and
//!   caches are warm; otherwise to the executor offering the most credits.
//! - Owed sessions no credit was free for wait in a queue, oldest first,
//!   and are handed out as credits arrive; the scheduler's periodic sweep
//!   covers anything the queue missed.
//! - A pull whose executor went away (its connection dropped, so the
//!   handler never finished) is withdrawn at once: its credits would
//!   otherwise stay on offer, and soft affinity would keep handing that
//!   executor's sessions to nobody until they ran out.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use everruns_contracts::typed_id::SessionId;
use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::remote::ExecutorId;

#[derive(Default)]
pub struct Dispatch {
    pulls: Mutex<HashMap<ExecutorId, Pull>>,
    waiting: Mutex<Waiting>,
    /// The executor that last got each session.
    last: Mutex<HashMap<SessionId, ExecutorId>>,
    next_token: std::sync::atomic::AtomicU64,
}

#[derive(Default)]
struct Waiting {
    order: VecDeque<SessionId>,
    queued: HashSet<SessionId>,
}

struct Pull {
    token: u64,
    credits: usize,
    offered: Vec<SessionId>,
    notify: Arc<Notify>,
}

/// A registered pull: wait on `notify`, then [`Dispatch::take`] it.
pub struct Ticket {
    pub executor: ExecutorId,
    pub token: u64,
    pub notify: Arc<Notify>,
}

impl Dispatch {
    /// Hold `credits` from `executor` until taken. A newer pull from the
    /// same executor replaces an older one (whose waiter then finds it
    /// gone and returns what it was offered, if anything).
    pub fn register(&self, executor: ExecutorId, credits: usize) -> Ticket {
        let token = self
            .next_token
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let notify = Arc::new(Notify::new());
        let replaced = self.pulls.lock().insert(
            executor,
            Pull {
                token,
                credits,
                offered: Vec::new(),
                notify: notify.clone(),
            },
        );
        if let Some(old) = replaced {
            // Its offers would be lost with it: put them back first.
            for session in old.offered {
                self.queue_front(session);
            }
            old.notify.notify_one();
        }
        Ticket {
            executor,
            token,
            notify,
        }
    }

    /// Whether `ticket`'s pull has used all its credits.
    pub fn full(&self, ticket: &Ticket) -> bool {
        self.pulls
            .lock()
            .get(&ticket.executor)
            .is_none_or(|p| p.token != ticket.token || p.credits == 0)
    }

    /// End `ticket`'s pull and take what it was offered.
    pub fn take(&self, ticket: &Ticket) -> Vec<SessionId> {
        let mut pulls = self.pulls.lock();
        match pulls.get(&ticket.executor) {
            Some(pull) if pull.token == ticket.token => pulls
                .remove(&ticket.executor)
                .map(|p| p.offered)
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Withdraw `ticket`'s pull, abandoned before [`take`](Self::take):
    /// its credits go, and what it was offered waits again, first in line.
    pub fn abandon(&self, ticket: &Ticket) {
        let offered = {
            let mut pulls = self.pulls.lock();
            match pulls.get(&ticket.executor) {
                Some(pull) if pull.token == ticket.token => pulls
                    .remove(&ticket.executor)
                    .map(|p| p.offered)
                    .unwrap_or_default(),
                _ => return,
            }
        };
        for session in offered.into_iter().rev() {
            self.queue_front(session);
        }
    }

    /// Hand `session` to a pull with a free credit, the executor that ran
    /// it last first. `None`: no credit is free.
    pub fn offer(&self, session: SessionId) -> Option<ExecutorId> {
        let mut pulls = self.pulls.lock();
        let last = self.last.lock().get(&session).copied();
        let executor = last
            .filter(|last| pulls.get(last).is_some_and(|p| p.credits > 0))
            .or_else(|| {
                pulls
                    .iter()
                    .filter(|(_, p)| p.credits > 0)
                    .max_by_key(|(id, p)| (p.credits, std::cmp::Reverse(**id)))
                    .map(|(id, _)| *id)
            })?;
        let pull = pulls.get_mut(&executor)?;
        pull.credits -= 1;
        pull.offered.push(session);
        pull.notify.notify_one();
        drop(pulls);
        self.last.lock().insert(session, executor);
        Some(executor)
    }

    /// Credits held and not yet used.
    pub fn free_credits(&self) -> usize {
        self.pulls.lock().values().map(|p| p.credits).sum()
    }

    /// Owed `session` waits for a credit.
    pub fn queue(&self, session: SessionId) {
        let mut waiting = self.waiting.lock();
        if waiting.queued.insert(session) {
            waiting.order.push_back(session);
        }
    }

    fn queue_front(&self, session: SessionId) {
        let mut waiting = self.waiting.lock();
        if waiting.queued.insert(session) {
            waiting.order.push_front(session);
        }
    }

    /// Up to `n` waiting sessions, oldest first, out of the queue.
    pub fn waiting(&self, n: usize) -> Vec<SessionId> {
        let mut waiting = self.waiting.lock();
        let n = n.min(waiting.order.len());
        let out: Vec<SessionId> = waiting.order.drain(..n).collect();
        for session in &out {
            waiting.queued.remove(session);
        }
        out
    }

    pub fn queued(&self) -> usize {
        self.waiting.lock().order.len()
    }

    /// Forget sessions that are no longer here.
    pub fn retain(&self, here: impl Fn(&SessionId) -> bool) {
        self.last.lock().retain(|session, _| here(session));
        let mut waiting = self.waiting.lock();
        waiting.order.retain(|session| here(session));
        let order: HashSet<SessionId> = waiting.order.iter().copied().collect();
        waiting.queued = order;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_goes_back_to_its_last_executor_while_it_has_credit() {
        let dispatch = Dispatch::default();
        let s = SessionId::new();
        let a = dispatch.register(1, 1);
        assert_eq!(dispatch.offer(s), Some(1));
        assert_eq!(dispatch.take(&a), vec![s]);
        let _a = dispatch.register(1, 1);
        let _b = dispatch.register(2, 5);
        assert_eq!(dispatch.offer(s), Some(1), "affinity beats more credits");
        assert_eq!(dispatch.offer(SessionId::new()), Some(2));
    }

    #[test]
    fn no_credit_no_activation() {
        let dispatch = Dispatch::default();
        assert_eq!(dispatch.offer(SessionId::new()), None);
        let ticket = dispatch.register(1, 1);
        assert!(dispatch.offer(SessionId::new()).is_some());
        assert!(dispatch.full(&ticket));
        assert_eq!(dispatch.offer(SessionId::new()), None);
    }

    #[test]
    fn a_replaced_pull_gives_its_offers_back() {
        let dispatch = Dispatch::default();
        let s = SessionId::new();
        let old = dispatch.register(1, 2);
        dispatch.offer(s);
        let _new = dispatch.register(1, 2);
        assert!(dispatch.take(&old).is_empty());
        assert_eq!(dispatch.waiting(10), vec![s]);
    }

    #[test]
    fn an_abandoned_pull_frees_its_credits_and_gives_its_offers_back() {
        let dispatch = Dispatch::default();
        let (a, b) = (SessionId::new(), SessionId::new());
        let gone = dispatch.register(1, 4);
        assert_eq!(dispatch.offer(a), Some(1));
        dispatch.abandon(&gone);
        assert_eq!(dispatch.free_credits(), 0, "the gone executor's credits");
        assert_eq!(dispatch.waiting(10), vec![a], "its offer waits again");
        let _other = dispatch.register(2, 4);
        assert_eq!(
            dispatch.offer(a),
            Some(2),
            "affinity no longer points at the executor that went away"
        );
        assert_eq!(dispatch.offer(b), Some(2));
    }

    #[test]
    fn abandoning_a_replaced_pull_leaves_the_newer_one() {
        let dispatch = Dispatch::default();
        let old = dispatch.register(1, 2);
        let _new = dispatch.register(1, 3);
        dispatch.abandon(&old);
        assert_eq!(dispatch.free_credits(), 3);
    }

    #[test]
    fn waiting_sessions_leave_in_order_once() {
        let dispatch = Dispatch::default();
        let (a, b) = (SessionId::new(), SessionId::new());
        dispatch.queue(a);
        dispatch.queue(b);
        dispatch.queue(a);
        assert_eq!(dispatch.waiting(10), vec![a, b]);
        assert!(dispatch.waiting(10).is_empty());
    }
}
