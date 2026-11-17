//! Agents using agents on the data plane (data plane v2, step 11.4): a
//! session's subagent is a session of its own, which the gateway may place
//! on another cell, and its answer comes back to the parent, inline in the
//! foreground or as a wake in the background.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::time::{Duration, Instant};

use serde_json::Value;
use support::{Fleet, answers, types};

/// The cell holding `session`.
async fn cell_of(fleet: &Fleet, session: &str) -> u32 {
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    for cell in [0u32, 1] {
        for (_, addr) in fleet.addrs(cell) {
            let url = format!("http://{addr}/cell/sessions/{session}");
            if let Ok(reply) = http.get(&url).send().await
                && let Ok(found) = reply.json::<Value>().await
                && found["found"] == "here"
            {
                return cell;
            }
        }
    }
    panic!("no cell holds {session}");
}

/// The child the parent's `spawn_agent` call made.
fn child_of(events: &[Value]) -> String {
    events
        .iter()
        .filter(|e| e["type"] == "tool.completed")
        .find_map(|e| {
            let text = serde_json::to_string(&e["data"]).ok()?;
            let at = text.find("session_")?;
            Some(text[at..at + 40].to_owned())
        })
        .expect("a spawn result names its child")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subagent_on_another_cell_answers_its_parent() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    fleet.executor(2);
    let parent = fleet.create_in("team", 0).await;
    let started = Instant::now();
    fleet.say(&parent, "ask answer 42").await;
    fleet.turns(&parent, 1).await;
    let took = started.elapsed();

    let events = fleet.events(&parent).await;
    let answer = answers(&events).pop().unwrap_or_default();
    assert!(
        answer.contains("The answer is 42."),
        "the parent quotes its subagent: {answer}"
    );
    let child = child_of(&events);
    let child_events = fleet.events(&child).await;
    assert_eq!(answers(&child_events), vec!["The answer is 42.".to_owned()]);
    let (parent_cell, child_cell) = (
        cell_of(&fleet, &parent).await,
        cell_of(&fleet, &child).await,
    );
    println!("parent on cell {parent_cell}, child on cell {child_cell}, answered in {took:?}");
    // The gateway places the child by load; the parent's cell carries the
    // running parent, so the child lands on the idle cell.
    assert_ne!(parent_cell, child_cell, "the child ran on another cell");

    // The parent's task records the finished subagent, in the API's shape.
    let tasks = fleet.get(&format!("/v1/sessions/{parent}/tasks")).await;
    let task = &tasks[0];
    assert_eq!(task["state"], "succeeded", "{tasks}");
    assert_eq!(task["links"]["child_session_id"], child.as_str());
    let id = task["id"].as_str().unwrap();
    let detail = fleet
        .get(&format!("/v1/sessions/{parent}/tasks/{id}"))
        .await;
    assert_eq!(detail["task"]["id"], id);
    let running = fleet
        .get(&format!("/v1/sessions/{parent}/tasks?state=running"))
        .await;
    assert_eq!(running, serde_json::json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_background_subagent_wakes_its_parent() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let parent = fleet.create_in("team", 0).await;
    fleet.say(&parent, "later answer 7").await;

    // The spawn returns at once; the child's finish wakes the parent, which
    // runs a second turn on the wake.
    fleet.turns(&parent, 2).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    let events = loop {
        let events = fleet.events(&parent).await;
        let inputs = types(&events)
            .iter()
            .filter(|t| **t == "input.message")
            .count();
        if inputs >= 2 {
            break events;
        }
        assert!(Instant::now() < deadline, "{:?}", types(&events));
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let wake = events
        .iter()
        .filter(|e| e["type"] == "input.message")
        .nth(1)
        .unwrap();
    let text = serde_json::to_string(&wake["data"]["message"]).unwrap();
    println!("wake: {text}");
    assert!(
        text.contains("helper"),
        "the wake names the subagent: {text}"
    );
    let child = child_of(&events);
    assert_eq!(
        answers(&fleet.events(&child).await),
        vec!["The answer is 7.".to_owned()]
    );
}
