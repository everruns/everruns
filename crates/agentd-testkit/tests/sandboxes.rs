//! A session's managed sandbox on the data plane (data plane v2, step
//! 11.3): the session's workspace in the cell stays the source of truth.
//! Before a sandbox call the sandbox catches up with it, after a mutating
//! call it takes what the sandbox changed; a lost sandbox, another
//! executor or a killed one all come back to the same files, with one
//! sandbox per session.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::time::{Duration, Instant};

use everruns_cell_testkit::local_sandbox;
use serde_json::{Value, json};
use support::{Fleet, answers};

/// The workspace file at `path`, through the console's file API.
async fn file(fleet: &Fleet, session: &str, path: &str) -> Option<String> {
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("{}/v1/sessions/{session}/fs{path}", fleet.base()))
        .send()
        .await
        .unwrap();
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return None;
    }
    let body: Value = response.json().await.unwrap();
    let file = if body["content"].is_null() {
        &body["data"]
    } else {
        &body
    };
    Some(file["content"].as_str().unwrap_or_default().to_owned())
}

/// Say `text` and wait for the answer to turn `turn`.
async fn ask(fleet: &Fleet, session: &str, turn: u64, text: &str) -> String {
    fleet.say(session, text).await;
    fleet.turns(session, turn).await;
    let events = fleet.events(session).await;
    answers(&events).pop().unwrap_or_default()
}

fn the_sandbox(session: &str) -> String {
    let sandboxes = local_sandbox::sandboxes_of(session);
    assert_eq!(
        sandboxes.len(),
        1,
        "one sandbox for {session}: {sandboxes:?}"
    );
    sandboxes[0].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sandbox_and_its_workspace_stay_together() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create("sandbox").await;

    // A file the sandbox's file tool writes reaches the workspace.
    let answer = ask(&fleet, &session, 1, "write /notes/a.md hello").await;
    assert!(!answer.contains("rror"), "{answer}");
    assert_eq!(
        file(&fleet, &session, "/notes/a.md").await.as_deref(),
        Some("hello")
    );
    let sandbox = the_sandbox(&session);

    // So does what a command makes, and what it removes leaves.
    ask(
        &fleet,
        &session,
        2,
        "sh echo built > /workspace/out.txt && rm /workspace/notes/a.md && mkdir -p /workspace/node_modules/x && echo cache > /workspace/node_modules/x/i.js",
    )
    .await;
    assert_eq!(
        file(&fleet, &session, "/out.txt").await.as_deref(),
        Some("built\n")
    );
    assert_eq!(file(&fleet, &session, "/notes/a.md").await, None);
    assert_eq!(
        file(&fleet, &session, "/node_modules/x/i.js").await,
        None,
        "tool caches stay in the sandbox"
    );

    // A person's upload reaches the sandbox before its next call.
    fleet
        .post(
            &format!("/v1/sessions/{session}/fs/in.txt"),
            json!({ "content": "from a person" }),
        )
        .await;
    let answer = ask(&fleet, &session, 3, "sh cat /workspace/in.txt").await;
    assert!(answer.contains("from a person"), "{answer}");

    // A lost sandbox is replaced, and gets the workspace back.
    local_sandbox::lose(&sandbox);
    let answer = ask(
        &fleet,
        &session,
        4,
        "sh cat /workspace/out.txt /workspace/in.txt",
    )
    .await;
    assert!(
        answer.contains("built") && answer.contains("from a person"),
        "{answer}"
    );
    let replaced = the_sandbox(&session);
    assert_ne!(replaced, sandbox);

    // Another executor does not know what the sandbox holds: it lists it,
    // and finds it as the workspace says.
    fleet.kill_executor(1);
    fleet.executor(2);
    std::fs::write(
        local_sandbox::workspace(&replaced).join("stray.txt"),
        "never synced",
    )
    .unwrap();
    let answer = ask(&fleet, &session, 5, "sh ls /workspace").await;
    assert!(
        answer.contains("out.txt") && answer.contains("in.txt"),
        "{answer}"
    );
    assert!(
        !answer.contains("stray.txt"),
        "a file the workspace never had is gone: {answer}"
    );
    assert_eq!(the_sandbox(&session), replaced);
    fleet.check();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sandbox_a_dead_executor_created_is_removed_before_the_next() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create("sandbox").await;
    // A sandbox labeled with the session that the session never recorded:
    // its executor died between the provider's create and the record.
    let orphan = local_sandbox::orphan(&session);

    ask(&fleet, &session, 1, "sh echo one > /workspace/one.txt").await;
    let sandbox = the_sandbox(&session);
    assert_ne!(sandbox, orphan, "the orphan is gone");
    assert_eq!(
        file(&fleet, &session, "/one.txt").await.as_deref(),
        Some("one\n")
    );
}

/// Engine does not run a cut-off call again (`bash` is at most once): the
/// turn reports its outcome unknown. The session keeps its one sandbox,
/// and the workspace stays as the last finished call left it: the next
/// call finds the sandbox put back to it.
#[tokio::test(flavor = "multi_thread")]
async fn a_call_cut_off_with_its_executor_leaves_one_sandbox_and_the_workspace() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create("sandbox").await;
    ask(&fleet, &session, 1, "sh echo ready > /workspace/ready.txt").await;
    let sandbox = the_sandbox(&session);

    fleet
        .say(
            &session,
            "sh echo started > /workspace/started.txt && sleep 2 && echo slow > /workspace/slow.txt",
        )
        .await;
    // Kill the executor while the command runs.
    let started = local_sandbox::workspace(&sandbox).join("started.txt");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !started.exists() {
        assert!(Instant::now() < deadline, "the command never started");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    fleet.kill_executor(1);
    fleet.executor(2);
    fleet.turns(&session, 2).await;
    let events = fleet.events(&session).await;
    let cut_off = answers(&events).pop().unwrap_or_default();
    assert!(cut_off.contains("cut off"), "{cut_off}");
    assert_eq!(the_sandbox(&session), sandbox, "no second sandbox");
    assert_eq!(file(&fleet, &session, "/started.txt").await, None);

    let answer = ask(&fleet, &session, 3, "sh ls /workspace").await;
    assert!(answer.contains("ready.txt"), "{answer}");
    assert!(!answer.contains("started.txt"), "{answer}");
    assert_eq!(the_sandbox(&session), sandbox, "no second sandbox");
    assert_eq!(
        file(&fleet, &session, "/ready.txt").await.as_deref(),
        Some("ready\n")
    );
}

/// A session with the local provider taking `latency_ms` per call and
/// `create_ms` more to create.
async fn create_with(fleet: &Fleet, latency_ms: u64, create_ms: u64) -> String {
    let mut definition = everruns_cell_testkit::definition("sandbox").unwrap();
    definition["capabilities"] = json!([local_sandbox::capability(latency_ms, create_ms)]);
    let created = fleet
        .post("/v1/sessions", json!({ "agent": definition }))
        .await;
    created["id"].as_str().unwrap().to_owned()
}

/// The duration of the last `tool.completed`, in ms.
async fn last_tool_ms(fleet: &Fleet, session: &str) -> u64 {
    let events = fleet.events(session).await;
    events
        .iter()
        .rev()
        .find(|e| e["type"] == "tool.completed")
        .and_then(|e| e["data"]["duration_ms"].as_u64())
        .unwrap()
}

/// Step 11.3's gate (design.md): a sandbox starts within 10% of v1, and
/// syncing an unchanged workspace takes under 50 ms. Printed for the
/// step's record (`-- --nocapture`).
///
/// Start: the provider takes 1.5 s to create and 50 ms per call, about
/// Daytona's warm numbers. A v1 start calls create, then exec; on the data
/// plane the first call also looks for an orphan before the create and
/// scans after the command: two round trips more, and nothing else.
#[tokio::test(flavor = "multi_thread")]
async fn sandbox_start_and_unchanged_sync_costs() {
    let mut fleet = Fleet::start().await;
    let executor = fleet.executor(1);
    const CREATE_MS: u64 = 1_500;
    const CALL_MS: u64 = 50;
    let mut starts = Vec::new();
    for _ in 0..5 {
        let session = create_with(&fleet, CALL_MS, CREATE_MS).await;
        fleet.say(&session, "sh true").await;
        fleet.turns(&session, 1).await;
        starts.push(last_tool_ms(&fleet, &session).await);
        let calls = local_sandbox::calls_of(&session);
        assert_eq!(
            calls,
            [("create", 1), ("exec", 2), ("list_owned", 1)]
                .into_iter()
                .map(|(k, n)| (k.to_owned(), n))
                .collect(),
            "a first call: an orphan check, the create, the command, one scan"
        );
    }
    starts.sort_unstable();
    let start = starts[starts.len() / 2];
    // v1's provider time for the same call: create, then exec.
    let v1 = CREATE_MS + 2 * CALL_MS;
    let v2 = v1 + 2 * CALL_MS;
    println!(
        "start: median {start} ms; provider time v1 {v1} ms, v2 {v2} ms (+{:.1}%)",
        (v2 - v1) as f64 * 100.0 / v1 as f64
    );
    assert!(v2 * 100 <= v1 * 110);
    assert!(
        start < v2 + 500,
        "{start} ms against {v2} ms of provider time"
    );

    // Unchanged sync: a warm session with 1000 files in its workspace.
    let session = create_with(&fleet, 0, 0).await;
    let answer = ask(
        &fleet,
        &session,
        1,
        "sh mkdir -p /workspace/src && for i in $(seq 1000); do echo $i > /workspace/src/f$i.txt; done",
    )
    .await;
    assert!(!answer.contains("rror"), "{answer}");
    assert_eq!(
        file(&fleet, &session, "/src/f1000.txt").await.as_deref(),
        Some("1000\n")
    );
    let push = |e: &everruns_cell::executor::Executor| {
        e.sandboxes
            .sync
            .total(&[("direction", "push"), ("kind", "skip")])
    };
    let pull = |e: &everruns_cell::executor::Executor| {
        e.sandboxes
            .sync
            .total(&[("direction", "pull"), ("kind", "unchanged")])
    };
    let (push_before, pull_before) = (push(&executor), pull(&executor));
    let reads_before = local_sandbox::calls_of(&session).get("read_file").copied();
    for turn in 2..=11 {
        ask(&fleet, &session, turn, "sh true").await;
    }
    let (pushes, push_s) = push(&executor);
    let (pulls, pull_s) = pull(&executor);
    let push_ms = (push_s - push_before.1) * 1000.0 / (pushes - push_before.0) as f64;
    let pull_ms = (pull_s - pull_before.1) * 1000.0 / (pulls - pull_before.0) as f64;
    println!(
        "unchanged 1000-file workspace: push {push_ms:.2} ms ({} calls), pull {pull_ms:.1} ms ({} calls)",
        pushes - push_before.0,
        pulls - pull_before.0,
    );
    assert_eq!(
        pushes - push_before.0,
        10,
        "every push before a call is a skip"
    );
    assert_eq!(
        pulls - pull_before.0,
        10,
        "every pull after it finds nothing changed"
    );
    assert_eq!(
        local_sandbox::calls_of(&session).get("read_file").copied(),
        reads_before,
        "an unchanged workspace reads no file"
    );
    assert!(push_ms < 50.0 && pull_ms < 50.0);
}
