//! Running, crashing, resuming, steering, cancelling and sealing turns.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::store::Cancel;
use super::*;
use crate::actor_lease_store::ActorLeaseStore;

#[tokio::test]
async fn a_message_runs_one_turn_and_leaves_the_session_idle() {
    let fixture = Fixture::new(model("Done.", vec![vec![call("c1", "read_inbox")]])).await;
    let (worker, _) = fixture.worker("w1");
    let input = fixture.send("Check my inbox.").await;

    assert_eq!(run_until_idle(&worker).await, 1);

    assert_eq!(fixture.count("turn.started").await, 1);
    assert_eq!(fixture.count("turn.completed").await, 1);
    assert_eq!(fixture.count("session.idled").await, 1);
    assert_eq!(fixture.answers().await, vec!["Done."]);
    assert_eq!(fixture.tool_calls(), vec!["read_inbox"]);
    let started = fixture
        .events()
        .await
        .into_iter()
        .find(|event| event.event_type == "turn.started")
        .unwrap();
    assert_eq!(started.context.input_message_id, Some(input));
    fixture.assert_idle();
}

#[tokio::test]
async fn a_crash_before_the_first_reason_is_resumed_by_another_worker() {
    let fixture = Fixture::new(model("Done.", vec![])).await;
    let (crashing, probe) = fixture.worker("w1");
    // The first flush is the first reason's `take`.
    probe.hang_at_flush.store(1, Ordering::SeqCst);
    fixture.send("Hello.").await;

    crash_at(&fixture, &crashing, &probe.hung).await;
    assert_eq!(fixture.count("turn.started").await, 1);
    assert!(fixture.answers().await.is_empty());

    let (survivor, _) = fixture.worker("w2");
    assert_eq!(run_until_idle(&survivor).await, 1);
    assert_eq!(fixture.count("turn.started").await, 1, "the same turn");
    assert_eq!(fixture.count("turn.completed").await, 1);
    assert_eq!(fixture.answers().await, vec!["Done."]);
    fixture.assert_idle();
}

#[tokio::test]
async fn a_crash_after_the_answer_completes_the_turn_without_a_second_answer() {
    let fixture = Fixture::new(model("Only once.", vec![])).await;
    let (crashing, probe) = fixture.worker("w1");
    // Flush 1 precedes the reason; flush 2 follows its answer.
    probe.hang_at_flush.store(2, Ordering::SeqCst);
    fixture.send("Answer me.").await;

    crash_at(&fixture, &crashing, &probe.hung).await;
    assert_eq!(fixture.answers().await, vec!["Only once."]);
    assert_eq!(fixture.count("turn.completed").await, 0);

    let (survivor, _) = fixture.worker("w2");
    run_until_idle(&survivor).await;
    assert_eq!(fixture.answers().await, vec!["Only once."], "no duplicate");
    assert_eq!(fixture.count("turn.completed").await, 1);
    assert_eq!(fixture.count("session.idled").await, 1);
    fixture.assert_idle();
}

#[tokio::test]
async fn a_non_idempotent_call_cut_off_mid_act_is_settled_as_interrupted() {
    let fixture = Fixture::new(model("Reported.", vec![vec![call("c1", "send_email")]])).await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (crashing, _) = fixture.worker("w1");
    fixture.send("Email the team.").await;

    crash_at(&fixture, &crashing, &fixture.gate.held).await;

    let (survivor, _) = fixture.worker("w2");
    run_until_idle(&survivor).await;
    assert_eq!(fixture.tool_calls(), vec!["send_email"], "never run twice");
    let completed: Vec<String> = fixture
        .events()
        .await
        .into_iter()
        .filter(|event| event.event_type == "tool.completed")
        .map(|event| serde_json::to_string(&event.data).unwrap())
        .collect();
    assert_eq!(completed.len(), 1, "{completed:?}");
    assert!(completed[0].contains("interrupted"), "{completed:?}");
    assert_eq!(fixture.count("turn.started").await, 1);
    assert_eq!(fixture.count("turn.completed").await, 1);
    assert_eq!(fixture.answers().await, vec!["Reported."]);
    fixture.assert_idle();
}

#[tokio::test]
async fn an_idempotent_call_cut_off_mid_act_runs_again() {
    let fixture = Fixture::new(model("Read.", vec![vec![call("c1", "read_inbox")]])).await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (crashing, _) = fixture.worker("w1");
    fixture.send("Read my inbox.").await;

    crash_at(&fixture, &crashing, &fixture.gate.held).await;

    let (survivor, _) = fixture.worker("w2");
    run_until_idle(&survivor).await;
    assert_eq!(fixture.tool_calls(), vec!["read_inbox", "read_inbox"]);
    let completed: Vec<String> = fixture
        .events()
        .await
        .into_iter()
        .filter(|event| event.event_type == "tool.completed")
        .map(|event| serde_json::to_string(&event.data).unwrap())
        .collect();
    assert_eq!(completed.len(), 1, "{completed:?}");
    assert!(!completed[0].contains("interrupted"), "{completed:?}");
    assert_eq!(fixture.count("turn.completed").await, 1);
    assert_eq!(fixture.answers().await, vec!["Read."]);
}

#[tokio::test]
async fn input_that_arrives_mid_turn_steers_it() {
    let fixture = Fixture::new(model("Both done.", vec![vec![call("c1", "read_inbox")]])).await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (worker, _) = fixture.worker("w1");
    fixture.send("Read my inbox.").await;

    let running = {
        let worker = worker.clone();
        tokio::spawn(async move { run_once(&worker).await })
    };
    tokio::time::timeout(WAIT, fixture.gate.held.notified())
        .await
        .unwrap();
    let steering = fixture.send("Also check the calendar.").await;
    fixture.gate.release.notify_one();
    running.await.unwrap();

    // Delivered to the running turn: no second turn, nothing left to run.
    assert_eq!(run_until_idle(&worker).await, 0, "nothing left to run");
    assert_eq!(fixture.count("turn.started").await, 1, "one turn");
    assert_eq!(fixture.count("turn.completed").await, 1);
    let input = fixture
        .events()
        .await
        .into_iter()
        .find(|event| {
            matches!(&event.data, crate::core::events::EventData::InputMessage(data)
                if data.message.id == steering)
        })
        .unwrap();
    assert_eq!(
        input.context.turn_id, None,
        "steering stays the person's input"
    );
    fixture.assert_idle();
}

#[tokio::test]
async fn a_message_that_lands_as_the_turn_ends_starts_the_next_turn() {
    let fixture = Fixture::new(model("Answered.", vec![])).await;
    let (worker, probe) = fixture.worker("w1");
    let first = fixture.send("First.").await;
    let second = fixture.store_message("Second.").await;
    // The second message's mark lands after the turn's last inbox check.
    let store = fixture.store.clone();
    let session_id = fixture.session_id;
    let sequence = fixture
        .events()
        .await
        .iter()
        .rev()
        .find_map(|event| event.sequence)
        .map(i64::from)
        .unwrap();
    *probe.on_turn_ended.lock().unwrap() = Some(Box::pin(async move {
        store.mark_runnable(session_id, ORG_ID, sequence);
    }));

    assert_eq!(run_until_idle(&worker).await, 2, "two leases, two turns");

    let started: Vec<_> = fixture
        .events()
        .await
        .into_iter()
        .filter(|event| event.event_type == "turn.started")
        .map(|event| event.context.input_message_id)
        .collect();
    assert_eq!(started, vec![Some(first), Some(second)]);
    assert_eq!(fixture.count("turn.completed").await, 2);
    fixture.assert_idle();
}

#[tokio::test]
async fn a_cancel_ends_the_running_turn_with_one_terminal_event() {
    let fixture = Fixture::new(model("Never.", vec![vec![call("c1", "read_inbox")]])).await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (worker, _) = fixture.worker("w1");
    fixture.send("Read my inbox.").await;

    let running = {
        let worker = worker.clone();
        tokio::spawn(async move { run_once(&worker).await })
    };
    tokio::time::timeout(WAIT, fixture.gate.held.notified())
        .await
        .unwrap();
    assert_eq!(fixture.store.cancel(fixture.session_id), Cancel::Requested);
    // The cancel notification: running leases renew now.
    worker.renew_poke().notify_waiters();
    tokio::time::timeout(WAIT, running).await.unwrap().unwrap();

    let types = fixture.types().await;
    let terminal: Vec<&String> = types
        .iter()
        .filter(|seen| {
            matches!(
                seen.as_str(),
                "turn.completed" | "turn.failed" | "turn.cancelled" | "turn.sealed"
            )
        })
        .collect();
    assert_eq!(terminal, vec!["turn.cancelled"], "{types:?}");
    assert_eq!(
        types.last().map(String::as_str),
        Some("session.idled"),
        "{types:?}"
    );
    assert!(fixture.answers().await.is_empty());
    assert_eq!(run_until_idle(&worker).await, 0);
    fixture.assert_idle();
    assert!(!fixture.store.row(fixture.session_id).cancel_requested);
}

#[tokio::test]
async fn a_lost_lease_stops_the_turn_from_writing() {
    let fixture = Fixture::new(model("Late.", vec![vec![call("c1", "read_inbox")]])).await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (stalled, _) = fixture.worker("w1");
    fixture.send("Read my inbox.").await;

    let running = {
        let stalled = stalled.clone();
        tokio::spawn(async move { run_once(&stalled).await })
    };
    tokio::time::timeout(WAIT, fixture.gate.held.notified())
        .await
        .unwrap();
    // The stalled worker's lease runs out and another worker takes the
    // session: its next renew finds the fence moved.
    fixture.store.expire(fixture.session_id);
    let taken = fixture
        .store
        .lease_actors("w2", 1, Duration::from_secs(30), None)
        .await
        .unwrap();
    assert_eq!(taken.len(), 1);
    let before = fixture.events().await.len();
    stalled.renew_poke().notify_waiters();
    tokio::time::timeout(WAIT, running).await.unwrap().unwrap();
    // The tool's result would be the next write; the turn stopped first.
    fixture.gate.release.notify_one();
    tokio::task::yield_now().await;

    assert_eq!(
        fixture.events().await.len(),
        before,
        "no write after the loss"
    );
    let row = fixture.store.row(fixture.session_id);
    assert_eq!(
        row.holder.as_deref(),
        Some("w2"),
        "the stalled holder released nothing"
    );
}

/// The control plane refuses an append whose fence is no longer the
/// holder's (`create_events_fenced`); the refusal stops the turn exactly as a
/// lost renew does: nothing more is written and nothing is released.
#[tokio::test]
async fn a_refused_append_stops_the_turn_like_a_lost_lease() {
    let fixture = Fixture::new(model("Late.", vec![vec![call("c1", "read_inbox")]])).await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (worker, probe) = fixture.worker("w1");
    fixture.send("Read my inbox.").await;

    let running = {
        let worker = worker.clone();
        tokio::spawn(async move { run_once(&worker).await })
    };
    tokio::time::timeout(WAIT, fixture.gate.held.notified())
        .await
        .unwrap();
    let fence = probe
        .fence
        .lock()
        .unwrap()
        .clone()
        .expect("hosts carry the fence");
    assert_eq!(fence.holder, "w1");
    assert_eq!(fence.session_id, fixture.session_id);
    let before = fixture.events().await.len();
    // An append of this turn comes back refused for its fence.
    fence.observe::<()>(&Err(crate::actor_lease_store::actor_lease_lost(
        fixture.session_id,
        "another holder's fence",
    )));
    tokio::time::timeout(WAIT, running).await.unwrap().unwrap();
    fixture.gate.release.notify_one();
    tokio::task::yield_now().await;

    assert_eq!(
        fixture.events().await.len(),
        before,
        "no write after the refusal"
    );
    let row = fixture.store.row(fixture.session_id);
    assert_eq!(
        row.holder.as_deref(),
        Some("w1"),
        "the lease is left to expire, not released"
    );
    assert!(row.runnable.is_some(), "the next holder resumes the turn");
}

#[tokio::test]
async fn a_turn_that_keeps_killing_its_worker_is_sealed() {
    let mut fixture = Fixture::new(model("Never.", vec![])).await;
    fixture.poison_attempts = 1;
    let (crashing, probe) = fixture.worker("w1");
    probe.hang_at_flush.store(1, Ordering::SeqCst);
    fixture.send("Crash.").await;

    crash_at(&fixture, &crashing, &probe.hung).await;

    let (next, _) = fixture.worker("w2");
    assert_eq!(run_until_idle(&next).await, 1);
    let types = fixture.types().await;
    assert_eq!(fixture.count("turn.started").await, 1, "{types:?}");
    assert_eq!(fixture.count("turn.sealed").await, 1, "{types:?}");
    assert_eq!(fixture.count("turn.completed").await, 0, "{types:?}");
    assert!(fixture.answers().await.iter().all(|text| text != "Never."));
    fixture.assert_idle();
    assert_eq!(fixture.store.row(fixture.session_id).attempts, 0);
}

#[tokio::test]
async fn two_workers_never_run_one_session_at_once() {
    let fixture = Fixture::new(model("Ok.", vec![vec![call("c1", "read_inbox")]])).await;
    let probe: Arc<Probe> = Arc::default();
    let (first, _) = fixture.worker_with_probe("w1", probe.clone());
    let (second, _) = fixture.worker_with_probe("w2", probe.clone());
    let mut sent = Vec::new();
    for text in ["One.", "Two.", "Three."] {
        sent.push(fixture.send(text).await);
        // Both workers race for the session.
        let (a, b) = tokio::join!(run_once(&first), run_once(&second));
        assert!(a ^ b, "exactly one worker leased the session ({a}, {b})");
        assert!(
            !probe.overlapped.load(Ordering::SeqCst),
            "two holders ran the session"
        );
    }
    fixture.assert_idle();
    let events = fixture.events().await;
    let started: Vec<_> = events
        .iter()
        .filter(|event| event.event_type == "turn.started")
        .map(|event| event.context.input_message_id.unwrap())
        .collect();
    assert_eq!(started, sent, "one turn per message, in order");
    let mut open = false;
    for event in &events {
        match event.event_type.as_str() {
            "turn.started" => {
                assert!(!open, "a turn started inside another");
                open = true;
            }
            "turn.completed" => open = false,
            _ => {}
        }
    }
    assert!(!open);
    assert_eq!(turn_ids(&events, "turn.completed").len(), 3);
}
