//! Agents that hand work to other agents (`AgentBuilder::member`) under the
//! coherent SQLite authority: a member runs as itself, in its own session,
//! and its session resumes after a restart with only the lead registered.
use super::*;
use everruns::hosting::SqliteHosting;
use everruns::{ContentPart, LlmSimConfig, SimToolCall, SimTurn};
use serde_json::{Value, json};

fn reviewer() -> Agent {
    Agent::builder()
        .name("reviewer")
        .description("Reviews a change and lists what to fix.")
        .instructions("Review the change you are given.")
        .model(Model::simulated("REVIEWED: ship it"))
        .build()
        .unwrap()
}

fn lead() -> Agent {
    Agent::builder()
        .name("lead")
        .instructions("Hand each change to reviewer before you answer.")
        .model(Model::simulated_with_config(LlmSimConfig::scripted(vec![
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "spawn_agent".into(),
                arguments: json!({
                    "name": "Review",
                    "instructions": "Review the login change.",
                    "target": {"type": "agent", "id": "reviewer"},
                    "mode": "foreground",
                }),
                id: Some("review_1".into()),
            }]),
            SimTurn::Assistant("The reviewer says ship it.".into()),
        ])))
        .member(reviewer())
        .build()
        .unwrap()
}

async fn handoff_result(session: &everruns::Session) -> Value {
    let history = session.history().page().await.unwrap();
    history
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|part| match part {
            ContentPart::ToolResult(result) if result.tool_call_id == "review_1" => {
                assert!(
                    result.error.is_none(),
                    "hand-off failed: {:?}",
                    result.error
                );
                result.result.clone()
            }
            _ => None,
        })
        .expect("recorded hand-off result")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_runs_as_itself_and_resumes_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authority.db");
    let engine = Engine::builder()
        .hosting(SqliteHosting::open(&path).unwrap())
        .register(build(), lead())
        .build();
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let turn = session
        .send_and_wait("Ship the login change.")
        .await
        .unwrap();
    assert!(turn.success, "{:?}", turn.error);

    let result = handoff_result(&session).await;
    let rendered = result.to_string();
    assert!(
        rendered.contains("REVIEWED: ship it"),
        "the reviewer's own model answered: {rendered}"
    );
    assert_eq!(result["status"], "completed", "{rendered}");
    let child: SessionId = result["handoff_id"]
        .as_str()
        .unwrap_or_else(|| panic!("hand-off names its session: {rendered}"))
        .parse()
        .unwrap();
    drop(session);
    drop(engine);

    // Only the lead is registered; the member's build is derived from it.
    let restarted = Engine::builder()
        .hosting(SqliteHosting::open(&path).unwrap())
        .register(build(), lead())
        .build();
    let child = restarted.resume(child).await.unwrap();
    let history = child.history().page().await.unwrap();
    let transcript = serde_json::to_string(
        &history
            .messages
            .iter()
            .map(|message| &message.content)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(
        transcript.contains("Review the login change."),
        "{transcript}"
    );
    assert!(transcript.contains("REVIEWED: ship it"), "{transcript}");
}
