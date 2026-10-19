//! The stall watchdog: a holder stuck where it writes nothing stops renewing,
//! so its lease expires and the next one runs the turn. Incident: a webhook
//! input's lease kept renewing for two days with no `turn.started`.

use std::sync::atomic::Ordering;
use std::time::Instant;

use super::*;

const STALL: Duration = Duration::from_millis(500);

/// Wait (real time) until the lease's expiry passed: proof it stopped
/// renewing. The fixture's TTL is 3 s.
async fn wait_for_expiry(fixture: &Fixture) {
    tokio::time::timeout(WAIT, async {
        while fixture
            .store
            .row(fixture.session_id)
            .expires
            .is_some_and(|at| at > Instant::now())
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the stalled lease expires");
}

#[tokio::test]
async fn a_worker_stuck_before_the_turn_starts_lets_the_next_lease_answer() {
    let mut fixture = Fixture::new(model("Done.", vec![])).await;
    fixture.stall_timeout = STALL;
    let (stuck, probe) = fixture.worker("w1");
    probe.hang_inputs_consumed.store(true, Ordering::SeqCst);
    fixture.send("Hello.").await;

    // Without the watchdog this run never ends (`run_once` bounds it).
    assert!(run_once(&stuck).await);
    assert_eq!(fixture.count("turn.started").await, 0);
    let row = fixture.store.row(fixture.session_id);
    assert_eq!(row.holder.as_deref(), Some("w1"), "not released: {row:?}");
    let renews = fixture.store.renews();
    wait_for_expiry(&fixture).await;
    assert_eq!(fixture.store.renews(), renews, "renewed after the stall");

    let (next, _) = fixture.worker("w2");
    assert_eq!(run_until_idle(&next).await, 1);
    assert_eq!(fixture.count("turn.started").await, 1);
    assert_eq!(fixture.count("turn.completed").await, 1);
    assert_eq!(fixture.answers().await, vec!["Done."]);
    fixture.assert_idle();
}

#[tokio::test]
async fn a_worker_stuck_after_the_answer_lets_the_next_lease_finish_without_a_second_answer() {
    let mut fixture = Fixture::new(model("Only once.", vec![])).await;
    fixture.stall_timeout = STALL;
    let (stuck, probe) = fixture.worker("w1");
    // Checkpoint 2 is the one after the turn, before the release.
    probe.hang_at_checkpoint.store(2, Ordering::SeqCst);
    fixture.send("Answer me.").await;

    assert!(run_once(&stuck).await);
    assert_eq!(fixture.answers().await, vec!["Only once."]);
    let row = fixture.store.row(fixture.session_id);
    assert_eq!(row.holder.as_deref(), Some("w1"), "not released: {row:?}");
    let renews = fixture.store.renews();
    wait_for_expiry(&fixture).await;
    assert_eq!(fixture.store.renews(), renews, "renewed after the stall");

    let (next, _) = fixture.worker("w2");
    run_until_idle(&next).await;
    assert_eq!(fixture.answers().await, vec!["Only once."], "no duplicate");
    assert_eq!(fixture.count("turn.completed").await, 1);
    fixture.assert_idle();
}

#[tokio::test]
async fn a_turn_whose_setup_keeps_stalling_is_sealed() {
    let mut fixture = Fixture::new(model("Never.", vec![])).await;
    fixture.stall_timeout = STALL;
    fixture.poison_attempts = 1;
    let (stuck, probe) = fixture.worker("w1");
    probe.hang_inputs_consumed.store(true, Ordering::SeqCst);
    fixture.send("Stall.").await;
    assert!(run_once(&stuck).await);
    wait_for_expiry(&fixture).await;

    // The next lease counts a second attempt, past the threshold.
    let (next, _) = fixture.worker("w2");
    assert_eq!(run_until_idle(&next).await, 1);
    let types = fixture.types().await;
    assert_eq!(fixture.count("turn.sealed").await, 1, "{types:?}");
    assert_eq!(fixture.count("turn.completed").await, 0, "{types:?}");
    fixture.assert_idle();
}
