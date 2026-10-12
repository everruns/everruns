//! Running, crashing, resuming, steering, cancelling and sealing turns.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::store::Cancel;
use super::*;
use crate::actor_lease_store::ActorLeaseStore;
use crate::core::events::Event;

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

// Chaos harness: a pool timeout while a fresh turn was being set up failed
// the turn before `turn.started`, after the renewer had already committed the
// cursor over its input. The next holder found no open turn and nothing past
// the cursor, released, and the message was never answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_crash_before_the_turn_starts_still_answers_after_a_renew() {
    let fixture = Fixture::new(model("Done.", vec![])).await;
    let (crashing, probe) = fixture.worker("w1");
    let reached = Arc::new(tokio::sync::Notify::new());
    let (resume, held) = std::sync::mpsc::channel::<()>();
    {
        let reached = reached.clone();
        *probe.at_turn_host.lock().unwrap() = Some(Box::new(move || {
            reached.notify_one();
            // Hold the turn's setup (on this worker thread) until a renew
            // ran, then die before the turn starts.
            let _ = held.recv();
            panic!("worker died before the turn started");
        }));
    }
    fixture.send("Hello.").await;

    let run = {
        let worker = crashing.clone();
        tokio::spawn(async move { run_once(&worker).await })
    };
    tokio::time::timeout(WAIT, reached.notified())
        .await
        .expect("the turn's setup is reached");
    let renews = fixture.store.renews();
    tokio::time::timeout(WAIT, async {
        while fixture.store.renews() == renews {
            // Poke until the renewer is waiting for it.
            crashing.renew_poke().notify_waiters();
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the renewer runs while the turn is set up");
    resume.send(()).unwrap();
    assert!(run.await.is_err(), "the worker died");
    assert_eq!(fixture.count("turn.started").await, 0);
    fixture.store.expire(fixture.session_id);

    let (survivor, _) = fixture.worker("w2");
    assert_eq!(run_until_idle(&survivor).await, 1);
    assert_eq!(fixture.count("turn.started").await, 1);
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

    // The session is usable afterwards: the next message runs a fresh turn
    // to its end on a healthy worker.
    fixture.send("Again.").await;
    assert_eq!(run_until_idle(&next).await, 1);
    let types = fixture.types().await;
    assert_eq!(fixture.count("turn.started").await, 2, "{types:?}");
    assert_eq!(fixture.count("turn.sealed").await, 1, "{types:?}");
    assert_eq!(fixture.count("turn.completed").await, 1, "{types:?}");
    assert_eq!(
        fixture.answers().await.last().map(String::as_str),
        Some("Never.")
    );
    fixture.assert_idle();
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

/// The `message_id` of every `event_type` event (`output.message.*`), in order.
fn message_ids(events: &[Event], event_type: &str) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.event_type == event_type)
        .map(|event| {
            let data = serde_json::to_value(&event.data).unwrap();
            data.get("message_id")
                .or_else(|| data.pointer("/message/id"))
                .and_then(Value::as_str)
                .expect("an output message event names its message")
                .to_owned()
        })
        .collect()
}

/// A worker dies while the model streams the turn's second reason. The
/// survivor resumes the open turn from the log and finds the dead attempt's
/// open stream through the host's partial-stream store (EVE-532): it
/// announces neither `reason.started`'s message nor `output.message.started`
/// again, completes the message under the dead attempt's id, and leaves
/// `reason.recovered` as the only trace of the lost attempt. Ported from the
/// per-step path's worker crash test.
#[tokio::test]
async fn a_crash_mid_reason_completes_the_open_stream_once() {
    let fixture = Fixture::new(model(
        "Survived the crash.",
        vec![vec![call("c1", "read_inbox")]],
    ))
    .await;
    // Call 0 asks for the tool; call 1, the reason after the act, hangs.
    fixture.model.block(1);
    let (crashing, _) = fixture.worker("w1");
    fixture.send("Read my inbox.").await;

    crash_at(&fixture, &crashing, &fixture.model.entered).await;
    assert_eq!(fixture.count("tool.completed").await, 1);
    assert_eq!(fixture.count("reason.started").await, 2);
    assert!(fixture.answers().await.is_empty());

    let (survivor, _) = fixture.worker("w2");
    assert_eq!(run_until_idle(&survivor).await, 1);

    let events = fixture.events().await;
    let types = fixture.types().await;
    for (event_type, expected) in [
        ("turn.started", 1),
        ("turn.completed", 1),
        ("turn.failed", 0),
        ("reason.started", 2),
        ("reason.recovered", 1),
        ("reason.completed", 2),
        ("output.message.completed", 2),
        ("tool.completed", 1),
    ] {
        assert_eq!(
            fixture.count(event_type).await,
            expected,
            "{event_type}: {types:?}"
        );
    }
    assert_eq!(
        fixture.tool_calls(),
        vec!["read_inbox"],
        "the tool ran once"
    );
    let started = message_ids(&events, "output.message.started");
    let completed = message_ids(&events, "output.message.completed");
    assert_eq!(started.len(), 2, "{types:?}");
    assert_eq!(
        started, completed,
        "every started message completes, in order"
    );
    assert_eq!(fixture.answers().await, vec!["Survived the crash."]);
    fixture.assert_idle();
}

/// Steering that reached the log while the turn ran, and that the dead
/// worker never handed to the model, is answered by the survivor.
#[tokio::test]
async fn steering_sent_before_a_crash_is_answered_after_the_resume() {
    let fixture = Fixture::new(model("Both done.", vec![vec![call("c1", "read_inbox")]])).await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (crashing, _) = fixture.worker("w1");
    fixture.send("Read my inbox.").await;

    let running = {
        let crashing = crashing.clone();
        tokio::spawn(async move { run_once(&crashing).await })
    };
    tokio::time::timeout(WAIT, fixture.gate.held.notified())
        .await
        .expect("the tool call starts");
    fixture.send("Also check the calendar.").await;
    running.abort();
    let _ = running.await;
    fixture.store.expire(fixture.session_id);
    let asked_before = fixture.model.seen().len();
    assert!(
        fixture
            .model
            .seen()
            .iter()
            .all(|call| !call.contains("Also check the calendar.")),
        "the dead worker never handed the steering to the model"
    );

    let (survivor, _) = fixture.worker("w2");
    run_until_idle(&survivor).await;
    let seen = fixture.model.seen();
    assert!(
        seen[asked_before..]
            .iter()
            .any(|call| call.contains("Also check the calendar.")),
        "the resumed work hands the steering to the model"
    );
    let types = fixture.types().await;
    assert_eq!(
        fixture.count("turn.started").await,
        fixture.count("turn.completed").await,
        "every turn ends: {types:?}"
    );
    assert_eq!(fixture.count("turn.failed").await, 0, "{types:?}");
    assert_eq!(
        fixture.answers().await.last().map(String::as_str),
        Some("Both done.")
    );
    fixture.assert_idle();
}

/// A worker dies after a tool call completed and before the next reason:
/// the survivor resumes at the reason and never runs the tool again.
#[tokio::test]
async fn a_crash_between_act_and_reason_does_not_rerun_the_tool() {
    let fixture = Fixture::new(model("Sent.", vec![vec![call("c1", "send_email")]])).await;
    let (crashing, probe) = fixture.worker("w1");
    // Flush 1 precedes the first reason; flush 2 follows the act.
    probe.hang_at_flush.store(2, Ordering::SeqCst);
    fixture.send("Email the team.").await;

    crash_at(&fixture, &crashing, &probe.hung).await;
    assert_eq!(fixture.count("act.completed").await, 1);
    assert_eq!(fixture.count("reason.started").await, 1);

    let (survivor, _) = fixture.worker("w2");
    assert_eq!(run_until_idle(&survivor).await, 1);
    assert_eq!(fixture.tool_calls(), vec!["send_email"], "never run twice");
    let types = fixture.types().await;
    assert_eq!(fixture.count("tool.completed").await, 1, "{types:?}");
    assert_eq!(fixture.count("turn.started").await, 1, "{types:?}");
    assert_eq!(fixture.count("turn.completed").await, 1, "{types:?}");
    assert_eq!(fixture.answers().await, vec!["Sent."]);
    fixture.assert_idle();
}

/// EVE-1235: the session is deleted while its turn runs. The holder's next
/// renew finds it gone and the turn stops at once, quietly: no `turn.failed`
/// (there is nowhere to store it), no answer, and no lease to release.
#[tokio::test]
async fn a_turn_whose_session_is_deleted_stops_at_the_next_renew() {
    let fixture = Fixture::new(model("Never shown.", vec![vec![call("c1", "read_inbox")]])).await;
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
    fixture.store.delete(fixture.session_id);
    worker.renew_poke().notify_waiters();

    // Well inside the lease TTL (3s): a deleted session is not a store error
    // the renewer waits out.
    tokio::time::timeout(Duration::from_millis(1500), running)
        .await
        .expect("the turn stops at the renew")
        .unwrap();
    assert_eq!(fixture.count("turn.failed").await, 0);
    assert_eq!(fixture.count("turn.completed").await, 0);
    assert!(fixture.answers().await.is_empty());
    assert!(!run_once(&worker).await, "nothing left to lease");
}

/// EVE-1235: a turn whose session was deleted fails on whatever it reads next
/// (its input message, the conversation), with errors that do not name the
/// session. The holder asks the renew, which answers for the actor row, before
/// treating such a failure as the turn's own.
#[tokio::test]
async fn a_failure_after_the_session_was_deleted_reads_as_the_delete() {
    let fixture = Fixture::new(model("Done.", vec![])).await;
    let (worker, _) = fixture.worker("w1");
    fixture.send("Hello.").await;
    let actor = worker.lease(1).await.unwrap().pop().expect("leased");
    let lease = super::super::lease::Lease::new(
        fixture.store.clone(),
        "w1".to_string(),
        &actor,
        Duration::from_secs(3),
        3,
    );
    let unrelated =
        everruns_contracts::error::AgentLoopError::store("User message not found: message_x");

    assert!(
        !super::super::turn::session_deleted(&lease, &unrelated).await,
        "the session is still there: the failure is the turn's"
    );
    fixture.store.delete(fixture.session_id);
    assert!(super::super::turn::session_deleted(&lease, &unrelated).await);
}

/// EVE-1235, ported from the retired turn driver: the session is deleted
/// after its actor was leased and before the turn's first step. The turn ends
/// without error and without retry: no `turn.failed`, no answer, no seal, and
/// nothing left to lease.
#[tokio::test]
async fn a_turn_whose_session_is_deleted_before_it_runs_stops_without_error() {
    let fixture = Fixture::new(model("Never shown.", vec![])).await;
    let (worker, _) = fixture.worker("w1");
    fixture.send("Hello.").await;
    let actor = worker.lease(1).await.unwrap().pop().expect("leased");

    // Gone under the leased actor: its row goes with it (FK cascade), and the
    // turn's first step finds the session missing.
    fixture.store.delete(fixture.session_id);
    *fixture.model.deleted_session.lock().unwrap() = Some(fixture.session_id);
    tokio::time::timeout(WAIT, worker.run_actor(actor))
        .await
        .expect("the turn stops");

    assert_eq!(fixture.count("turn.failed").await, 0);
    assert_eq!(fixture.count("turn.sealed").await, 0);
    assert_eq!(fixture.count("turn.completed").await, 0);
    assert!(fixture.answers().await.is_empty());
    assert!(!run_once(&worker).await, "nothing left to lease");
}
