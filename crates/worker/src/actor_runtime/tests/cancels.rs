//! A cancel ends the turn it was requested for, however its holder dies, and
//! never a turn whose input came after it.

use std::sync::atomic::Ordering;

use super::store::Cancel;
use super::*;

/// Terminal turn events in log order.
async fn terminals(fixture: &Fixture) -> Vec<String> {
    fixture
        .types()
        .await
        .into_iter()
        .filter(|seen| {
            matches!(
                seen.as_str(),
                "turn.completed" | "turn.failed" | "turn.cancelled" | "turn.sealed"
            )
        })
        .collect()
}

/// Run `worker` until its tool gate holds a call, then return the run.
async fn run_until_held(
    fixture: &Fixture,
    worker: &ActorRuntime<TestHosts>,
) -> tokio::task::JoinHandle<bool> {
    let running = {
        let worker = worker.clone();
        tokio::spawn(async move { run_once(&worker).await })
    };
    tokio::time::timeout(WAIT, fixture.gate.held.notified())
        .await
        .expect("the turn reaches its tool");
    running
}

/// Kill the worker running `run` and let its lease run out.
async fn kill(fixture: &Fixture, run: tokio::task::JoinHandle<bool>) {
    run.abort();
    let _ = run.await;
    fixture.store.expire(fixture.session_id);
}

// Chaos harness: turn 1 was cancelled and its holder recorded the cancel,
// then failed its next write and its lease expired without a release. The
// cancel stayed pending on the row, and the next holder applied it to the
// follow-up message's turn, cancelling it 5 ms after it started.
#[tokio::test]
async fn a_cancel_whose_holder_dies_before_release_spares_the_next_turn() {
    let fixture = Fixture::new(model(
        "Done.",
        vec![
            vec![call("c1", "read_inbox")],
            vec![call("c2", "read_inbox")],
        ],
    ))
    .await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (first, probe) = fixture.worker("w1");
    let ended = Arc::new(Notify::new());
    {
        let ended = ended.clone();
        // The holder recorded the cancel and dies before it releases.
        *probe.on_turn_ended.lock().unwrap() = Some(Box::pin(async move {
            ended.notify_one();
            std::future::pending::<()>().await;
        }));
    }
    let cancelled_input = fixture.send("Read my inbox.").await;
    let run = run_until_held(&fixture, &first).await;
    assert_eq!(fixture.store.cancel(fixture.session_id), Cancel::Requested);
    first.renew_poke().notify_waiters();
    tokio::time::timeout(WAIT, ended.notified())
        .await
        .expect("the holder ends the cancelled turn");
    kill(&fixture, run).await;
    assert_eq!(terminals(&fixture).await, vec!["turn.cancelled"]);

    // The follow-up message's turn: its holder dies mid-turn too, so the one
    // after it resumes a turn that started after the cancel.
    let follow_up = fixture.send("Try again.").await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (second, _) = fixture.worker("w2");
    let run = run_until_held(&fixture, &second).await;
    kill(&fixture, run).await;
    assert_eq!(
        terminals(&fixture).await,
        vec!["turn.cancelled"],
        "the follow-up turn is not cancelled"
    );

    let (third, _) = fixture.worker("w3");
    run_until_idle(&third).await;
    assert_eq!(
        terminals(&fixture).await,
        vec!["turn.cancelled", "turn.completed"]
    );
    let started: Vec<_> = fixture
        .events()
        .await
        .into_iter()
        .filter(|event| event.event_type == "turn.started")
        .map(|event| event.context.input_message_id)
        .collect();
    assert_eq!(started, vec![Some(cancelled_input), Some(follow_up)]);
    assert_eq!(fixture.answers().await, vec!["Done."]);
    fixture.assert_idle();
    assert!(!fixture.store.row(fixture.session_id).cancel_requested);
}

// The holder dies before it hears of the cancel: the next holder ends the
// cancelled turn, and a message sent after the cancel still gets its turn
// instead of being consumed with the cancelled one.
#[tokio::test]
async fn a_cancel_the_dead_holder_never_heard_ends_its_turn_and_spares_later_input() {
    let fixture = Fixture::new(model("Done.", vec![vec![call("c1", "read_inbox")]])).await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (first, _) = fixture.worker("w1");
    let cancelled_input = fixture.send("Read my inbox.").await;
    let run = run_until_held(&fixture, &first).await;
    assert_eq!(fixture.store.cancel(fixture.session_id), Cancel::Requested);
    kill(&fixture, run).await;
    let follow_up = fixture.send("Try again.").await;

    let (second, _) = fixture.worker("w2");
    assert_eq!(run_until_idle(&second).await, 2, "the cancel, then a turn");
    assert_eq!(
        terminals(&fixture).await,
        vec!["turn.cancelled", "turn.completed"]
    );
    let started: Vec<_> = fixture
        .events()
        .await
        .into_iter()
        .filter(|event| event.event_type == "turn.started")
        .map(|event| event.context.input_message_id)
        .collect();
    assert_eq!(started, vec![Some(cancelled_input), Some(follow_up)]);
    assert_eq!(fixture.answers().await, vec!["Done."]);
    fixture.assert_idle();
}

// A message queued before the cancel was requested is part of what the
// person cancelled: it is consumed with the turn, as a withdraw consumes it.
#[tokio::test]
async fn input_queued_before_the_cancel_is_consumed_with_the_turn() {
    let fixture = Fixture::new(model("Never.", vec![vec![call("c1", "read_inbox")]])).await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (worker, _) = fixture.worker("w1");
    fixture.send("Read my inbox.").await;
    let run = run_until_held(&fixture, &worker).await;
    fixture.send("And the calendar.").await;
    assert_eq!(fixture.store.cancel(fixture.session_id), Cancel::Requested);
    worker.renew_poke().notify_waiters();
    tokio::time::timeout(WAIT, run).await.unwrap().unwrap();

    assert_eq!(run_until_idle(&worker).await, 0, "nothing left to run");
    assert_eq!(terminals(&fixture).await, vec!["turn.cancelled"]);
    assert!(fixture.answers().await.is_empty());
    fixture.assert_idle();
}
