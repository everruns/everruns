//! Actual workerd authority + native executor process crash. See README for setup.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use everruns::{Agent, FunctionTool, LlmSimConfig, Model, ToolCall};
use everruns_core::host::AcceptedTurnInput;
use everruns_durable_object::{AuthorityClient, Executor, SessionBinding};
use serde_json::json;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const AUTHORITY_TOKEN: &str = "authority-test-token-at-least-32-bytes";
const EXECUTOR_TOKEN: &str = "executor-test-token-at-least-32-bytes";
fn binding() -> SessionBinding {
    SessionBinding {
        agent: "recovery-test".into(),
        build_id: std::env::var("DO_TEST_BUILD").unwrap_or_else(|_| "v1".into()),
    }
}
fn client() -> AuthorityClient {
    AuthorityClient::new("http://127.0.0.1:8789", "test", AUTHORITY_TOKEN).unwrap()
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(mode: &str, effect: &std::path::Path, build: &str) -> Process {
    Process(
        Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "fixture_server", "--nocapture"])
            .env("DO_TEST_BUILD", build)
            .env("DO_TEST_MODE", mode)
            .env("DO_TEST_EFFECT", effect)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    )
}

/// Test-only executable fixture: scripted providers stay outside runnable examples.
#[tokio::test]
#[ignore = "spawned by the recovery test"]
async fn fixture_server() {
    let mode = std::env::var("DO_TEST_MODE").expect("test fixture mode");
    let effect = std::env::var("DO_TEST_EFFECT").unwrap();
    let tool = FunctionTool::new(
        "mutate",
        "Record an external effect",
        json!({"type":"object","properties":{}}),
        move |_: serde_json::Value| {
            let effect = effect.clone();
            async move {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(effect)
                    .unwrap();
                writeln!(file, "effect").unwrap();
                file.sync_all().unwrap();
                tokio::time::sleep(Duration::from_secs(120)).await;
                Ok::<_, String>(json!({"done":true}))
            }
        },
    );
    let model = if mode == "crash" {
        Model::simulated_with_config(LlmSimConfig::fixed("Recovered.").with_tool_call_sequence(
            vec![
                vec![ToolCall {
                    id: "call_mutate".into(),
                    name: "mutate".into(),
                    arguments: json!({}),
                }],
                vec![],
            ],
        ))
    } else {
        Model::simulated("Recovered.")
    };
    let agent = Agent::builder()
        .instructions("Test the shared durable runtime.")
        .name("recovery-test")
        .model(model)
        .tool(tool)
        .build()
        .unwrap();
    let factory = move |id, backends| {
        let agent = agent.clone();
        async move { agent.execution_host(id, backends).await }
    };
    let executor = Executor::new(client(), binding(), EXECUTOR_TOKEN, factory, 4).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8790")
        .await
        .unwrap();
    axum::serve(listener, executor.router()).await.unwrap();
}

#[tokio::test]
#[ignore = "requires the local workerd authority on port 8789"]
async fn a_killed_native_executor_recovers_from_do_without_resending_input() {
    let session = client().session(everruns::SessionId::new());
    let build = SessionBinding {
        agent: "recovery-test".into(),
        build_id: uuid::Uuid::new_v4().to_string(),
    };
    session.bind(&build).await.unwrap();
    let effect = std::env::temp_dir().join(format!("everruns-do-effect-{}", uuid::Uuid::new_v4()));
    let mut initial = spawn("crash", &effect, &build.build_id);
    let input = AcceptedTurnInput::new("mutate once");
    let message = session.send(input.clone()).await.unwrap();
    assert_eq!(
        session.send(input).await.unwrap(),
        message,
        "lost admission reply is safe to retry"
    );
    tokio::time::timeout(Duration::from_secs(40), async {
        while !effect.exists() {
            assert!(
                initial.0.try_wait().unwrap().is_none(),
                "initial executor exited before dispatch"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the initial tool produces its external effect");
    initial.0.kill().unwrap();
    initial.0.wait().unwrap();
    let _replacement = spawn("resume", &effect, &build.build_id);
    // No new input, claim or resume request: the previously committed alarm
    // has to pick the session up after its owner's lease expires.
    let events = tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            let events = session.events(0, None, 256).await.unwrap().events;
            if events
                .iter()
                .any(|event| event.event_type == "turn.completed")
            {
                break events;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    })
    .await
    .expect("the alarm activates the replacement executor");
    assert_eq!(
        std::fs::read_to_string(&effect).unwrap(),
        "effect\n",
        "at-most-once tool is not dispatched again"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "input.message" && event.context.turn_id.is_none())
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "turn.started")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "turn.completed")
            .count(),
        1
    );
    let interrupted = events
        .iter()
        .find(|event| event.event_type == "tool.completed")
        .expect("unknown tool result is explicit");
    assert_eq!(
        serde_json::to_value(&interrupted.data).unwrap()["status"],
        "interrupted"
    );
    std::fs::remove_file(effect).unwrap();
}
