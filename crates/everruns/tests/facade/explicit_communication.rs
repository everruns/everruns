//! An explicit agent's `no_reply` or send-only step ends the turn: the model
//! is not asked again.

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
async fn a_send_only_step_ends_the_turn_without_another_generation() {
    // The person already has the sent text; asking the model again only
    // produced private notes or a duplicate message.
    let sim = LlmSimConfig::scripted(vec![
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "send_message".into(),
            arguments: serde_json::json!({"text":"delivered reply"}),
            id: None,
        }]),
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "send_message".into(),
            arguments: serde_json::json!({"text":"should never be sent"}),
            id: None,
        }]),
        SimTurn::Assistant("should never be generated".into()),
    ])
    .with_on_exhausted(OnExhausted::Loop);
    let agent = Agent::builder()
        .instructions("Send the answer explicitly.")
        .model(Model::simulated_with_config(sim))
        .communication(Communication::Explicit)
        .build()
        .unwrap();
    let session = InMemoryEngine::new().create(agent);
    let mut events = session.events();

    let turn = session.run("work").await.expect("turn runs");

    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.tool_calls, 1);
    assert_eq!(turn.response, "delivered reply");
    let mut sent = Vec::new();
    while let Ok(Some(event)) = events.try_recv() {
        if let everruns::SessionEventKind::MessageSent { text, .. } = event.kind {
            sent.push(text);
        }
    }
    assert_eq!(sent, ["delivered reply"]);
}
