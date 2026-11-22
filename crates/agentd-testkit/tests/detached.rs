//! Detached tool calls on the data plane (data plane v2, step 11.4): a long
//! call leaves its turn, the turn parks and frees its executor, and the
//! session's shard keeps the call's record. The call's result resumes the
//! turn; an executor that dies mid-call stops renewing the record, and the
//! turn resumes on another executor once it expires.
//!
//! Its own test binary: the knobs are process-wide settings.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::time::{Duration, Instant};

use serde_json::Value;
use support::{Fleet, answers, types};

/// `work` takes 3 s, detaches after 200 ms, and a record lives 2 s between
/// renewals.
fn knobs() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: set once, before this binary starts any fleet thread.
        unsafe {
            std::env::set_var("AGENTD_WORK_MS", "3000");
            std::env::set_var("AGENTD_TOOL_DETACH_AFTER_MS", "200");
            std::env::set_var("AGENTD_TOOL_DETACH_TTL_MS", "2000");
        }
    });
}

/// Wait until `session`'s view satisfies `check`.
async fn view_until(fleet: &Fleet, session: &str, what: &str, check: impl Fn(&Value) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let view = fleet.get(&format!("/v1/sessions/{session}")).await;
        if check(&view) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: {view}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_call_parks_its_turn_and_resumes_it() {
    knobs();
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create("worker").await;
    fleet.say(&session, "go").await;

    // The call detached: the turn parked, no epoch holds the session.
    view_until(&fleet, &session, "the call detaches", |v| {
        v["detached"] == 1 && v["active"].is_null()
    })
    .await;
    // Input that arrives meanwhile waits for the call.
    fleet.say(&session, "and again").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let view = fleet.get(&format!("/v1/sessions/{session}")).await;
    assert!(view["active"].is_null(), "{view}");
    assert_eq!(view["turns_completed"], 0);

    // The result resumes the turn, which takes in the input that waited
    // (as on v1, input that arrives during a turn joins it) and finishes.
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    let kinds = types(&events);
    let count = |kind: &str| kinds.iter().filter(|t| **t == kind).count();
    assert_eq!(count("input.message"), 2, "nothing is lost: {kinds:?}");
    assert_eq!(
        count("tool.completed"),
        1,
        "one result for the call: {kinds:?}"
    );
    let last_input = kinds.iter().rposition(|t| *t == "input.message");
    let completed = kinds.iter().rposition(|t| *t == "turn.completed");
    assert!(
        last_input < completed,
        "the turn took the input in: {kinds:?}"
    );
    assert_eq!(answers(&events), vec!["All work done.".to_owned()]);
    let view = fleet.get(&format!("/v1/sessions/{session}")).await;
    assert_eq!(view["detached"], 0);
    assert_eq!(view["inbox"], 0, "{view}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_detached_call_survives_its_executor_dying() {
    knobs();
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create("worker").await;
    fleet.say(&session, "go").await;
    view_until(&fleet, &session, "the call detaches", |v| {
        v["detached"] == 1
    })
    .await;

    // Its executor dies mid-call. Another one picks the turn up once the
    // record expires, and the turn finishes.
    fleet.kill_executor(1);
    let killed = Instant::now();
    fleet.executor(2);
    fleet.turns(&session, 1).await;
    let resumed = killed.elapsed();
    let events = fleet.events(&session).await;
    assert_eq!(answers(&events), vec!["All work done.".to_owned()]);
    assert_eq!(
        types(&events)
            .iter()
            .filter(|t| **t == "turn.completed")
            .count(),
        1
    );
    println!("turn finished {resumed:?} after its executor died");
    assert!(resumed < Duration::from_secs(15), "{resumed:?}");
}
