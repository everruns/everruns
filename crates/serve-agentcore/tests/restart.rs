//! A turn parked on a person survives a microVM restart: AgentCore stops an
//! idle session's microVM and boots a new one on the same session storage,
//! and the approval or question it was waiting on is still open there, and
//! answering it finishes the turn.
//!
//! The first process is a real process, killed outright, so nothing it held
//! in memory (or would have written on a graceful shutdown) survives: the
//! test binary runs itself as that process (`restart_child_process`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use serve::prelude::*;
use serve::{Mode, sim};
use serve_agentcore::{Options, SESSION_HEADER, router};
use tokio::io::{AsyncBufReadExt, BufReader};

/// Set in the first, killed process: the storage directory it boots on.
const CHILD: &str = "SERVE_AGENTCORE_RESTART_CHILD";

/// The AgentCore session (and AG-UI thread) of the approval flow.
const THREAD: &str = "agentcore-session-restart-0123456789abcdef01234";

fn first_process() -> bool {
    std::env::var_os(CHILD).is_some()
}

/// The simulator keeps its place in a script in process memory, so a
/// restarted process starts its script over. The first process asks for the
/// tool call; the restarted one carries on from the step after it.
fn script(call: sim::SimTurn, reply: &str) -> sim::LlmSimConfig {
    if first_process() {
        sim::script([call, sim::reply(reply)])
    } else {
        sim::script([sim::reply(reply)])
    }
}

/// Shares the report, which needs a person's approval.
#[agent(default)]
fn assistant() -> Agent {
    Agent::builder()
        .model("sim")
        .instructions("Test agent.")
        .offline(script(
            sim::call("share", json!({ "to": "team" })),
            "shared",
        ))
        .build()
}

/// Asks where to deploy, through the built-in `ask_user`.
#[agent]
fn asker() -> Agent {
    Agent::builder()
        .model("sim")
        .instructions("Ask before deploying.")
        .tools(Vec::<String>::new())
        .offline(script(
            sim::call(
                "ask_user",
                json!({ "questions": [{
                    "header": "Target",
                    "question": "Where should I deploy?",
                    "options": [
                        { "label": "Staging", "description": "Safe", "default": true },
                        { "label": "Production", "description": "Live" }
                    ]
                }] }),
            ),
            "deploying",
        ))
        .build()
}

/// Calls of `share` that ran in this process.
static SHARED: AtomicUsize = AtomicUsize::new(0);

/// Share the report with someone.
#[tool(needs_approval)]
async fn share(to: String) -> Result<String> {
    SHARED.fetch_add(1, Ordering::SeqCst);
    Ok(format!("shared with {to}"))
}

async fn boot_in(dir: &Path) -> String {
    let app = App::builder().discover().build();
    assert!(app.errors().is_empty(), "{:?}", app.errors());
    let mut options = Options::new(Mode::Eval);
    options.data_dir = Some(dir.join("data"));
    options.workspace = Some(dir.join("workspace"));
    let (app, _) = router(app, options).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

/// Not a test of its own: the first process of
/// `parked_requests_survive_a_restart_on_the_same_storage`, which runs this
/// binary with [`CHILD`] set. Serves until it is killed.
#[tokio::test]
async fn restart_child_process() {
    let Some(dir) = std::env::var_os(CHILD) else {
        return;
    };
    let base = boot_in(Path::new(&dir)).await;
    println!("LISTENING {base}");
    std::future::pending::<()>().await;
}

/// The `data:` payloads of an AG-UI run.
async fn invoke(client: &reqwest::Client, base: &str, body: Value) -> Vec<Value> {
    let response = client
        .post(format!("{base}/invocations"))
        .header(SESSION_HEADER, THREAD)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = tokio::time::timeout(Duration::from_secs(30), response.text())
        .await
        .expect("the run ends")
        .unwrap();
    let events: Vec<Value> = body
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|data| serde_json::from_str(data.trim()).ok())
        .collect();
    assert_eq!(events.last().unwrap()["type"], "RUN_FINISHED", "{events:?}");
    events
}

fn outcome(events: &[Value]) -> &Value {
    &events.last().unwrap()["outcome"]
}

fn text(events: &[Value]) -> String {
    events
        .iter()
        .filter(|e| e["type"] == "TEXT_MESSAGE_CONTENT")
        .filter_map(|e| e["delta"].as_str())
        .collect()
}

async fn get(client: &reqwest::Client, url: String) -> Value {
    client.get(url).send().await.unwrap().json().await.unwrap()
}

async fn post(client: &reqwest::Client, url: String, body: Value) -> reqwest::Response {
    client.post(url).json(&body).send().await.unwrap()
}

/// A `/v1` session of `agent` with one message sent, once it waits on a person.
async fn parked_session(client: &reqwest::Client, base: &str, agent: &str) -> Value {
    let session = post(
        client,
        format!("{base}/v1/sessions"),
        json!({ "agent_name": agent }),
    )
    .await
    .json::<Value>()
    .await
    .unwrap();
    let id = session["id"].as_str().unwrap().to_string();
    let message =
        json!({ "message": { "role": "user", "content": [{ "type": "text", "text": "go" }] } });
    let sent = post(client, format!("{base}/v1/sessions/{id}/messages"), message).await;
    assert!(sent.status().is_success(), "{}", sent.status());
    wait_for(client, base, &id, |s| {
        s["status"] == "waitingfortoolresults"
    })
    .await
}

async fn wait_for(
    client: &reqwest::Client,
    base: &str,
    id: &str,
    check: impl Fn(&Value) -> bool,
) -> Value {
    for _ in 0..400 {
        let view = get(client, format!("{base}/v1/sessions/{id}")).await;
        if check(&view) {
            return view;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("session {id} never reached the expected state");
}

/// The session's events once its log holds a finished turn.
async fn finished(client: &reqwest::Client, base: &str, id: &str) -> Vec<Value> {
    for _ in 0..400 {
        let events = get(client, format!("{base}/v1/sessions/{id}/events")).await;
        let events = events["data"].as_array().cloned().unwrap();
        if events.iter().any(|e| e["type"] == "turn.completed") {
            return events;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("session {id} never finished its turn");
}

fn completed<'a>(events: &'a [Value], tool: &str) -> &'a Value {
    events
        .iter()
        .rev()
        .find(|e| e["type"] == "tool.completed" && e["data"]["tool_name"] == tool)
        .unwrap_or_else(|| panic!("no {tool} completion in {events:?}"))
}

#[tokio::test]
async fn parked_requests_survive_a_restart_on_the_same_storage() {
    if first_process() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();

    // The first microVM.
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "restart_child_process",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, dir.path())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let first = loop {
        let line = tokio::time::timeout(Duration::from_secs(120), lines.next_line())
            .await
            .expect("the first process boots")
            .unwrap()
            .expect("the first process prints its address");
        // The test harness prints the test's name on the same line.
        if let Some((_, base)) = line.split_once("LISTENING ") {
            break base.trim().to_string();
        }
    };

    // An AG-UI run parks on the approval and ends with its interrupt.
    let run = invoke(&client, &first, json!({ "prompt": "share the report" })).await;
    assert_eq!(outcome(&run)["type"], "interrupt", "{run:?}");
    let interrupt = &outcome(&run)["interrupts"][0];
    assert_eq!(interrupt["reason"], "tool_approval");
    let call = interrupt["id"].as_str().unwrap().to_string();

    // A `/v1` approval and an `ask_user` question park the same way.
    let denied = parked_session(&client, &first, "assistant").await;
    let denied_id = denied["id"].as_str().unwrap().to_string();
    let denied_call = denied["pending_approvals"][0]["tool_call_id"].clone();
    let asked = parked_session(&client, &first, "asker").await;
    let asked_id = asked["id"].as_str().unwrap().to_string();
    let question = asked["pending_questions"][0].clone();

    // AgentCore stops the microVM.
    child.kill().await.unwrap();
    child.wait().await.unwrap();

    // A new microVM on the same session storage.
    let second = boot_in(dir.path()).await;

    // The approval is still open: a new message is asked it again.
    let again = invoke(&client, &second, json!({ "prompt": "hello?" })).await;
    assert_eq!(outcome(&again)["type"], "interrupt", "{again:?}");
    assert_eq!(outcome(&again)["interrupts"][0]["id"], call);

    // Approving it runs the tool and finishes the turn.
    let resumed = invoke(
        &client,
        &second,
        json!({
            "threadId": THREAD,
            "runId": "resume",
            "messages": [],
            "resume": [{ "interruptId": call, "status": "resolved", "payload": { "decision": "allow" } }],
        }),
    )
    .await;
    assert!(outcome(&resumed).is_null(), "{resumed:?}");
    assert_eq!(text(&resumed), "shared");
    assert_eq!(
        SHARED.load(Ordering::SeqCst),
        1,
        "the approved call ran once"
    );

    // The `/v1` session view still shows its approval; denying it finishes
    // the turn without running the tool.
    let view = wait_for(&client, &second, &denied_id, |s| {
        s["status"] == "waitingfortoolresults"
    })
    .await;
    assert_eq!(view["pending_approvals"][0]["tool_call_id"], denied_call);
    let denial = post(
        &client,
        format!(
            "{second}/v1/sessions/{denied_id}/approvals/{}",
            denied_call.as_str().unwrap()
        ),
        json!({ "decision": "deny" }),
    )
    .await;
    assert_eq!(denial.status(), 200);
    let events = finished(&client, &second, &denied_id).await;
    let rejected = completed(&events, "share");
    assert!(
        rejected["data"].to_string().contains("rejected"),
        "{rejected}"
    );
    assert_eq!(SHARED.load(Ordering::SeqCst), 1, "a denied call never runs");

    // The question is still open, and its answer reaches the model.
    let view = wait_for(&client, &second, &asked_id, |s| {
        s["status"] == "waitingfortoolresults"
    })
    .await;
    assert_eq!(view["pending_questions"][0], question);
    let answered = post(
        &client,
        format!("{second}/v1/sessions/{asked_id}/question-answers"),
        json!({
            "tool_call_id": question["tool_call_id"],
            "answers": [{ "id": question["questions"][0]["id"], "selected": ["Production"] }],
        }),
    )
    .await;
    assert_eq!(answered.status(), 200);
    let events = finished(&client, &second, &asked_id).await;
    assert!(
        completed(&events, "ask_user")["data"]
            .to_string()
            .contains("Production")
    );
}
