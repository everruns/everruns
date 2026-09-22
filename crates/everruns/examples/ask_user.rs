//! Answer an agent's structured questions in-process — using only `everruns`.
//!
//! Run with:
//!
//! ```text
//! cargo run -p everruns --example ask_user
//! ```
//!
//! Runs entirely offline against a scripted simulator, so the agent asks the
//! same two questions every time and the answers are the only variable.
//!
//! Three shapes, in order: a single-select question, a multi-select one, and
//! the defaults path taken when no responder is registered.

use everruns::ask_user::{
    Answer, AnsweredBy, AskUser, AskUserOption, DefaultsResponder, Outcome, Question, QuestionKind,
    Status, async_trait,
};
use everruns::{Agent, Engine, LlmSimConfig, Model, SimToolCall, SimTurn};

/// The batch the agent asks. Built as values rather than JSON so the same
/// questions drive both the scripted tool call and the responder below.
fn questions() -> Vec<Question> {
    vec![
        Question {
            kind: QuestionKind::Choice,
            id: Some("region".into()),
            header: "Region".into(),
            question: "Which region should I deploy to?".into(),
            multi_select: false,
            allow_other: true,
            options: vec![
                AskUserOption {
                    label: "eu-west-1".into(),
                    description: "Ireland".into(),
                    is_default: true,
                },
                AskUserOption {
                    label: "us-east-1".into(),
                    description: "N. Virginia".into(),
                    is_default: false,
                },
            ],
            // Both belong to `kind: Secret` only, which answers with a
            // `secret_ref` instead of a selection.
            secret_name: None,
            purpose: None,
        },
        Question {
            kind: QuestionKind::Choice,
            id: Some("checks".into()),
            header: "Checks".into(),
            question: "Which checks should run first?".into(),
            // The one field that changes an answer's shape: `selected` carries
            // several labels instead of one.
            multi_select: true,
            allow_other: true,
            options: vec![
                AskUserOption {
                    label: "lint".into(),
                    description: "Fast".into(),
                    is_default: true,
                },
                AskUserOption {
                    label: "unit".into(),
                    description: "Fast".into(),
                    is_default: true,
                },
                AskUserOption {
                    label: "e2e".into(),
                    description: "Slow".into(),
                    is_default: false,
                },
            ],
            secret_name: None,
            purpose: None,
        },
    ]
}

/// A responder that answers from a script instead of from a person.
///
/// A real host reads the terminal, opens a dialog, or posts to a channel. The
/// contract is the same either way: one `ask` receives the whole batch and
/// returns one answer per question, keyed by the question's `id`.
struct ScriptedResponder;

#[async_trait]
impl AskUser for ScriptedResponder {
    async fn ask(&self, questions: &[Question]) -> Outcome {
        let answers = questions
            .iter()
            .map(|question| {
                let selected: Vec<String> = question
                    .options
                    .iter()
                    .filter(|option| option.is_default)
                    .map(|option| option.label.clone())
                    .collect();
                println!(
                    "  asked {:<7} [{}] -> {selected:?}",
                    question.header,
                    if question.multi_select {
                        "multi "
                    } else {
                        "single"
                    },
                );
                Answer {
                    id: question.id.clone().unwrap_or_default(),
                    selected,
                    other_text: None,
                    // No credential ever rides here. A `secret` question answers
                    // with `secret_ref`; a choice question leaves it absent.
                    secret_ref: None,
                }
            })
            .collect();
        Outcome {
            status: Status::Answered,
            answered_by: AnsweredBy::User,
            answers,
        }
    }
}

/// One scripted turn: the model calls `ask_user`, then reports what it heard.
fn asking_script() -> LlmSimConfig {
    LlmSimConfig::scripted(vec![
        SimTurn::Mixed {
            text: "Let me check two things first.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "ask_user".to_string(),
                arguments: serde_json::json!({ "questions": questions() }),
                id: Some("call_ask".to_string()),
            }],
        },
        SimTurn::Assistant("Deploying to eu-west-1 after lint and unit.".to_string()),
    ])
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- With a responder: the host answers, in-process ---------------------
    // `.ask_user(...)` both enables the capability and supplies the responder.
    // It runs inside the tool call, so the turn never parks waiting for a
    // browser or a server.
    println!("with a responder:");
    let agent = Agent::builder()
        .instructions("Ask before deploying.")
        .model(Model::simulated_with_config(asking_script()))
        .ask_user(ScriptedResponder)
        .build()?;

    let turn = Engine::new()
        .create(agent)
        .run("deploy the service")
        .await?;
    println!("  response: {}", turn.response);

    // --- Without one: declared defaults, resolved unattended -----------------
    // `.capability("ask_user")` on its own installs `DefaultsResponder`, so the
    // turn still completes rather than hanging on a question nobody will read.
    println!("\nwithout a responder:");
    let unattended = Agent::builder()
        .instructions("Ask before deploying.")
        .model(Model::simulated_with_config(asking_script()))
        .capability("ask_user")
        .build()?;

    let turn = Engine::new()
        .create(unattended)
        .run("deploy the service")
        .await?;
    println!("  response: {}", turn.response);

    // `DefaultsResponder` is an ordinary `AskUser`, so what the model received
    // above is worth showing directly: each question's declared default (or its
    // first option), marked `Unattended` so the model can tell that nobody
    // actually answered.
    let outcome = DefaultsResponder.ask(&questions()).await;
    println!("  answered_by: {:?}", outcome.answered_by);
    for answer in &outcome.answers {
        println!("  {:<7} -> {:?}", answer.id, answer.selected);
    }

    Ok(())
}
