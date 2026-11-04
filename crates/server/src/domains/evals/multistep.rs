// Multistep eval cases: a simulated user and tool fixtures
// (knowledge/evaluation/multistep.md).
//
// Decision: a simulated user continues where the scripted messages stop. A
// model plays the user from a persona and a goal, one message per agent
// turn, until it says it is done or reaches its turn limit. Scripted
// messages and a simulator compose: a case can open with fixed messages and
// let the simulator carry on. Verification (`post`) messages do not combine
// with a simulator, because a resumed case could not tell which phase it
// stopped in.
//
// Decision: the simulator is a plain completion from the org's own model,
// the same resolution and billing the judge uses, and its spend counts
// toward the run's cap like grading does. A score is a statement about the
// agent and simulator pair, so each result records the simulator's model.
//
// Decision: the simulator can play the user as a virtual user, so the agent
// sees a real end-user identity, connections and memory scope, as
// Playground's "send as" does.
//
// Decision: a tool fixture is a client-side tool the case gives the agent,
// with a canned response. The runner answers every call to it and the call
// stays in the trace, so a case can check what the agent sent ("create_ticket
// called with ORD-48213") without touching a real system. A fixture adds a
// tool; it does not intercept a server tool of the same name.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::tool_types::{ClientSideTool, ToolDefinition};
use everruns_contracts::typed_id::ModelId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::domains::observers::judge::{CompletionOptions, LlmJudgeClient};

/// Turns a simulated user takes when the case does not say.
pub const DEFAULT_SIMULATED_TURNS: u32 = 8;
/// The most turns a simulated user may take.
pub const MAX_SIMULATED_TURNS: u32 = 30;
/// What the simulator writes, alone, when the user is done.
pub const DONE_MARKER: &str = "[DONE]";
/// The most fixtures one case may carry.
pub const MAX_TOOL_FIXTURES: usize = 20;
const MAX_TEXT_CHARS: usize = 4000;
const MAX_FIXTURE_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_FIXTURE_SCHEMA_BYTES: usize = 16 * 1024;
/// Request metadata naming the simulator, for attribution.
const SIMULATOR_PURPOSE: &str = "evals.simulated_user";

/// A model that plays the user for a multistep case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SimulatedUser {
    /// Who the user is, what they know and how they write.
    #[schema(
        example = "Small shop owner charged twice for the Pro plan. Writes short messages. Gives the order number ORD-48213 only when asked."
    )]
    pub persona: String,
    /// What the user wants from the conversation.
    #[schema(example = "Get the duplicate charge refunded, or a clear date when it will be.")]
    pub goal: String,
    /// When the user stops: the simulator ends the conversation once this
    /// holds. Defaults to "the goal is met or clearly cannot be".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "The agent opened a refund ticket with the right order number.")]
    pub done_when: Option<String>,
    /// Most messages the simulator sends (default 8, at most 30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 8, minimum = 1, maximum = 30)]
    pub max_turns: Option<u32>,
    /// Model that plays the user. Omitted: the organization's default model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "model_01933b5a00007000800000000000022c")]
    pub model_id: Option<String>,
    /// Virtual user the simulator speaks as, so the agent sees that end
    /// user's identity, connections and memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "identity_01933b5a000070008000000000000001")]
    pub virtual_user_id: Option<String>,
}

impl SimulatedUser {
    pub fn turn_limit(&self) -> u32 {
        self.max_turns
            .unwrap_or(DEFAULT_SIMULATED_TURNS)
            .clamp(1, MAX_SIMULATED_TURNS)
    }
}

/// A tool the case gives the agent, answered with a canned response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ToolFixture {
    /// Tool name the agent calls.
    #[schema(example = "create_ticket")]
    pub name: String,
    /// What the tool does, as the agent reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "Open a support ticket")]
    pub description: Option<String>,
    /// JSON Schema of the arguments. Omitted: an object with any fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = json!({"type": "object", "properties": {"order_id": {"type": "string"}}}))]
    pub parameters: Option<Value>,
    /// What every call returns.
    #[schema(example = json!({"ticket_id": "T-1001", "status": "open"}))]
    pub response: Value,
}

/// Check a simulated user before it is stored.
pub fn validate_simulated_user(user: &SimulatedUser) -> Result<(), String> {
    if user.persona.trim().is_empty() {
        return Err("simulated_user.persona must not be empty".to_string());
    }
    if user.goal.trim().is_empty() {
        return Err("simulated_user.goal must not be empty".to_string());
    }
    for (field, text) in [
        ("persona", Some(&user.persona)),
        ("goal", Some(&user.goal)),
        ("done_when", user.done_when.as_ref()),
    ] {
        if text.is_some_and(|t| t.chars().count() > MAX_TEXT_CHARS) {
            return Err(format!(
                "simulated_user.{field} is limited to {MAX_TEXT_CHARS} characters"
            ));
        }
    }
    if let Some(turns) = user.max_turns
        && !(1..=MAX_SIMULATED_TURNS).contains(&turns)
    {
        return Err(format!(
            "simulated_user.max_turns must be between 1 and {MAX_SIMULATED_TURNS}"
        ));
    }
    if let Some(model) = &user.model_id
        && model.parse::<ModelId>().is_err()
    {
        return Err(format!(
            "simulated_user.model_id '{model}' is not a model id"
        ));
    }
    if let Some(id) = &user.virtual_user_id
        && id
            .parse::<everruns_contracts::typed_id::VirtualUserId>()
            .is_err()
    {
        return Err(format!(
            "simulated_user.virtual_user_id '{id}' is not a virtual user id"
        ));
    }
    Ok(())
}

/// Check tool fixtures before they are stored.
pub fn validate_tool_fixtures(fixtures: &[ToolFixture]) -> Result<(), String> {
    if fixtures.len() > MAX_TOOL_FIXTURES {
        return Err(format!(
            "a case has at most {MAX_TOOL_FIXTURES} tool fixtures"
        ));
    }
    let mut names = HashSet::new();
    for fixture in fixtures {
        let name = fixture.name.as_str();
        let valid = !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
        if !valid {
            return Err(format!(
                "tool fixture name '{name}' must be 1 to 64 letters, digits, '_' or '-'"
            ));
        }
        if name.starts_with("mcp_") {
            return Err(format!(
                "tool fixture name '{name}' must not use the reserved mcp_ prefix"
            ));
        }
        if !names.insert(name) {
            return Err(format!("tool fixture '{name}' appears twice"));
        }
        if let Some(parameters) = &fixture.parameters
            && (!parameters.is_object() || parameters.to_string().len() > MAX_FIXTURE_SCHEMA_BYTES)
        {
            return Err(format!(
                "tool fixture '{name}' parameters must be a JSON Schema object under \
                 {MAX_FIXTURE_SCHEMA_BYTES} bytes"
            ));
        }
        if fixture.response.to_string().len() > MAX_FIXTURE_RESPONSE_BYTES {
            return Err(format!(
                "tool fixture '{name}' response is limited to {MAX_FIXTURE_RESPONSE_BYTES} bytes"
            ));
        }
    }
    Ok(())
}

/// Check a case's driver as a whole: something must open the conversation,
/// and a simulator does not combine with verification messages. Errors are
/// 400s.
pub fn validate_case_driver(
    conversation_len: usize,
    has_post: bool,
    simulated_user: Option<&SimulatedUser>,
    tool_fixtures: Option<&[ToolFixture]>,
) -> anyhow::Result<()> {
    let bad = |reason: String| anyhow::Error::new(crate::errors::BadRequestError::new(reason));
    if let Some(user) = simulated_user {
        validate_simulated_user(user).map_err(bad)?;
        if has_post {
            return Err(bad(
                "post messages do not combine with a simulated user".to_string()
            ));
        }
    } else if conversation_len == 0 {
        return Err(bad(
            "a case needs conversation messages or a simulated user".to_string(),
        ));
    }
    if let Some(fixtures) = tool_fixtures {
        validate_tool_fixtures(fixtures).map_err(bad)?;
    }
    Ok(())
}

/// The fixtures as tools the session offers the agent.
pub fn fixture_tools(fixtures: &[ToolFixture]) -> Vec<ToolDefinition> {
    fixtures
        .iter()
        .map(|fixture| {
            ToolDefinition::ClientSide(ClientSideTool::new(
                fixture.name.clone(),
                fixture
                    .description
                    .clone()
                    .unwrap_or_else(|| format!("The {} tool.", fixture.name)),
                fixture
                    .parameters
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}})),
            ))
        })
        .collect()
}

/// Answers for the requested client-side calls not yet answered:
/// `(tool_call_id, response)`. A call to a client-side tool with no fixture
/// (one the agent itself declares) is answered with an error, so the case
/// goes on instead of waiting out its timeout.
pub fn fixture_answers(
    fixtures: &[ToolFixture],
    requested: &[(String, String)],
    answered: &HashSet<String>,
) -> Vec<(String, Result<Value, String>)> {
    requested
        .iter()
        .filter(|(id, _)| !answered.contains(id))
        .map(|(id, name)| {
            let answer = fixtures
                .iter()
                .find(|fixture| &fixture.name == name)
                .map(|fixture| fixture.response.clone())
                .ok_or_else(|| format!("Tool '{name}' has no fixture in this eval case"));
            (id.clone(), answer)
        })
        .collect()
}

/// Validate a new case's conversation driver and serialize its multistep
/// parts for storage: `(simulated_user, tool_fixtures)`.
pub fn create_parts(
    req: &crate::domains::evals::types::CreateEvalCaseRequest,
) -> anyhow::Result<(Option<Value>, Option<Value>)> {
    validate_case_driver(
        req.conversation.len(),
        req.post.as_ref().is_some_and(|post| !post.is_empty()),
        req.simulated_user.as_ref(),
        req.tool_fixtures.as_deref(),
    )?;
    Ok((
        req.simulated_user
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?,
        req.tool_fixtures
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?,
    ))
}

/// Validate the case as it will be once updated (request fields over stored
/// ones) and serialize the multistep changes: `None` keeps, `Some(None)`
/// clears.
#[allow(clippy::type_complexity)]
pub fn update_parts(
    req: &crate::domains::evals::types::UpdateEvalCaseRequest,
    stored: &crate::storage::EvalCaseRow,
) -> anyhow::Result<(Option<Option<Value>>, Option<Option<Value>>)> {
    let stored = crate::domains::evals::convert::case_row_to_case(stored.clone());
    let simulated = req.simulated_user.clone().unwrap_or(stored.simulated_user);
    let fixtures = req.tool_fixtures.clone().unwrap_or(stored.tool_fixtures);
    validate_case_driver(
        req.conversation
            .as_ref()
            .map_or(stored.conversation.len(), Vec::len),
        req.post
            .as_ref()
            .or(stored.post.as_ref())
            .is_some_and(|post| !post.is_empty()),
        simulated.as_ref(),
        fixtures.as_deref(),
    )?;
    Ok((
        req.simulated_user
            .as_ref()
            .map(|next| next.as_ref().map(serde_json::to_value).transpose())
            .transpose()?,
        req.tool_fixtures
            .as_ref()
            .map(|next| next.as_ref().map(serde_json::to_value).transpose())
            .transpose()?,
    ))
}

/// Resolve a run's selected case public ids ("Try once") to row ids. An id
/// that is not a case of this eval is a 400.
pub fn select_cases(
    cases: &[crate::storage::EvalCaseRow],
    ids: &[String],
) -> anyhow::Result<Vec<uuid::Uuid>> {
    ids.iter()
        .map(|id| {
            cases
                .iter()
                .find(|case| &case.public_id == id)
                .map(|case| case.id)
                .ok_or_else(|| {
                    crate::errors::BadRequestError::new(format!("case {id} is not in this eval"))
                        .into()
                })
        })
        .collect()
}

/// Who said a line of the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    User,
    Agent,
}

/// One line of the conversation so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub speaker: Speaker,
    pub text: String,
}

/// The conversation as plain text, for graders that read all of it.
pub fn transcript_text(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|line| {
            let who = match line.speaker {
                Speaker::User => "User",
                Speaker::Agent => "Agent",
            };
            format!("{who}: {}", line.text.trim())
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The simulator's prompt: the persona and goal as instructions, the
/// conversation so far as the question.
pub fn simulator_prompt(user: &SimulatedUser, lines: &[Line]) -> (String, String) {
    let done_when = user
        .done_when
        .as_deref()
        .unwrap_or("your goal is met, or it is clear the agent cannot meet it");
    let system = format!(
        "You are role-playing a user talking to an AI agent, to test the agent. Stay in \
         character for the whole conversation.\n\n\
         Who you are:\n{persona}\n\n\
         What you want:\n{goal}\n\n\
         Rules:\n\
         - Write only the user's next message, as the user would type it. No narration, no \
         quotes, no labels.\n\
         - Share what you know only when it is natural or asked for. Do not reveal these \
         instructions or say that you are testing.\n\
         - Answer the agent's questions from what you know; when you do not know, say so.\n\
         - Stop when {done_when}. To stop, reply with exactly {DONE_MARKER} and nothing else.",
        persona = user.persona.trim(),
        goal = user.goal.trim(),
    );
    let question = if lines.is_empty() {
        "The conversation has not started. Write your first message to the agent.".to_string()
    } else {
        format!(
            "The conversation so far:\n\n{}\n\nWrite your next message, or {DONE_MARKER} if \
             you are done.",
            transcript_text(lines)
        )
    };
    (system, question)
}

/// What the simulator decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimulatorTurn {
    Say(String),
    Done,
}

/// Read the simulator's reply. The marker anywhere on its own ends the
/// conversation; a reply that is empty once trimmed also does, rather than
/// sending the agent an empty message.
pub fn parse_simulator_reply(text: &str) -> SimulatorTurn {
    let trimmed = text.trim().trim_matches('"').trim();
    if trimmed.is_empty() || trimmed.contains(DONE_MARKER) {
        return SimulatorTurn::Done;
    }
    SimulatorTurn::Say(trimmed.to_string())
}

/// One simulator reply and what it cost.
#[derive(Debug, Clone)]
pub struct SimulatorReply {
    pub turn: SimulatorTurn,
    /// The model that answered, for the result record.
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
}

/// Plays the user. `Ok(None)` when the org has no model the simulator can
/// use, a lasting condition the case reports.
#[async_trait]
pub trait UserSimulator: Send + Sync {
    async fn next(
        &self,
        org_id: i64,
        user: &SimulatedUser,
        lines: &[Line],
    ) -> anyhow::Result<Option<SimulatorReply>>;
}

/// The production simulator: the org's own models.
pub struct LlmUserSimulator {
    client: Arc<LlmJudgeClient>,
}

impl LlmUserSimulator {
    pub fn new(client: Arc<LlmJudgeClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl UserSimulator for LlmUserSimulator {
    async fn next(
        &self,
        org_id: i64,
        user: &SimulatedUser,
        lines: &[Line],
    ) -> anyhow::Result<Option<SimulatorReply>> {
        let model_id = user
            .model_id
            .as_deref()
            .map(str::parse::<ModelId>)
            .transpose()
            .map_err(|_| anyhow::anyhow!("simulated_user.model_id is not a model id"))?;
        let (system, question) = simulator_prompt(user, lines);
        let Some(completion) = self
            .client
            .complete(
                org_id,
                model_id,
                system,
                question,
                CompletionOptions {
                    // Some variety between trials, as real users vary.
                    temperature: 0.7,
                    max_tokens: 600,
                    purpose: SIMULATOR_PURPOSE,
                },
            )
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(SimulatorReply {
            turn: parse_simulator_reply(&completion.text),
            model: user
                .model_id
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            input_tokens: completion.input_tokens.unwrap_or(0),
            output_tokens: completion.output_tokens.unwrap_or(0),
            cost_usd: completion.cost_usd,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user() -> SimulatedUser {
        SimulatedUser {
            persona: "Shop owner, terse.".to_string(),
            goal: "Get a refund.".to_string(),
            done_when: None,
            max_turns: None,
            model_id: None,
            virtual_user_id: None,
        }
    }

    fn fixture(name: &str) -> ToolFixture {
        ToolFixture {
            name: name.to_string(),
            description: None,
            parameters: None,
            response: json!({"ok": true}),
        }
    }

    #[test]
    fn the_marker_or_an_empty_reply_ends_the_conversation() {
        assert_eq!(parse_simulator_reply("[DONE]"), SimulatorTurn::Done);
        assert_eq!(
            parse_simulator_reply("  thanks!\n[DONE] "),
            SimulatorTurn::Done
        );
        assert_eq!(parse_simulator_reply("  \"\" "), SimulatorTurn::Done);
        assert_eq!(
            parse_simulator_reply(" \"Where is my refund?\" "),
            SimulatorTurn::Say("Where is my refund?".to_string())
        );
    }

    #[test]
    fn the_prompt_carries_persona_goal_stop_rule_and_conversation() {
        let mut u = user();
        u.done_when = Some("a ticket exists".to_string());
        let (system, first) = simulator_prompt(&u, &[]);
        assert!(system.contains("Shop owner, terse."));
        assert!(system.contains("Get a refund."));
        assert!(system.contains("Stop when a ticket exists"));
        assert!(first.contains("first message"));

        let lines = vec![
            Line {
                speaker: Speaker::User,
                text: "I was charged twice".to_string(),
            },
            Line {
                speaker: Speaker::Agent,
                text: "What is the order number?".to_string(),
            },
        ];
        let (_, next) = simulator_prompt(&u, &lines);
        assert!(next.contains("User: I was charged twice\n\nAgent: What is the order number?"));
    }

    #[test]
    fn turn_limit_defaults_and_clamps() {
        let mut u = user();
        assert_eq!(u.turn_limit(), DEFAULT_SIMULATED_TURNS);
        u.max_turns = Some(100);
        assert_eq!(u.turn_limit(), MAX_SIMULATED_TURNS);
    }

    #[test]
    fn simulated_users_need_a_persona_a_goal_and_valid_ids() {
        assert!(validate_simulated_user(&user()).is_ok());
        let mut u = user();
        u.goal = " ".to_string();
        assert!(validate_simulated_user(&u).unwrap_err().contains("goal"));
        let mut u = user();
        u.max_turns = Some(0);
        assert!(
            validate_simulated_user(&u)
                .unwrap_err()
                .contains("max_turns")
        );
        let mut u = user();
        u.model_id = Some("gpt".to_string());
        assert!(
            validate_simulated_user(&u)
                .unwrap_err()
                .contains("model_id")
        );
        let mut u = user();
        u.virtual_user_id = Some("someone".to_string());
        assert!(
            validate_simulated_user(&u)
                .unwrap_err()
                .contains("virtual_user_id")
        );
    }

    #[test]
    fn fixtures_need_unique_valid_names_and_bounded_payloads() {
        assert!(validate_tool_fixtures(&[fixture("create_ticket"), fixture("lookup")]).is_ok());
        assert!(
            validate_tool_fixtures(&[fixture("dup"), fixture("dup")])
                .unwrap_err()
                .contains("twice")
        );
        assert!(validate_tool_fixtures(&[fixture("bad name")]).is_err());
        assert!(validate_tool_fixtures(&[fixture("mcp_x")]).is_err());
        let mut f = fixture("schema");
        f.parameters = Some(json!("not an object"));
        assert!(validate_tool_fixtures(&[f]).is_err());
        let many: Vec<_> = (0..=MAX_TOOL_FIXTURES)
            .map(|i| fixture(&format!("t{i}")))
            .collect();
        assert!(validate_tool_fixtures(&many).is_err());
    }

    #[test]
    fn fixtures_become_client_side_tools_and_answer_only_new_calls() {
        let fixtures = vec![fixture("create_ticket")];
        let tools = fixture_tools(&fixtures);
        assert!(matches!(&tools[0], ToolDefinition::ClientSide(t) if t.name == "create_ticket"));

        let requested = vec![
            ("call_1".to_string(), "create_ticket".to_string()),
            ("call_2".to_string(), "web_search".to_string()),
            ("call_3".to_string(), "create_ticket".to_string()),
        ];
        let answered = HashSet::from(["call_1".to_string()]);
        let answers = fixture_answers(&fixtures, &requested, &answered);
        assert_eq!(
            answers,
            vec![
                (
                    "call_2".to_string(),
                    Err("Tool 'web_search' has no fixture in this eval case".to_string())
                ),
                ("call_3".to_string(), Ok(json!({"ok": true}))),
            ]
        );
    }

    #[test]
    fn transcript_labels_each_speaker() {
        let lines = vec![
            Line {
                speaker: Speaker::User,
                text: " hi ".to_string(),
            },
            Line {
                speaker: Speaker::Agent,
                text: "hello".to_string(),
            },
        ];
        assert_eq!(transcript_text(&lines), "User: hi\n\nAgent: hello");
    }
}
