//! A long tool call detaches: its turn parks and frees the lease, the call
//! finishes on the worker's detached-call runner, and its completion resumes
//! the turn (knowledge/execution/detached-tool-calls.md).

use super::*;
use crate::actor_runtime::{DetachedCalls, DetachedCallsConfig};
use crate::engine::tool_detach;

const DETACH_AFTER: Duration = Duration::from_millis(50);

impl Fixture {
    /// A detached-call runner for `holder`, appending to the session's log.
    fn detached(&self, holder: &str) -> Arc<DetachedCalls> {
        self.detached_capped(holder, 10)
    }

    fn detached_capped(&self, holder: &str, max_calls: usize) -> Arc<DetachedCalls> {
        let runtime = self.runtime.clone();
        Arc::new(DetachedCalls::new(
            self.store.clone(),
            holder,
            DetachedCallsConfig {
                detach_after: DETACH_AFTER,
                max_calls,
                ttl: Duration::from_secs(3),
            },
            Arc::new(move |_fence| runtime.event_emitter()),
        ))
    }

    async fn wait_until(&self, what: &str, done: impl Fn(&Self) -> bool) {
        tokio::time::timeout(WAIT, async {
            while !done(self) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    }
}

/// Lease the actor on `worker` and run its turn with `detached` available,
/// as the worker's lease loop does.
async fn run_detaching(worker: &ActorRuntime<TestHosts>, detached: &Arc<DetachedCalls>) {
    let mut leased = worker.lease(1).await.expect("lease");
    let actor = leased.pop().expect("a runnable actor");
    let detacher = detached.for_lease(actor.fence, Arc::new(Notify::new()));
    tokio::time::timeout(
        WAIT,
        tool_detach::provide(detacher, worker.run_actor(actor)),
    )
    .await
    .expect("the actor's turn ends");
}

/// Send a message whose turn calls `tool` (held), and run it until the turn
/// parks on the detached call.
async fn park_on(tool: &str) -> (Fixture, ActorRuntime<TestHosts>, Arc<DetachedCalls>) {
    let fixture = Fixture::new(model("Done.", vec![vec![call("c1", tool)]])).await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (worker, _) = fixture.worker("w1");
    let detached = fixture.detached("w1");
    fixture.send("Check the inbox.").await;
    run_detaching(&worker, &detached).await;

    // The turn parked while the tool still runs: nobody holds the actor and
    // nothing makes it runnable until the call ends.
    fixture.assert_idle();
    assert_eq!(fixture.store.detached(fixture.session_id), ["c1"]);
    assert_eq!(detached.len(), 1);
    assert_eq!(fixture.count("tool.started").await, 1);
    assert_eq!(fixture.count("tool.completed").await, 0);
    assert_eq!(fixture.count("act.completed").await, 0);
    assert!(fixture.open_turn().await.is_some(), "the turn stays open");
    (fixture, worker, detached)
}

#[tokio::test]
async fn a_long_call_parks_its_turn_and_its_completion_resumes_it() {
    let (fixture, worker, detached) = park_on("read_inbox").await;
    let parked = fixture.open_turn().await.unwrap();

    // Input sent meanwhile waits for the call, as it waits for any act.
    fixture.send("Also check spam.").await;
    assert!(worker.lease(1).await.unwrap().is_empty());

    fixture.gate.release.notify_one();
    fixture
        .wait_until("the completion", |fixture| {
            fixture.store.detached(fixture.session_id).is_empty()
        })
        .await;
    assert!(detached.is_empty());
    assert_eq!(fixture.count("tool.completed").await, 1);

    run_until_idle(&worker).await;
    fixture.assert_idle();
    let events = fixture.events().await;
    assert_eq!(
        turn_ids(&events, "tool.completed"),
        vec![Some(parked.turn_id())],
        "the call completed under its own turn"
    );
    assert_eq!(
        turn_ids(&events, "turn.completed")[0],
        Some(parked.turn_id())
    );
    assert_eq!(fixture.tool_calls(), ["read_inbox"], "the tool ran once");
    assert_eq!(fixture.answers().await[0], "Done.");
}

#[tokio::test]
async fn a_safe_call_lost_with_its_worker_runs_again_on_the_next_one() {
    let (fixture, _, detached) = park_on("read_inbox").await;

    // The worker stops: the call is dropped and its record expires now.
    detached.stop().await;
    assert!(detached.is_empty());

    let (other, _) = fixture.worker("w2");
    run_until_idle(&other).await;
    fixture.assert_idle();
    assert_eq!(fixture.tool_calls(), ["read_inbox", "read_inbox"]);
    assert_eq!(fixture.count("tool.completed").await, 1);
    assert!(fixture.open_turn().await.is_none());
}

#[tokio::test]
async fn an_at_most_once_call_lost_with_its_worker_settles_interrupted() {
    let (fixture, _, detached) = park_on("send_email").await;
    detached.stop().await;

    let (other, _) = fixture.worker("w2");
    run_until_idle(&other).await;
    fixture.assert_idle();
    assert_eq!(fixture.tool_calls(), ["send_email"], "never run twice");
    let interrupted = fixture.events().await.into_iter().any(|event| {
        matches!(&event.data, crate::core::events::EventData::ToolCompleted(data)
            if data.status == "interrupted")
    });
    assert!(interrupted, "the uncertain call settles as interrupted");
    assert!(fixture.open_turn().await.is_none());
}

#[tokio::test]
async fn a_cancel_stops_the_detached_call() {
    let (fixture, worker, detached) = park_on("read_inbox").await;

    // Nothing holds the parked turn: the caller records the cancel, and the
    // call's record is gone.
    assert_eq!(
        fixture.store.cancel(fixture.session_id),
        store::Cancel::Idle
    );
    let poke = worker.renew_poke();
    let (_stop, shutdown) = tokio::sync::watch::channel(false);
    let renewer = tokio::spawn(detached.clone().run(poke.clone(), shutdown));
    fixture
        .wait_until("the call to stop", |_| {
            poke.notify_waiters();
            detached.is_empty()
        })
        .await;
    renewer.abort();

    // The tool never finishes, and nothing resumes the turn.
    fixture.gate.release.notify_one();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fixture.count("tool.completed").await, 0);
    assert!(worker.lease(1).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_turn_that_does_not_park_drops_its_detached_calls() {
    let fixture = Fixture::new(model("Done.", vec![vec![call("c1", "read_inbox")]])).await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let probe = Arc::new(Probe::default());
    // The turn's queued events are not stored when it would park (its second
    // flush, after the one taking its input): it runs again from the log.
    probe.fail_at_flush.store(2, Ordering::SeqCst);
    let (worker, _) = fixture.worker_with_probe("w1", probe);
    let detached = fixture.detached("w1");
    fixture.send("Check the inbox.").await;
    run_detaching(&worker, &detached).await;

    // The detached call was stopped; the retry ran the safe call again and
    // finished the turn on the same lease.
    assert!(detached.is_empty());
    assert_eq!(fixture.tool_calls(), ["read_inbox", "read_inbox"]);
    assert_eq!(fixture.count("tool.completed").await, 1);
    assert!(fixture.open_turn().await.is_none());
    fixture.assert_idle();
}

#[tokio::test]
async fn past_the_cap_a_long_call_stays_with_its_turn() {
    let fixture = Fixture::new(model("Done.", vec![vec![call("c1", "read_inbox")]])).await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (worker, _) = fixture.worker("w1");
    let detached = fixture.detached_capped("w1", 0);
    fixture.send("Check the inbox.").await;
    let release = {
        let gate = fixture.gate.clone();
        tokio::spawn(async move {
            tokio::time::sleep(DETACH_AFTER * 4).await;
            gate.release.notify_one();
        })
    };
    run_detaching(&worker, &detached).await;
    release.await.unwrap();

    // The turn held its lease through the call and finished on it.
    assert!(detached.is_empty());
    assert!(fixture.store.detached(fixture.session_id).is_empty());
    assert_eq!(fixture.tool_calls(), ["read_inbox"]);
    assert_eq!(fixture.count("act.completed").await, 1);
    assert!(fixture.open_turn().await.is_none());
    fixture.assert_idle();
}
