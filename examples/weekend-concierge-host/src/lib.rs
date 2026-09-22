pub mod terminal;

use everruns::ask_user::{Answer, AnsweredBy, AskUser, Outcome, Question, Status, async_trait};
use everruns::{Agent, FunctionTool, InMemoryEngine, LlmSimConfig, Model, ToolCall};
use serde::Serialize;
use serde_json::{Value, json};

type ExampleError = Box<dyn std::error::Error + Send + Sync>;
type ExampleResult<T> = Result<T, ExampleError>;

const WEEKEND_BRIEF: &str = "Group: 6 coworkers\nBudget: under $40/person\nMood: playful but low-planning\nMust-have: snacks or desserts\nBackup option: quieter venue if people are tired";

#[derive(Clone, Serialize)]
struct Spot {
    name: &'static str,
    neighborhood: &'static str,
    vibe: &'static str,
    best_for: &'static [&'static str],
    max_group_size: u64,
    budget_per_person_usd: u64,
    signature: &'static str,
}

fn curated_spots() -> Vec<Spot> {
    vec![
        Spot {
            name: "Neon Arcade",
            neighborhood: "West Loop",
            vibe: "playful",
            best_for: &["arcade", "team outing", "snacks"],
            max_group_size: 10,
            budget_per_person_usd: 34,
            signature: "Retro cabinets, pinball, and surprisingly good grilled cheese.",
        },
        Spot {
            name: "Moonrise Mini Golf",
            neighborhood: "River North",
            vibe: "bright",
            best_for: &["mini golf", "celebration", "group"],
            max_group_size: 8,
            budget_per_person_usd: 38,
            signature: "Indoor mini golf with late-night desserts and mocktails.",
        },
        Spot {
            name: "Quiet Boardroom Cafe",
            neighborhood: "Logan Square",
            vibe: "cozy",
            best_for: &["board games", "conversation", "coffee"],
            max_group_size: 6,
            budget_per_person_usd: 22,
            signature: "Hundreds of board games, comfy booths, and zero pressure to rush.",
        },
    ]
}

fn lookup_neighborhood_spot() -> FunctionTool {
    FunctionTool::new(
        "lookup_neighborhood_spot",
        "Return curated local outing ideas from the application's private venue list.",
        json!({
            "type": "object",
            "properties": {
                "occasion": { "type": "string" },
                "group_size": { "type": "integer" },
                "vibe": { "type": "string" }
            },
            "required": ["occasion", "group_size", "vibe"],
            "additionalProperties": false
        }),
        |arguments: Value| async move {
            let occasion = arguments
                .get("occasion")
                .and_then(Value::as_str)
                .ok_or_else(|| "'occasion' is required and must be a string".to_string())?
                .to_lowercase();
            let group_size = arguments
                .get("group_size")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    "'group_size' is required and must be an unsigned integer".to_string()
                })?;
            let vibe = arguments
                .get("vibe")
                .and_then(Value::as_str)
                .ok_or_else(|| "'vibe' is required and must be a string".to_string())?
                .to_lowercase();

            let mut matches: Vec<_> = curated_spots()
                .into_iter()
                .filter(|spot| group_size <= spot.max_group_size)
                .filter(|spot| {
                    vibe == "any"
                        || spot.vibe.eq_ignore_ascii_case(&vibe)
                        || spot
                            .best_for
                            .iter()
                            .any(|tag| tag.eq_ignore_ascii_case(&vibe))
                })
                .filter(|spot| {
                    occasion == "outing"
                        || spot
                            .best_for
                            .iter()
                            .any(|tag| tag.eq_ignore_ascii_case(&occasion))
                })
                .collect();
            if matches.is_empty() {
                matches = curated_spots()
                    .into_iter()
                    .filter(|spot| group_size <= spot.max_group_size)
                    .collect();
            }

            Ok::<_, String>(json!({
                "occasion": occasion,
                "group_size": group_size,
                "vibe": vibe,
                "recommendations": matches,
                "application_note": "Backed by the application's curated list, not a remote API."
            }))
        },
    )
}

/// What the concierge asks before it plans anything.
///
/// A concierge that never asks about preferences is a bad concierge, so the
/// capability is motivated by the example rather than bolted onto it. One
/// single-select and one multi-select, which is the shape most hosts hit first.
fn weekend_questions() -> Value {
    json!([
        {
            "kind": "choice", "id": "energy", "header": "Energy",
            "question": "How much energy does the group have on Friday?",
            "multi_select": false, "allow_other": true,
            "options": [
                {"label": "Up for anything", "description": "Games, noise, moving around.", "default": true},
                {"label": "Low-key", "description": "Sitting, talking, snacks."}
            ]
        },
        {
            "kind": "choice", "id": "must_haves", "header": "Must-haves",
            "question": "What does the evening need?",
            "multi_select": true, "allow_other": true,
            "options": [
                {"label": "Snacks", "description": "Food on site."},
                {"label": "Desserts", "description": "Something sweet late."},
                {"label": "Quiet corner", "description": "Somewhere to actually talk."}
            ]
        }
    ])
}

/// A responder with the answers already decided, so the offline test stays
/// deterministic. The terminal responder in [`terminal`] is the real one.
struct ScriptedResponder;

#[async_trait]
impl AskUser for ScriptedResponder {
    async fn ask(&self, questions: &[Question]) -> Outcome {
        let pick = |question: &Question| -> Vec<String> {
            match question.id.as_deref() {
                Some("energy") => vec!["Up for anything".to_string()],
                Some("must_haves") => vec!["Snacks".to_string(), "Desserts".to_string()],
                _ => Vec::new(),
            }
        };
        Outcome {
            status: Status::Answered,
            answered_by: AnsweredBy::User,
            answers: questions
                .iter()
                .map(|question| Answer {
                    id: question.id.clone().unwrap_or_default(),
                    selected: pick(question),
                    other_text: None,
                    secret_ref: None,
                })
                .collect(),
        }
    }
}

pub struct ExampleRun {
    pub tool_names: Vec<String>,
    pub seeded_brief: String,
    pub final_response: String,
    pub success: bool,
    pub iterations: usize,
    pub tool_calls_count: usize,
    pub transcript: Vec<String>,
    pub event_types: Vec<String>,
    /// What the host answered, as the model received it.
    pub answers: Vec<String>,
}

pub async fn run_weekend_concierge_demo() -> ExampleResult<ExampleRun> {
    run_weekend_concierge(ScriptedResponder).await
}

/// The demo, against whichever host answers its questions.
///
/// `main.rs` passes [`terminal::TerminalResponder`] for a real terminal; the
/// test passes [`ScriptedResponder`] so it stays deterministic offline. The
/// agent is identical either way — swapping who answers is the only difference,
/// which is the point of the trait.
pub async fn run_weekend_concierge(
    responder: impl everruns::ask_user::AskUser + 'static,
) -> ExampleResult<ExampleRun> {
    let model = Model::simulated_with_config(
        LlmSimConfig::sequence(vec![
            "Before I look anything up, two quick questions.".into(),
            "Let me check the concierge book with that in mind.".into(),
            "Go with Neon Arcade in the West Loop. It fits six people, stays under the budget, and gives the group an easy mix of games and snacks. Backup: Quiet Boardroom Cafe if the team wants something calmer.".into(),
        ])
        .with_tool_call_sequence(vec![
            vec![ToolCall {
                id: "call_ask_1".into(),
                name: "ask_user".into(),
                arguments: json!({ "questions": weekend_questions() }),
            }],
            vec![ToolCall {
                id: "call_spot_1".into(),
                name: "lookup_neighborhood_spot".into(),
                arguments: json!({
                    "occasion": "team outing",
                    "group_size": 6,
                    "vibe": "playful"
                }),
            }],
            vec![],
        ]),
    );
    let agent = Agent::builder()
        .name("weekend-concierge")
        .instructions(
            "You are a concise local concierge. Ask the group about energy and must-haves with `ask_user` before you look anything up. Use `lookup_neighborhood_spot` for venue facts. Check `/workspace/weekend-brief.md` for the group's constraints before answering.",
        )
        .model(model)
        .tool(lookup_neighborhood_spot())
        .ask_user(responder)
        .readonly_file(
            "/workspace/welcome-note.md",
            "This session is hosted entirely in-process by the embedding application.",
        )
        .file("/workspace/weekend-brief.md", WEEKEND_BRIEF)
        .max_iterations(8)
        .build()?;

    let session = InMemoryEngine::new().create(agent.clone());
    let mut events = session.events();
    let context = session.inspect().await?;
    let tool_names = context.tools.iter().map(|tool| tool.name.clone()).collect();
    let seeded_brief = WEEKEND_BRIEF.to_string();
    let turn = session
        .run("Find a Friday outing for six teammates. Keep it playful, snack-friendly, and under $40 per person.")
        .await?;
    let context = session.inspect().await?;
    let transcript: Vec<String> = context
        .messages
        .iter()
        .map(|message| format!("[{}] {:?}", message.role, message.content))
        .collect();
    let mut event_types = Vec::new();
    let mut answers = Vec::new();
    while let Ok(Some(event)) = events.try_recv() {
        event_types.push(event.event_type().to_string());
        if let Some(found) = answers_from_event(event.canonical_json()) {
            answers = found;
        }
    }

    Ok(ExampleRun {
        tool_names,
        seeded_brief,
        final_response: turn.response,
        success: turn.success,
        iterations: turn.iterations,
        tool_calls_count: turn.tool_calls,
        transcript,
        event_types,
        answers,
    })
}

/// Read the answers out of the `ask_user` tool result the model received.
///
/// Taken from the emitted event rather than from the responder: this is what
/// actually reached the model, which is the only thing that can change its
/// plan. Free text counts — a person who typed something instead of picking is
/// still answering.
fn answers_from_event(data: &Value) -> Option<Vec<String>> {
    if data["data"]["tool_name"].as_str()? != "ask_user" {
        return None;
    }
    let result = parse_tool_result(&data["data"]["result"])?;
    let mut answers = Vec::new();
    for answer in result["answers"].as_array()? {
        for label in answer["selected"].as_array().into_iter().flatten() {
            if let Some(label) = label.as_str() {
                answers.push(label.to_string());
            }
        }
        if let Some(text) = answer["other_text"].as_str() {
            answers.push(format!("{text} (typed)"));
        }
    }
    Some(answers)
}

/// A tool result arrives as content parts whose `text` is the JSON the tool
/// returned, so the payload has to be parsed back out of a string:
///
/// ```json
/// [{"text": "{\"status\":\"answered\", ...}", "type": "text"}]
/// ```
fn parse_tool_result(result: &Value) -> Option<Value> {
    result
        .as_array()?
        .iter()
        .filter_map(|part| part["text"].as_str())
        .find_map(|text| serde_json::from_str(text).ok())
}

#[cfg(test)]
mod tests {
    use super::{lookup_neighborhood_spot, run_weekend_concierge_demo};
    use everruns::{Agent, InMemoryEngine, Model};

    #[tokio::test]
    async fn weekend_concierge_demo_stays_deterministic() {
        let run = run_weekend_concierge_demo()
            .await
            .expect("example run should succeed");

        assert_eq!(run.tool_names, vec!["ask_user", "lookup_neighborhood_spot"]);
        assert!(run.seeded_brief.contains("Budget: under $40/person"));
        assert!(run.final_response.contains("Neon Arcade"));
        assert!(run.final_response.contains("Quiet Boardroom Cafe"));
        assert!(run.success);
        assert_eq!(run.iterations, 3);
        assert_eq!(run.tool_calls_count, 2);
        assert!(
            run.transcript
                .iter()
                .any(|line| line.contains("call_spot_1"))
        );
        assert!(
            run.event_types
                .iter()
                .any(|event| event == "tool.completed")
        );
        assert!(
            run.event_types
                .iter()
                .any(|event| event == "reason.completed")
        );
    }

    /// The host answered, and what it answered is what reached the model.
    #[tokio::test]
    async fn the_hosts_answers_reach_the_model() {
        let run = run_weekend_concierge_demo()
            .await
            .expect("example run should succeed");

        assert!(
            run.transcript
                .iter()
                .any(|line| line.contains("call_ask_1"))
        );
        assert_eq!(
            run.answers,
            vec![
                "Up for anything".to_string(),
                "Snacks".to_string(),
                "Desserts".to_string(),
            ]
        );
        assert!(!run.answers.iter().any(|label| label == "Quiet corner"));
    }

    #[tokio::test]
    async fn lookup_tool_is_visible_through_framework_inspection() {
        let agent = Agent::builder()
            .instructions("test")
            .model(Model::simulated("done"))
            .tool(lookup_neighborhood_spot())
            .build()
            .expect("tool should be valid");
        let context = InMemoryEngine::new()
            .create(agent.clone())
            .inspect()
            .await
            .expect("context");
        assert_eq!(context.tools[0].name, "lookup_neighborhood_spot");
    }
}
