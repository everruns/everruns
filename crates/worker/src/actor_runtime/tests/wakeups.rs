//! The lease loop picks work up on wake-ups, not on its poll: every loop
//! here polls once a minute, so a turn that starts within the test's wait
//! was started by a wake-up.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::Duration;

use tokio::sync::watch;

use super::*;
use crate::actor_runtime::{ActorSlots, ActorWakeups};
use crate::engine::model_wait::ModelWaitSlots;

/// Long enough that a test waiting on the poll times out first.
const NO_POLL: Duration = Duration::from_secs(60);

/// Model waits keep their slot.
struct KeepSlots;

impl ModelWaitSlots for KeepSlots {
    fn release(&self) -> bool {
        false
    }
    fn reacquire(&self) {}
}

fn slots(max_concurrent: usize) -> ActorSlots {
    ActorSlots {
        in_flight: Arc::new(AtomicUsize::new(0)),
        max_concurrent,
        claim_batch: max_concurrent,
        model_waits: Arc::new(KeepSlots),
        draining: Arc::new(AtomicBool::new(false)),
        poll_interval: NO_POLL,
        poll_backoff_max: NO_POLL,
        shutdown_grace: Duration::from_secs(1),
    }
}

/// Run `worker`'s lease loop until the returned sender fires.
fn spawn_loop(
    worker: &ActorRuntime<TestHosts>,
    slots: ActorSlots,
) -> (watch::Sender<bool>, tokio::task::JoinHandle<()>) {
    let (stop, shutdown) = watch::channel(false);
    let worker = worker.clone();
    let handle = tokio::spawn(async move { worker.run(slots, shutdown).await });
    (stop, handle)
}

async fn wait_for_completed(fixture: &Fixture, turns: usize) {
    tokio::time::timeout(WAIT, async {
        while fixture.count("turn.completed").await < turns {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{turns} turn(s) completed without waiting for a poll"));
}

async fn stop(stop: watch::Sender<bool>, handle: tokio::task::JoinHandle<()>) {
    let _ = stop.send(true);
    tokio::time::timeout(WAIT, handle)
        .await
        .expect("the loop stops")
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wake_up_starts_an_actor_marked_while_the_loop_sleeps() {
    let fixture = Fixture::new(model("Done.", vec![])).await;
    let wakeups = ActorWakeups::new();
    let (worker, _) = fixture.worker("w1");
    let worker = worker.with_wakeups(&wakeups);
    let (stopper, handle) = spawn_loop(&worker, slots(4));
    // The loop's first lease finds nothing; it is asleep on its poll now.
    tokio::time::sleep(Duration::from_millis(100)).await;

    fixture.send("Hello.").await;
    // What the turn backend does once the marking transaction committed.
    wakeups.runnable();

    wait_for_completed(&fixture, 1).await;
    stop(stopper, handle).await;
    fixture.assert_idle();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn input_that_lands_as_the_turn_ends_runs_in_the_freed_slot_at_once() {
    let fixture = Fixture::new(model("Answered.", vec![])).await;
    let (worker, probe) = fixture.worker("w1");
    fixture.send("First.").await;
    fixture.store_message("Second.").await;
    let sequence = fixture
        .events()
        .await
        .iter()
        .rev()
        .find_map(|event| event.sequence)
        .map(i64::from)
        .unwrap();
    let (store, session_id) = (fixture.store.clone(), fixture.session_id);
    // The mark lands after the turn's last inbox check: the release leaves
    // the actor runnable, with no wake-up from the control plane.
    *probe.on_turn_ended.lock().unwrap() = Some(Box::pin(async move {
        store.mark_runnable(session_id, ORG_ID, sequence);
    }));

    // One slot: the second turn can start only once the first one's slot is
    // free, so the finished actor's own wake-up must come after that.
    let (stopper, handle) = spawn_loop(&worker, slots(1));
    wait_for_completed(&fixture, 2).await;
    stop(stopper, handle).await;
    fixture.assert_idle();
}
