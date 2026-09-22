//! Answer an agent's structured questions from the host process.
//!
//! Run with:
//!
//! ```text
//! cargo run -p everruns --example ask_user
//! ```
//!
//! `ask_user` lets a model collect one small batch of decisions without ending
//! the turn in prose. A hosted client answers through the browser; an embedded
//! host implements [`AskUser`] and answers inside the tool call, so the turn
//! never parks. This example shows both the responder path and what happens
//! when nobody registers one.
//!
//! It is for decisions and preferences. It is never a consent gate — a
//! destructive or outward-facing action needs `request_approval`, whose wait
//! does not auto-resolve.

use everruns::ask_user::{
    Answer, AnsweredBy, AskUser, DefaultsResponder, Outcome, Question, QuestionKind, Status,
    async_trait,
};
use everruns::{Agent, InMemoryEngine, LlmSimConfig, Model, ToolCall};
use serde_json::json;

/// A responder that answers from what the host already knows.
///
/// The host, not the model, decides. Here the rule is canned so the example
/// stays deterministic; a real one would prompt a person, read a config file,
/// or consult the signed-in user's saved preferences.
struct HouseRules;

#[async_trait]
impl AskUser for HouseRules {
    async fn ask(&self, questions: &[Question]) -> Outcome {
        let answers = questions
            .iter()
            .map(|question| {
                // A secret question carries no options and must never be
                // answered with a fabricated value — decline it instead.
                let selected = if question.kind == QuestionKind::Secret {
                    Vec::new()
                } else if question.multi_select {
                    // Take everything that is not the most expensive option.
                    question
                        .options
                        .iter()
                        .filter(|option| !option.label.contains("Premium"))
                        .map(|option| option.label.clone())
                        .collect()
                } else {
                    // Single-select: prefer what the model marked as default.
                    question
                        .options
                        .iter()
                        .find(|option| option.is_default)
                        .or_else(|| question.options.first())
                        .map(|option| option.label.clone())
                        .into_iter()
                        .collect()
                };
                Answer {
                    id: question.id.clone().unwrap_or_default(),
                    selected,
                    other_text: None,
                    secret_ref: None,
                }
            })
            .collect();

        Outcome {
            status: Status::Answered,
            // Only an outcome a person actually produced is `User`. This one
            // came from a rule, so it says so — the model reads provenance
            // before acting on a choice nobody made.
            answered_by: AnsweredBy::Unattended,
            answers,
        }
    }
}

/// The one question set this example uses: a single-select and a multi-select.
///
/// Options are ordered most-applicable-first, because an unattended resolution
/// takes the marked default, or the first option when none is marked.
fn deployment_questions() -> serde_json::Value {
    json!([
        {
            "kind": "choice", "id": "target", "header": "Target",
            "question": "Where should I deploy?",
            "multi_select": false, "allow_other": true,
            "options": [
                {"label": "Staging", "description": "Safe and reversible.", "default": true},
                {"label": "Production", "description": "Serves live traffic."}
            ]
        },
        {
            "kind": "choice", "id": "checks", "header": "Checks",
            "question": "Which checks should run first?",
            "multi_select": true, "allow_other": false,
            "options": [
                {"label": "Lint", "description": "Seconds."},
                {"label": "Tests", "description": "A few minutes."},
                {"label": "Premium scan", "description": "Billed per run."}
            ]
        }
    ])
}

/// A model that asks that set, then answers. `llmsim` keeps the example
/// deterministic offline.
fn asking_model() -> Model {
    Model::simulated_with_config(
        LlmSimConfig::sequence(vec![
            "Let me confirm how you want this deployed.".into(),
            "Deploying to Staging with lint and tests.".into(),
        ])
        .with_tool_call_sequence(vec![
            vec![ToolCall {
                id: "call_ask_1".into(),
                name: "ask_user".into(),
                arguments: json!({ "questions": deployment_questions() }),
            }],
            vec![],
        ]),
    )
}

fn render(label: &str, outcome: &Outcome) {
    println!("{label}: {:?} by {:?}", outcome.status, outcome.answered_by);
    for answer in &outcome.answers {
        println!("  {:<7} {}", answer.id, answer.selected.join(", "));
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- What each responder decides -------------------------------------
    //
    // Same questions, two hosts. `DefaultsResponder` is what a headless run
    // gets when nobody registers one: it applies the options the model marked
    // `default`, falls back to the first option, and reports `Unattended` so
    // the model can tell a real choice from a fallback.
    let questions: Vec<Question> =
        serde_json::from_value(deployment_questions()).expect("fixture matches the contract");
    render("house rules", &HouseRules.ask(&questions).await);
    render("no responder", &DefaultsResponder.ask(&questions).await);

    // --- The same thing through a turn ------------------------------------
    //
    // The responder runs inside the tool call, so the turn never enters
    // `waiting_for_tool_results` and needs no browser to come back.
    let agent = Agent::builder()
        .instructions("Confirm deployment choices before acting.")
        .model(asking_model())
        .ask_user(HouseRules)
        .build()?;

    let turn = InMemoryEngine::new().create(agent).run("Ship it.").await?;
    println!("turn: {}", turn.response);
    assert!(turn.success);
    assert_eq!(turn.tool_calls, 1);

    Ok(())
}
