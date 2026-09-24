use std::sync::{Arc, Mutex};

use everruns::ask_user::{
    Answer, AnsweredBy, AskUser, Outcome, Question, QuestionKind, Status, async_trait,
};
use everruns::{Agent, InMemoryEngine, LlmSimConfig, Model, ToolCall};
use serde_json::json;

#[derive(Clone, Default)]
struct RecordingResponder {
    questions: Arc<Mutex<Vec<Question>>>,
}

struct TextResponder;

#[async_trait]
impl AskUser for TextResponder {
    async fn ask(&self, questions: &[Question]) -> Outcome {
        Outcome {
            status: Status::Answered,
            answered_by: AnsweredBy::User,
            answers: vec![Answer {
                id: questions[0].id.clone().unwrap(),
                selected: Vec::new(),
                other_text: Some("feature/open-question".to_string()),
                secret_ref: None,
            }],
        }
    }
}

#[async_trait]
impl AskUser for RecordingResponder {
    async fn ask(&self, questions: &[Question]) -> Outcome {
        *self.questions.lock().unwrap() = questions.to_vec();
        Outcome {
            status: Status::Answered,
            answered_by: AnsweredBy::User,
            answers: vec![Answer {
                id: questions[0].id.clone().unwrap(),
                selected: vec!["Production".to_string()],
                other_text: None,
                secret_ref: None,
            }],
        }
    }
}
fn asking_model() -> Model {
    Model::simulated_with_config(
        LlmSimConfig::fixed("Deploying to production.").with_tool_call_sequence(vec![
            vec![ToolCall {
                id: "call_target".to_string(),
                name: "ask_user".to_string(),
                arguments: json!({
                    "questions": [{
                        "header": "Target",
                        "question": "Where should I deploy?",
                        "options": [
                            {"label": "Staging", "description": "Safe", "default": true},
                            {"label": "Production", "description": "Live"}
                        ]
                    }]
                }),
            }],
            vec![],
        ]),
    )
}

#[tokio::test]
async fn builder_responder_answers_in_process() {
    let responder = RecordingResponder::default();
    let questions = responder.questions.clone();
    let agent = Agent::builder()
        .instructions("Ask before choosing a deployment target.")
        .model(asking_model())
        .ask_user(responder)
        .build()
        .expect("valid agent");

    let turn = InMemoryEngine::new()
        .create(agent)
        .run("Deploy the service.")
        .await
        .expect("turn runs");

    assert!(turn.success);
    assert_eq!(turn.tool_calls, 1);
    let questions = questions.lock().unwrap();
    assert_eq!(questions.len(), 1);
    assert_eq!(questions[0].id.as_deref(), Some("question_1"));
}

#[tokio::test]
async fn text_answer_round_trips_through_the_public_responder_contract() {
    let questions: Vec<Question> = serde_json::from_value(json!([{
        "kind": "text",
        "id": "branch_name",
        "header": "Branch",
        "question": "What should I call this branch?"
    }]))
    .expect("Text question uses the public contract");
    assert_eq!(questions[0].kind, QuestionKind::Text);

    let outcome = TextResponder.ask(&questions).await;
    let wire = serde_json::to_value(&outcome).expect("outcome serializes");
    let decoded: Outcome = serde_json::from_value(wire).expect("outcome deserializes");
    assert!(decoded.answers[0].selected.is_empty());
    assert_eq!(
        decoded.answers[0].other_text.as_deref(),
        Some("feature/open-question")
    );
}

#[tokio::test]
async fn capability_without_responder_uses_unattended_defaults() {
    let agent = Agent::builder()
        .instructions("Ask before choosing a deployment target.")
        .model(asking_model())
        .capability("ask_user")
        .build()
        .expect("valid agent");

    let turn = InMemoryEngine::new()
        .create(agent)
        .run("Deploy the service.")
        .await
        .expect("turn runs");

    assert!(turn.success);
    assert_eq!(turn.tool_calls, 1);
    assert_eq!(turn.response, "Deploying to production.");
}

fn asking_for_a_secret() -> Model {
    Model::simulated_with_config(
        LlmSimConfig::fixed("I cannot continue without the key.").with_tool_call_sequence(vec![
            vec![ToolCall {
                id: "call_secret".to_string(),
                name: "ask_user".to_string(),
                arguments: json!({
                    "questions": [{
                        "kind": "secret",
                        "header": "Stripe key",
                        "question": "Which Stripe restricted key should I use?",
                        "secret_name": "STRIPE_API_KEY",
                        "purpose": "Read-only charge lookups."
                    }]
                }),
            }],
            vec![],
        ]),
    )
}

/// EVE-1058: a headless Framework host has nobody to type a credential, and a
/// "default credential" is meaningless. The unattended responder declines
/// rather than inventing one, so proceeding without it stays the model's
/// explicit decision.
#[tokio::test]
async fn an_unattended_secret_question_is_declined_not_defaulted() {
    let agent = Agent::builder()
        .instructions("Ask for the key before calling Stripe.")
        .model(asking_for_a_secret())
        .capability("ask_user")
        .build()
        .expect("valid agent");

    let turn = InMemoryEngine::new()
        .create(agent)
        .run("Reconcile yesterday's charges.")
        .await
        .expect("turn runs");

    assert!(turn.success);
    assert_eq!(turn.tool_calls, 1);
}
