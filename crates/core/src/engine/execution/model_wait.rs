//! Lets a host hand back an execution slot while a turn waits on the model.
//!
//! Decision: a worker caps how many tasks it runs at once so a small
//! database and CPU are not swamped. A reason step spends most of its time
//! waiting on the model's stream, doing little database or CPU work, yet it
//! held its slot for the whole call. With real model latencies that cap, not
//! the box, limited how many turns could talk to a model at once (prod load
//! test, 2026-10-08: about 60 slots added ~2 s of queueing at 100 sessions).
//!
//! The host installs [`ModelWaitSlots`] around a task with [`scope`]; the
//! reason step frees the slot for the model call and takes it back after.
//! Taking it back never waits: the host may run over its cap briefly while
//! the step finishes, and claims no new work until it is back under. Hosts
//! that install nothing are unaffected.

use std::sync::Arc;

/// A host's execution slots, as seen by a step waiting on the model.
pub trait ModelWaitSlots: Send + Sync {
    /// Frees the caller's slot for a model wait. Returns `false` to keep it,
    /// for example when the host already has as many waits as it allows.
    fn release(&self) -> bool;
    /// Takes the slot back after a wait [`release`](Self::release) allowed.
    fn reacquire(&self);
}

tokio::task_local! {
    static SLOTS: Arc<dyn ModelWaitSlots>;
}

/// Runs `fut` with `slots` available to model waits inside it.
pub async fn scope<F: Future>(slots: Arc<dyn ModelWaitSlots>, fut: F) -> F::Output {
    SLOTS.scope(slots, fut).await
}

/// Holds a released slot; dropping it takes the slot back.
#[must_use = "the slot comes back when this is dropped"]
pub struct ModelWait(Option<Arc<dyn ModelWaitSlots>>);

impl Drop for ModelWait {
    fn drop(&mut self) {
        if let Some(slots) = self.0.take() {
            slots.reacquire();
        }
    }
}

/// Frees the current task's slot, if its host installed one and allows it.
pub fn begin() -> ModelWait {
    let slots = SLOTS
        .try_with(Arc::clone)
        .ok()
        .filter(|slots| slots.release());
    ModelWait(slots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counter {
        free: AtomicUsize,
        allow: bool,
    }

    impl ModelWaitSlots for Counter {
        fn release(&self) -> bool {
            if self.allow {
                self.free.fetch_add(1, Ordering::SeqCst);
            }
            self.allow
        }
        fn reacquire(&self) {
            self.free.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn counter(allow: bool) -> Arc<Counter> {
        Arc::new(Counter {
            free: AtomicUsize::new(0),
            allow,
        })
    }

    #[tokio::test]
    async fn wait_frees_the_slot_until_dropped() {
        let slots = counter(true);
        let observed = slots.clone();
        scope(slots, async move {
            let wait = begin();
            assert_eq!(observed.free.load(Ordering::SeqCst), 1);
            drop(wait);
            assert_eq!(observed.free.load(Ordering::SeqCst), 0);
        })
        .await;
    }

    #[tokio::test]
    async fn refused_release_is_not_given_back() {
        let slots = counter(false);
        let observed = slots.clone();
        scope(slots, async move {
            drop(begin());
            assert_eq!(observed.free.load(Ordering::SeqCst), 0);
        })
        .await;
    }

    #[tokio::test]
    async fn outside_a_scope_nothing_happens() {
        drop(begin());
    }

    #[tokio::test]
    async fn cancelled_wait_gives_the_slot_back() {
        let slots = counter(true);
        let observed = slots.clone();
        let task = tokio::spawn(scope(slots, async {
            let _wait = begin();
            std::future::pending::<()>().await;
        }));
        tokio::task::yield_now().await;
        assert_eq!(observed.free.load(Ordering::SeqCst), 1);
        task.abort();
        let _ = task.await;
        assert_eq!(observed.free.load(Ordering::SeqCst), 0);
    }
}
