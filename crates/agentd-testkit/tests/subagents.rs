//! Agents using agents on the data plane (data plane v2, step 11.4): a
//! session's subagent is a session of its own, which the gateway may place
//! on another shard, and its answer comes back to the parent, inline in the
//! foreground or as a wake in the background.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::time::{Duration, Instant};

use serde_json::Value;
use support::org::{ORG, org_fleet};
use support::{Fleet, answers, types};

/// The shard holding `session`.
async fn shard_of(fleet: &Fleet, session: &str) -> u32 {
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    for shard in [0u32, 1] {
        for (_, addr) in fleet.addrs(shard) {
            let url = format!("http://{addr}/shard/sessions/{session}");
            if let Ok(reply) = http.get(&url).send().await
                && let Ok(found) = reply.json::<Value>().await
                && found["found"] == "here"
            {
                return shard;
            }
        }
    }
    panic!("no shard holds {session}");
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
async fn a_subagent_on_another_shard_answers_its_parent() {
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
    let (parent_shard, child_shard) = (
        shard_of(&fleet, &parent).await,
        shard_of(&fleet, &child).await,
    );
    println!("parent on shard {parent_shard}, child on shard {child_shard}, answered in {took:?}");
    // The gateway places the child by load; the parent's shard carries the
    // running parent, so the child lands on the idle shard.
    assert_ne!(parent_shard, child_shard, "the child ran on another shard");

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
    // No helper key named: the child runs on the parent's model (`same`).
    assert_eq!(detail["task"]["spec"]["model"], "same", "{detail}");
    assert_eq!(detail["task"]["spec"]["model_pick"], "default", "{detail}");
    let running = fleet
        .get(&format!("/v1/sessions/{parent}/tasks?state=running"))
        .await;
    assert_eq!(running, serde_json::json!([]));

    // Its push configs (parity, Phase B): added through the API, refused a
    // signing secret with no control plane to keep it, refused for a
    // private address, deleted once.
    let configs = format!("/v1/sessions/{parent}/tasks/{id}/push-configs");
    let (status, _, _) = fleet
        .post_once(
            &configs,
            serde_json::json!({ "url": "https://hooks.example.com/tasks", "secret": "k" }),
        )
        .await;
    assert_eq!(status, 400, "a secret needs a control plane to keep it");
    let added = fleet
        .post(
            &configs,
            serde_json::json!({
                "url": "https://hooks.example.com/tasks",
                "event_filter": ["terminal", "message"],
            }),
        )
        .await;
    assert_eq!(added["has_secret"], false, "{added}");
    // Listings may hit a follower before it applies the committed add. Wait
    // for this exact config; an acknowledged config that never appears fails.
    let deadline = Instant::now() + Duration::from_secs(10);
    let listed = loop {
        let listed = fleet.get(&configs).await;
        if listed
            .as_array()
            .is_some_and(|rows| rows.iter().any(|config| config["id"] == added["id"]))
        {
            break listed;
        }
        assert!(
            Instant::now() < deadline,
            "committed push config did not become visible: {listed}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(listed[0]["id"], added["id"], "{listed}");
    assert_eq!(
        listed[0]["event_filter"],
        serde_json::json!(["terminal", "message"])
    );
    let (status, _, _) = fleet
        .post_once(
            &configs,
            serde_json::json!({ "url": "http://169.254.169.254/latest" }),
        )
        .await;
    assert_eq!(status, 400, "a private address is refused");
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let one = format!(
        "{}{configs}/{}",
        fleet.base(),
        added["id"].as_str().unwrap()
    );
    assert_eq!(http.delete(&one).send().await.unwrap().status(), 204);
    assert_eq!(http.delete(&one).send().await.unwrap().status(), 404);
    let missing = format!(
        "{}/v1/sessions/{parent}/tasks/nope/push-configs",
        fleet.base()
    );
    assert_eq!(http.get(&missing).send().await.unwrap().status(), 404);
}

/// A thread on another shard gets its parent tools and closes the
/// assignment its parent holds: before, a child's record knew no parent and
/// its shard could not reach the parent's tasks, so `finish` never existed
/// and the assignment stayed running.
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_on_another_shard_finishes_its_parents_assignment() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    fleet.executor(2);
    let parent = fleet.create_in("threads", 0).await;
    fleet
        .say(
            &parent,
            r#"call start_thread {"title":"Check","brief":"call finish {\"summary\":\"Checked it.\"}"}"#,
        )
        .await;
    fleet.turns(&parent, 1).await;
    let events = fleet.events(&parent).await;
    let thread = child_of(&events);

    // The thread's turn calls `finish`, which settles the parent's
    // assignment and wakes the parent.
    let deadline = Instant::now() + Duration::from_secs(60);
    let task = loop {
        let tasks = fleet.get(&format!("/v1/sessions/{parent}/tasks")).await;
        let task = tasks[0].clone();
        if task["state"] == "succeeded" {
            break task;
        }
        assert!(
            Instant::now() < deadline,
            "the assignment did not finish: {tasks}\nthread: {:?}",
            fleet.events(&thread).await
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(task["kind"], "assignment", "{task}");
    assert_eq!(task["links"]["child_session_id"], thread.as_str());
    let finished = fleet
        .events(&thread)
        .await
        .into_iter()
        .filter(|e| e["type"] == "tool.completed")
        .map(|e| e["data"].to_string())
        .collect::<Vec<_>>();
    assert!(
        finished
            .iter()
            .any(|d| d.contains("finish") && !d.contains("not found")),
        "the thread called finish: {finished:?}"
    );
    let (parent_shard, thread_shard) = (
        shard_of(&fleet, &parent).await,
        shard_of(&fleet, &thread).await,
    );
    println!("parent on shard {parent_shard}, thread on shard {thread_shard}");
    assert_ne!(
        parent_shard, thread_shard,
        "the thread ran on another shard"
    );
}

/// A thread is offered its parent tools and none of the thread tools, and
/// a thread turn that ends without `finish` flags its parent's assignment
/// as needing attention. Before, `start_thread` was offered inside a thread
/// (and refused when called), and on the data plane nothing settled the
/// assignment, so it stayed running and the parent never heard back.
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_that_stops_without_finishing_flags_its_parent() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let parent = fleet.create_in("threads", 0).await;
    fleet
        .say(
            &parent,
            r#"call start_thread {"title":"Look","brief":"tools"}"#,
        )
        .await;
    fleet.turns(&parent, 1).await;
    let thread = child_of(&fleet.events(&parent).await);

    let deadline = Instant::now() + Duration::from_secs(60);
    let task = loop {
        let tasks = fleet.get(&format!("/v1/sessions/{parent}/tasks")).await;
        let task = tasks[0].clone();
        if task["state"] == "awaiting_input" {
            break task;
        }
        assert!(
            Instant::now() < deadline,
            "the assignment was not flagged: {tasks}\nthread: {:?}",
            fleet.events(&thread).await
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert!(
        task["input_request"]["prompt"]
            .as_str()
            .is_some_and(|p| p.contains("without completing")),
        "{task}"
    );
    let offered = answers(&fleet.events(&thread).await).join(" ");
    assert!(offered.contains("finish"), "parent tools: {offered}");
    assert!(
        !offered.contains("start_thread") && !offered.contains("list_threads"),
        "thread tools inside a thread: {offered}"
    );
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

/// A subagent of an org's session goes on the feed for the control plane
/// to give it a session record, and runs under that id; a push config's
/// signing secret is kept sealed with the org's data key, never shown.
#[tokio::test(flavor = "multi_thread")]
async fn an_org_subagent_is_recorded_and_its_push_secret_is_not_kept() {
    let (fleet, cp) = org_fleet(&[]).await;
    let ack = cp
        .push_config(
            &fleet.base(),
            ORG,
            &everruns_agentd::dp::ConfigUpdate {
                epoch: 1,
                version: 2,
                base_version: 1,
                changes: vec![serde_json::json!({
                    "kind": "Agent",
                    "agent_id": "team",
                    "revision": 1,
                    "package": everruns_agentd_testkit::definition("team").unwrap(),
                })],
            },
        )
        .await
        .unwrap();
    assert_eq!(
        ack,
        everruns_agentd::dp::ConfigAck::Ok { applied_version: 2 }
    );
    let parent = everruns_contracts::typed_id::SessionId::new().to_string();
    let (status, created) = cp
        .call(
            &fleet.base(),
            reqwest::Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &serde_json::json!({ "id": parent, "agent_ref": "team", "servers": cp.servers() }),
        )
        .await
        .unwrap();
    assert_eq!(status, reqwest::StatusCode::CREATED, "{created}");
    fleet.say(&parent, "ask answer 42").await;
    fleet.turns(&parent, 1).await;

    let events = fleet.events(&parent).await;
    let child = child_of(&events);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let recorded = loop {
        let recorded = cp.subagents();
        if !recorded.is_empty() {
            break recorded;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the subagent never reached the feed"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0].0, ORG);
    assert_eq!(recorded[0].1, child);
    assert_eq!(recorded[0].2.parent_session_id, parent);
    assert_eq!(
        answers(&fleet.events(&child).await),
        vec!["The answer is 42.".to_owned()],
        "the child runs under the id its record has"
    );

    let tasks = fleet.get(&format!("/v1/sessions/{parent}/tasks")).await;
    let id = tasks[0]["id"].as_str().unwrap();
    let configs = format!("/v1/sessions/{parent}/tasks/{id}/push-configs");
    let added = fleet
        .post(
            &configs,
            serde_json::json!({ "url": "https://hooks.example.com/tasks", "secret": "s3cret" }),
        )
        .await;
    assert_eq!(added["has_secret"], true, "{added}");
    assert!(added.get("secret").is_none(), "{added}");
    let detail = fleet
        .get(&format!("/v1/sessions/{parent}/tasks/{id}"))
        .await;
    let listed = fleet.get(&configs).await;
    assert_eq!(listed[0]["has_secret"], true, "{listed}");
    assert!(
        !format!("{detail}{listed}").contains("s3cret"),
        "the secret is never shown: {detail} {listed}"
    );
}

/// A trigger's saved-script run on a shard: the control plane admits the
/// message with its script-run mark, the turn runs the script through the
/// shell with no model call, and the shell reads the agent's scripts from
/// the control plane's platform commands server, for the session its token
/// names.
#[tokio::test(flavor = "multi_thread")]
async fn a_triggers_saved_script_runs_on_a_shard() {
    let (fleet, cp) = org_fleet(&[]).await;
    cp.set_lookup(
        "list_agent_scripts",
        everruns_agentd_testkit::fake_cp::Answer::Value(serde_json::json!([{
            "id": "scr_1",
            "name": "triage",
            "description": "Triage the queue",
            "input_schema": null,
            "body": "echo triaged-queue",
        }])),
    );
    let ack = cp
        .push_config(
            &fleet.base(),
            ORG,
            &everruns_agentd::dp::ConfigUpdate {
                epoch: 1,
                version: 2,
                base_version: 1,
                changes: vec![serde_json::json!({
                    "kind": "Agent",
                    "agent_id": "scripts",
                    "revision": 1,
                    "package": everruns_agentd_testkit::definition("scripts").unwrap(),
                })],
            },
        )
        .await
        .unwrap();
    assert_eq!(
        ack,
        everruns_agentd::dp::ConfigAck::Ok { applied_version: 2 }
    );
    let session = everruns_contracts::typed_id::SessionId::new().to_string();
    let (status, created) = cp
        .call(
            &fleet.base(),
            reqwest::Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &serde_json::json!({ "id": session, "agent_ref": "scripts", "servers": cp.servers() }),
        )
        .await
        .unwrap();
    assert_eq!(status, reqwest::StatusCode::CREATED, "{created}");
    let (status, admitted) = support::org::command(
        &fleet,
        &cp,
        ORG,
        &session,
        "messages",
        serde_json::json!({
            "id": everruns_contracts::typed_id::MessageId::new(),
            "text": "Run the triage script",
            "metadata": {
                everruns_contracts::runtime::saved_scripts::SCRIPT_RUN_METADATA_KEY:
                    { "script": "triage" },
            },
        }),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{admitted}");
    fleet.turns(&session, 1).await;

    let events = serde_json::to_string(&fleet.events(&session).await).unwrap();
    assert!(events.contains("triaged-queue"), "the script ran: {events}");
    let listed: Vec<_> = cp
        .lookups()
        .into_iter()
        .filter(|(_, command, _)| command == "list_agent_scripts")
        .collect();
    assert!(!listed.is_empty());
    assert_eq!(listed[0].2["session_id"], session.as_str(), "{listed:?}");
}
