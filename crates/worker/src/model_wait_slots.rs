//! The worker's side of `engine::model_wait`: a reason step
//! waiting on the model hands its execution slot back, so the worker can claim
//! other work meanwhile.
//!
//! Decision: waits get their own cap (`MAX_CONCURRENT_MODEL_WAITS`) instead of
//! being unbounded. Each open stream still costs memory, a connection, and
//! batched delta writes; past the cap a wait simply keeps its slot, which is
//! the old behaviour. Set it to 0 to turn slot release off.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::engine::model_wait::ModelWaitSlots;
use tokio::sync::Notify;

/// Default for `MAX_CONCURRENT_MODEL_WAITS`, per worker.
pub const DEFAULT_MAX_CONCURRENT_MODEL_WAITS: usize = 200;

pub(crate) struct WorkerModelWaitSlots {
    in_flight: Arc<AtomicUsize>,
    waiting: AtomicUsize,
    max_waiting: usize,
    wake: Arc<Notify>,
}

impl WorkerModelWaitSlots {
    pub(crate) fn new(in_flight: Arc<AtomicUsize>, max_waiting: usize, wake: Arc<Notify>) -> Self {
        Self {
            in_flight,
            waiting: AtomicUsize::new(0),
            max_waiting,
            wake,
        }
    }
}

impl ModelWaitSlots for WorkerModelWaitSlots {
    fn release(&self) -> bool {
        let reserved = self
            .waiting
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < self.max_waiting).then_some(n + 1)
            })
            .is_ok();
        if reserved {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            // A slot just opened: let the poll loop claim into it now.
            self.wake.notify_one();
        }
        reserved
    }

    fn reacquire(&self) {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        self.waiting.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_frees_a_slot_until_reacquired() {
        let in_flight = Arc::new(AtomicUsize::new(2));
        let slots = WorkerModelWaitSlots::new(in_flight.clone(), 5, Arc::new(Notify::new()));
        assert!(slots.release());
        assert_eq!(in_flight.load(Ordering::SeqCst), 1);
        slots.reacquire();
        assert_eq!(in_flight.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn waits_past_the_cap_keep_their_slot() {
        let in_flight = Arc::new(AtomicUsize::new(3));
        let slots = WorkerModelWaitSlots::new(in_flight.clone(), 2, Arc::new(Notify::new()));
        assert!(slots.release());
        assert!(slots.release());
        assert!(!slots.release());
        assert_eq!(in_flight.load(Ordering::SeqCst), 1);
        slots.reacquire();
        assert!(slots.release());
    }

    #[test]
    fn zero_cap_turns_release_off() {
        let in_flight = Arc::new(AtomicUsize::new(1));
        let slots = WorkerModelWaitSlots::new(in_flight.clone(), 0, Arc::new(Notify::new()));
        assert!(!slots.release());
        assert_eq!(in_flight.load(Ordering::SeqCst), 1);
    }
}
