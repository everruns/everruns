use everruns_builtins::AskUserCapability;
use everruns_core::EventData;
use everruns_llmsim::{LlmSimConfig, SimToolCall, SimTurn};
use everruns_test_support::InMemoryAgenticLoop;
use serde_json::json;

fn option(label: &str, is_default: bool) -> serde_json::Value {
    json!({
        "label": label,
        "description": format!("{label} description"),
        "default": is_default
    })
}

fn question() -> serde_json::Value {
    json!({
        "header": "Target",
        "question": "Where should I deploy?",
        "options": [option("Staging", false), option("Production", false)]
    })
}

#[tokio::test]
async fn malformed_ask_user_calls_fail_without_parking() {
    let oversized_questions = vec![question(), question(), question(), question(), question()];
    let config = LlmSimConfig::scripted(vec![
        SimTurn::ToolCalls(vec![
            SimToolCall {
                name: "ask_user".to_string(),
                arguments: json!({
                    "questions": [{
                        "kind": "secret",
                        "header": "Token",
                        "question": "What is the token?",
                        "options": [option("One", false), option("Two", false)]
                    }]
                }),
                id: Some("call_secret".to_string()),
            },
            SimToolCall {
                name: "ask_user".to_string(),
                arguments: json!({
                    "questions": [{
                        "header": "Target",
                        "question": "Where should I deploy?",
                        "options": [option("Staging", true), option("Production", true)]
                    }]
                }),
                id: Some("call_defaults".to_string()),
            },
            SimToolCall {
                name: "ask_user".to_string(),
                arguments: json!({"questions": oversized_questions}),
                id: Some("call_oversized".to_string()),
            },
        ]),
        SimTurn::Assistant("Recovered after validation errors.".to_string()),
    ]);
    let agent = InMemoryAgenticLoop::builder()
        .capability(AskUserCapability)
        .with_llm_sim(config)
        .build()
        .await
        .expect("build loop");

    let turn = agent.run_turn("Ask me.").await.expect("run turn");
    assert_eq!(turn.response, "Recovered after validation errors.");

    let events = agent.events().await;
    let failed_ids: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.data {
            EventData::ToolCompleted(data) if !data.success => Some(data.tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        failed_ids,
        ["call_secret", "call_defaults", "call_oversized"]
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(&event.data, EventData::ToolCallRequested(_))),
        "invalid calls must not emit tool.call_requested or park the turn"
    );
}

#[tokio::test]
async fn valid_ask_user_call_emits_generated_question_id() {
    let config = LlmSimConfig::scripted(vec![
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "ask_user".to_string(),
            arguments: json!({"questions": [question()]}),
            id: Some("call_valid".to_string()),
        }]),
        SimTurn::Assistant("Question handled.".to_string()),
    ]);
    let agent = InMemoryAgenticLoop::builder()
        .capability(AskUserCapability)
        .with_llm_sim(config)
        .build()
        .await
        .expect("build loop");

    agent.run_turn("Ask me.").await.expect("run turn");

    let events = agent.events().await;
    let requested = events
        .iter()
        .find_map(|event| match &event.data {
            EventData::ToolCallRequested(data) => data.tool_calls.first(),
            _ => None,
        })
        .expect("tool.call_requested");
    assert_eq!(requested.id, "call_valid");
    assert_eq!(requested.arguments["questions"][0]["id"], "question_1");
}
