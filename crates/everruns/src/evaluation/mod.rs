//! Evaluation without Server records. Stability: alpha.
use crate::{Agent, Decisions, Engine, Model, SessionEvent};
/// The shared Server scoring vocabulary and result.
pub use everruns_contracts::runtime::evaluation::{Score, Scorer};
use everruns_contracts::runtime::message::TextAnnotation;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
mod online;
pub use online::{OnlineScoring, ScoreSink};
/// Evidence extracted from one turn, or a complete evaluation conversation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Evidence {
    /// Last user input.
    pub input: String,
    /// Last final assistant answer; commentary is excluded.
    pub answer: String,
    /// Citation annotations on the final answer.
    pub annotations: Vec<TextAnnotation>,
    /// Completed tools in execution order.
    pub tools: Vec<String>,
    /// Arguments aligned with tools; empty when unavailable.
    pub arguments: Vec<String>,
    /// User/assistant transcript for multi-turn grading.
    pub transcript: String,
    /// Logical turns.
    pub turns: u32,
    /// Total input and output tokens.
    pub tokens: u64,
    /// File contents explicitly collected by the application runner.
    pub files: BTreeMap<String, String>,
    #[serde(skip)]
    pending: BTreeMap<String, String>,
    #[serde(skip)]
    final_message: Option<String>,
    #[serde(skip)]
    generations: u64,
    #[serde(skip)]
    incomplete: bool,
}
impl Evidence {
    /// Extract from canonical events in order. The caller scopes events to the intended conversation.
    pub fn from_events<'a>(events: impl IntoIterator<Item = &'a SessionEvent>) -> Self {
        let mut evidence = Self::default();
        for event in events {
            evidence.apply(event)
        }
        evidence
    }
    pub(super) fn apply(&mut self, event: &SessionEvent) {
        let data = &event.canonical_json()["data"];
        match event.event_type() {
            "input.message" => {
                if let Some(text) = data["message"]["content"].as_array().map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| p["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                }) {
                    self.input = text;
                    self.transcript.push_str(&format!("User: {}\n", self.input));
                }
            }
            "turn.started" => {
                self.turns = self.turns.saturating_add(1);
                if let Some(text) = data["input_content"].as_str() {
                    self.input = text.into();
                }
            }
            "llm.generation" => {
                self.generations = self.generations.saturating_add(1);
            }
            "output.message.completed" => {
                if let Some(text) =
                    everruns_core::conversation::said_in_event(event.event_type(), data)
                {
                    self.answer = text;
                    self.final_message = data["message"]["id"].as_str().map(str::to_string);
                    self.annotations = data["message"]["content"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|part| {
                            part.get("annotations").and_then(|a| {
                                serde_json::from_value::<Vec<TextAnnotation>>(a.clone()).ok()
                            })
                        })
                        .flatten()
                        .collect();
                    self.transcript
                        .push_str(&format!("Agent: {}\n", self.answer));
                }
            }
            "tool.started" => {
                let call = &data["tool_call"];
                if let Some(id) = call["id"].as_str() {
                    self.pending
                        .insert(id.into(), call["arguments"].to_string());
                }
            }
            "tool.completed" => {
                if let Some(name) = data["tool_name"].as_str() {
                    self.tools.push(name.into());
                    let arguments = data["tool_call_id"]
                        .as_str()
                        .and_then(|id| self.pending.remove(id));
                    if arguments.is_none() {
                        self.incomplete = true;
                    }
                    self.arguments.push(arguments.unwrap_or_default());
                }
            }
            "turn.completed" => {
                self.tokens = self
                    .tokens
                    .saturating_add(data["usage"]["input_tokens"].as_u64().unwrap_or(0))
                    .saturating_add(data["usage"]["output_tokens"].as_u64().unwrap_or(0));
            }
            _ => {}
        }
    }
    pub(super) fn complete(&self, event: &SessionEvent) -> bool {
        let data = &event.canonical_json()["data"];
        !self.incomplete
            && self.pending.is_empty()
            && data["tool_call_count"].as_u64() == Some(self.tools.len() as u64)
            && data["llm_call_count"].as_u64() == Some(self.generations)
            && data["final_message_id"].as_str() == self.final_message.as_deref()
    }
    fn input(&self) -> everruns_core::evaluation::GradeInput<'_> {
        everruns_core::evaluation::GradeInput {
            input_message: &self.input,
            final_answer: &self.answer,
            annotations: &self.annotations,
            tool_calls: &self.tools,
            tool_arguments: &self.arguments,
            transcript: &self.transcript,
            turns: self.turns,
            tokens: self.tokens,
            session_id: None,
        }
    }
    pub(super) fn retained_bytes(&self) -> usize {
        self.input.len()
            + self.answer.len()
            + self.transcript.len()
            + self.arguments.iter().map(String::len).sum::<usize>()
            + self.pending.values().map(String::len).sum::<usize>()
            + self.tools.iter().map(String::len).sum::<usize>()
            + self
                .files
                .iter()
                .map(|(path, content)| path.len() + content.len())
                .sum::<usize>()
            + self
                .annotations
                .iter()
                .map(|a| serde_json::to_vec(a).map_or(0, |v| v.len()))
                .sum::<usize>()
    }
}
/// A grader that could not produce a valid score.
#[derive(Debug, thiserror::Error)]
#[error("evaluation: {0}")]
pub struct Error(pub String);
/// Application-owned model or domain grader for rules that need external evidence.
#[async_trait::async_trait]
pub trait Grader: Send + Sync {
    /// Score one validated rule; return an error when unavailable.
    async fn grade(&self, scorer: &Scorer, evidence: &Evidence) -> Result<Score, Error>;
}
/// Shared deterministic scorers, plus explicitly selected judge/decision models.
#[derive(Clone, Default)]
pub struct Evaluator {
    scorers: Vec<Scorer>,
    judge: Option<Model>,
    decisions: Option<Decisions>,
    custom: Option<Arc<dyn Grader>>,
}
impl Evaluator {
    /// Validate scoring rules before any model call.
    pub fn new(scorers: Vec<Scorer>) -> Result<Self, Error> {
        if scorers.len() > 100 {
            return Err(Error("At most 100 scorers are supported".into()));
        }
        for scorer in &scorers {
            scorer.validate().map_err(Error)?
        }
        Ok(Self {
            scorers,
            ..Self::default()
        })
    }
    /// Supply the exact application-owned chat model used for judge rules.
    pub fn judge(mut self, model: Model) -> Self {
        self.judge = Some(model);
        self
    }
    /// Supply the exact application-owned decision model.
    pub fn decisions(mut self, decisions: Decisions) -> Self {
        self.decisions = Some(decisions);
        self
    }
    /// Override model/file-dependent grading with an application implementation.
    pub fn grader(mut self, grader: impl Grader + 'static) -> Self {
        self.custom = Some(Arc::new(grader));
        self
    }
    /// Grade evidence in scorer order. Unavailable/provider failures are errors, never fabricated scores.
    pub async fn score(&self, evidence: &Evidence) -> Result<Vec<Score>, Error> {
        if evidence.retained_bytes() > 8 * 1024 * 1024 {
            return Err(Error("Evaluation evidence exceeds retention limit".into()));
        }
        let mut out = vec![];
        for scorer in &self.scorers {
            if let Some(score) = everruns_core::evaluation::score_rule(scorer, &evidence.input()) {
                out.push(score);
                continue;
            }
            let score = if let Some(custom) = &self.custom {
                custom.grade(scorer, evidence).await?
            } else {
                match scorer {
                    Scorer::CitationFaithful {
                        min_citations,
                        pass_threshold,
                        ..
                    } => everruns_core::evaluation::score_citation_faithful(
                        &evidence.annotations,
                        *min_citations,
                        *pass_threshold,
                    ),
                    Scorer::FileContains { path, text, .. } => {
                        let file = evidence
                            .files
                            .get(path)
                            .ok_or_else(|| Error(format!("File evidence unavailable: {path}")))?;
                        everruns_core::evaluation::score_file_contains(path, text, Some(file))
                    }
                    Scorer::Judge {
                        rubric,
                        model_id,
                        pass_threshold,
                        ..
                    } => {
                        if model_id.is_some() {
                            return Err(Error("Hosted model ids require an application Grader; select a Model with Evaluator::judge".into()));
                        }
                        self.judge_score(rubric, *pass_threshold, evidence).await?
                    }
                    Scorer::CitationJudged {
                        rubric,
                        model_id,
                        pass_threshold,
                        ..
                    } => {
                        if model_id.is_some() {
                            return Err(Error(
                                "Hosted model ids require an application Grader".into(),
                            ));
                        }
                        if evidence.annotations.is_empty() {
                            out.push(Score {
                                pass: false,
                                value: 0.0,
                                reason: "No citations to judge".into(),
                            });
                            continue;
                        }
                        let citation_evidence = Evidence {
                            answer: everruns_core::evaluation::build_citation_pairs(
                                &evidence.answer,
                                &evidence.annotations,
                            ),
                            ..evidence.clone()
                        };
                        self.judge_score(
                            rubric
                                .as_deref()
                                .unwrap_or(everruns_core::evaluation::CITATION_JUDGE_RUBRIC),
                            *pass_threshold,
                            &citation_evidence,
                        )
                        .await?
                    }
                    Scorer::Decision {
                        question,
                        yes,
                        no,
                        pass_threshold,
                        ..
                    } => {
                        let decisions = self
                            .decisions
                            .as_ref()
                            .ok_or_else(|| Error("No decision model configured".into()))?;
                        let question = format!(
                            "{question}\nYes: {}\nNo: {}",
                            yes.as_deref().unwrap_or("The condition holds"),
                            no.as_deref().unwrap_or("The condition does not hold")
                        );
                        let value=decisions.probability(question,serde_json::json!({"input":evidence.input,"answer":evidence.answer,"tools":evidence.tools,"transcript":evidence.transcript})).await.map_err(|_|Error("Decision grader failed".into()))?;
                        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                            return Err(Error("Invalid decision probability".into()));
                        }
                        Score {
                            pass: value >= *pass_threshold,
                            value,
                            reason: format!("Decision probability {value:.3}"),
                        }
                    }
                    _ => return Err(Error("Unsupported scorer".into())),
                }
            };
            if !score.value.is_finite() || !(0.0..=1.0).contains(&score.value) {
                return Err(Error("Grader returned an invalid score".into()));
            }
            out.push(score);
        }
        Ok(out)
    }
    async fn judge_score(
        &self,
        rubric: &str,
        threshold: f64,
        evidence: &Evidence,
    ) -> Result<Score, Error> {
        use everruns_core::evaluation_judge::{
            TurnEvidence, build_judge_messages, parse_judge_output,
        };
        let model = self
            .judge
            .as_ref()
            .ok_or_else(|| Error("No judge model configured".into()))?;
        let (system, user) = build_judge_messages(
            rubric,
            &TurnEvidence {
                input_message: evidence.input.clone(),
                final_answer: evidence.answer.clone(),
                tool_names: evidence.tools.clone(),
            },
        );
        let answer = model
            .completion()
            .system(system)
            .user(user)
            .max_tokens(5000)
            .timeout(Duration::from_secs(60))
            .text()
            .await
            .map_err(|_| Error("Judge model failed".into()))?;
        let grade = parse_judge_output(&answer)
            .map_err(|_| Error("Judge returned an invalid grade".into()))?;
        Ok(Score {
            pass: grade.value >= threshold,
            value: grade.value,
            reason: grade.reasoning,
        })
    }
    /// Run independent real sessions, bounded by the supplied concurrency (1..=32).
    /// Every case retains its session id, evidence, scores and error for inspection.
    pub async fn run(
        &self,
        engine: &Engine,
        agent: Agent,
        cases: Vec<Case>,
        concurrency: usize,
    ) -> Result<Vec<CaseResult>, Error> {
        if engine.has_session_hosting() {
            let build = crate::hosting::AgentBuild {
                agent: "framework-evaluation".into(),
                build_id: uuid::Uuid::new_v4().to_string(),
            };
            engine
                .register_agent(build.clone(), agent)
                .map_err(|_| Error("Could not register evaluation agent".into()))?;
            self.run_target(engine, RunTarget::Registered(build), cases, concurrency)
                .await
        } else {
            self.run_target(
                engine,
                RunTarget::Agent(Box::new(agent)),
                cases,
                concurrency,
            )
            .await
        }
    }
    /// Run a retained application build on any coherent Engine profile.
    /// Re-register the same build after restart to inspect/resume retained sessions.
    pub async fn run_registered(
        &self,
        engine: &Engine,
        build: crate::hosting::AgentBuild,
        cases: Vec<Case>,
        concurrency: usize,
    ) -> Result<Vec<CaseResult>, Error> {
        if !engine.has_agent_build(&build) {
            return Err(Error("Evaluation agent build is not registered".into()));
        }
        self.run_target(engine, RunTarget::Registered(build), cases, concurrency)
            .await
    }
    async fn run_target(
        &self,
        engine: &Engine,
        target: RunTarget,
        cases: Vec<Case>,
        concurrency: usize,
    ) -> Result<Vec<CaseResult>, Error> {
        use futures::{StreamExt, stream};
        if !(1..=32).contains(&concurrency) || cases.len() > 1000 {
            return Err(Error("Invalid concurrency or case count".into()));
        }
        if cases.iter().any(|case| {
            case.name.trim().is_empty()
                || case.name.len() > 200
                || case.inputs.is_empty()
                || case.inputs.len() > 100
                || case.inputs.iter().any(|i| i.len() > 64 * 1024)
                || case.timeout.is_zero()
                || case.timeout > Duration::from_secs(3600)
        }) {
            return Err(Error(
                "Invalid case name, input count/size or timeout".into(),
            ));
        }
        let mut results = stream::iter(cases.into_iter().enumerate())
            .map(|(index, case)| {
                let target = target.clone();
                async move {
                    let id = crate::SessionId::new();
                    let build = match &target {
                        RunTarget::Registered(build) => Some(build.clone()),
                        RunTarget::Agent(_) => None,
                    };
                    let session = match target {
                        RunTarget::Agent(agent) => Ok(engine.create(*agent)),
                        RunTarget::Registered(build) => engine.create_registered(id, &build).await,
                    };
                    let session = match session {
                        Ok(session) => session,
                        Err(_) => {
                            return (
                                index,
                                CaseResult {
                                    name: case.name,
                                    session_id: id.to_string(),
                                    build,
                                    evidence: Evidence::default(),
                                    scores: vec![],
                                    error: Some("Could not create evaluation session".into()),
                                },
                            );
                        }
                    };
                    let mut result = CaseResult {
                        name: case.name,
                        session_id: session.id(),
                        build,
                        evidence: Evidence::default(),
                        scores: vec![],
                        error: None,
                    };
                    for input in case.inputs {
                        match session
                            .run_with(input, crate::RunOptions::new().timeout(case.timeout))
                            .await
                        {
                            Ok(turn) if turn.success => {}
                            Ok(turn) => {
                                result.error =
                                    turn.error.or_else(|| Some("Agent turn failed".into()));
                                break;
                            }
                            Err(_) => {
                                result.error = Some("Session execution failed".into());
                                break;
                            }
                        }
                    }
                    let mut cursor = None;
                    loop {
                        match session.event_page(cursor, 32).await {
                            Ok(page) => {
                                for event in &page.events {
                                    result.evidence.apply(event);
                                    if result.evidence.retained_bytes() > 8 * 1024 * 1024 {
                                        result.error = Some(
                                            "Evaluation evidence exceeds retention limit".into(),
                                        );
                                        result.evidence = Evidence::default();
                                        break;
                                    }
                                }
                                if result.error.as_deref()
                                    == Some("Evaluation evidence exceeds retention limit")
                                {
                                    break;
                                }
                                match page.next_cursor {
                                    Some(next) => cursor = Some(next),
                                    None => break,
                                }
                            }
                            Err(_) => {
                                result.error = Some("Evidence unavailable".into());
                                break;
                            }
                        }
                    }
                    if let Some(head) = session.workspace_head() {
                        for scorer in &self.scorers {
                            if let Scorer::FileContains { path, .. } = scorer
                                && let Ok(Some(file)) = head
                                    .file_system()
                                    .read_file(session.session_id(), path)
                                    .await
                                && file.encoding == "text"
                                && let Some(content) = file
                                    .content
                                    .filter(|content| content.len() <= 8 * 1024 * 1024)
                            {
                                result.evidence.files.insert(path.clone(), content);
                                if result.evidence.retained_bytes() > 8 * 1024 * 1024 {
                                    result.error =
                                        Some("Evaluation evidence exceeds retention limit".into());
                                    result.evidence = Evidence::default();
                                    break;
                                }
                            }
                        }
                    }
                    if result.error.is_none() {
                        match self.score(&result.evidence).await {
                            Ok(scores) => result.scores = scores,
                            Err(error) => result.error = Some(error.to_string()),
                        }
                    }
                    (index, result)
                }
            })
            .buffer_unordered(concurrency)
            .collect::<Vec<_>>()
            .await;
        results.sort_by_key(|(index, _)| *index);
        Ok(results.into_iter().map(|(_, result)| result).collect())
    }
}
/// An evaluation conversation. Each case receives a fresh session.
#[derive(Debug, Clone)]
pub struct Case {
    /// Application case name.
    pub name: String,
    /// User inputs, in order.
    pub inputs: Vec<String>,
    /// Deadline per input. Timed-out turns are cooperatively cancelled.
    pub timeout: Duration,
}
/// Evidence and scores from one case, with a retained session for debugging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseResult {
    /// Exact behavior registration for retained sessions; absent on legacy hosting.
    pub build: Option<crate::hosting::AgentBuild>,
    /// Case name.
    pub name: String,
    /// Retained Engine session.
    pub session_id: String,
    /// Extracted evidence.
    pub evidence: Evidence,
    /// Scores in evaluator order.
    pub scores: Vec<Score>,
    /// Execution or scoring failure, when any.
    pub error: Option<String>,
}

#[derive(Clone)]
enum RunTarget {
    Agent(Box<Agent>),
    Registered(crate::hosting::AgentBuild),
}
