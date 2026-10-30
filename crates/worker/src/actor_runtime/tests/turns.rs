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

// Chaos harness: the queued store holding `session.activated` and
// `turn.started` failed and was dropped, and the turn went on to complete
// without them. A store that gives up now stops the turn before it commits
// its input or writes anything more; once the lease's retries are spent, the
// next holder resumes from the log.
#[tokio::test]
async fn a_queued_store_that_gives_up_ends_the_lease_without_committing_the_input() {
    let mut fixture = Fixture::new(model("Done.", vec![])).await;
    fixture.control_plane_retries = 0;
    let (failing, probe) = fixture.worker("w1");
    // The first flush is the first reason's `take`.
    probe.fail_at_flush.store(1, Ordering::SeqCst);
    let input = fixture.send("Hello.").await;
    let input_seq = fixture
        .events()
        .await
        .iter()
        .find(|event| matches!(&event.data, crate::core::events::EventData::InputMessage(data) if data.message.id == input))
        .and_then(|event| event.sequence)
        .map(i64::from)
        .unwrap();

    assert!(run_once(&failing).await);
    assert!(fixture.answers().await.is_empty(), "nothing after the gap");
    let row = fixture.store.row(fixture.session_id);
    assert_eq!(row.holder.as_deref(), Some("w1"), "not released: {row:?}");
    assert!(
        row.consumed_seq < input_seq,
        "the input is not committed: {row:?}"
    );

    fixture.store.expire(fixture.session_id);
    let (survivor, _) = fixture.worker("w2");
    assert_eq!(run_until_idle(&survivor).await, 1);
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

/// A worker dies while an act's first call runs and before its next call
/// started: the log has no `tool.started` for that call, which proves it
/// never ran, so the survivor runs it instead of settling it as interrupted.
#[tokio::test]
async fn a_non_idempotent_call_that_never_started_runs_on_resume() {
    let fixture = Fixture::new(model(
        "Read and sent.",
        vec![vec![call("c1", "read_inbox"), call("c2", "send_email")]],
    ))
    .await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (crashing, _) = fixture.worker("w1");
    fixture.send("Read my inbox, then email the team.").await;

    crash_at(&fixture, &crashing, &fixture.gate.held).await;
    assert_eq!(fixture.tool_calls(), vec!["read_inbox"]);

    let (survivor, _) = fixture.worker("w2");
    run_until_idle(&survivor).await;
    assert_eq!(
        fixture.tool_calls(),
        vec!["read_inbox", "read_inbox", "send_email"],
        "send_email runs once"
    );
    // Resume rebuilds each call's idempotency key: the rerun read_inbox
    // carries its first attempt's key, and the calls' keys differ.
    let keys = fixture.tool_keys();
    assert!(keys.iter().all(Option::is_some), "{keys:?}");
    assert_eq!(keys[0], keys[1]);
    assert_ne!(keys[1], keys[2]);
    let completed: Vec<String> = fixture
        .events()
        .await
        .into_iter()
        .filter(|event| event.event_type == "tool.completed")
        .map(|event| serde_json::to_string(&event.data).unwrap())
        .collect();
    assert_eq!(completed.len(), 2, "{completed:?}");
    assert!(
        completed.iter().all(|data| !data.contains("interrupted")),
        "{completed:?}"
    );
    assert_eq!(fixture.count("turn.completed").await, 1);
    assert_eq!(fixture.answers().await, vec!["Read and sent."]);
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

/// Three renews in a row fail while a tool runs. At a third of the TTL apart
/// they would span the whole TTL and lose the lease; retried sooner after a
/// failure, the fourth renew lands inside it and the turn carries on.
#[tokio::test]
async fn a_short_renew_outage_does_not_cost_the_turn() {
    let fixture = Fixture::new(model("Read.", vec![vec![call("c1", "read_inbox")]])).await;
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
    fixture.store.fail_renews(3);
    // Past the lease TTL (3s) while the tool holds.
    tokio::time::sleep(Duration::from_millis(3500)).await;
    fixture.gate.release.notify_one();
    tokio::time::timeout(WAIT, running).await.unwrap().unwrap();

    assert_eq!(fixture.tool_calls(), vec!["read_inbox"]);
    assert_eq!(fixture.count("turn.completed").await, 1);
    assert_eq!(fixture.answers().await, vec!["Read."]);
    assert_eq!(
        fixture.store.row(fixture.session_id).holder,
        None,
        "the turn released its lease"
    );
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

// Production, 2026-10-27: one turn's log grew to ~200 MB encoded, over the
// gRPC cap, so every lease failed reading it, ahead of the poison guard, and
// the session re-read the whole turn every few seconds for hours. The guard
// runs before the read and seals the turn from its first event.
#[tokio::test]
async fn a_turn_whose_log_cannot_be_read_is_still_sealed() {
    let mut fixture = Fixture::new(model("Never.", vec![])).await;
    fixture.poison_attempts = 1;
    let (crashing, probe) = fixture.worker("w1");
    probe.hang_at_flush.store(1, Ordering::SeqCst);
    fixture.send("Crash.").await;
    crash_at(&fixture, &crashing, &probe.hung).await;
    fixture.store.oversize_reads();

    let (next, _) = fixture.worker("w2");
    let mut leases = 0;
    while run_once(&next).await {
        leases += 1;
        assert!(leases < 5, "the turn keeps being leased");
        fixture.store.expire(fixture.session_id);
    }

    let types = fixture.types().await;
    assert_eq!(fixture.count("turn.started").await, 1, "{types:?}");
    assert_eq!(fixture.count("turn.sealed").await, 1, "{types:?}");
    assert_eq!(fixture.count("turn.completed").await, 0, "{types:?}");
    fixture.assert_idle();
}

// EVE-1301, chaos harness: a PostgreSQL pool exhaustion ended three leases in
// a row on control-plane errors (harness load, journal read, transport), and
// the fourth lease sealed the turn as one that keeps killing its worker: the
// person got "stopped because it repeatedly failed" for a database blip. The
// poison guard counts holder deaths, not failures to reach the control plane.
#[tokio::test]
async fn control_plane_failures_do_not_seal_the_turn() {
    let fixture = Fixture::new(model("Done.", vec![])).await;
    let (worker, _) = fixture.worker("w1");
    fixture.send("Hello.").await;
    // One failure per lease would reach the default threshold (3).
    fixture.store.fail_reads(3);

    let mut leases = 0;
    while run_once(&worker).await {
        leases += 1;
        assert!(leases < 10, "the actor keeps being runnable");
        // A lease left to expire is taken again (as the poll would).
        fixture.store.expire(fixture.session_id);
    }

    let types = fixture.types().await;
    assert_eq!(fixture.count("turn.sealed").await, 0, "{types:?}");
    assert_eq!(fixture.count("turn.started").await, 1, "{types:?}");
    assert_eq!(fixture.count("turn.completed").await, 1, "{types:?}");
    assert_eq!(fixture.answers().await, vec!["Done."]);
    fixture.assert_idle();
    assert_eq!(fixture.store.row(fixture.session_id).attempts, 0);
}

// The same gap with retries left: the lease runs the turn again from the log
// itself, and the input is committed only once the turn is stored.
#[tokio::test]
async fn a_queued_store_that_gives_up_is_retried_on_the_same_lease() {
    let fixture = Fixture::new(model("Done.", vec![])).await;
    let (failing, probe) = fixture.worker("w1");
    probe.fail_at_flush.store(1, Ordering::SeqCst);
    fixture.send("Hello.").await;

    assert_eq!(run_until_idle(&failing).await, 1, "one lease");
    let types = fixture.types().await;
    assert_eq!(fixture.count("turn.started").await, 1, "{types:?}");
    assert_eq!(fixture.count("turn.completed").await, 1, "{types:?}");
    assert_eq!(fixture.answers().await, vec!["Done."]);
    assert!(
        !probe.overlapped.load(Ordering::SeqCst),
        "the failed run's turn ended before the retry"
    );
    fixture.assert_idle();
}

// The retries are a separate bound, not a way around the poison guard: a turn
// whose control-plane failures outlast a lease's retries lets the lease expire,
// and that counts an attempt like a holder death.
#[tokio::test]
async fn failures_that_outlast_the_retries_still_reach_the_poison_guard() {
    let mut fixture = Fixture::new(model("Never.", vec![])).await;
    fixture.poison_attempts = 1;
    fixture.control_plane_retries = 1;
    let (worker, _) = fixture.worker("w1");
    fixture.send("Hello.").await;
    // Two leases spend two failures each (a run and its one retry).
    fixture.store.fail_reads(4);

    let mut leases = 0;
    while run_once(&worker).await {
        leases += 1;
        assert!(leases < 10, "the actor keeps being runnable");
        fixture.store.expire(fixture.session_id);
    }

    assert_eq!(leases, 3, "two expired leases, then the seal");
    let types = fixture.types().await;
    assert_eq!(fixture.count("turn.sealed").await, 1, "{types:?}");
    assert_eq!(fixture.count("turn.completed").await, 0, "{types:?}");
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

/// Chaos harness (EVE-1279, `steered_message_unread`): steering landed while
/// the turn's reason ran, the reason answered without calls, and the turn took
/// the steering for one more reason. The holder committed the cursor over it
/// at that reason's renew and died in its model call. The survivor folded the
/// log as answered (its latest agent message made no calls), completed the
/// turn, and the message was never answered: no turn of its own, and no
/// reason after it in the turn it steered.
#[tokio::test]
async fn steering_taken_after_the_answer_is_answered_after_a_crash() {
    let fixture = Fixture::new(model("Answered.", vec![])).await;
    let (crashing, probe) = fixture.worker("w1");
    // Flush 2 follows the first reason's answer: the steering lands there,
    // before the turn's last inbox check, as a send mid-reason does.
    let steering = {
        let runtime = fixture.runtime.clone();
        let store = fixture.store.clone();
        let session_id = fixture.session_id;
        let (sent, steering) = tokio::sync::oneshot::channel();
        *probe.at_flush.lock().unwrap() = Some((
            2,
            Box::pin(async move {
                let message_id = runtime
                    .persist_accepted_input(
                        session_id,
                        AcceptedTurnInput::new(crate::core::InputMessage::user(
                            "Also check the calendar.",
                        )),
                    )
                    .await
                    .expect("steering stored");
                let sequence = runtime
                    .events()
                    .await
                    .expect("events")
                    .iter()
                    .rev()
                    .find_map(|event| event.sequence)
                    .map(i64::from)
                    .expect("the steering is in the log");
                store.mark_runnable(session_id, ORG_ID, sequence);
                let _ = sent.send((message_id, sequence));
            }),
        ));
        steering
    };
    // Call 0 answers; call 1, the reason the steering joins, hangs.
    fixture.model.block(1);
    fixture.send("Answer me.").await;

    crash_at(&fixture, &crashing, &fixture.model.entered).await;
    let (_, steered_at) = steering.await.expect("the steering was sent");
    assert_eq!(fixture.answers().await, vec!["Answered."]);
    assert_eq!(fixture.count("turn.completed").await, 0);
    assert!(
        fixture.store.row(fixture.session_id).consumed_seq >= steered_at,
        "the dead holder committed the steering as consumed"
    );
    let asked_before = fixture.model.seen().len();

    let (survivor, _) = fixture.worker("w2");
    run_until_idle(&survivor).await;
    let seen = fixture.model.seen();
    assert!(
        seen[asked_before..]
            .iter()
            .any(|call| call.contains("Also check the calendar.")),
        "the survivor hands the steering to the model"
    );
    let events = fixture.events().await;
    let types = fixture.types().await;
    assert!(
        events.iter().any(|event| {
            matches!(
                event.event_type.as_str(),
                "reason.started" | "reason.recovered"
            ) && event.sequence.map_or(0, i64::from) > steered_at
        }),
        "the steered turn reasons again after the steering: {types:?}"
    );
    assert_eq!(fixture.count("turn.started").await, 1, "{types:?}");
    assert_eq!(fixture.count("turn.completed").await, 1, "{types:?}");
    assert_eq!(fixture.answers().await.len(), 2, "{types:?}");
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

/// The log of a holder that stored `terminal` for a turn on `input` and died
/// before `session.idled`; with `committed`, a renew had already committed
/// the cursor over the input.
async fn died_before_idling(fixture: &Fixture, terminal: &str, committed: bool) -> MessageId {
    let input = fixture.send("Hello.").await;
    let turn_id = everruns_contracts::typed_id::TurnId::new();
    let context = EventContext::turn(turn_id, input);
    let emitter = fixture.runtime.event_emitter();
    emitter
        .emit(EventRequest::new(
            fixture.session_id,
            context.clone(),
            crate::core::events::TurnStartedData {
                turn_id,
                input_message_id: input,
                input_content: None,
                agent_id: None,
                agent_name: None,
                agent_description: None,
            },
        ))
        .await
        .unwrap();
    let request = match terminal {
        "turn.completed" => EventRequest::new(
            fixture.session_id,
            context,
            crate::core::events::TurnCompletedData {
                termination_reason: None,
                turn_id,
                iterations: 1,
                status: Some("completed".to_string()),
                ..Default::default()
            },
        ),
        "turn.failed" => EventRequest::new(
            fixture.session_id,
            context,
            crate::core::events::TurnFailedData {
                turn_id,
                error: "provider rejected the request".to_string(),
                error_code: None,
                error_fields: None,
                error_disclosure: None,
            },
        ),
        other => panic!("unexpected terminal {other}"),
    };
    emitter.emit(request).await.unwrap();
    let started = fixture
        .events()
        .await
        .iter()
        .find(|event| event.event_type == "turn.started")
        .and_then(|event| event.sequence)
        .map(i64::from)
        .unwrap();
    // The holder held the lease when it died.
    let (dead, _) = fixture.worker("w1");
    let leased = dead.lease(1).await.unwrap();
    assert_eq!(leased.len(), 1);
    if committed {
        fixture.store.commit(fixture.session_id, started);
    }
    fixture.store.expire(fixture.session_id);
    input
}

// Chaos harness: a holder stored `turn.completed`, then died before
// `session.idled`. The next holder found no open turn and no fresh input and
// released, leaving the session `active` with nothing running, forever.
#[tokio::test]
async fn a_holder_that_dies_before_idling_leaves_the_next_holder_to_idle_the_session() {
    for terminal in ["turn.completed", "turn.failed"] {
        for committed in [true, false] {
            let fixture = Fixture::new(model("Never.", vec![])).await;
            died_before_idling(&fixture, terminal, committed).await;

            let (survivor, _) = fixture.worker("w2");
            assert_eq!(run_until_idle(&survivor).await, 1, "{terminal} {committed}");
            assert_eq!(
                fixture.count("session.idled").await,
                1,
                "{terminal} committed={committed}: the session is idled"
            );
            assert_eq!(
                fixture.count("turn.started").await,
                1,
                "{terminal} committed={committed}: the answered input is not run again"
            );
            assert!(fixture.answers().await.is_empty());
            fixture.assert_idle();

            // Idempotent: another lease of the idled session adds nothing.
            fixture
                .store
                .mark_resumable(fixture.session_id, ORG_ID)
                .await;
            run_until_idle(&survivor).await;
            assert_eq!(fixture.count("session.idled").await, 1);
            fixture.assert_idle();
        }
    }
}

/// A draining worker hands the lease back between steps instead of holding
/// the turn until its lease runs out: the next worker leases the actor at once
/// and finishes the same turn, without running the finished act again.
#[tokio::test]
async fn a_draining_worker_hands_the_turn_off_between_steps() {
    let fixture = Fixture::new(model("Read.", vec![vec![call("c1", "read_inbox")]])).await;
    fixture.gate.hold_first.store(true, Ordering::SeqCst);
    let (draining, _) = fixture.worker("w1");
    fixture.send("Read my inbox.").await;

    let run = {
        let draining = draining.clone();
        tokio::spawn(async move { run_once(&draining).await })
    };
    tokio::time::timeout(WAIT, fixture.gate.held.notified())
        .await
        .expect("the act starts");
    // Shutdown begins mid-act: the act finishes, the next reason does not.
    draining.hand_off.store(true, Ordering::SeqCst);
    fixture.gate.release.notify_one();
    assert!(run.await.unwrap());

    assert_eq!(fixture.store.hand_offs(), 1);
    let row = fixture.store.row(fixture.session_id);
    assert!(row.holder.is_none(), "the lease was handed back");
    assert!(row.runnable.is_some(), "and the actor is runnable at once");
    assert_eq!(fixture.count("act.completed").await, 1);
    assert!(fixture.answers().await.is_empty());

    let (survivor, _) = fixture.worker("w2");
    assert_eq!(run_until_idle(&survivor).await, 1);
    assert_eq!(fixture.tool_calls(), vec!["read_inbox"], "never run twice");
    assert_eq!(fixture.count("turn.started").await, 1, "the same turn");
    assert_eq!(fixture.count("turn.completed").await, 1);
    assert_eq!(fixture.answers().await, vec!["Read."]);
    fixture.assert_idle();
}
