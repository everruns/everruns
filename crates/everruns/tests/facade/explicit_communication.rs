//! An explicit agent's `no_reply` ends the turn: the model is not asked again.

use everruns::conversation::Communication;
use everruns::prelude::*;
use everruns::{LlmSimConfig, OnExhausted, SimToolCall, SimTurn};

fn no_reply() -> SimTurn {
    SimTurn::ToolCalls(vec![SimToolCall {
        name: "no_reply".into(),
        arguments: serde_json::json!({"reason": "nothing to add"}),
        id: None,
    }])
}

#[tokio::test]
async fn no_reply_ends_the_turn_without_another_generation() {
    // Before the fix the turn went back to the model after `no_reply`, and a
    // model that kept calling it looped until the iteration cap.
    let sim = LlmSimConfig::scripted(vec![
        no_reply(),
        no_reply(),
        SimTurn::Assistant("should never be generated".into()),
    ])
    .with_on_exhausted(OnExhausted::Loop);
    let agent = Agent::builder()
        .instructions("Answer only when needed.")
        .model(Model::simulated_with_config(sim))
        .communication(Communication::Explicit)
        .build()
        .expect("valid agent");
    let session = InMemoryEngine::new().create(agent);

    let turn = session.run("thanks!").await.expect("turn runs");

    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.tool_calls, 1);
    assert!(turn.response.is_empty(), "{}", turn.response);
}

#[tokio::test]
async fn completed_private_notes_do_not_delay_or_replace_sent_messages() {
    let sim = LlmSimConfig::scripted(vec![
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "send_message".into(),
            arguments: serde_json::json!({"text":"delivered reply"}),
            id: None,
        }]),
        SimTurn::Assistant("private working notes".into()),
    ])
    .with_response_delay(std::time::Duration::from_millis(100));
    let agent = Agent::builder()
        .instructions("Send the answer explicitly.")
        .model(Model::simulated_with_config(sim))
        .communication(Communication::Explicit)
        .build()
        .unwrap();
    let session = InMemoryEngine::new().create(agent);
    let mut events = session.events();
    let running = session.clone();
    let turn = tokio::spawn(async move { running.run("work").await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap().expect("live session");
            if let everruns::SessionEventKind::MessageSent { text, .. } = event.kind {
                assert_eq!(text, "delivered reply");
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(
        !turn.is_finished(),
        "send_message must be visible before the remaining model work ends"
    );
    let turn = turn.await.unwrap().unwrap();
    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.tool_calls, 1);
    assert_eq!(turn.response, "delivered reply");
}
